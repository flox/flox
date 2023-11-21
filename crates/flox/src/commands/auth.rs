//this module liberally borrows from github-device-flox crate
use std::time;

use anyhow::{Context, Result};
use bpaf::Bpaf;
use chrono::offset::Utc;
use chrono::{DateTime, Duration};
use flox_rust_sdk::flox::{Auth0Client, Flox};
use log::{debug, info};
use oauth2::basic::{
    BasicErrorResponse,
    BasicRevocationErrorResponse,
    BasicTokenIntrospectionResponse,
    BasicTokenType,
};
use oauth2::{
    AuthUrl,
    Client,
    ClientId,
    DeviceAuthorizationUrl,
    Scope,
    StandardDeviceAuthorizationResponse,
    StandardRevocableToken,
    StandardTokenResponse,
    TokenResponse,
    TokenUrl,
};
use serde::{Deserialize, Serialize};

use crate::commands::general::update_config;
use crate::config::Config;
use crate::subcommand_metric;

const TOKEN_INPUT_TIMEOUT: time::Duration = time::Duration::new(30, 0);

#[derive(Debug, Default, Clone, Serialize)]
pub struct Credential {
    pub token: String,
    pub expiry: String,
}

/// construct an oauth client using compile time constants or environment variables
///
/// Environment variables can be used to override the compile time constants during testing.
/// For use in production, the compile time constants should be used.
/// For multitenency, we will integrate with the config subsystem later.
fn create_oauth_client() -> Result<FloxTokenClient> {
    let auth_url = AuthUrl::new(
        std::env::var("FLOX_OAUTH_AUTH_URL").unwrap_or(env!("OAUTH_AUTH_URL").to_string()),
    )
    .context("Invalid auth url")?;
    let token_url = TokenUrl::new(
        std::env::var("FLOX_OAUTH_TOKEN_URL").unwrap_or(env!("OAUTH_TOKEN_URL").to_string()),
    )
    .context("Invalid token url")?;
    let device_auth_url = DeviceAuthorizationUrl::new(
        std::env::var("FLOX_OAUTH_DEVICE_AUTH_URL")
            .unwrap_or(env!("OAUTH_DEVICE_AUTH_URL").to_string()),
    )
    .context("Invalid device auth url")?;
    let client_id = ClientId::new(
        std::env::var("FLOX_OAUTH_CLIENT_ID").unwrap_or(env!("OAUTH_CLIENT_ID").to_string()),
    );
    let client = FloxTokenClient::new(client_id, None, auth_url, Some(token_url))
        .set_device_authorization_url(device_auth_url);
    Ok(client)
}

pub type FloxTokenClient = Client<
    BasicErrorResponse,
    StandardTokenResponse<ExtraFields, BasicTokenType>,
    BasicTokenType,
    BasicTokenIntrospectionResponse,
    StandardRevocableToken,
    BasicRevocationErrorResponse,
>;

pub async fn authorize(client: FloxTokenClient) -> Result<Credential> {
    let details: StandardDeviceAuthorizationResponse = client
        .exchange_device_code()
        .unwrap()
        .add_scope(Scope::new("openid".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .request_async(oauth2::reqwest::async_http_client)
        .await
        .context("Could not request device code")?;

    debug!("Device code details: {details:#?}");

    eprintln!(
        "Please visit {} in your browser",
        details.verification_uri().as_str()
    );
    eprintln!("And enter code: {}", details.user_code().secret());

    let token_result = client
        .exchange_device_access_token(&details)
        .request_async(
            oauth2::reqwest::async_http_client,
            tokio::time::sleep,
            Some(TOKEN_INPUT_TIMEOUT),
        )
        .await
        .context("Could not authorize via oauth")
        .unwrap();

    dbg!(token_result.extra_fields().id_token.secret());

    Ok(Credential {
        token: token_result.access_token().secret().to_string(),
        expiry: calculate_expiry(token_result.expires_in().unwrap().as_secs() as i64),
    })
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ExtraFields {
    id_token: oauth2::AccessToken,
}
impl oauth2::ExtraDeviceAuthorizationFields for ExtraFields {}
impl oauth2::ExtraTokenFields for ExtraFields {}

fn calculate_expiry(expires_in: i64) -> String {
    let expires_in = Duration::seconds(expires_in);
    let mut expiry: DateTime<Utc> = Utc::now();
    expiry += expires_in;
    expiry.to_rfc3339()
}

/// floxHub authentication commands
#[derive(Clone, Debug, Bpaf)]
pub enum Auth2 {
    /// Login to floxhub (requires an existing github account)
    #[bpaf(command)]
    Login,

    /// Logout from floxhub
    #[bpaf(command)]
    Logout,

    /// Get current username
    #[bpaf(command, hide)]
    User,
}

impl Auth2 {
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("auth2");

        let client = create_oauth_client()?;

        match self {
            Auth2::Login => {
                let cred = authorize(client)
                    .await
                    .context("Could not authorize via oauth")?;

                debug!("Credentials received: {cred:#?}");
                debug!("Writing token to config");

                update_config(
                    &flox.config_dir,
                    &flox.temp_dir,
                    "floxhub_token",
                    Some(cred.token),
                )
                .context("Could not write token to config")?;

                info!("Login successful");

                Ok(())
            },
            Auth2::Logout => {
                if config.flox.floxhub_token.is_none() {
                    info!("You are not logged in");
                    return Ok(());
                }

                update_config::<String>(&flox.config_dir, &flox.temp_dir, "floxhub_token", None)
                    .context("Could not remove token from user config")?;

                info!("Logout successful");

                Ok(())
            },
            Auth2::User => {
                let token = config.flox.floxhub_token.context("You are not logged in")?;

                let user = Auth0Client::new(env!("OAUTH_BASE_URL").to_string(), token)
                    .get_username()
                    .await
                    .context("Could not get user details")?;
                println!("{user}");
                Ok(())
            },
        }
    }
}
