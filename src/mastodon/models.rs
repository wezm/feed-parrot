use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer};
use std::str::FromStr;
use time::OffsetDateTime;

#[derive(Deserialize)]
pub(crate) struct Status {
    #[serde(deserialize_with = "de_id")]
    pub id: u64,
    content: String,
    pub account: Account,
    #[serde(default)]
    media_attachments: Vec<MediaAttachment>,
    pub reblog: Option<Reblog>,
    #[serde(with = "time::serde::iso8601")]
    pub created_at: OffsetDateTime,
}

#[derive(Deserialize)]
struct MediaAttachment {
    description: Option<String>,
}

#[derive(Deserialize)]
pub struct Reblog {
    content: String,
}

#[derive(Deserialize)]
pub struct Account {
    pub acct: String,
}

impl Status {
    pub fn content(&self) -> &str {
        self.reblog
            .as_ref()
            .map_or(self.content.as_str(), |reblog| reblog.content.as_str())
    }

    pub fn media_descriptions(&self) -> Vec<&str> {
        self.media_attachments
            .iter()
            .filter_map(|attach| attach.description.as_deref())
            .collect()
    }
}

fn de_id<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    struct IdVisitor;

    impl<'de> Visitor<'de> for IdVisitor {
        type Value = u64;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("id")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: de::Error,
        {
            FromStr::from_str(value).map_err(de::Error::custom)
        }
    }

    deserializer.deserialize_str(IdVisitor)
}
