use std::fmt;

use chrono::{DateTime, Utc};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::str::FromStr;
use url::Url;

#[derive(Serialize, Deserialize)]
pub struct MastodonState {
    pub client_id: String,
    pub client_secret: String,
    pub instance: Url,
    pub(crate) access_token: String,
}

#[derive(Serialize)]
pub enum Visibility {
    Public,
    Unlisted,
    Private,
    Direct,
}

#[derive(Serialize)]
pub(crate) struct NewStatus {
    /// The text content of the status.
    ///
    /// If media_ids is provided, this becomes optional.
    /// Attaching a poll is optional while status is provided.
    pub(crate) status: String,

    /// Include Attachment IDs to be attached as media.
    ///
    /// If provided, status becomes optional, and poll cannot be used.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) media_ids: Vec<String>,

    // If provided, media_ids cannot be used, and poll[expires_in] must be provided.
    // poll: Option<Poll>,
    /// ID of the status being replied to, if status is a reply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) in_reply_to_id: Option<String>,

    /// Mark status and attached media as sensitive? Defaults to false.
    pub(crate) sensitive: bool,

    /// Text to be shown as a warning or subject before the actual content.
    ///
    /// Statuses are generally collapsed behind this field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) spoiler_text: Option<String>,

    /// Sets the visibility of the posted status.
    pub(crate) visibility: Visibility,

    /// ISO 639 language code for this status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) language: Option<String>,
    // ISO 8601 Datetime at which to schedule a status.
    //
    // Providing this parameter will cause ScheduledStatus to be returned instead of Status.
    // Must be at least 5 minutes in the future.
    // #[serde(skip_serializing_if = "Option::is_none")]
    // scheduled_at: Option<DateTime<Utc>>,
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

/// Account record returned from `verify_credentials`.
///
/// https://docs.joinmastodon.org/entities/Account/#CredentialAccount
#[allow(unused)]
#[derive(Deserialize)]
pub struct CredentialAccount {
    pub id: String,
    pub username: String,
    pub acct: String,
    pub display_name: String,
    pub locked: bool,
    #[serde(default)]
    pub bot: bool,
    pub created_at: String,
    pub note: String,
    pub url: String,
    pub avatar: String,
    pub avatar_static: String,
    pub header: String,
    pub header_static: String,
    pub followers_count: i64,
    pub following_count: i64,
    pub statuses_count: i64,
    pub last_status_at: Option<String>,
    // pub(crate) source: Source,
    // pub(crate) emojis: Vec<Emoji>,
    // pub(crate) fields: Vec<Field>,
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
