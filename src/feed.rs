use atom_syndication as atom;
use chrono::{DateTime, ParseResult, Utc};
use rss::Channel;

use crate::json_feed::{self, JsonFeed};

pub enum ParsedFeed {
    Rss(Channel),
    Atom(atom::Feed),
    Json(JsonFeed),
}

// TODO: Rename
#[derive(Debug)]
pub struct NewFeedItem {
    pub guid: String, // TODO: Ensure not empty
    pub guid_id_permalink: bool,
    pub url: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub tags: Vec<String>,
    pub date_published: Option<DateTime<Utc>>,
    pub date_modified: Option<DateTime<Utc>>,
}

impl From<atom::Entry> for NewFeedItem {
    fn from(entry: atom::Entry) -> Self {
        // TODO: Use intersperse when stable
        // https://doc.rust-lang.org/std/iter/trait.Iterator.html#method.intersperse
        let author = join_to_string::join(entry.authors.into_iter().map(|person| person.name))
            .separator(",")
            .to_string();
        let url = entry.links.first().map(|link| link.href.to_owned()); // FIXME: Better way to select link that filters on rel and mime type
        NewFeedItem {
            guid: entry.id,
            guid_id_permalink: url.is_none(),
            url,
            title: Some(entry.title.value), // FIXME: This can be HTML as well; handle that
            author: (!author.is_empty()).then_some(author),
            summary: entry.summary.map(|summary| summary.value), // FIXME: This can be HTML as well; handle that
            content: entry.content.and_then(|content| content.value),
            tags: entry
                .categories
                .into_iter()
                .filter_map(|cat| cat.scheme.is_none().then_some(cat.term))
                .collect(),
            date_published: entry.published.map(|published| published.to_utc()),
            date_modified: Some(entry.updated.to_utc()),
        }
    }
}

pub struct ParsedFeedItemsIter<'feed> {
    feed: &'feed ParsedFeed,
    index: usize,
}

impl ParsedFeed {
    pub fn items(&self) -> ParsedFeedItemsIter<'_> {
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

impl Iterator for ParsedFeedItemsIter<'_> {
    type Item = NewFeedItem;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.feed.item_count() {
            let item = match self.feed {
                ParsedFeed::Rss(feed) => {
                    // FIXME: Reconsider this for feed parrot

                    // This hackery is to skip RSS items that lack a guid. Items without a guid
                    // don't allow us to know if the item is new or not... which is kinda important
                    // when sending notifications
                    let mut item = None;
                    while item.is_none() && self.index < self.feed.item_count() {
                        item = feed.items[self.index].clone().try_into().ok();
                        self.index += 1;
                    }
                    return item;
                }
                ParsedFeed::Atom(feed) => Some(feed.entries[self.index].clone().into()),
                ParsedFeed::Json(feed) => Some(feed.items[self.index].clone().into()),
            };
            self.index += 1;
            item
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.feed.item_count() - self.index;
        (remaining, Some(remaining))
    }
}

pub enum TryFromRssItemError {
    NoGuid,
}

impl TryFrom<rss::Item> for NewFeedItem {
    type Error = TryFromRssItemError;

    fn try_from(item: rss::Item) -> Result<Self, TryFromRssItemError> {
        let date_published = item
            .pub_date()
            .and_then(|pub_date| parse_rfc2822(pub_date).ok())
            .map(|pub_date| pub_date.to_utc());
        let guid_id_permalink = item.guid.as_ref().map_or(true, |guid| guid.permalink);
        Ok(NewFeedItem {
            guid: item
                .guid
                .map(|guid| guid.value)
                .ok_or_else(|| TryFromRssItemError::NoGuid)?,
            guid_id_permalink,
            url: item.link,
            title: item.title,
            author: item.author,
            summary: item.description,
            content: item.content,
            tags: item
                .categories
                .into_iter()
                .filter_map(|cat| cat.domain.is_none().then_some(cat.name))
                .collect(),
            date_published,      // FIXME: Handle atom:published
            date_modified: None, // FIXME: Handle atom:updated
        })
    }
}

impl From<json_feed::Item> for NewFeedItem {
    fn from(item: json_feed::Item) -> Self {
        let date_published = item
            .date_published
            .and_then(|pub_date| parse_rfc2822(&pub_date).ok());
        let date_modified = item
            .date_modified
            .and_then(|mod_date| parse_rfc2822(&mod_date).ok());
        NewFeedItem {
            guid: item.id,
            guid_id_permalink: item.url.is_none(),
            url: item.url,
            title: item.title,
            author: item.author.and_then(|author| author.name),
            summary: item.summary,
            content: item.content_html,
            date_published,
            date_modified,
            tags: item.tags,
        }
    }
}

fn parse_rfc2822(s: &str) -> ParseResult<DateTime<Utc>> {
    let date = DateTime::parse_from_rfc2822(s)?;
    Ok(date.to_utc())
}
