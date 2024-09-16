use std::fmt;

use chrono::{DateTime, Utc};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::str::FromStr;

#[derive(Serialize, Deserialize)]
pub struct MastodonState {
    pub client_id: String,
    pub client_secret: String,
    pub instance: String,
    pub(crate) access_token: String,
}

#[derive(Deserialize)]
pub(crate) struct Status {
    #[serde(deserialize_with = "de_id")]
    pub id: u64,
    content: String,
    pub account: Account,
    #[serde(default)]
    media_attachments: Vec<MediaAttachment>,
    pub reblog: Option<Reblog>,
    // Mastodon specifies this as "ISO 8601 Datetime". RFC3339 is a profile of that.
    #[serde(with = "rfc3339")]
    pub created_at: DateTime<Utc>,
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

mod rfc3339 {
    use std::fmt;

    use chrono::{DateTime, Utc};
    use serde::{
        de::{self, Visitor},
        Deserializer, Serialize, Serializer,
    };

    /// Serialize a `DateTime<Utc>` into RFC3339 representation.
    pub fn serialize<S: Serializer>(
        datetime: &DateTime<Utc>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        datetime.to_rfc3339().serialize(serializer)
    }

    /// Deserialize a `DateTime<Utc>` from RFC3339 representation.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Rfc3339Visitor;

        impl<'de> Visitor<'de> for Rfc3339Visitor {
            type Value = DateTime<Utc>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an RFC3339 datetime")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                DateTime::parse_from_rfc3339(value)
                    .map(|date| date.to_utc())
                    .map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_str(Rfc3339Visitor)
    }
}
