use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use chrono::offset::Utc;
use chrono::{DateTime, Duration};
use flox_rust_sdk::flox::{FLOX_VERSION, Flox, FloxhubToken};
use indoc::formatdoc;
use oauth2::basic::{
    BasicClient,
    BasicErrorResponse,
    BasicRevocationErrorResponse,
    BasicTokenIntrospectionResponse,
    BasicTokenResponse,
};
use oauth2::{
    AuthUrl,
    ClientId,
    DeviceAuthorizationUrl,
    DeviceCodeErrorResponseType,
    EndpointNotSet,
    EndpointSet,
    RequestTokenError,
    Scope,
    StandardDeviceAuthorizationResponse,
    StandardRevocableToken,
    TokenResponse,
    TokenUrl,
};
use reqwest::redirect;
use serde::Serialize;
use tracing::{debug, instrument};
use url::Url;

use crate::commands::general::update_config;
use crate::config::Config;
use crate::utils::dialog::{Checkpoint, Dialog, WaitResult};
use crate::utils::message;
use crate::utils::openers::Browser;
use crate::{Exit, subcommand_metric};

#[derive(Debug, Default, Clone, Serialize)]
pub struct Credential {
    pub token: String,
    pub expiry: String,
}

type ConfiguredClient<
    HasAuthUrl = EndpointSet,
    HasDeviceAuthUrl = EndpointSet,
    HasIntrospectionUrl = EndpointNotSet,
    HasRevocationUrl = EndpointNotSet,
    HasTokenUrl = EndpointSet,
> = oauth2::Client<
    BasicErrorResponse,
    BasicTokenResponse,
    BasicTokenIntrospectionResponse,
    StandardRevocableToken,
    BasicRevocationErrorResponse,
    HasAuthUrl,
    HasDeviceAuthUrl,
    HasIntrospectionUrl,
    HasRevocationUrl,
    HasTokenUrl,
>;

/// construct an oauth client using compile time constants or environment variables
///
/// Environment variables can be used to override the compile time constants during testing.
/// For use in production, the compile time constants should be used.
/// For multitenancy, we will integrate with the config subsystem later.
fn create_oauth_client() -> Result<ConfiguredClient> {
    let auth_url = AuthUrl::new(
        std::env::var("_FLOX_OAUTH_AUTH_URL").unwrap_or(env!("OAUTH_AUTH_URL").to_string()),
    )
    .context("Invalid auth url")?;
    let token_url = TokenUrl::new(
        std::env::var("_FLOX_OAUTH_TOKEN_URL").unwrap_or(env!("OAUTH_TOKEN_URL").to_string()),
    )
    .context("Invalid token url")?;
    let device_auth_url = DeviceAuthorizationUrl::new(
        std::env::var("_FLOX_OAUTH_DEVICE_AUTH_URL")
            .unwrap_or(env!("OAUTH_DEVICE_AUTH_URL").to_string()),
    )
    .context("Invalid device auth url")?;
    let client_id = ClientId::new(
        std::env::var("_FLOX_OAUTH_CLIENT_ID").unwrap_or(env!("OAUTH_CLIENT_ID").to_string()),
    );
    let client = BasicClient::new(client_id)
        .set_auth_uri(auth_url)
        .set_token_uri(token_url)
        .set_device_authorization_url(device_auth_url);
    Ok(client)
}

