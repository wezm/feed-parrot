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
