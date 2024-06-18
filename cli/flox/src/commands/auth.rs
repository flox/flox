use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bpaf::Bpaf;
use chrono::offset::Utc;
use chrono::{DateTime, Duration};
use flox_rust_sdk::flox::{Flox, FloxhubToken};
use indoc::formatdoc;
use log::debug;
use miette::{bail, Context, IntoDiagnostic, Result};
use oauth2::basic::BasicClient;
use oauth2::{
    AuthUrl,
    ClientId,
    DeviceAuthorizationUrl,
    DeviceCodeErrorResponseType,
    RequestTokenError,
    Scope,
    StandardDeviceAuthorizationResponse,
    TokenResponse,
    TokenUrl,
};
use serde::Serialize;
use tracing::instrument;
use url::Url;

use crate::commands::general::update_config;
use crate::config::Config;
use crate::subcommand_metric;
use crate::utils::dialog::{Checkpoint, Dialog};
use crate::utils::message;
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
        std::env::var("_FLOX_OAUTH_AUTH_URL").unwrap_or(env!("OAUTH_AUTH_URL").to_string()),
    )
    .into_diagnostic()
    .wrap_err("Invalid auth url")?;
    let token_url = TokenUrl::new(
        std::env::var("_FLOX_OAUTH_TOKEN_URL").unwrap_or(env!("OAUTH_TOKEN_URL").to_string()),
    )
    .into_diagnostic()
    .wrap_err("Invalid token url")?;
    let device_auth_url = DeviceAuthorizationUrl::new(
        std::env::var("_FLOX_OAUTH_DEVICE_AUTH_URL")
            .unwrap_or(env!("OAUTH_DEVICE_AUTH_URL").to_string()),
    )
    .into_diagnostic()
    .wrap_err("Invalid device auth url")?;
    let client_id = ClientId::new(
        std::env::var("_FLOX_OAUTH_CLIENT_ID").unwrap_or(env!("OAUTH_CLIENT_ID").to_string()),
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
        .into_diagnostic()
        .wrap_err("Could not request device code")?;

    debug!("Device code details: {details:#?}");

    let opener = Browser::detect();

    let done = Arc::new(AtomicBool::default());

    let verification_uri = details
        .verification_uri_complete()
        .expect("Verification URI is always provided by the auth server")
        .secret()
        .as_str();
    let code = details.user_code().secret();

    match opener {
        Ok(opener) => {
            let message = formatdoc! {"
            Your one-time activation code is: {code}

            Press enter to open {url} in your browser...
            ",
                url = floxhub_url.host_str().unwrap_or(floxhub_url.as_str()),
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
            .checkpoint()
            .into_diagnostic()?;

            let mut command = opener.to_command();
            command.arg(verification_uri);

            if command.spawn().is_err() {
                message::warning(format!(
                    "Could not open browser. Please open the following URL manually: {verification_uri}"
                ));
            }
        },
        Err(e) => {
            debug!("Unable to open browser: {e}");

            message::plain(formatdoc! {"
            Go to {verification_uri} in your browser

            Your one-time activation code is: {code}
            "
            });
        },
    }

    let token_result = client
        .exchange_device_access_token(&details)
        .request_async(
            oauth2::reqwest::async_http_client,
            tokio::time::sleep,
            Some(details.expires_in()),
        )
        .await;

    let token = match token_result {
        Err(RequestTokenError::ServerResponse(r))
            if r.error() == &DeviceCodeErrorResponseType::ExpiredToken =>
        {
            bail!("failed to authenticate before the device code expired. Please retry to get a new code.");
        },
        _ => token_result.into_diagnostic()?,
    };

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

// FloxHub authentication commands
#[derive(Clone, Debug, Bpaf)]
pub enum Auth {
    /// Login to FloxHub
    #[bpaf(command)]
    Login,

    /// Logout from FloxHub
    #[bpaf(command)]
    Logout,

    /// Print your current login status
    #[bpaf(command)]
    Status,
}

impl Auth {
    #[instrument(name = "auth", skip_all)]
    pub async fn handle(self, config: Config, mut flox: Flox) -> Result<()> {
        subcommand_metric!("auth2");

        match self {
            Auth::Login => {
                let span = tracing::info_span!("login");
                let _guard = span.enter();
                login_flox(&mut flox).await?;
                Ok(())
            },
            Auth::Logout => {
                let span = tracing::info_span!("logout");
                let _guard = span.enter();
                if config.flox.floxhub_token.is_none() {
                    message::warning("You are not logged in");
                    return Ok(());
                }

                update_config::<String>(&flox.config_dir, &flox.temp_dir, "floxhub_token", None)
                    .wrap_err("Could not remove token from user config")?;

                message::updated("Logout successful");

                Ok(())
            },
            Auth::Status => {
                let span = tracing::info_span!("status");
                let _guard = span.enter();
                let Some(token) = flox.floxhub_token else {
                    message::warning("You are not currently logged in to FloxHub.");
                    return Ok(());
                };

                let handle = token.handle();

                message::plain(format!(
                    "You are logged in as {handle} on {}",
                    flox.floxhub.base_url()
                ));

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
        .wrap_err("Could not authorize via oauth")?;

    debug!("Credentials received: {cred:#?}");
    debug!("Writing token to config");

    // set the token in the runtime config
    let token = flox
        .floxhub_token
        .insert(FloxhubToken::new(cred.token).into_diagnostic()?);
    let handle = token.handle();

    // write the token to the config file
    update_config(
        &flox.config_dir,
        &flox.temp_dir,
        "floxhub_token",
        Some(token.clone()),
    )
    .wrap_err("Could not write token to config")?;

    message::updated("Authentication complete");
    message::updated(format!("Logged in as {handle}"));

    Ok(())
}
