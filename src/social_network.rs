use redb::{Database, WriteTransaction};
use reqwest::blocking::Client;

use crate::db::Tooted;
use crate::feed::NewFeedItem;
use crate::models::Service;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    ReadOnly,
    ReadWrite,
}

pub trait Registration {
    fn register(&self, db: &Database, client: Client) -> eyre::Result<()>;
}

pub trait SocialNetwork: Send + Sync {
    // fn from_env(access_mode: AccessMode) -> Result<Self, Box<dyn Error>>;

    // fn register() -> eyre::Result<()>;
    // fn register(&self, db: &Database, client: Client) -> eyre::Result<()>;

    // fn unpublished_posts(connection: &PgConnection) -> QueryResult<Vec<Post>>;
    fn service(&self) -> Service;

    fn is_writeable(&self) -> bool;

    fn publish_post(&self, client: &Client, item: &NewFeedItem) -> eyre::Result<String>;

    fn mark_post_published(
        &self,
        tx: &WriteTransaction,
        service: Service,
        toot: Tooted,
    ) -> eyre::Result<()>;
}

/// Turn tags with spaces and dashes into PascalCase
pub fn process_tags(raw_tags: &[String]) -> Vec<String> {
    raw_tags.iter().map(|tag| process_tag(tag)).collect()
}

fn process_tag(raw_tag: &str) -> String {
    let split_chars = |c: char| c.is_ascii_punctuation() || c.is_whitespace();
    if !raw_tag.chars().any(split_chars) {
        return ucfirst(raw_tag);
    }

    join_to_string::join(raw_tag.split(split_chars).filter_map(|s| {
        if s.is_empty() {
            None
        } else {
            Some(ucfirst(s))
        }
    }))
    .separator("")
    .to_string()
}

fn ucfirst(input: &str) -> String {
    let mut chars = input.chars();
    match chars.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().chain(chars).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_tag() {
        assert_eq!(process_tag("dog-cow"), "DogCow".to_string());
        assert_eq!(process_tag("dog_cow"), "DogCow".to_string());
        assert_eq!(process_tag("dog cow"), "DogCow".to_string());
        assert_eq!(process_tag("dog\u{00A0}cow"), "DogCow".to_string());
        assert_eq!(process_tag("dog/cow"), "DogCow".to_string());
        assert_eq!(process_tag("🦜🦜🦜"), "🦜🦜🦜".to_string());
        assert_eq!(process_tag("東京カメラ部"), "東京カメラ部".to_string());
        assert_eq!(process_tag("dog--cow"), "DogCow".to_string());
        assert_eq!(process_tag("-dog-cow-"), "DogCow".to_string());
        assert_eq!(process_tag("MacOS X"), "MacOSX".to_string());
        assert_eq!(process_tag("system7"), "System7".to_string());
        assert_eq!(process_tag("introdução"), "Introdução".to_string());
        assert_eq!(process_tag("Côtes-d'Armor"), "CôtesDArmor".to_string());
        assert_eq!(process_tag("écalgrain bay"), "ÉcalgrainBay".to_string());
        assert_eq!(process_tag("καῦνος"), "Καῦνος".to_string());
        assert_eq!(process_tag("العربية"), "العربية".to_string());
    }
}
