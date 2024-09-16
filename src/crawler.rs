use std::borrow::Cow;
use std::fmt::{self, Formatter};
use std::io::Cursor;

use atom_syndication as atom;
use blake3::Hash;
use chrono::{DateTime, Utc};
// use lockable::LockPool;
use mime::Mime;
use reqwest::header::{
    HeaderMap, HeaderValue, CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED,
};
use reqwest::StatusCode;
use rss::Channel;

use crate::db::Feed;
use crate::feed::ParsedFeed;

const MAX_RESPONSE_SIZE: u64 = 5 * 1024 * 1024; // 5 MiB

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum ConditionalRequest {
    Enabled,
    Disabled,
}

#[derive(Debug)]
pub enum CrawlError {
    Database(redb::Error),
    Fetch(FetchError),
    Process(ProcessError),
}

#[derive(Debug)]
pub enum FetchError {
    Database(redb::Error),
    Request(reqwest::Error),
    /// Feed response body exceeded the limit
    ResponseTooBig,
    ResponseUnsuccessful(StatusCode),
    UnknownEncoding,
}

#[derive(Debug)]
enum ProcessError {
    Rss(rss::Error),
    Atom(atom::Error),
    Database(redb::Error),
    // Request(reqwest::Error),
    /// Feed response body exceeded the limit
    // ResponseTooBig,
    // ResponseUnsuccessful(StatusCode),
    // The feed is in an unknown format
    Json(serde_json::Error),
    UnknownFormat,
    // Database(sqlx::Error),
}

// pub struct Feed {
//     pub title: String,
//     pub url: String,
//     pub etag: Option<String>,
//     pub last_modified: Option<OffsetDateTime>,
//     pub last_refresh_hash: Option<Vec<u8>>,
//     pub source_slug: String,
//     pub next_refresh_at: Option<OffsetDateTime>,
//     pub refreshed_at: Option<OffsetDateTime>,
//     pub created_at: OffsetDateTime,
//     pub updated_at: OffsetDateTime,
// }

pub enum FeedData<T> {
    NotModified,
    Updated(T),
}

struct HashedData {
    data: Vec<u8>,
    hash: Hash,
}

impl HashedData {
    fn new(data: Vec<u8>) -> Self {
        let hash = blake3::hash(&data);
        HashedData { data, hash }
    }
}

