use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use bpaf::Bpaf;
use chrono::offset::Utc;
use chrono::{DateTime, Duration};
use flox_rust_sdk::flox::{Auth0Client, Flox};
use indoc::{eprintdoc, formatdoc};
use log::{debug, info};
use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl,
    ClientId,
    DeviceAuthorizationUrl,
    Scope,
    StandardDeviceAuthorizationResponse,
    TokenResponse,
    TokenUrl,
};
use serde::Serialize;
use url::Url;

use crate::commands::general::update_config;
use crate::config::Config;
use crate::subcommand_metric;
use crate::utils::dialog::{Checkpoint, Dialog};
use crate::utils::openers::Browser;

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
fn create_oauth_client() -> Result<BasicClient> {
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
    let client = BasicClient::new(client_id, None, auth_url, Some(token_url))
        .set_device_authorization_url(device_auth_url);
    Ok(client)
}

pub async fn authorize(client: BasicClient, floxhub_url: &Url) -> Result<Credential> {
    if !Dialog::can_prompt() {
        bail!("Cannot prompt for user input")
    }

    let details: StandardDeviceAuthorizationResponse = client
        .exchange_device_code()
        .unwrap()
        .add_scope(Scope::new("openid".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .add_extra_param(
            "audience".to_string(),
            "https://hub.flox.dev/api".to_string(),
        )
        .request_async(oauth2::reqwest::async_http_client)
        .await
        .context("Could not request device code")?;

    debug!("Device code details: {details:#?}");

    let opener = Browser::detect();

    let done = Arc::new(AtomicBool::default());

    match opener {
        Ok(opener) => {
            let message = formatdoc! {"
            First copy your one-time code: {code}

            Press enter to open {url} in your browser...
            ",
                url = floxhub_url.host_str().unwrap_or(floxhub_url.as_str()),
                code = details.user_code().secret()
            };

            debug!(
                "Waiting for user to enter code (timeout: {}s)",
                details.expires_in().as_secs()
            );

            Dialog {
                message: &message,
                help_message: None,
                typed: Checkpoint,
            }
            .checkpoint()?;

            let url = details.verification_uri().url().as_str();
            let mut command = opener.to_command();
            command.arg(url);

            if command.spawn().is_err() {
                info!("Could not open browser. Please open the following URL manually: {url}");
            }
        },
        Err(e) => {
            debug!("Unable to open browser: {e}");

            eprintdoc! {"
            Go to {url} in your browser

            Then enter your one-time code: {code}
            ",
                url = details.verification_uri().url(),
                code = details.user_code().secret()
            };
        },
    }

    let token = client
        .exchange_device_access_token(&details)
        .request_async(
            oauth2::reqwest::async_http_client,
            tokio::time::sleep,
            Some(details.expires_in()),
        )
        .await
        .context("Could not authorize via oauth")?;

    done.store(true, Ordering::Relaxed);

    Ok(Credential {
        token: token.access_token().secret().to_string(),
        expiry: calculate_expiry(token.expires_in().unwrap().as_secs() as i64),
    })
}

fn calculate_expiry(expires_in: i64) -> String {
    let expires_in = Duration::seconds(expires_in);
    let mut expiry: DateTime<Utc> = Utc::now();
    expiry += expires_in;
    expiry.to_rfc3339()
}

/// floxHub authentication commands
#[derive(Clone, Debug, Bpaf)]
pub enum Auth {
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

impl Auth {
    pub async fn handle(self, config: Config, mut flox: Flox) -> Result<()> {
        subcommand_metric!("auth2");

        match self {
            Auth::Login => {
                login_flox(&mut flox).await?;
                info!("Login successful");
                Ok(())
            },
            Auth::Logout => {
                create_oauth_client()?;

                if config.flox.floxhub_token.is_none() {
                    info!("You are not logged in");
                    return Ok(());
                }

                update_config::<String>(&flox.config_dir, &flox.temp_dir, "floxhub_token", None)
                    .context("Could not remove token from user config")?;

                info!("Logout successful");

                Ok(())
            },
            Auth::User => {
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

/// run the login flow
///
/// * updates the config file with the received token
/// * updates the floxhub_token field in the config struct
pub async fn login_flox(flox: &mut Flox) -> Result<()> {
    let client = create_oauth_client()?;
    let cred = authorize(client, flox.floxhub.base_url())
        .await
        .context("Could not authorize via oauth")?;

    debug!("Credentials received: {cred:#?}");
    debug!("Writing token to config");

    // set the token in the runtime config
    let token = flox.floxhub_token.insert(cred.token);

    // write the token to the config file
    update_config(
        &flox.config_dir,
        &flox.temp_dir,
        "floxhub_token",
        Some(token),
    )
    .context("Could not write token to config")?;

    Ok(())
}
