use std::borrow::Cow;
use std::fmt::{self, Formatter};
use std::io::Cursor;

use atom_syndication as atom;
use blake3::Hash;
use chrono::{DateTime, Utc};
// use lockable::LockPool;
use mime::Mime;
use redb::{Database, WriteTransaction};
use reqwest::header::{
    HeaderMap, HeaderValue, CONTENT_TYPE, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED,
};
use reqwest::StatusCode;
// use rocket::tokio;
// use rocket::tokio::task::{JoinHandle, JoinSet};
// use rocket::tokio::time::MissedTickBehavior;
// use rocket_db_pools::Database;
use rss::Channel;
// use sqlx::postgres::PgListener;
// use sqlx::{Connection, PgConnection, PgPool};
use url::Url;
// use tracing::{debug, error, event, info, instrument, Level};
// use tracing_subscriber::filter::{EnvFilter, LevelFilter};

use crate::db::{self, Feed};
// use crate::db::Db;
use crate::json_feed::JsonFeed;
// use crate::models::feed::{Feed, FeedId, FeedItem, NewFeedItem};

const MAX_RESPONSE_SIZE: u64 = 5 * 1024 * 1024; // 5 MiB

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

// fn main() -> Result<(), rocket::Error> {
//     let filter = EnvFilter::builder()
//         // Set the base level when not matched by other directives to INFO.
//         .with_env_var("BELLBIRD_LOG")
//         .with_default_directive(LevelFilter::INFO.into())
//         .from_env()
//         .expect("unable to build log filter");
//     tracing_subscriber::fmt()
//         .with_env_filter(filter)
//         .try_init()
//         .expect("unable to initialise logging");
//     // Find feeds that need refreshing
//     // Kick off an update task for each one (somehow preventing the next loop from duplicating the task)
//     // Note new entries
//     // Something else needs to notice the new entries and send notifications
//
//     // Goals for this implementation:
//     // - For a given server rely heavily on async Rust + Tokio for task scheduling
//     // - There is just one process handling all tasks. As more capacity is needed
//     //   just add more CPUs. In an ideal scenario it would run on a many core ARM
//     //   server.
//     // - There needs to be some way to run multiple instances of the crawler on
//     //   separate machines for redundancy and if network bandwidth becomes an
//     //   issue. Perhaps this isn't a high priority initially but should be factored
//     //   into the design.
//     // - This inherently means some sort of distributed locking... maybe this isn't
//     //   worth the effort initially
//     tokio::runtime::Builder::new_multi_thread()
//         .thread_name("bellbird-crawler-thread")
//         // .max_blocking_threads(sync) // default 512
//         .enable_all()
//         .build()
//         .expect("unable to create tokio runtime")
//         .block_on(async { async_main().await })
// }
//
// async fn async_main() -> Result<(), rocket::Error> {
//     std::env::set_var("ROCKET_LOG_LEVEL", "off");
//     let ignite = rocket::build().attach(Db::init()).ignite();
//     let rocket = ignite.await?;
//
//     let db = Db::fetch(&rocket).unwrap(); // Won't panic as we've called Db::init
//     // FIXME: Do we want to hold this connection the whole time or only acquire it to fetch feeds
//     let mut conn = db.acquire().await.expect("unable to acquire db connection");
//
//     // From the docs:
//     // The Client holds a connection pool internally, so it is advised that you create one and reuse it.
//     // You do not have to wrap the Client in an Rc or Arc to reuse it, because it already uses an Arc internally.
//     // TODO: 'on redirect'
//     // TODO: Set user agent
//     let client = reqwest::Client::new();
//
//     // Locks to prevent concurrently syncing of feeds
//     let lock_pool = Arc::new(LockPool::new());
//
//     // TODO: Make it possible to shutdown cleanly with signals too
//     let shutdown = Arc::new(AtomicBool::new(false));
//
//     // Listen for subscription events that might mean there are feeds that need their initial
//     // sync.
//     let mut listener = PgListener::connect_with(&db.0).await.expect("FIXME");
//     listener
//         .listen("bellbird.subscription")
//         .await
//         .expect("FIXME");
//     listener.listen("bellbird.shutdown").await.expect("FIXME");
//     let listener_task = {
//         // NOTE(clone): These are all Arc::clones
//         let shutdown = Arc::clone(&shutdown);
//         let client = client.clone();
//         let pool = db.0.clone();
//         let lock_pool = Arc::clone(&lock_pool);
//         tokio::spawn(async move {
//             listen_for_notifications(client, pool, lock_pool, listener, shutdown).await
//         })
//     };
//
//     // Start the periodic check for missed notifications resulting in feeds that need their initial
//     // sync
//     // TODO: Move this into a function?
//     let missed_notification_task = {
//         let shutdown = Arc::clone(&shutdown);
//         let pool = db.0.clone();
//         tokio::spawn(async {
//             let shutdown_task = tokio::spawn(async move {
//                 let mut shutdown_interval =
//                     tokio::time::interval(tokio::time::Duration::from_secs(1));
//                 shutdown_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
//                 loop {
//                     shutdown_interval.tick().await;
//                     if shutdown.load(Ordering::Relaxed) {
//                         info!("shutdown missed notification task");
//                         break;
//                     }
//                 }
//             });
//             let initial_sync_task: JoinHandle<Result<(), CrawlError>> = tokio::spawn(async move {
//                 let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));
//                 interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
//                 loop {
//                     interval.tick().await;
//                     let mut db = pool.acquire().await?;
//                     initial_sync_check(&mut db).await?; // FIXME: Do we actually want to bail on error here
//                 }
//             });
//             // Wait for one of the tasks to complete. If shutdown is first then we want to shut
//             // down. If initial_sync is first then it errored or panicked.
//             tokio::select! {
//                 _ = shutdown_task => { Ok(()) }
//                 res = initial_sync_task => { Err(res) }
//             }
//         })
//     };
//
//     // TODO: Move this to a function
//     info!("entering main loop");
//     loop {
//         // TODO: Add signal handling to shutdown cleanly
//         // https://tokio.rs/tokio/topics/shutdown
//         let last_check = tokio::time::Instant::now();
//         let feeds = Feed::for_refresh(&mut conn).await.expect("FIXME"); // FIXME: How should this error be handled
//         let count = feeds.len();
//
//         if count > 0 {
//             event!(Level::INFO, "feed count" = count, "refresh");
//             let mut tasks = JoinSet::new();
//             for feed_id in feeds {
//                 // NOTE(clone): These are Arc::clones
//                 let client = client.clone();
//                 let pool = db.0.clone();
//                 let lock_pool = Arc::clone(&lock_pool);
//                 tasks.spawn(async move {
//                     let _lock = match lock_pool.try_lock(feed_id) {
//                         Some(lock) => lock,
//                         None => {
//                             info!("refresh already in progress");
//                             return;
//                         }
//                     };
//                     let mut conn = pool
//                         .acquire()
//                         .await
//                         .expect("unable to acquire db connection");
//                     match refresh_and_schedule_next(
//                         client,
//                         &mut conn,
//                         SyncType::Incremental,
//                         feed_id,
//                     )
//                         .await
//                     {
//                         Ok(()) => {}
//                         Err(err) => error!(%feed_id, %err),
//                     }
//                 });
//             }
//
//             // When join_next returns None all tasks in the set have completed
//             while let Some(res) = tasks.join_next().await {
//                 match res {
//                     Ok(()) => {}
//                     // FIXME: I think JoinError means a panic happened, which should probably float up to main
//                     Err(err) => error!(?err, "FIXME: join error"),
//                 }
//             }
//         }
//
//         if shutdown.load(Ordering::Relaxed) {
//             info!("shutdown");
//             break;
//         }
//
//         // FIXME: Replace with interval
//         // Throttle passes through this loop to at most approximately once every second
//         let now = tokio::time::Instant::now();
//         let since_last_check = now - last_check;
//         if since_last_check < tokio::time::Duration::from_secs(1) {
//             tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
//         }
//     }
//
//     listener_task
//         .await
//         .expect("join error")
//         .expect("FIXME: listen for notifications error");
//     missed_notification_task
//         .await
//         .expect("join error")
//         .expect("FIXME: missed notification task error");
//
//     Ok(())
// }

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum SyncType {
    Initial,
    Incremental,
}

