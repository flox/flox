use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
//this module liberally borrows from github-device-flox crate
use std::time;

use anyhow::{bail, Context, Result};
use bpaf::Bpaf;
use chrono::offset::Utc;
use chrono::{DateTime, Duration};
use flox_rust_sdk::flox::{Auth0Client, Flox};
use indoc::eprintdoc;
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
use tokio::process::Command;
use url::Url;

use crate::commands::general::update_config;
use crate::config::Config;
use crate::subcommand_metric;

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

pub async fn authorize(client: BasicClient) -> Result<Credential> {
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

    let opener = match std::env::consts::OS {
        "linux" => "xdg-open",
        "macos" => "open",
        sys => {
            bail!("Unsupported OS: {sys}")
        },
    };

    let opener_exists = std::env::split_paths(&std::env::var("PATH").unwrap_or_default())
        .any(|p| p.ends_with(opener));

    let done = Arc::new(AtomicBool::default());

    if opener_exists {
        eprintdoc! {"
            Verification Code: {code}

            Press [enter] to open {url} in your browser and verify you see the code above.
            ",
            url = details.verification_uri().url().host_str().unwrap_or(details.verification_uri()),
            code = details.user_code().secret()
        };

        debug!(
            "Waiting for user to enter code (timeout: {}s)",
            details.expires_in().as_secs()
        );

        // in the background listen for `[enter]` key presses
        // if the user presses enter, open the browser using the system default opener
        // on linux this should be `xdg-open`
        // on macos this should be `open`
        //
        // I'm not sure this is the best way to do this, but it works for now.
        // Particularly, if the user opens the URL manually,
        // there will be one less newline in the terminal than expected.
        fun_name(
            &done,
            opener.to_string(),
            details
                .verification_uri_complete()
                .map(|u| u.secret())
                .cloned(),
            details.verification_uri().url().clone(),
        );
    } else {
        eprintdoc! {"
            First copy your one-time code: {code}

            Then visit {url} in your browser to continue...
            ",
            url = details.verification_uri().url().host_str().unwrap_or(details.verification_uri()),
            code = details.user_code().secret()
        };
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

fn fun_name(done: &Arc<AtomicBool>, opener: String, complete: Option<String>, fallback: Url) {
    let done_clone = done.clone();
    tokio::task::spawn(async move {
        loop {
            if done_clone.load(Ordering::Relaxed) {
                return;
            }

            match crossterm::event::poll(time::Duration::from_millis(100)) {
                Err(_) => {
                    info!(
                        "Could not read input. Please open the following URL manually: {fallback}",
                    );
                    return;
                },
                Ok(false) => {
                    continue;
                },
                Ok(true) => match crossterm::event::read() {
                    Err(_) => {
                        info!("Could not read input. Please open the following URL manually: {fallback}",);
                        return;
                    },
                    Ok(crossterm::event::Event::Key(key))
                        if key.code == crossterm::event::KeyCode::Enter =>
                    {
                        break;
                    },
                    _ => {
                        continue;
                    },
                },
            }
        }

        let mut command = Command::new(opener);
        command.arg(complete.unwrap_or(fallback.to_string()));

        if command.spawn().is_err() {
            info!("Could not open browser. Please open the following URL manually: {fallback}",);
        }
    });
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
    let cred = authorize(client)
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
