use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};

use crate::ErrorMessage;

// use crate::schema::posts;

// #[derive(Identifiable, Queryable)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub twitter_url: Option<String>,
    pub mastodon_url: Option<String>,
    pub author: String,
    pub summary: String,
    pub tweeted_at: Option<DateTime<Utc>>,
    pub tooted_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// #[derive(Queryable)]
pub struct PostCategory {
    pub id: i64,
    pub post_id: i64,
    pub category_id: i16,
}

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum Service {
    Mastodon = 1,
    Twitter = 2,
}

pub enum Services {
    All,
    Specific(Vec<Service>),
}

impl TryFrom<u8> for Service {
    type Error = ErrorMessage;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Service::Mastodon),
            2 => Ok(Service::Twitter),
            _ => Err(ErrorMessage("invalid service number".into())),
        }
    }
}

impl FromStr for Service {
    type Err = ErrorMessage;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mastodon" => Ok(Service::Mastodon),
            "twitter" => Ok(Service::Twitter),
            _ => Err(ErrorMessage(format!("'{s}' is not a known service"))),
        }
    }
}

impl fmt::Display for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Service::Mastodon => f.write_str("Mastodon"),
            Service::Twitter => f.write_str("Twitter"),
        }
    }
}
