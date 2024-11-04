use std::collections::HashMap;
use std::io;
use std::io::Write;

use eyre::{eyre, Context};
use log::{debug, error};
use mime::APPLICATION_JSON;
use reqwest::blocking::{multipart, Client, Response};
use reqwest::header::{ACCEPT, AUTHORIZATION};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use simple_eyre::eyre;
use url::Url;

use crate::mastodon::models::{MastodonState, NewStatus};

use super::models::NewMedia;

const REDIRECT_URI: &str = "urn:ietf:wg:oauth:2.0:oob";

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
    error_description: Option<String>,
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

#[derive(Deserialize)]
struct OAuthServerConfiguration {
    scopes_supported: Vec<String>,
}

/// Perform the OAuth flow to obtain credentials
pub fn auth(client: Client, instance: Url) -> eyre::Result<MastodonState> {
    // The profile scope is only available on Mastodon 4.3 onwards. 4.3
    // also added an endpoint that allows checking what scopes are supported
    // so we can use this to determine whether to use `profile` or `read:accounts`.
    let mut scopes: [&str; 3] = ["write:statuses", "write:media", "read:accounts"];
    let url = instance.join("/.well-known/oauth-authorization-server")?;
    let resp = client
        .get(url)
        .header(ACCEPT, APPLICATION_JSON.essence_str())
        .send()?;
    if resp.status().is_success() {
        debug!("/.well-known/oauth-authorization-server success");
        let body = resp.text()?;
        if let Ok(oauth_config) = serde_json::from_str::<'_, OAuthServerConfiguration>(&body) {
            debug!("supported scopes: {:?}", oauth_config.scopes_supported);
            let has_profile = oauth_config
                .scopes_supported
                .iter()
                .find(|scope| scope.as_str() == "profile")
                .is_some();
            if has_profile {
                scopes[2] = "profile";
            }
        };
    }
    let scopes = scopes.join(" ");

    // Register application to obtain client id and secret
    let url = instance.join("/api/v1/apps")?;
    let resp = client
        .post(url)
        .header(ACCEPT, APPLICATION_JSON.essence_str())
        .form(&[
            ("client_name", "Feed Parrot"),
            ("redirect_uris", REDIRECT_URI),
            ("scopes", scopes.as_str()),
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
        .append_pair("scope", scopes.as_str());
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
        .header(ACCEPT, APPLICATION_JSON.essence_str())
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", client_id.as_str()),
            ("client_secret", &client_secret),
            ("redirect_uri", REDIRECT_URI),
            ("scope", scopes.as_str()),
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
    idempotency_key: &str,
) -> eyre::Result<super::models::Status> {
    let url = state.instance.join("/api/v1/statuses")?;
    let bearer_token = format!("Bearer {}", state.access_token);

    let resp = client
        .post(url.clone())
        .header(AUTHORIZATION, &bearer_token)
        .header(ACCEPT, APPLICATION_JSON.essence_str())
        // Provide this header with any arbitrary string to prevent duplicate submissions of the
        // same status. Consider using a hash or UUID generated client-side. Idempotency keys are
        // stored for up to 1 hour.
        .header("Idempotency-Key", idempotency_key)
        .json(status)
        .send()?;
    let xstatus: super::models::Status = json_or_error(resp)?;
    Ok(xstatus)
}

pub fn verify_credentials(
    client: &Client,
    state: &MastodonState,
) -> eyre::Result<super::models::CredentialAccount> {
    let url = state.instance.join("/api/v1/accounts/verify_credentials")?;
    let bearer_token = format!("Bearer {}", state.access_token);

    let resp = client
        .get(url.clone())
        .header(AUTHORIZATION, &bearer_token)
        .header(ACCEPT, APPLICATION_JSON.essence_str())
        .send()?;
    let account: super::models::CredentialAccount = json_or_error(resp)?;
    Ok(account)
}

pub fn upload_media(
    client: &Client,
    state: &MastodonState,
    media: NewMedia,
) -> eyre::Result<super::models::NewMediaAttachment> {
    let url = state.instance.join("/api/v2/media")?;
    let bearer_token = format!("Bearer {}", state.access_token);

    // We need to stream a multipart form date request
    let mut file_part = multipart::Part::file(&media.file)?;
    let mut form = multipart::Form::new();
    if let Some(desc) = media.description {
        form = form.text("description", desc);
    }
    if let Some(focus) = media.focus {
        form = form.text("focus", focus);
    }

    // If there's no extension to guess the mime type from then use the supplied type
    match (media.file.extension(), media.mime) {
        (None, Some(mime)) => file_part = file_part.mime_str(mime.as_ref())?,
        _ => {}
    }
    form = form.part("file", file_part);

    debug!("uploading media at: {}", media.file.display());
    let resp = client
        .post(url.clone())
        .header(AUTHORIZATION, &bearer_token)
        .header(ACCEPT, APPLICATION_JSON.essence_str())
        .multipart(form)
        .send()?;
    let new_media: super::models::NewMediaAttachment = json_or_error(resp)?;
    Ok(new_media)
}

fn json_or_error<T: DeserializeOwned>(response: Response) -> eyre::Result<T> {
    let status = response.status();
    let body = response.text()?;
    if status.is_success() {
        let decoded = serde_json::from_str(&body)
            .wrap_err_with(|| format!("unable to parse response: {body}"))?;
        Ok(decoded)
    } else {
        error!("Request was unsuccessful ({})", status.as_u16());
        // TODO: Distinguish 4xx and 5xx responses
        let err: ErrorResponse = serde_json::from_str(&body)
            .wrap_err_with(|| format!("unable to parse error response: {body}"))?;
        let msg = join_to_string::join(
            [Some(err.error.as_str()), err.error_description.as_deref()]
                .into_iter()
                .flatten(),
        )
        .separator(": ")
        .to_string();
        Err(eyre!(msg))
    }
}
