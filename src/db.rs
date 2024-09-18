use std::path::Path;

use chrono::{DateTime, Utc};
use eyre::eyre;
use redb::{
    Database, DatabaseError, Key, ReadableTable, TableDefinition, TableError, TypeName, Value,
    WriteTransaction,
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::models::{Service, Services};
use crate::social_network::Posted;

const HASH_LEN: usize = 32;

// service name -> configuration/tokens MessagePack
//
// This table is used to store the auth tokens etc, for a service.
const SERVICE_TABLE: TableDefinition<u8, &[u8]> = TableDefinition::new("services");

// // item guid -> timestamp
// //
// // This table is used to quickly check if a feed entry has been posted before.
// const TOOTED_TABLE: TableDefinition<&str, i64> = TableDefinition::new("tooted");

// // hash of toot -> timestamp
// //
// // This table is a last defence against posting a duplicate post. The key is a hash of
// // the toot content, which if it exists suggests duplicate content.
// const TOOT_TABLE: TableDefinition<[u8; HASH_LEN], i64> = TableDefinition::new("toots");

// (service, item guid) -> timestamp
const POSTED_ITEMS_TABLE: TableDefinition<(Service, &str), i64> =
    TableDefinition::new("posted_items");

// (service, post hash) -> timestamp
//
// This table is a last defence against posting a duplicate post. The key is a hash of
// the post content, which if it exists suggests duplicate content.
const POSTS_TABLE: TableDefinition<(Service, [u8; HASH_LEN]), i64> = TableDefinition::new("posts");

// feed url -> Feed MessagePack
const FEED_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("feeds");

#[derive(Serialize, Deserialize)]
pub struct Feed {
    pub url: Url,
    /// `true` if this feed has had its initial sync
    #[serde(default)]
    pub had_initial_sync: bool,
    pub etag: Option<String>,
    pub last_modified: Option<DateTime<Utc>>,
    pub last_refresh_hash: Option<[u8; HASH_LEN]>,
}

pub struct ServiceData {
    pub service: Service,
    pub data: Vec<u8>,
}

pub fn establish_connection<P: AsRef<Path>>(database_path: P) -> Result<Database, DatabaseError> {
    Database::create(database_path)
}

pub fn load_feed(db: &Database, feed_url: &Url) -> Result<Feed, redb::Error> {
    let read_txn = db.begin_read()?;
    let table = match read_txn.open_table(FEED_TABLE) {
        Ok(table) => table,
        Err(TableError::TableDoesNotExist(_)) => {
            return Ok(Feed {
                url: feed_url.clone(),
                had_initial_sync: false,
                etag: None,
                last_modified: None,
                last_refresh_hash: None,
            })
        }
        Err(e) => return Err(e.into()),
    };

    let access = table.get(feed_url.as_str())?;
    let Some(data) = access.as_ref().map(|guard| guard.value()) else {
        return Ok(Feed {
            url: feed_url.clone(),
            had_initial_sync: false,
            etag: None,
            last_modified: None,
            last_refresh_hash: None,
        });
    };

    let feed = rmp_serde::from_slice::<Feed>(data).expect("FIXME: unable to deserialize feed");

    Ok(feed)
}

pub fn load_services(db: &Database, services: &Services) -> eyre::Result<Vec<ServiceData>> {
    let read_txn = db.begin_read()?;
    let table = match read_txn.open_table(SERVICE_TABLE) {
        Ok(table) => table,
        Err(TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };

    let results = match services {
        Services::All => table
            .iter()?
            .map(|item| {
                let (k, v) = item?;
                let service = Service::try_from(k.value())?;
                Ok(ServiceData {
                    service,
                    data: v.value().to_vec(),
                })
            })
            .collect::<Result<Vec<_>, eyre::Report>>()?,
        Services::Specific(specified) => specified
            .iter()
            .copied()
            .map(|service| {
                let item = table
                    .get(service as u8)?
                    .ok_or_else(|| eyre!("{} is not configured", service))?;
                Ok(ServiceData {
                    service,
                    data: item.value().to_vec(),
                })
            })
            .collect::<Result<Vec<_>, eyre::Report>>()?,
    };

    Ok(results)
}

pub fn save_service<D>(db: &Database, service: Service, data: &D) -> Result<(), redb::Error>
where
    D: Serialize,
{
    let tx = db.begin_write()?;
    {
        let mut table = tx.open_table(SERVICE_TABLE)?;
        let serialised = rmp_serde::to_vec(data).expect("FIXME: unable to serialise service");
        table.insert(service as u8, serialised.as_slice())?;
    }
    tx.commit()?;
    Ok(())
}

pub fn save_feed(tx: &WriteTransaction, feed: &Feed) -> Result<(), redb::Error> {
    // let write_txn = db.begin_write()?;
    // {
    let mut table = tx.open_table(FEED_TABLE)?;
    let serialised = rmp_serde::to_vec(feed).expect("FIXME: unable to serialise feed");
    table.insert(feed.url.as_str(), serialised.as_slice())?;
    // }
    // write_txn.commit()?;
    Ok(())
}

pub enum AlreadyPosted {
    Yes,
    No(blake3::Hash),
}

/// Checks if the supplied content has been tooted before.
///
/// Returns `true` if tooted before.
pub fn already_posted(
    db: &Database,
    service: Service,
    content: &str,
) -> Result<AlreadyPosted, redb::Error> {
    let hash = blake3::hash(content.as_bytes());

    let read_txn = db.begin_read()?;
    let table = match read_txn.open_table(POSTS_TABLE) {
        Ok(table) => table,
        Err(TableError::TableDoesNotExist(_)) => return Ok(AlreadyPosted::No(hash)),
        Err(e) => return Err(e.into()),
    };

    table
        .get((service, *hash.as_bytes()))
        .map(|access| {
            if access.is_some() {
                AlreadyPosted::Yes
            } else {
                AlreadyPosted::No(hash)
            }
        })
        .map_err(redb::Error::from)
}

/// Checks if the supplied feed item guid has been tooted before.
///
/// Returns `true` if tooted before.
pub fn item_posted(db: &Database, service: Service, guid: &str) -> Result<bool, redb::Error> {
    let read_txn = db.begin_read()?;
    let table = match read_txn.open_table(POSTED_ITEMS_TABLE) {
        Ok(table) => table,
        Err(TableError::TableDoesNotExist(_)) => return Ok(false),
        Err(e) => return Err(e.into()),
    };

    table
        .get((service, guid))
        .map(|access| access.is_some())
        .map_err(redb::Error::from)
}

pub fn mark_post_tooted(
    tx: &WriteTransaction,
    service: Service,
    toot: Posted,
) -> Result<(), redb::Error> {
    // let write_txn = db.begin_write()?;
    // {
    let mut tooted_table = tx.open_table(POSTED_ITEMS_TABLE)?;
    let mut toots_table = tx.open_table(POSTS_TABLE)?;
    tooted_table.insert((service, toot.guid.as_str()), toot.at.timestamp())?;
    toots_table.insert((service, *toot.hash.as_bytes()), toot.at.timestamp())?;
    // }
    // write_txn.commit()?;
    Ok(())
}

// Take an iterator that yields guids
pub fn mark_items_seen<I>(
    db: &Database,
    service: Service,
    at: DateTime<Utc>,
    items: I,
) -> Result<(), redb::Error>
where
    I: Iterator<Item = String>,
{
    let tx = db.begin_write()?;
    {
        let mut tooted_table = tx.open_table(POSTED_ITEMS_TABLE)?;
        for guid in items {
            tooted_table.insert((service, guid.as_str()), at.timestamp())?;
        }
    }
    tx.commit()?;
    Ok(())
}

impl Value for Service {
    type SelfType<'a> = Service;

    type AsBytes<'a> = [u8; 1];

    fn fixed_width() -> Option<usize> {
        Some(1)
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        let val = u8::from_be_bytes(data.try_into().unwrap());
        Service::try_from(val).expect("invalid Service value")
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        (*value as u8).to_le_bytes()
    }

    fn type_name() -> redb::TypeName {
        TypeName::new("feed_parrot::Service")
    }
}

impl Key for Service {
    fn compare(data1: &[u8], data2: &[u8]) -> std::cmp::Ordering {
        Self::from_bytes(data1).cmp(&Self::from_bytes(data2))
    }
}

#[cfg(debug_assertions)]
pub mod debug {
    #![allow(dead_code)]

    //! Database debugging functions.

    use super::*;

    pub fn delete_feeds(db: &Database) -> eyre::Result<()> {
        let tx = db.begin_write()?;
        tx.delete_table(FEED_TABLE)?;
        tx.commit()?;
        Ok(())
    }

    pub fn mark_as_new(db: &Database, service: Service, guid: &str) -> eyre::Result<()> {
        let tx = db.begin_write()?;
        {
            let mut tooted_table = tx.open_table(POSTED_ITEMS_TABLE)?;
            tooted_table.remove((service, guid))?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn dump_seen_items(db: &Database) -> eyre::Result<()> {
        let tx = db.begin_read()?;
        let seen_items = tx.open_table(POSTED_ITEMS_TABLE)?;
        for item in seen_items.iter()? {
            let (k, v) = item?;
            let (service, guid) = k.value();
            let seen = v.value();
            println!("{}: {} -> {}", service, guid, seen);
        }
        Ok(())
    }
}
