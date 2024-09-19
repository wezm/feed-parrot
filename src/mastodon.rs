mod client;
pub mod models;

use blake3::hash;
use eyre::eyre;
use models::MastodonState;
use redb::Database;
use reqwest::blocking::Client;
use std::borrow::Cow;
use std::iter;
use unicode_segmentation::UnicodeSegmentation;
use url::Url;

use crate::db::{self};
use crate::feed::NewFeedItem;
use crate::mastodon::models::{NewStatus, Visibility};
use crate::models::Service;
use crate::social_network::{
    process_tags, AccessMode, Posted, PotentialPost, ReadyPost, Registration, SocialNetwork,
};

// A Mastodon instance
pub struct Instance(pub Url);

pub struct Mastodon {
    pub access_mode: AccessMode,
    pub state: MastodonState,
}

impl Registration for Instance {
    fn register(&self, db: &Database, client: Client) -> eyre::Result<()> {
        let state = client::auth(client, self.0.clone())?;

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
        let text = toot_text_from_post(item)
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

fn toot_text_from_post(item: &NewFeedItem) -> Option<String> {
    let hashtags = join_to_string::join(
        process_tags(&item.tags)
            .into_iter()
            .map(|tag| format!("#{tag}")),
    )
    .separator(" ")
    .to_string();

    let mut content = item
        .title
        .as_deref()
        .or(item.summary.as_deref())
        .map(Cow::from);
    let link = item.url.as_deref();

    if content.is_none() && link.is_none() {
        return None;
    }

    // The default character limit is 500 characters.
    // All links are counted as 23 characters.

    // Compose the toot
    let hashtags = (!hashtags.is_empty()).then(|| hashtags.as_str());

    // TODO: Require a minimum amount of content and remove hash tags if needed
    // TODO: Do word based truncation and fall back on character based if necessary
    // Attempt to trim the content
    let toot_len = calculate_length(content.as_deref(), link, hashtags);
    if toot_len > 500 {
        if let Some(text) = content {
            let diff = toot_len - 500;
            // +1 for ellipsis
            let target_len = text.graphemes(true).count().checked_sub(diff + 1)?;
            let trimmed = text
                .graphemes(true)
                .take(target_len)
                .chain(iter::once("…"))
                .collect::<String>();
            content = Some(Cow::from(trimmed));
        } else {
            // Not enough content to trim
            return None;
        }
    }

    let toot = assemble_text(content.as_deref(), link, hashtags);
    Some(toot)
}

fn calculate_length(content: Option<&str>, link: Option<&str>, hashtags: Option<&str>) -> usize {
    // All links count as 23 chars
    let link = link.map(|_| std::str::from_utf8(&[b'*'; 23]).unwrap());
    let text = assemble_text(content, link, hashtags);
    text.graphemes(true).count()
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;

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
        let text = toot_text_from_post(&item).unwrap();
        assert_eq!(
            text,
            "This is the title of the post\nhttps://example.com\n\n#Tag1 #Tag2"
        );

        // No tags
        item.tags = vec![];
        let text = toot_text_from_post(&item).unwrap();
        assert_eq!(text, "This is the title of the post\nhttps://example.com");

        // No tags; no URL
        item.url = None;
        let text = toot_text_from_post(&item).unwrap();
        assert_eq!(text, "This is the title of the post");

        // No URL
        item.tags = vec![
            "hello-world".to_string(),
            "spaces what".to_string(),
            "cookie-pizza".to_string(),
        ];
        let text = toot_text_from_post(&item).unwrap();
        assert_eq!(
            text,
            "This is the title of the post\n\n#HelloWorld #SpacesWhat #CookiePizza"
        );

        // No title; no URL
        item.title = None;
        let text = toot_text_from_post(&item).unwrap();
        assert_eq!(
            text,
            "This is the summary of the post\n\n#HelloWorld #SpacesWhat #CookiePizza"
        );

        // Not summary; no URL
        item.summary = None;
        let text = toot_text_from_post(&item);
        assert_eq!(text, None);

        // URL only
        item.url = Some("https://example.com/".to_string());
        item.tags = vec![];
        let text = toot_text_from_post(&item).unwrap();
        assert_eq!(text, "https://example.com/");
    }

    #[test]
    fn toot_text_from_long_post() {
        let item = NewFeedItem {
            guid: "text".to_string(),
            url: Some("https://example.com/this/is/a/very/long/url?but=it-only-counts-for-23-characters".to_string()),
            title: Some("This is the title of the post 👨‍👩‍👧‍👦. For some reason it's a really long title that is more than the limit allowed. It goes on and on. However URLs only count for 23 characters no matter how long they are. Words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words".to_string()),
            author: Some("Raymond Holt".to_string()),
            summary: Some("This is the summary of the post".to_string()),
            content: Some("This is the content of the post".to_string()),
            tags: vec!["tag1".to_string(), "tag2".to_string()],
            date_published: Some(DateTime::parse_from_rfc2822("Wed, 18 Feb 2015 23:16:09 GMT").unwrap().to_utc()),
            date_modified: None,
        };

        let text = toot_text_from_post(&item).unwrap();
        assert_eq!(text, "This is the title of the post 👨‍👩‍👧‍👦. For some reason it's a really long title that is more than the limit allowed. It goes on and on. However URLs only count for 23 characters no matter how long they are. Words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words words wor…\nhttps://example.com/this/is/a/very/long/url?but=it-only-counts-for-23-characters\n\n#Tag1 #Tag2");
    }

    #[test]
    fn test_calculate_length() {
        // TODO
    }
}
