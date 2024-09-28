use std::borrow::Cow;
use std::fmt::{self, Formatter};
use std::io::{self, Cursor, Read};

use atom_syndication as atom;
use blake3::Hash;
use chrono::{DateTime, Utc};
use mime::Mime;
use reqwest::blocking::Client;
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
    Io(io::Error),
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
    Json(serde_json::Error),
    // The feed is in an unknown format
    UnknownFormat,
}

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

#[derive(Copy, Clone)]
enum FeedType {
    Atom,
    Rss,
}

pub fn refresh_feed(
    client: Client,
    cond_req: ConditionalRequest,
    feed: &mut Feed,
) -> Result<FeedData<ParsedFeed>, CrawlError> {
    info!("begin");
    let outcome = fetch_feed(&client, &feed, cond_req)?;
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

            let parsed_feed = {
                // Update the etag, last_modified, and content type on the feed.
                update_feed_cache_keys(feed, &outcome.headers, Some(hash));
                parse_feed(&data, &content_type)?
            };
            FeedData::Updated(parsed_feed)
        }
    };

    Ok(feed_data)
}

fn fetch_feed(
    client: &Client,
    feed: &Feed,
    cond_req: ConditionalRequest,
) -> Result<CrawlOutcome, FetchError> {
    // info!(url = feed.url);
    // Build a request to fetch the feed, setting conditional request headers as appropriate
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
    let mut response = client.get(feed.url.as_str()).headers(headers).send()?;
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
    let mut buf = [0; 8092];
    loop {
        match response.read(&mut buf) {
            // EOF
            Ok(0) => break,
            // Read `n` bytes
            Ok(n) => {
                if body
                    .len()
                    .checked_add(n)
                    .map_or(true, |len| len > MAX_RESPONSE_SIZE as usize)
                {
                    return Err(FetchError::ResponseTooBig);
                }
                body.extend_from_slice(&buf[0..n]);
            }
            // Try again
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            // Error
            Err(err) => return Err(err.into()),
        }
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

fn parse_feed(data: &str, content_type: &Option<Mime>) -> Result<ParsedFeed, ProcessError> {
    // Parse the data. JSON Feed has to start with {
    if data.trim_start().starts_with('{') {
        // JSON
        debug!("detected JSON format");
        return serde_json::from_str(data)
            .map(ParsedFeed::Json)
            .map_err(ProcessError::Json);
    }

    // Parse as one of the XML formats
    let media_type = content_type.as_ref().map(|media| media.essence_str());
    let parse_order = match media_type {
        Some("application/atom+xml") => [FeedType::Atom, FeedType::Rss],
        // application/rss+xml | text/xml | None => try RSS first
        _ => [FeedType::Rss, FeedType::Atom],
    };

    let mut type_iter = parse_order.iter().copied().peekable();
    while let Some(feed_type) = type_iter.next() {
        match feed_type {
            FeedType::Atom => {
                match atom::Feed::read_from(data.as_bytes()) {
                    Ok(feed) => {
                        debug!("parsed as Atom");
                        return Ok(ParsedFeed::Atom(feed));
                    }
                    // Unable to parse and no more feed types to try
                    Err(atom::Error::InvalidStartTag) if type_iter.peek().is_none() => {}
                    // Not Atom, other types to try
                    Err(atom::Error::InvalidStartTag) => continue,
                    // Invalid
                    Err(err) => return Err(ProcessError::Atom(err)),
                }
            }
            FeedType::Rss => {
                match Channel::read_from(data.as_bytes()) {
                    Ok(feed) => {
                        debug!("parsed as RSS");
                        return Ok(ParsedFeed::Rss(feed));
                    }
                    // Unable to parse and no more feed types to try
                    Err(rss::Error::InvalidStartTag) if type_iter.peek().is_none() => {}
                    // Not RSS, other types to try
                    Err(rss::Error::InvalidStartTag) => continue,
                    // Invalid
                    Err(err) => return Err(ProcessError::Rss(err)),
                }
            }
        }
    }

    Err(ProcessError::UnknownFormat)
}

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

impl From<io::Error> for FetchError {
    fn from(err: io::Error) -> Self {
        FetchError::Io(err)
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
            FetchError::Io(err) => {
                f.write_str("i/o: ")?;
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
