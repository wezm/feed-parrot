mod client;
mod models;

use std::rc::Rc;

use redb::Database;
use reqwest::Client;
use url::Url;

use crate::categories::Category;
use crate::db;
use crate::models::{Post, Service};
use crate::social_network::{AccessMode, SocialNetwork};

pub struct Mastodon {
    pub access_mode: AccessMode,
    pub instance: Url,
}

impl SocialNetwork for Mastodon {
    fn register(&self, db: &Database, client: Client) -> eyre::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let state = runtime.block_on(client::auth(client, self.instance.clone()))?;

        // Persist the state
        db::save_service(db, Service::Mastodon, &state)?;

        // TODO: Return the state
        Ok(())
    }

    fn publish_post(&self, post: &Post, categories: &[Rc<Category>]) -> eyre::Result<()> {
        // if let Some(status_url) = &post.mastodon_url {
        //     // Need to reblog this status. Doing so requires knowing the id of the status on the
        //     // instance on which it will be reblogged from. It appears the only way to turn
        //     // a status URL into an ID is via search.
        //     info!("Searching for {}", status_url);
        //     let resolve = true; // Attempt WebFinger look-up
        //     let results = self.client.search_v2(status_url, resolve)?;
        //     if let Some(status) = results
        //         .statuses
        //         .iter()
        //         .find(|status| status.url.as_ref() == Some(status_url))
        //     {
        //         info!("🔁 Boost {}", status_url);
        //         if self.is_read_write() {
        //             self.client.reblog(&status.id)?;
        //         }
        //     } else {
        //         return Err(ErrorMessage(format!(
        //             "Unable to find status {}, got {} search results",
        //             status_url,
        //             results.statuses.len()
        //         ))
        //         .into());
        //     }
        // } else {
        //     let status_text = toot_text_from_post(post, categories);
        //     info!("Toot {}", status_text);
        //
        //     if self.is_read_write() {
        //         let _toot = self.client.new_status(
        //             StatusBuilder::new()
        //                 .status(status_text)
        //                 .visibility(Visibility::Unlisted)
        //                 .build()?,
        //         )?;
        //     }
        // }
        //
        // Ok(())
        todo!()
    }

    // fn mark_post_published(&self, connection: &PgConnection, post: Post) -> QueryResult<()> {
    //     if self.is_read_write() {
    //         db::mark_post_tooted(connection, post)?;
    //     }
    //
    //     Ok(())
    // }
}

impl Mastodon {
    fn is_read_write(&self) -> bool {
        self.access_mode == AccessMode::ReadWrite
    }
}

fn toot_text_from_post(post: &Post, categories: &[Rc<Category>]) -> String {
    let hashtags = categories
        .iter()
        .map(|category| category.hashtag.as_str())
        .collect::<Vec<&str>>()
        .join(" ");

    format!(
        "{title} by {author}: {url} #Rust {tags}",
        title = post.title,
        author = post.author,
        url = post.url,
        tags = hashtags
    )
}