struct CrawlOutcome {
    refreshed_at: DateTime<Utc>,
    data: FeedData<HashedData>,
    headers: HeaderMap,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum SyncType {
    Initial,
    Incremental,
}

pub async fn refresh_feed(
    client: reqwest::Client,
    sync_type: SyncType,
    cond_req: ConditionalRequest,
    feed: &mut Feed,
) -> Result<FeedData<ParsedFeed>, CrawlError> {
    info!("begin");
    let outcome = fetch_feed(&client, &feed, cond_req).await?;
    let feed_data = match outcome.data {
        FeedData::NotModified => {
            info!("not modified");
            // FIXME: Handle updated headers here
            FeedData::NotModified
        }
        FeedData::Updated(HashedData { data, hash }) => {
            info!("updated");
            let content_type = outcome
                .headers
                .get(CONTENT_TYPE)
                .and_then(|val| val.to_str().ok())
                .and_then(|val| val.parse().ok());
            let data = to_utf8(data, &content_type)?;

            // conn.transaction::<'_, _, _, ProcessError>(|conn| {
            // let mut tx = conn.begin_write()?;
            let parsed_feed = {
                // Box::pin(async move {
                // Update the etag, last_modified, and content type on the feed. This is done inside the
                // transaction so that if processing the feed fails we will fetch the feed
                // with the old cache headers again next time. If new headers were used we
                // would potentially get a 304 response and not process any of the items that
                // failed to import initially.
                update_feed_cache_keys(feed, &outcome.headers, Some(hash));
                parse_feed(&data, &content_type)?
                // })
                // })
                // .await?
            };
            // tx.commit()?;
            FeedData::Updated(parsed_feed)
        }
    };

    Ok(feed_data)
}

async fn fetch_feed(
    client: &reqwest::Client,
    feed: &Feed,
    cond_req: ConditionalRequest,
) -> Result<CrawlOutcome, FetchError> {
    // info!(url = feed.url);
    // Build a request to fetch the feed, setting condititional request headers as appropriate
    let mut headers = HeaderMap::new();
    if cond_req == ConditionalRequest::Enabled {
        let last_modified = feed
            .last_modified
            .map(|d| d.to_rfc2822())
            .and_then(|val| val.parse::<HeaderValue>().ok());
        if let Some(last_modified) = last_modified {
            headers.insert(IF_MODIFIED_SINCE, last_modified);
        }
        if let Some(etag) = feed.etag.as_ref().and_then(|val| val.parse().ok()) {
            headers.insert(IF_NONE_MATCH, etag);
        }
    }
    let mut response = client
        .get(feed.url.as_str())
        .headers(headers)
        .send()
        .await?;
    let refreshed_at = Utc::now();

    // Check if not modified
    // info!(status = response.status().as_u16());
    if response.status() == StatusCode::NOT_MODIFIED {
        return Ok(CrawlOutcome {
            data: FeedData::NotModified,
            headers: response.headers().to_owned(),
            refreshed_at,
        });
    } else if !response.status().is_success() {
        return Err(FetchError::ResponseUnsuccessful(response.status()));
    }

    // TODO: Handle redirects

    // Check if content length suggests it will be too big
    let mut body = if let Some(content_length) = response.content_length() {
        if content_length > MAX_RESPONSE_SIZE {
            return Err(FetchError::ResponseTooBig);
        }
        // NOTE(cast): Safe as we have verified content_length is smaller than MAX_RESPONSE_SIZE
        Vec::with_capacity(content_length as usize)
    } else {
        Vec::new()
    };

    // Read the response body
    while let Some(chunk) = response.chunk().await? {
        if body
            .len()
            .checked_add(chunk.len())
            .map_or(true, |len| len > MAX_RESPONSE_SIZE as usize)
        {
            return Err(FetchError::ResponseTooBig);
        }
        body.extend_from_slice(&chunk);
    }

    // Calculate a hash of the body
    let hashed_data = HashedData::new(body);

    // Compare it to the existing one if present
    if cond_req == ConditionalRequest::Enabled {
        let prev_hash = feed.last_refresh_hash.map(Hash::from_bytes);
        if matches!(prev_hash, Some(prev_hash) if hashed_data.hash == prev_hash) {
            // info!(%hash, "hash matches");
            // Not modified
            return Ok(CrawlOutcome {
                data: FeedData::NotModified,
                headers: response.headers().to_owned(),
                refreshed_at,
            });
        }
    }

    Ok(CrawlOutcome {
        data: FeedData::Updated(hashed_data),
        headers: response.headers().to_owned(),
        refreshed_at,
    })
}

fn update_feed_cache_keys(feed: &mut Feed, headers: &HeaderMap, hash: Option<Hash>) {
    let etag = headers.get(ETAG).and_then(|val| val.to_str().ok());
    let last_modified = headers
        .get(LAST_MODIFIED)
        .and_then(|val| val.to_str().ok())
        .and_then(|last_mod| DateTime::parse_from_rfc2822(last_mod).ok())
        .map(|date| date.to_utc());

    feed.etag = etag.map(ToString::to_string);
    feed.last_modified = last_modified;
    feed.last_refresh_hash = hash.map(|h| *h.as_bytes());
}

fn parse_feed(
    // db: &mut Database,
    // _tx: &mut WriteTransaction,
    // feed_id: FeedId,
    // sync_type: SyncType,
    data: &str,
    // TODO: Use the content type to help drive the parse order
    content_type: &Option<Mime>,
) -> Result<ParsedFeed, ProcessError> {
    // Parse the data
    let parsed = if data.trim_start().starts_with('{') {
        // JSON
        debug!("detected JSON format");
        serde_json::from_str(data).map(ParsedFeed::Json)?
    } else {
        // Try to parse as RSS, if that fails, try Atom
        Channel::read_from(data.as_bytes())
            .map(|rss| {
                debug!("parsed as RSS");
                ParsedFeed::Rss(rss)
            })
            .or_else(|err| {
                match err {
                    rss::Error::InvalidStartTag => {
                        atom::Feed::read_from(data.as_bytes()).map_err(|err| match err {
                            atom::Error::InvalidStartTag => ProcessError::UnknownFormat,
                            _ => ProcessError::Atom(err),
                        })
                    }
                    _ => Err(err.into()),
                }
                .map(|feed| {
                    debug!("parsed as Atom");
                    ParsedFeed::Atom(feed)
                })
            })?
    };

    // Process updates to the feed's title
    // let feed = Feed::from_id(db, feed_id).await?;
    // let title = parsed.title();
    // if !title.is_empty() && feed.title != title {
    //     Feed::update_title(db, feed_id, title).await?;
    // }

    // Process each entry, creating, or updating items as necessary
    // debug!(count = parsed.item_count(), "process items");
    // let now = OffsetDateTime::now_utc();
    // for mut item in parsed.items() {
    //     if FeedItem::exists(db, &item.guid).await? {
    //         // TODO: Update existing record if needed
    //         // Check date_updated and potentially compare the content to see if the item
    //         // should be updated
    //         // debug!(guid = item.guid, "Skipping existing item");
    //         continue;
    //     }

    //     // Create new item
    //     // info!(guid = item.guid, "create new item");
    //     if sync_type == SyncType::Initial {
    //         // When doing an initial sync all newly created feed items are marked as already
    //         // notified so that only items that are received after the initial sync may trigger
    //         // notifications to users.
    //         item.notified_at = Some(now);
    //     }
    //     FeedItem::create(db, feed_id, item).await?;
    // }

    Ok(parsed)
}

// async fn listen_for_notifications(
//     client: reqwest::Client,
//     pool: PgPool,
//     lock_pool: Arc<LockPool<FeedId>>,
//     mut listener: PgListener,
//     shutdown: Arc<AtomicBool>,
// ) -> Result<(), sqlx::Error> {
//     // let backoff = tokio::time::Duration::from_secs(0);
//     let (send, mut recv) = tokio::sync::mpsc::channel::<()>(1);
//     'quit: loop {
//         // start handling notifications, connecting if needed
//         while let Some(notification) = listener.try_recv().await? {
//             match notification.channel() {
//                 "bellbird.subscription" => {
//                     let payload = notification.payload();
//                     info!(feed_id = payload, "new subscription");
//                     let feed_id = match payload.parse::<i64>().map(FeedId::from) {
//                         Ok(id) => id,
//                         Err(err) => {
//                             // error!(%err, "invalid subscription payload");
//                             continue;
//                         }
//                     };
//
//                     tokio::spawn(handle_subscription_notification(
//                         client.clone(),
//                         pool.clone(),
//                         Arc::clone(&lock_pool),
//                         feed_id,
//                         send.clone(),
//                     ));
//                 }
//                 "bellbird.shutdown" => {
//                     info!("received shutdown notification");
//                     listener.unlisten_all().await?;
//
//                     // Wait for tasks to finish
//                     //
//                     // We drop our sender first because the recv() call otherwise
//                     // sleeps forever.
//                     drop(send);
//
//                     // When every sender has gone out of scope, the recv call
//                     // will return with an error. We ignore the error.
//                     info!("waiting for pending tasks");
//                     let _ = recv.recv().await;
//
//                     info!("exiting");
//                     shutdown.store(true, Ordering::SeqCst);
//                     break 'quit;
//                 }
//                 _ => {}
//             }
//         }
//
//         // connection lost, wait before retrying
//         // tokio::time::sleep(Duration::from_secs(2))
//     }
//
//     Ok(())
// }

// async fn handle_subscription_notification(
//     client: reqwest::Client,
//     pool: PgPool,
//     lock_pool: Arc<LockPool<FeedId>>,
//     feed_id: FeedId,
//     _sender: Sender<()>,
// ) -> Result<(), CrawlError> {
//     let _lock = match lock_pool.try_lock(feed_id) {
//         Some(lock) => lock,
//         None => {
//             info!("refresh already in progress");
//             return Ok(());
//         }
//     };
//
//     // If this feed has next_refresh_at of NULL then perform initial sync
//     let mut db = pool
//         .acquire()
//         .await
//         .map_err(|err| CrawlError::Fetch(FetchError::Database(err)))?;
//     // TODO: Avoid fetching entire Feed
//     let feed = Feed::from_id(&mut db, feed_id)
//         .await
//         .map_err(|err| CrawlError::Fetch(FetchError::Database(err)))?;
//     if feed.next_refresh_at.is_none() {
//         // Do an initial sync of this feed
//         refresh_and_schedule_next(client, &mut db, SyncType::Initial, feed_id).await?;
//     }
//     Ok(())
// }

// async fn initial_sync_check(db: &mut Database) -> Result<(), CrawlError> {
//     info!("checking for missed initial syncs");
//     let feeds = Feed::for_initial_sync(db).await?;
//     for feed_id in feeds {
//         info!(%feed_id, "notifying");
//         sqlx::query!(
//             r#"SELECT pg_notify('bellbird.subscription', CAST($1 AS text))"#,
//             i64::from(feed_id) as _
//         )
//             .execute(&mut *db)
//             .await?;
//     }
//
//     Ok(())
// }

fn to_utf8(text: Vec<u8>, content_type: &Option<Mime>) -> Result<String, FetchError> {
    // Does the content type tell us anything about the encoding?
    // let &Mime(_, _, ref content_type_params) = content_type;
    // let charset_param = content_type_params.iter()
    //     .find(|&&(ref attr, _)| *attr == mime::Attr::Charset);
    // let param_value = charset_param.map(|&(_, ref value)| value.to_string());
    let param_value = content_type.as_ref().and_then(|content_type| {
        content_type
            .get_param(mime::CHARSET)
            .map(|value| value.to_string())
    }); // FIXME: avoid this to_string
    let mut text_cursor = Cursor::new(text);

    // Detect character set
    // encoding crate doesn't support lookup of iso-8859-9 encoder but Windows-1254 is the same.
    let detected_charsets: Vec<_> = xhtmlchardet::detect(&mut text_cursor, param_value)
        .map_err(|_| FetchError::UnknownEncoding)?
        .into_iter()
        .map(|charset| {
            if charset == "iso-8859-9" {
                Cow::from("windows-1254")
            } else {
                Cow::from(charset)
            }
        })
        .collect();

    let mut text = text_cursor.into_inner();

    // Handle response encoding
    for detected_charset in &detected_charsets {
        if detected_charset == "utf-8" || detected_charset == "ascii" {
            match String::from_utf8(text) {
                Ok(string) => return Ok(string),
                Err(err) => text = err.into_bytes(),
            }
        } else {
            // FIXME: Replace encoding crate with encoding_rs
            // response, is not utf-8, so try to convert it
            // println!("transcode {} -> utf-8 ({})", detected_charset, feed.id);
            // let available_encodings: Vec<&str> = encoding::all::encodings().iter().map(|e| e.whatwg_name().unwrap_or("unknown")).collect();
            // println!("available encoders: {:?}", available_encodings);
            let encoder = encoding::all::encodings().iter().find(|encoder| {
                encoder.whatwg_name().unwrap_or_else(|| encoder.name()) == detected_charset
            });
            if encoder.is_none() {
                continue;
            }

            match encoder
                .unwrap()
                .decode(&text, encoding::types::DecoderTrap::Strict)
            {
                Ok(str) => return Ok(str),
                Err(_) => continue,
            }
        }
    }

    // Nothing succeeded, if it claims to be utf-8 or ascii then go for a lossy import
    if detected_charsets
        .iter()
        .any(|charset| charset == "ascii" || charset == "utf-8")
    {
        Ok(String::from_utf8_lossy(&text).into_owned())
    } else {
        // If we get here transcoding failed
        Err(FetchError::UnknownEncoding)
    }
}

impl From<redb::Error> for FetchError {
    fn from(err: redb::Error) -> Self {
        FetchError::Database(err)
    }
}

impl From<reqwest::Error> for FetchError {
    fn from(err: reqwest::Error) -> Self {
        FetchError::Request(err)
    }
}

impl From<rss::Error> for ProcessError {
    fn from(err: rss::Error) -> Self {
        ProcessError::Rss(err)
    }
}

impl From<serde_json::Error> for ProcessError {
    fn from(err: serde_json::Error) -> Self {
        ProcessError::Json(err)
    }
}

impl From<atom::Error> for ProcessError {
    fn from(err: atom::Error) -> Self {
        ProcessError::Atom(err)
    }
}

impl From<redb::Error> for ProcessError {
    fn from(err: redb::Error) -> Self {
        ProcessError::Database(err)
    }
}

impl From<FetchError> for CrawlError {
    fn from(err: FetchError) -> Self {
        CrawlError::Fetch(err)
    }
}

impl From<ProcessError> for CrawlError {
    fn from(err: ProcessError) -> Self {
        CrawlError::Process(err)
    }
}

impl From<redb::CommitError> for CrawlError {
    fn from(err: redb::CommitError) -> Self {
        CrawlError::Database(err.into())
    }
}

impl From<redb::TransactionError> for CrawlError {
    fn from(err: redb::TransactionError) -> Self {
        CrawlError::Database(err.into())
    }
}

impl From<redb::Error> for CrawlError {
    fn from(err: redb::Error) -> Self {
        CrawlError::Database(err)
    }
}

impl fmt::Display for CrawlError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            CrawlError::Fetch(err) => err.fmt(f),
            CrawlError::Process(err) => err.fmt(f),
            CrawlError::Database(err) => err.fmt(f),
        }
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("fetch error: ")?;
        match self {
            FetchError::Database(err) => {
                f.write_str("database: ")?;
                err.fmt(f)
            }
            FetchError::Request(err) => {
                f.write_str("request: ")?;
                err.fmt(f)
            }
            FetchError::ResponseTooBig => f.write_str("response too big"),
            FetchError::ResponseUnsuccessful(err) => err.fmt(f),
            FetchError::UnknownEncoding => f.write_str("unknown encoding"),
        }
    }
}

impl fmt::Display for ProcessError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("process error: ")?;
        match self {
            ProcessError::Rss(err) => err.fmt(f),
            ProcessError::Atom(err) => err.fmt(f),
            ProcessError::Json(err) => err.fmt(f),
            ProcessError::UnknownFormat => f.write_str("unknown feed format"),
            ProcessError::Database(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for CrawlError {}
impl std::error::Error for FetchError {}
impl std::error::Error for ProcessError {}