// async fn refresh_and_schedule_next(
//     client: reqwest::Client,
//     db: &mut PgConnection,
//     sync_type: SyncType,
//     feed_id: FeedId,
// ) -> Result<(), CrawlError> {
//     match refresh_feed(client, db, sync_type, feed_id).await {
//         Ok(refreshed_at) => Feed::schedule_next_refresh(db, feed_id, refreshed_at).await?,
//         Err(err) => {
//             error!(%feed_id, %err);
//             // FIXME: Reschedule fetch with error backoff
//             Feed::schedule_next_refresh(db, feed_id, OffsetDateTime::now_utc()).await?
//         }
//     }
//     Ok(())
// }

// #[instrument(skip(client, conn), err)]
pub async fn refresh_feed(
    client: reqwest::Client,
    conn: &Database,
    sync_type: SyncType,
    // feed_id: FeedId,
    feed_url: Url,
) -> Result<FeedData<ParsedFeed>, CrawlError> {
    info!("begin");
    // Load the feed from the db
    let mut feed = db::load_feed(conn, &feed_url)
        .map_err(|err| CrawlError::Fetch(FetchError::Database(err)))?;

    let outcome = fetch_feed(&client, &feed).await?;
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
            let mut tx = conn.begin_write()?;
            let parsed_feed = {
                // Box::pin(async move {
                // Update the etag, last_modified, and content type on the feed. This is done inside the
                // transaction so that if processing the feed fails we will fetch the feed
                // with the old cache headers again next time. If new headers were used we
                // would potentially get a 304 response and not process any of the items that
                // failed to import initially.
                update_feed_cache_keys(&mut tx, &mut feed, &outcome.headers, Some(hash)).await?;
                parse_feed(&mut tx, &data, &content_type).await?
                // })
                // })
                // .await?
            };
            tx.commit()?;
            FeedData::Updated(parsed_feed)
        }
    };

    Ok(feed_data)
}

