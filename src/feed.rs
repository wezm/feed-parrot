use std::str::FromStr;

use atom_syndication as atom;
use chrono::{DateTime, ParseResult, Utc};
use mime::Mime;
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
    pub url: Option<String>,
    pub title: Option<String>,
    pub author: Option<String>,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub tags: Vec<String>,
    pub date_published: Option<DateTime<Utc>>,
    pub date_modified: Option<DateTime<Utc>>,
}

pub struct PostGuid(String);

impl PostGuid {
    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl NewFeedItem {
    pub fn guid(&self) -> PostGuid {
        PostGuid(self.guid.clone())
    }
}

impl From<atom::Entry> for NewFeedItem {
    fn from(entry: atom::Entry) -> Self {
        // TODO: Use intersperse when stable
        // https://doc.rust-lang.org/std/iter/trait.Iterator.html#method.intersperse
        let author = join_to_string::join(entry.authors.into_iter().map(|person| person.name))
            .separator(",")
            .to_string();
        let url = choose_atom_link(&entry.links);
        NewFeedItem {
            guid: entry.id,
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

fn choose_atom_link(links: &[atom::Link]) -> Option<String> {
    // atom:link elements MAY have a "rel" attribute that indicates the link
    // relation type.  If the "rel" attribute is not present, the link
    // element MUST be interpreted as if the link relation type is
    // "alternate".
    //
    // The value "alternate" signifies that the IRI in the value of the
    // href attribute identifies an alternate version of the resource
    // described by the containing element.
    //
    // On the link element, the "type" attribute's value is an advisory
    // media type: it is a hint about the type of the representation that is
    // expected to be returned when the value of the href attribute is
    // dereferenced.
    //
    // https://datatracker.ietf.org/doc/html/rfc4287#section-4.2.7.2

    // For the most part we would post the alternate link with text/html media type
    // However Daring Fireball for example sets the alternate link to the
    // linked item and related link to the DF page. This seems backwards. The
    // alternate link is the DF one and the related one is the link that the
    // post is talking about.
    //
    // It's likely this is done for the widest RSS reader support. The
    // snag for the purposes of a tool like this though is that we'd
    // probably want to post the related link in this case like the
    // DF Tooter does. This seems pretty rare though so for now I'll
    // stick with the alternate link.
    //
    // It's als worth noting that DF's JSON feed uses the url and external_url
    // fields as intended so that would be an option if subscribing to the
    // DF feed was desired.

    for link in links {
        // atom_syndication defaults rel to alternate
        if link.rel != "alternate" {
            continue;
        }

        let Ok(mime_type) = link
            .mime_type
            .as_ref()
            .map(|s| Mime::from_str(s))
            .transpose()
        else {
            // Failed to parse media type:
            // Link elements MAY have a type attribute, whose value MUST conform to the syntax of a MIME media type.
            continue;
        };
        if link.mime_type.is_none()
            || mime_type.as_ref().map(|mime| mime.essence_str()) == Some("text/html")
        {
            return Some(link.href.clone());
        }
    }

    None
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

    pub fn item_count(&self) -> usize {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choose_atom_link_no_attrs() {
        let links = &[atom::Link {
            href: "https://www.example.com".to_string(),
            ..Default::default()
        }];
        assert_eq!(choose_atom_link(links).unwrap(), "https://www.example.com");
    }

    #[test]
    fn choose_atom_link_type() {
        let links = &[atom::Link {
            href: "https://www.example.com".to_string(),
            mime_type: Some("text/html; charset=utf-8".to_string()),
            ..Default::default()
        }];
        assert_eq!(choose_atom_link(links).unwrap(), "https://www.example.com");
    }

    #[test]
    fn choose_atom_link_multiple() {
        let links = &[
            atom::Link {
                href: "https://www.example.com/related".to_string(),
                rel: "related".to_string(),
                ..Default::default()
            },
            atom::Link {
                href: "https://www.example.com/feed".to_string(),
                rel: "alternate".to_string(),
                mime_type: Some("application/atom+xml".to_string()),
                ..Default::default()
            },
            atom::Link {
                href: "https://www.example.com".to_string(),
                mime_type: Some("text/html; charset=utf-8".to_string()),
                ..Default::default()
            },
        ];
        assert_eq!(choose_atom_link(links).unwrap(), "https://www.example.com");
    }

    #[test]
    fn choose_atom_link_unviable() {
        let links = &[
            atom::Link {
                href: "https://www.example.com/related".to_string(),
                rel: "related".to_string(),
                ..Default::default()
            },
            atom::Link {
                href: "https://www.example.com/feed".to_string(),
                rel: "alternate".to_string(),
                mime_type: Some("application/atom+xml".to_string()),
                ..Default::default()
            },
        ];
        assert_eq!(choose_atom_link(links), None);
    }
}