pub async fn authorize(client: ConfiguredClient, floxhub_url: &Url) -> Result<Credential> {
    if !Dialog::can_prompt() {
        bail!("Cannot prompt for user input")
    }

    let http_client = reqwest::ClientBuilder::new()
        .redirect(redirect::Policy::none())
        .user_agent(format!("flox-cli/{}", &*FLOX_VERSION))
        .build()
        .expect("Failed to build OAuth HTTP client");

    let details: StandardDeviceAuthorizationResponse = client
        .exchange_device_code()
        .add_scope(Scope::new("openid".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .add_extra_param(
            "audience".to_string(),
            "https://hub.flox.dev/api".to_string(),
        )
        .request_async(&http_client)
        .await
        .context("Could not request device code")?;

    debug!("Device code details: {details:#?}");

    let opener = Browser::detect();

    let verification_uri = details
        .verification_uri_complete()
        .expect("Verification URI is always provided by the auth server")
        .secret()
        .as_str();
    let code = details.user_code().secret();

    // Start token polling — shared by both the browser and no-browser paths.
    let token_future = client.exchange_device_access_token(&details).request_async(
        &http_client,
        tokio::time::sleep,
        Some(details.expires_in()),
    );
    tokio::pin!(token_future);

    let token_result = match opener {
        Ok(opener) => {
            let message = formatdoc! {"
            Logging in to {url}
            Your one-time activation code is: {code}

            Open this URL in any browser:
            {verification_uri}

            Or press Enter to open your default browser...
            ",
                url = floxhub_url.host_str().unwrap_or(floxhub_url.as_str()),
            };

            debug!(
                "Waiting for user to enter code (timeout: {}s)",
                details.expires_in().as_secs()
            );

            let enter_future = Dialog {
                message: &message,
                help_message: None,
                typed: Checkpoint,
            }
            .checkpoint_async();
            tokio::pin!(enter_future);

            // Race token polling against Enter-key listening.
            //   - Enter pressed  → open the browser, then await the token
            //   - Token received → drop enter_future (RawModeGuard cleans up)
            //   - Ctrl-C         → bail with cancellation message
            tokio::select! {
                enter_result = &mut enter_future => {
                    if enter_result == WaitResult::Interrupted {
                        bail!("Authentication cancelled.");
                    }

                    let mut command = opener.to_command();
                    command.arg(verification_uri);
                    if command.spawn().is_err() {
                        message::warning(format!(
                            "Could not open browser. \
                             Please open the following URL manually: \
                             {verification_uri}"
                        ));
                    }

                    token_future.await
                },
                token_result = &mut token_future => token_result,
            }
        },
        Err(e) => {
            debug!("Unable to open browser: {e}");

            message::plain(formatdoc! {"
            Go to {verification_uri} in your browser

            Your one-time activation code is: {code}
            "
            });

            token_future.await
        },
    };

    let token = match token_result {
        Err(RequestTokenError::ServerResponse(ref r))
            if r.error() == &DeviceCodeErrorResponseType::ExpiredToken =>
        {
            bail!(
                "failed to authenticate before the device code expired. \
                 Please retry to get a new code."
            );
        },
        _ => token_result?,
    };

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

    /// Print your token to stdout
    #[bpaf(command)]
    Token,
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

                update_config::<String>(&flox.config_dir, "floxhub_token", None)
                    .context("Could not remove token from user config")?;

                message::updated("Logout successful");

                Ok(())
            },
            Auth::Status => {
                let span = tracing::info_span!("status");
                let _guard = span.enter();
                let Some(token) = flox.floxhub_token else {
                    message::warning("You are not currently logged in to FloxHub.");
                    return Err(Exit(1.into()).into());
                };

                let handle = token.handle();

                message::plain(format!(
                    "You are logged in as {handle} on {}",
                    flox.floxhub.base_url()
                ));

                Ok(())
            },
            Auth::Token => {
                let span = tracing::info_span!("token");
                let _guard = span.enter();

                let Some(token) = flox.floxhub_token else {
                    message::warning("You are not currently logged in to FloxHub.");
                    return Err(Exit(1.into()).into());
                };

                println!("{}", token.secret());
                Ok(())
            },
        }
    }
}

/// run the login flow
///
/// * updates the config file with the received token
/// * updates the floxhub_token field in the config struct
// TODO: `flox auth login` is currently Auth0-specific. It should be abstracted
// to handle different auth methods — for Kerberos, it should print a warning
// that login is not needed (Kerberos authentication is handled externally via
// `kinit`).
pub async fn login_flox(flox: &mut Flox) -> Result<&FloxhubToken> {
    let client = create_oauth_client()?;
    let cred = authorize(client, flox.floxhub.base_url())
        .await
        .context("Could not authorize via oauth")?;

    debug!("Credentials received: {cred:#?}");
    debug!("Writing token to config");

    // set the token in the runtime config
    let token = FloxhubToken::new(cred.token)?;
    let handle = token.handle().to_string();

    // write the token to the config file
    update_config(&flox.config_dir, "floxhub_token", Some(token.clone()))
        .context("Could not write token to config")?;

    flox.set_floxhub_token(token.clone())?;

    message::updated("Authentication complete");
    message::updated(format!("Logged in as {handle}"));

    Ok(flox.floxhub_token.as_ref().unwrap())
}
