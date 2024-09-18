mod client;
pub mod models;

use eyre::eyre;
use models::MastodonState;
use redb::Database;
use reqwest::blocking::Client;
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

    let content = item
        .title
        .as_deref()
        .or(item.summary.as_deref())
        .or(item.content.as_deref());
    let link = item.url.as_deref();

    if content.is_none() && link.is_none() {
        return None;
    }

    // The default character limit is 500 characters.
    // All links are counted as 23 characters.

    // Compose the toot
    let hashtags = (!hashtags.is_empty()).then(|| hashtags.as_str());
    let toot = join_to_string::join([content, link, hashtags].iter().flatten())
        .separator("\n\n")
        .to_string();

    // FIXME: Do a proper length check and truncate if needed
    if toot.chars().count() > 500 {
        return None;
    }

    Some(toot)
}