// #[instrument(skip_all, err)]
async fn fetch_feed(client: &reqwest::Client, feed: &Feed) -> Result<CrawlOutcome, FetchError> {
    // info!(url = feed.url);
    // Build a request to fetch the feed, setting condititional request headers as appropriate
    let mut headers = HeaderMap::new();
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

    // Calculate a hash of the body and compare it to the existing one if present
    let hashed_data = HashedData::new(body);
    // let hash = blake3::hash(&body);
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

    Ok(CrawlOutcome {
        data: FeedData::Updated(hashed_data),
        headers: response.headers().to_owned(),
        refreshed_at,
    })
}

// #[instrument(skip_all, err)]
async fn update_feed_cache_keys(
    tx: &mut WriteTransaction,
    feed: &mut Feed,
    headers: &HeaderMap,
    hash: Option<Hash>,
) -> Result<(), redb::Error> {
    let etag = headers.get(ETAG).and_then(|val| val.to_str().ok());
    let last_modified = headers
        .get(LAST_MODIFIED)
        .and_then(|val| val.to_str().ok())
        .and_then(|last_mod| DateTime::parse_from_rfc2822(last_mod).ok())
        .map(|date| date.to_utc());

    feed.etag = etag.map(ToString::to_string);
    feed.last_modified = last_modified;
    feed.last_refresh_hash = hash.map(|h| *h.as_bytes());

    db::save_feed(tx, &feed)
}

pub enum ParsedFeed {
    Rss(Channel),
    Atom(atom::Feed),
    Json(JsonFeed),
}

// #[instrument(skip_all, err)]
async fn parse_feed(
    // db: &mut Database,
    _tx: &mut WriteTransaction,
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

// #[instrument(skip_all, err)]
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

// #[instrument(skip(client, pool, lock_pool, _sender))]
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

// #[instrument(skip_all, err)]
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

struct ParsedFeedItemsIter<'feed> {
    feed: &'feed ParsedFeed,
    index: usize,
}

impl ParsedFeed {
    fn items(&self) -> ParsedFeedItemsIter<'_> {
        ParsedFeedItemsIter {
            feed: self,
            index: 0,
        }
    }

    fn item_count(&self) -> usize {
        match self {
            ParsedFeed::Rss(feed) => feed.items.len(),
            ParsedFeed::Atom(feed) => feed.entries.len(),
            ParsedFeed::Json(feed) => feed.items.len(),
        }
    }

    fn title(&self) -> &str {
        (match self {
            ParsedFeed::Rss(feed) => feed.title.as_str(),
            // FIXME: text in Atom can be HTML; handle this
            ParsedFeed::Atom(feed) => &feed.title,
            ParsedFeed::Json(feed) => &feed.title,
        })
        .trim()
    }

    fn description(&self) -> Option<&str> {
        match self {
            ParsedFeed::Rss(feed) => Some(feed.description.as_str()),
            // FIXME: text in Atom can be HTML; handle this
            ParsedFeed::Atom(feed) => feed.subtitle.as_deref(),
            ParsedFeed::Json(feed) => feed.description.as_deref(),
        }
    }
}

// impl Iterator for ParsedFeedItemsIter<'_> {
//     type Item = NewFeedItem;

//     fn next(&mut self) -> Option<Self::Item> {
//         if self.index < self.feed.item_count() {
//             let item = match self.feed {
//                 ParsedFeed::Rss(feed) => {
//                     // This hackery is to skip RSS items that lack a guid. Items without a guid
//                     // don't allow us to know if the item is new or not... which is kinda important
//                     // when sending notifications
//                     let mut item = None;
//                     while item.is_none() && self.index < self.feed.item_count() {
//                         item = feed.items[self.index].clone().try_into().ok();
//                         self.index += 1;
//                     }
//                     return item;
//                 }
//                 ParsedFeed::Atom(feed) => Some(feed.entries[self.index].clone().into()),
//                 ParsedFeed::Json(feed) => Some(feed.items[self.index].clone().into()),
//             };
//             self.index += 1;
//             item
//         } else {
//             None
//         }
//     }

//     fn size_hint(&self) -> (usize, Option<usize>) {
//         let remaining = self.feed.item_count() - self.index;
//         (remaining, Some(remaining))
//     }
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
            FetchError::Database(err) => err.fmt(f),
            FetchError::Request(err) => err.fmt(f),
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
