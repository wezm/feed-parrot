//! A definition of JSON Feed 1.0
//! <https://www.jsonfeed.org/>

use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct JsonFeed {
    pub version: String,
    pub title: String,
    pub home_page_url: Option<String>,
    pub feed_url: Option<String>,
    pub description: Option<String>,
    pub user_comment: Option<String>,
    pub next_url: Option<String>,
    pub icon: Option<String>,
    pub favicon: Option<String>,
    pub author: Option<Author>,
    pub expired: Option<bool>,
    pub items: Vec<Item>,
}

#[derive(Deserialize, Clone)]
pub struct Author {
    pub name: Option<String>,
    pub url: Option<String>,
    pub avatar: Option<String>,
}

#[derive(Deserialize, Clone)]
pub struct Item {
    pub id: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub content_html: Option<String>,
    pub content_text: Option<String>,
    pub summary: Option<String>,
    pub image: Option<String>,
    pub date_published: Option<String>,
    pub date_modified: Option<String>,
    pub author: Option<Author>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
}

#[derive(Deserialize, Clone)]
pub struct Attachment {
    pub url: String,
    pub mime_type: String,
    pub title: Option<String>,
    pub size_in_bytes: Option<u64>,
    pub duration_in_seconds: Option<u64>,
}
