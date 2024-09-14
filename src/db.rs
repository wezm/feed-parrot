use std::path::Path;

use redb::{Database, DatabaseError, TableDefinition};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use url::Url;

const HASH_LEN: usize = 32;

// item guid -> timestamp
//
// This table is used to quickly check if a feed entry has been posted before.
const TOOTED_TABLE: TableDefinition<&str, i64> = TableDefinition::new("tooted");

// hash of toot -> timestamp
//
// This table is a last defence against posting a duplicate post. The key is a hash of
// the toot content, which if it exists suggests duplicate content.
const TOOTS_TABLE: TableDefinition<[u8; HASH_LEN], i64> = TableDefinition::new("toots");

// feed url -> Feed MessagePack
const FEED_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("feed");

#[derive(Serialize, Deserialize)]
pub struct Feed {
    // pub title: String,
    pub url: Url,
    pub etag: Option<String>,
    pub last_modified: Option<OffsetDateTime>,
    pub last_refresh_hash: Option<Vec<u8>>,
}

pub struct Tooted {
    pub guid: String,
    pub status: String,
    pub at: OffsetDateTime,
}

pub fn establish_connection<P: AsRef<Path>>(database_path: P) -> Result<Database, DatabaseError> {
    Database::create(database_path)
}

pub fn load_feed(db: &Database, feed_url: &Url) -> Result<Feed, redb::Error> {
    let read_txn = db.begin_read()?;
    let table = read_txn.open_table(FEED_TABLE)?;

    let access = table.get(feed_url.as_str())?;
    let Some(data) = access.as_ref().map(|guard| guard.value()) else {
        return Ok(Feed {
            url: feed_url.clone(),
            etag: None,
            last_modified: None,
            last_refresh_hash: None,
        });
    };

    let feed = rmp_serde::from_slice::<Feed>(data).expect("FIXME: unable to deserialize feed");

    Ok(feed)
}

pub fn save_feed(db: &Database, feed: &Feed) -> Result<(), redb::Error> {
    let write_txn = db.begin_write()?;
    {
        let mut table = write_txn.open_table(FEED_TABLE)?;
        let serialised = rmp_serde::to_vec(feed).expect("FIXME: unable to serialise feed");
        table.insert(feed.url.as_str(), serialised.as_slice())?;
    }
    write_txn.commit()?;
    Ok(())
}

/// Checks if the supplied content has been tooted before.
///
/// Returns `true` if tooted before.
pub fn already_tooted(db: &Database, content: &str) -> Result<bool, redb::Error> {
    let hash = blake3::hash(content.as_bytes());

    let read_txn = db.begin_read()?;
    let table = read_txn.open_table(TOOTS_TABLE)?;
    table
        .get(hash.as_bytes())
        .map(|access| access.is_some())
        .map_err(redb::Error::from)
}

/// Checks if the supplied feed item guid has been tooted before.
///
/// Returns `true` if tooted before.
pub fn item_tooted(db: &Database, guid: &str) -> Result<bool, redb::Error> {
    let read_txn = db.begin_read()?;
    let table = read_txn.open_table(TOOTED_TABLE)?;
    table
        .get(guid)
        .map(|access| access.is_some())
        .map_err(redb::Error::from)
}

pub fn mark_post_tooted(db: &Database, toot: Tooted) -> Result<(), redb::Error> {
    let write_txn = db.begin_write()?;
    {
        let mut tooted_table = write_txn.open_table(TOOTED_TABLE)?;
        let mut toots_table = write_txn.open_table(TOOTS_TABLE)?;
        let hash = blake3::hash(toot.status.as_bytes());
        tooted_table.insert(toot.guid.as_str(), toot.at.unix_timestamp())?;
        toots_table.insert(hash.as_bytes(), toot.at.unix_timestamp())?;
    }
    write_txn.commit()?;
    Ok(())
}
