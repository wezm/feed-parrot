mod client;
mod extractor;
pub mod models;

use std::borrow::Cow;
use std::io::Write;
use std::{io, iter};

use eyre::{eyre, Context};
use models::MastodonState;
use redb::Database;
use reqwest::blocking::Client;
use unicode_segmentation::UnicodeSegmentation;

use crate::db::{self};
use crate::feed::NewFeedItem;
use crate::mastodon::extractor::Entity;
use crate::mastodon::models::{NewStatus, Visibility};
use crate::models::Service;
use crate::social_network::{
    process_tags, AccessMode, Posted, PotentialPost, ReadyPost, Registration, SocialNetwork,
};

// The default character limit is 500 characters.
const MAX_LEN: usize = 500;

// All links are counted as 23 characters.
const URL_LEN: usize = 23;

pub struct Mastodon {
    pub access_mode: AccessMode,
    pub state: MastodonState,
}

impl Registration for Mastodon {
    fn register(db: &Database, client: Client) -> eyre::Result<()> {
        print!("\nInstance URL: ");
        io::stdout().flush()?;
        let mut instance = String::new();
        io::stdin().read_line(&mut instance)?;

        let instance = instance
            .trim()
            .parse()
            .wrap_err("unable to parse instance URL")?;

        let state = client::auth(client, instance)?;

        // Persist the state
        db::save_service(db, Service::Mastodon, &state)?;

        // TODO: Return the state?
        Ok(())
    }
}

impl SocialNetwork for Mastodon {
    fn service(&self) -> Service {
        Service::Mastodon
    }

    fn is_writeable(&self) -> bool {
        self.access_mode == AccessMode::ReadWrite
    }

    fn prepare_post(&self, item: &NewFeedItem) -> eyre::Result<PotentialPost> {
        let text = toot_text_from_post(item, MAX_LEN)
            .ok_or_else(|| eyre!("Unable to compose toot for {:?}", item))?;
        Ok(PotentialPost(text, item.guid()))
    }

    fn publish_post(&self, client: &Client, post: ReadyPost) -> eyre::Result<Posted> {
        info!("Post: {}", post.text());

        if self.is_writeable() {
            let status = NewStatus {
                status: post.text().to_string(),
                media_ids: Vec::new(),
                in_reply_to_id: None,
                sensitive: false,
                spoiler_text: None,
                visibility: Visibility::Public,
                language: None,
            };

            let _status = client::post_status(client, &self.state, &status)?;
        }
        Ok(Posted::from(post))
    }
}

// FIXME: Return result
fn toot_text_from_post(item: &NewFeedItem, max_len: usize) -> Option<String> {
    let hashtags = join_to_string::join(
        process_tags(&item.tags)
            .into_iter()
            .map(|tag| format!("#{tag}")),
    )
    .separator(" ")
    .to_string();

    let content = item
        .title
        .as_deref()
        .or(item.summary.as_deref())
        .map(Cow::from);
    let link = item.url.as_deref();

    if content.is_none() && link.is_none() {
        return None;
    }

    // Compose the toot
    let hashtags = (!hashtags.is_empty()).then(|| hashtags.as_str());
    let mut toot = assemble_text(content.as_deref(), link, hashtags);

    // TODO: Require a minimum amount of content and remove hash tags if needed
    // TODO: Do word based truncation and fall back on character based if necessary
    // Attempt to trim the content
    let toot_len = calculate_length(&toot).ok()?;
    if toot_len > max_len {
        if let Some(text) = content {
            let diff = toot_len - max_len;
            // +1 for ellipsis
            let target_len = text.graphemes(true).count().checked_sub(diff + 1)?;
            let trimmed = text
                .graphemes(true)
                .take(target_len)
                .chain(iter::once("…"))
                .collect::<String>();
            toot = assemble_text(Some(&trimmed), link, hashtags);
        } else {
            // Not enough content to trim
            return None;
        }
    }

    debug_assert!(calculate_length(&toot).unwrap() <= max_len);

    Some(toot)
}

fn calculate_length(toot: &str) -> eyre::Result<usize> {
    let normalised = process_for_length_calculation(toot)?;
    Ok(normalised.graphemes(true).count())
}

fn process_for_length_calculation(toot: &str) -> eyre::Result<String> {
    let entities = extractor::detect_entities(toot)?;
    Ok(replace_entities(toot, skip_overlapping(&entities)).collect())
}

struct SkipOverlapping<'a, 'e> {
    index: usize,
    entities: &'a [Entity<'e>],
}

fn skip_overlapping<'a, 'e>(entities: &'a [Entity<'e>]) -> SkipOverlapping<'a, 'e> {
    SkipOverlapping { index: 0, entities }
}

impl<'a, 'e> Iterator for SkipOverlapping<'a, 'e> {
    type Item = &'a Entity<'e>;

