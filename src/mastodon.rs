mod client;
pub mod models;

use eyre::bail;
use models::MastodonState;
use redb::{Database, WriteTransaction};
use reqwest::Client;
use url::Url;

use crate::db::{self, Tooted};
use crate::feed::NewFeedItem;
use crate::models::Service;
use crate::social_network::{AccessMode, Registration, SocialNetwork};

// A Mastodon instance
pub struct Instance(pub Url);

pub struct Mastodon {
    pub access_mode: AccessMode,
    pub state: MastodonState,
}

impl Registration for Instance {
    fn register(&self, db: &Database, client: Client) -> eyre::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let state = runtime.block_on(client::auth(client, self.0.clone()))?;

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

    fn publish_post(&self, tx: &WriteTransaction, item: &NewFeedItem) -> eyre::Result<String> {
        let Some(status_text) = toot_text_from_post(item) else {
            bail!("Unable to compose toot for {:?}", item);
        };

        info!("Post: {}", status_text);

        if self.is_writeable() {
            // let _toot = self.client.new_status(
            //     StatusBuilder::new()
            //         .status(status_text)
            //         .visibility(Visibility::Unlisted)
            //         .build()?,
            // )?;
            todo!("post toot")
        }
        Ok(status_text)
    }

    fn mark_post_published(
        &self,
        tx: &WriteTransaction,
        service: Service,
        post: Tooted,
    ) -> eyre::Result<()> {
        if self.is_writeable() {
            db::mark_post_tooted(tx, service, post)?;
        }

        Ok(())
    }
}

fn toot_text_from_post(item: &NewFeedItem) -> Option<String> {
    // let hashtags = categories
    //     .iter()
    //     .map(|category| category.hashtag.as_str())
    //     .collect::<Vec<&str>>()
    //     .join(" ");
    let hashtags = "#todo"; // FIXME: Pull from feed item

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
    let toot = join_to_string::join([content, link, Some(hashtags)].iter().flatten())
        .separator("\n\n")
        .to_string();

    if toot.chars().count() > 500 {
        return None;
    }

    Some(toot)
}
