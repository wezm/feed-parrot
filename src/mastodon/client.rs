use std::collections::HashMap;
use std::io;
use std::io::Write;

use eyre::eyre;
use log::{debug, error};
use reqwest::blocking::{Client, Response};
use reqwest::header::AUTHORIZATION;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use simple_eyre::eyre;
use url::Url;

use crate::mastodon::models::{MastodonState, NewStatus};

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
pub fn auth(client: Client, instance: Url) -> eyre::Result<MastodonState> {
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
        .send()?; // TODO: Add context info to error
    let app: Application = json_or_error(resp)?;

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
        .send()?; // TODO: Add context info to error
    let token_resp: TokenResponse = json_or_error(resp)?;
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

pub fn post_status(
    client: &Client,
    state: &MastodonState,
    status: &NewStatus,
) -> eyre::Result<super::models::Status> {
    let url = state.instance.join("/api/v1/statuses")?;
    let bearer_token = format!("Bearer {}", state.access_token);
    let idempotency_key = "TODO";

    let resp = client
        .post(url.clone())
        .header(AUTHORIZATION, &bearer_token)
        .json(status)
        .send()?;
    let xstatus: super::models::Status = json_or_error(resp)?;
    Ok(xstatus)
}

fn json_or_error<T: DeserializeOwned>(response: Response) -> eyre::Result<T> {
    if response.status().is_success() {
        let app = response.json()?;
        Ok(app)
    } else {
        error!("Request was unsuccessful ({})", response.status().as_u16());
        // TODO: Distinguish 4xx and 5xx responses
        let err: ErrorResponse = response.json()?;
        Err(eyre!(err.error_description))
    }
}