    fn next(&mut self) -> Option<Self::Item> {
        let current = self.entities.get(self.index)?;
        let prev = self
            .index
            .checked_sub(1)
            .and_then(|prev| self.entities.get(prev));
        self.index += 1;
        match prev {
            // Skip overlapping entity
            Some(prev) if prev.end() > current.start() => self.next(),
            Some(_) | None => Some(current),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.entities.len() - self.index;
        // We can potentially filter out all entities as overlapping or yield all
        // remaining as non-overlapping
        (0, Some(remaining))
    }
}

struct ReplaceEntities<'a, 'e, I> {
    string: &'a str,
    entity: Option<&'a Entity<'e>>,
    index: usize,
    inner: I,
}

impl<'a, 'e, I> Iterator for ReplaceEntities<'a, 'e, I>
where
    I: Iterator<Item = &'a Entity<'e>>,
{
    type Item = Cow<'a, str>;

    fn next(&mut self) -> Option<Self::Item> {
        // FIXME: don't call if already finished
        if self.entity.is_none() {
            self.entity = self.inner.next();
        }

        match self.entity {
            // yield text preceding entity
            Some(entity) if self.index < entity.start() => {
                let start = self.index;
                self.index = entity.start();
                self.string.get(start..self.index).map(Cow::from)
            }
            // yield replacement for entity
            Some(entity) => {
                self.index = entity.end();
                self.entity = None;
                match entity {
                    Entity::Url(_) => std::str::from_utf8(&[b'*'; URL_LEN]).ok().map(Cow::from),
                    // Only the username part of mention counts, not the domain
                    Entity::Mention(mention) => Some(Cow::from(format!("@{}", mention.username()))),
                }
            }
            // yield text after last entity
            None if self.index < self.string.len() => {
                let start = self.index;
                self.index = self.string.len();
                self.string.get(start..).map(Cow::from)
            }
            // finished
            None => None,
        }
    }
}

fn replace_entities<'a, 'e, I>(string: &'a str, entities: I) -> ReplaceEntities<'a, 'e, I> {
    ReplaceEntities {
        string,
        entity: None,
        index: 0,
        inner: entities,
    }
}

fn assemble_text(content: Option<&str>, link: Option<&str>, hashtags: Option<&str>) -> String {
    let mut text = String::new();
    if let Some(content) = content {
        text.push_str(&content);
    }
    if let Some(link) = link {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(link)
    }
    if let Some(hashtags) = hashtags {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(hashtags)
    }

    text
}

pub fn precompile_regex() {
    let _ = extractor::VALID_URL.as_str();
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

    const MAX_LEN: usize = 100;

    #[test]
    fn test_toot_text_from_post() {
        let mut item = NewFeedItem {
            guid: "text".to_string(),
            url: Some("https://example.com".to_string()),
            title: Some("This is the title of the post".to_string()),
            author: Some("Raymond Holt".to_string()),
            summary: Some("This is the summary of the post".to_string()),
            content: Some("This is the content of the post".to_string()),
            tags: vec!["tag1".to_string(), "tag2".to_string()],
            date_published: Some(
                DateTime::parse_from_rfc2822("Wed, 18 Feb 2015 23:16:09 GMT")
                    .unwrap()
                    .to_utc(),
            ),
            date_modified: None,
        };

        // All fields
        let text = toot_text_from_post(&item, MAX_LEN).unwrap();
        assert_eq!(
            text,
            "This is the title of the post\nhttps://example.com\n\n#Tag1 #Tag2"
        );

        // No tags
        item.tags = vec![];
        let text = toot_text_from_post(&item, MAX_LEN).unwrap();
        assert_eq!(text, "This is the title of the post\nhttps://example.com");

        // No tags; no URL
        item.url = None;
        let text = toot_text_from_post(&item, MAX_LEN).unwrap();
        assert_eq!(text, "This is the title of the post");

        // No URL
        item.tags = vec![
            "hello-world".to_string(),
            "spaces what".to_string(),
            "cookie-pizza".to_string(),
        ];
        let text = toot_text_from_post(&item, MAX_LEN).unwrap();
        assert_eq!(
            text,
            "This is the title of the post\n\n#HelloWorld #SpacesWhat #CookiePizza"
        );

        // No title; no URL
        item.title = None;
        let text = toot_text_from_post(&item, MAX_LEN).unwrap();
        assert_eq!(
            text,
            "This is the summary of the post\n\n#HelloWorld #SpacesWhat #CookiePizza"
        );

        // Not summary; no URL
        item.summary = None;
        let text = toot_text_from_post(&item, MAX_LEN);
        assert_eq!(text, None);

        // URL only
        item.url = Some("https://example.com/".to_string());
        item.tags = vec![];
        let text = toot_text_from_post(&item, MAX_LEN).unwrap();
        assert_eq!(text, "https://example.com/");
    }

    #[test]
    fn toot_text_from_long_post() {
        let item = NewFeedItem {
            guid: "text".to_string(),
            url: Some("https://example.com/this/is/a/very/long/url?but=it-only-counts-for-23-characters".to_string()),
            title: Some("This is the title of the post 👨‍👩‍👧‍👦. For some reason it's a really long title that is more than the limit allowed. It goes on and on. However URLs only count for 23 characters no matter how long they are.".to_string()),
            author: Some("Raymond Holt".to_string()),
            summary: Some("This is the summary of the post".to_string()),
            content: Some("This is the content of the post".to_string()),
            tags: vec!["tag1".to_string(), "tag2".to_string()],
            date_published: Some(DateTime::parse_from_rfc2822("Wed, 18 Feb 2015 23:16:09 GMT").unwrap().to_utc()),
            date_modified: None,
        };

        let text = toot_text_from_post(&item, MAX_LEN).unwrap();
        assert_eq!(text, "This is the title of the post 👨\u{200d}👩\u{200d}👧\u{200d}👦. For some reason it's a really…\nhttps://example.com/this/is/a/very/long/url?but=it-only-counts-for-23-characters\n\n#Tag1 #Tag2");
    }

    #[test]
    fn test_calculate_length() {
        // TODO
    }

    #[test]
    fn test_replace_entities() {
        let input = "Hello @test@example.com here is a link https://www.example.com/ I found it on https://www.wezm.net/ and though you'd like it.";
        let entities = extractor::detect_entities(input).unwrap();
        let segments = replace_entities(input, skip_overlapping(&entities)).collect::<Vec<_>>();

        assert_eq!(
            segments,
            &[
                "Hello ",
                "@test",
                " here is a link ",
                "***********************",
                " I found it on ",
                "***********************",
                " and though you'd like it."
            ]
        );
    }
}
