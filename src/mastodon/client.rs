// mod config;
// #[cfg(windows)]
// mod dirs;
// mod models;
// mod search;
// mod top_tooters;
// #[cfg(not(windows))]
// mod xdg;
//
// #[cfg(not(windows))]
// use crate::xdg as dirs;

use std::collections::HashMap;
use std::io;
use std::io::Write;

use eyre::eyre;
use log::{debug, error};
use reqwest::{Client, Response, Url};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use simple_eyre::eyre;

use crate::mastodon::models::MastodonState;
// pub use search::search;
// pub use top_tooters::top_tooters;

const SCOPES: &str = "write:statuses";
const REDIRECT_URI: &str = "urn:ietf:wg:oauth:2.0:oob";

#[derive(Deserialize)]
struct ErrorResponse {
    // error: String,
    error_description: String,
}

#[derive(Deserialize)]
struct Application {
    name: String,
    // website: Option<String>,
    // vapid_key: String,
    client_id: Option<String>,
    client_secret: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize, Serialize)]
struct Status {
    id: String,
    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

/// Perform the OAuth flow to obtain credentials
pub async fn auth(client: Client, instance: Url) -> eyre::Result<MastodonState> {
    // Register application to obtain client id and secret
    let url = instance.join("/api/v1/apps")?;
    let resp = client
        .post(url)
        .form(&[
            ("client_name", "Feed Parrot"),
            ("redirect_uris", REDIRECT_URI),
            ("scopes", SCOPES),
            ("website", "https://feedparrot.com/"),
        ])
        .send()
        .await?; // TODO: Add context info to error
    let app: Application = json_or_error(resp).await?;

    let client_id = app
        .client_id
        .ok_or_else(|| eyre!("app response is missing client id"))?;
    let client_secret = app
        .client_secret
        .ok_or_else(|| eyre!("app response is missing client secret"))?;
    debug!("Got application: {}, ID: {}", app.name, client_id);

    // Show the approval page
    let mut url = instance.join("/oauth/authorize")?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", REDIRECT_URI)
        .append_pair("scope", SCOPES);
    println!(
        "\nOpen this page in your browser and paste the code:\n{}",
        url
    );
    print!("\nCode: ");
    io::stdout().flush()?;
    let mut code = String::new();
    io::stdin().read_line(&mut code)?;

    let code = code.trim();
    if code.is_empty() {
        return Err(eyre!("code is required"));
    }

    // Use client id, secret, and code to get a token
    let url = instance.join("/oauth/token")?;
    let resp = client
        .post(url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", client_id.as_str()),
            ("client_secret", &client_secret),
            ("redirect_uri", REDIRECT_URI),
            ("scope", SCOPES),
        ])
        .send()
        .await?; // TODO: Add context info to error
    let token_resp: TokenResponse = json_or_error(resp).await?;
    debug!("Got token");

    // Save the token (and client credentials)
    let state = MastodonState {
        client_id,
        client_secret,
        instance,
        access_token: token_resp.access_token,
    };

    Ok(state)
}

// pub async fn update_archive() -> eyre::Result<()> {
//     let mut config = Config::read(None)?;
//     let archive_path = Path::new(&config.archive_path);
//     let mut archive = File::options()
//         .create(true)
//         .append(true)
//         .open(archive_path)
//         .wrap_err_with(|| format!("unable to open archive at: {}", archive_path.display()))?;
//     info!("Opened archive: {}", archive_path.display());
//
//     let client = Client::builder()
//         .user_agent(format!("MArchive {}", env!("CARGO_PKG_VERSION")))
//         .build()?;
//
//     let instance = config.instance_url()?;
//     let mut url = instance.join("/api/v1/timelines/home")?;
//     let bearer_token = format!("Bearer {}", config.access_token);
//     loop {
//         url.query_pairs_mut().clear().append_pair("limit", "40");
//         if let Some(ref last_seen_id) = config.last_seen_id {
//             info!("Fetching statuses since id: {}", last_seen_id);
//             url.query_pairs_mut().append_pair("min_id", last_seen_id);
//         } else {
//             info!("Fetching new statuses");
//         }
//
//         // Fetch home timeline since the last id we have
//         let resp = client
//             .get(url.clone())
//             .header(AUTHORIZATION, &bearer_token)
//             .send()
//             .await?;
//         let statuses: Vec<Status> = json_or_error(resp).await?;
//         info!("Read {} statuses", statuses.len());
//
//         if statuses.is_empty() {
//             info!("Finished reading statuses");
//             break;
//         }
//
//         // Persist the statuses we read by appending to the archive, oldest first
//         for status in statuses.into_iter().rev() {
//             // FIXME: Ensure config is updated before exiting early from this loop
//             let json_line = serde_json::to_string(&status)?;
//             archive.write_all(json_line.as_bytes())?;
//             archive.write_all(b"\n")?;
//             config.last_seen_id = Some(status.id);
//         }
//         info!("Wrote statuses to archive");
//
//         // update the config
//         config.write()?;
//         debug!("Config updated");
//     }
//
//     Ok(())
// }

async fn json_or_error<T: DeserializeOwned>(response: Response) -> eyre::Result<T> {
    if response.status().is_success() {
        let app = response.json().await?;
        Ok(app)
    } else {
        error!("Request was unsuccessful ({})", response.status().as_u16());
        // TODO: Distinguish 4xx and 5xx responses
        let err: ErrorResponse = response.json().await?;
        Err(eyre!(err.error_description))
    }
}
