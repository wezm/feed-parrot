use chrono::{DateTime, Utc};
use redb::Database;
use reqwest::blocking::Client;

use crate::db::{self, AlreadyPosted};
use crate::feed::{NewFeedItem, PostGuid};
use crate::models::Service;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    ReadOnly,
    ReadWrite,
}

pub trait Registration {
    fn register(db: &Database, client: Client) -> eyre::Result<()>;
}

/// A potential post for sending
pub struct PotentialPost(pub String, pub PostGuid);

/// A post that is ready for sending
pub struct ReadyPost {
    text: String,
    guid: PostGuid,
    hash: blake3::Hash,
}

/// A post that has been posted
pub struct Posted {
    pub text: String,
    pub guid: PostGuid,
    pub hash: blake3::Hash,
    pub at: DateTime<Utc>,
}

pub enum ValidationResult {
    Ok(ReadyPost),
    Duplicate(PotentialPost),
    Error(redb::Error),
}

impl PotentialPost {
    // FIXME: Bind the result to the service
    pub fn validate(self, db: &Database, service: Service) -> ValidationResult {
        // Check that that is a new post
        match db::already_posted(db, service, &self.0) {
            Ok(AlreadyPosted::No(hash)) => ValidationResult::Ok(ReadyPost {
                text: self.0,
                guid: self.1,
                hash,
            }),
            Ok(AlreadyPosted::Yes) => ValidationResult::Duplicate(self),
            Err(err) => ValidationResult::Error(err),
        }
    }
}

impl ReadyPost {
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn into_text(self) -> String {
        self.text
    }

    pub(crate) fn hash(&self) -> blake3::Hash {
        self.hash
    }
}

impl From<ReadyPost> for Posted {
    fn from(post: ReadyPost) -> Self {
        Posted {
            text: post.text,
            guid: post.guid,
            hash: post.hash,
            at: Utc::now(),
        }
    }
}

pub trait SocialNetwork: Send + Sync {
    fn service(&self) -> Service;

    fn is_writeable(&self) -> bool;

    fn prepare_post(&self, item: &NewFeedItem) -> eyre::Result<PotentialPost>;

    fn publish_post(&self, client: &Client, post: ReadyPost) -> eyre::Result<Posted>;
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
