use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use chrono::offset::Utc;
use chrono::{DateTime, Duration};
use flox_config::{Config, FLOX_CONFIG_FILE, TokenStorageMode};
use flox_rust_sdk::flox::{FLOX_VERSION, Flox, FloxhubToken};
use floxhub_client::{AuthContext, AuthnMode};
use indoc::{formatdoc, indoc};
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
use crate::utils::credential_store::{CredentialSource, CredentialStores, TokenStorage};
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
    Login {
        /// Read a FloxHub token from PATH instead of logging in interactively (use '-' for stdin)
        #[bpaf(long("token-file"), argument("PATH"))]
        token_file: Option<PathBuf>,
        /// Store the token in plain text in flox.toml instead of the OS keyring
        #[bpaf(long("insecure-storage"))]
        insecure_storage: bool,
        /// With --insecure-storage, store plain text only for this login without
        /// changing the saved storage preference
        #[bpaf(long("once"))]
        once: bool,
    },

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
            Auth::Login {
                token_file,
                insecure_storage,
                once,
            } => {
                let span = tracing::info_span!("login");
                let _guard = span.enter();
                // `--once` only modulates whether `--insecure-storage` persists
                // the plain-text preference, so it is meaningless on its own.
                // Reject the combination rather than silently ignoring it
                // (mirrors '--generation' requiring '--copy' in 'flox pull').
                if once && !insecure_storage {
                    bail!("'--once' has no effect without '--insecure-storage'.");
                }
                match token_file {
                    Some(path) => {
                        login_with_token_file(
                            &mut flox,
                            &path,
                            insecure_storage,
                            once,
                            config.flox.floxhub_token_storage,
                        )?;
                    },
                    None => {
                        login_flox(
                            &mut flox,
                            insecure_storage,
                            once,
                            config.flox.floxhub_token_storage,
                        )
                        .await?;
                    },
                }
                Ok(())
            },
            Auth::Logout => {
                let span = tracing::info_span!("logout");
                let _guard = span.enter();
                if config.flox.floxhub_token.is_none() {
                    message::warning("You are not logged in");
                    return Ok(());
                }

                let stores = CredentialStores::from_flox(&flox);
                // Probe before removal: this identifies which source supplies
                // the active token, so logout can say when clearing the stores
                // is not enough to end the session.
                let source = stores.probe_source(&config);

                stores
                    .remove_all()
                    .context("Could not remove the stored token")?;

                match source {
                    CredentialSource::Env => message::warning(indoc! {"
                        Removed stored credentials, but 'FLOX_FLOXHUB_TOKEN' still supplies a token.
                        Unset 'FLOX_FLOXHUB_TOKEN' to complete the logout."}),
                    CredentialSource::SystemConfig => message::warning(indoc! {"
                        Removed stored credentials, but the system config still supplies a token.
                        Remove 'floxhub_token' from the system 'flox.toml' to complete the logout."}),
                    _ => message::updated("Logout successful"),
                }

                Ok(())
            },
            // TODO(ENT-105): handle Kerberos — show principal instead of
            // "not logged in", and explain that bearer tokens don't apply.
            Auth::Status => {
                let span = tracing::info_span!("status");
                let _guard = span.enter();

                // Check login state before probing the credential source. The
                // startup resolver may already have read the keyring; this guard
                // avoids an *additional* keyring read (and a possible unlock
                // prompt) during source probing when the user is not logged in.
                let AuthContext::Auth0(Some(token)) = &flox.auth_context else {
                    message::warning("You are not currently logged in to FloxHub.");
                    return Err(Exit(1).into());
                };

                let handle = token.handle();

                message::plain(format!(
                    "You are logged in as {handle} on {}",
                    flox.floxhub.base_url()
                ));

                let stores = CredentialStores::from_flox(&flox);
                let source = stores.probe_source(&config);

                match source.describe_storage(&stores.plaintext_path()) {
                    // Plain text is the one storage worth warning about.
                    Some(line) if source == CredentialSource::UserConfigPlaintext => {
                        message::warning(line)
                    },
                    Some(line) => message::plain(line),
                    None => {},
                }

                if config.flox.floxhub_token_storage == TokenStorageMode::Plaintext {
                    message::plain("Token storage preference is set to plain text.");
                    // Suggest the revert only when the preference actually
                    // lives in the user's own flox.toml — 'flox config
                    // --delete' cannot remove a value supplied by the
                    // environment or the system config.
                    if user_config_sets_token_storage(&flox.config_dir) {
                        message::plain(
                            "To use the keyring again, run 'flox config --delete floxhub_token_storage'.",
                        );
                    }
                }

                Ok(())
            },
            Auth::Token => {
                let span = tracing::info_span!("token");
                let _guard = span.enter();

                let AuthContext::Auth0(Some(token)) = flox.auth_context else {
                    message::warning("You are not currently logged in to FloxHub.");
                    return Err(Exit(1).into());
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
pub async fn login_flox(
    flox: &mut Flox,
    insecure_storage: bool,
    once: bool,
    storage_pref: TokenStorageMode,
) -> Result<String> {
    let client = create_oauth_client()?;
    let cred = authorize(client, flox.floxhub.base_url())
        .await
        .context("Could not authorize via oauth")?;

    debug!("Credentials received: {cred:#?}");
    debug!("Writing token to config");

    // set the token in the runtime config
    let token = FloxhubToken::new(cred.token)?;

    complete_login(flox, token, insecure_storage, once, storage_pref)
}

/// Finish a login with a fresh, validated token, shared by the interactive
/// and token-file flows: store the token in the OS keyring (falling back to
/// the plaintext config file, or forced there by `--insecure-storage`), set
/// the auth context, and report where the credential landed.
fn complete_login(
    flox: &mut Flox,
    token: FloxhubToken,
    insecure_storage: bool,
    once: bool,
    storage_pref: TokenStorageMode,
) -> Result<String> {
    let handle = token.handle().to_string();

    // `--insecure-storage` forces plain text for this login; otherwise honor the
    // standing storage preference.
    let target = if insecure_storage {
        TokenStorageMode::Plaintext
    } else {
        storage_pref
    };

    // Store the token where `target` says: the OS keyring (with a plaintext
    // fallback and warning on keyring failure) or the plaintext config file
    // (explicit 0600).
    let stores = CredentialStores::from_flox(flox);
    let storage = stores
        .persist_login_token(token.secret(), target)
        .context("Could not store token")?;

    // Persist the plain-text choice as a standing preference only when
    // `--insecure-storage` is given without `--once`. `--once` stores plain text
    // this one time without changing where future tokens go, so the token is
    // re-secured to the keyring once the keyring is available again.
    // Written only after the token store succeeded: a failed login must not
    // leave a stale plaintext preference behind, which would also suppress the
    // migration that re-secures an existing plain-text token.
    if insecure_storage && !once {
        update_config(
            &flox.config_dir,
            "floxhub_token_storage",
            Some(TokenStorageMode::Plaintext),
        )
        .context("Could not save the token-storage preference")?;
    }

    let auth_context = AuthContext::from_mode(
        &AuthnMode::Auth0,
        Some(token.secret()),
        &flox.floxhub.api_url_str(),
    )?;
    let _ = flox.set_auth_context(auth_context);

    print_login_success(&handle);

    if storage == TokenStorage::Plaintext {
        let notice = CredentialSource::plaintext_notice(&stores.plaintext_path());
        // Suggest a next step only where one exists: 'flox config --delete'
        // only works when the preference is in the user's own flox.toml, and
        // the re-secure note only holds while the standing preference is
        // still the keyring.
        if target != TokenStorageMode::Plaintext {
            // The keyring was the target but storing fell back to plain text.
            message::warning(formatdoc! {"
                {notice}
                No OS keyring is available."});
        } else if user_config_sets_token_storage(&flox.config_dir) {
            message::warning(formatdoc! {"
                {notice}
                To use the keyring instead, run 'flox config --delete floxhub_token_storage'."});
        } else if storage_pref == TokenStorageMode::Keyring {
            // Reached via '--insecure-storage --once': the standing
            // preference is untouched, so the next command re-secures the
            // token to the keyring.
            message::warning(formatdoc! {"
                {notice}
                The token will be moved to the OS keyring on the next command."});
        } else {
            // Plain-text preference supplied by the environment or the system
            // config: 'flox config --delete' cannot remove it, and no
            // migration will run while it stands.
            message::warning(notice);
        }
    }

    Ok(handle)
}

/// Whether the user's own `flox.toml` sets `floxhub_token_storage`.
///
/// The merged [Config] cannot distinguish where the preference came from, and
/// the revert suggestion in `flox auth status` is only actionable when the key
/// is in the user file: `flox config --delete` cannot remove a value supplied
/// by the environment or the system config.
fn user_config_sets_token_storage(config_dir: &Path) -> bool {
    std::fs::read_to_string(config_dir.join(FLOX_CONFIG_FILE))
        .ok()
        .and_then(|contents| contents.parse::<toml_edit::DocumentMut>().ok())
        .is_some_and(|document| document.get("floxhub_token_storage").is_some())
}

/// Print the success message shared by all login flows.
fn print_login_success(handle: &str) {
    message::updated("Authentication complete");
    message::updated(format!("Logged in as {handle}"));
}

/// Log in non-interactively with a token read from a file, or from stdin if
/// the path is `-`.
///
/// * validates the token and rejects expired tokens
/// * stores the token like an interactive login (OS keyring, plaintext
///   fallback, or forced plaintext via `--insecure-storage`)
/// * updates the auth context of the [Flox] instance
pub fn login_with_token_file(
    flox: &mut Flox,
    token_file: &Path,
    insecure_storage: bool,
    once: bool,
    storage_pref: TokenStorageMode,
) -> Result<String> {
    let contents = if token_file == Path::new("-") {
        let mut contents = String::new();
        std::io::stdin()
            .read_to_string(&mut contents)
            .context("Could not read token from stdin.")?;
        contents
    } else {
        std::fs::read_to_string(token_file)
            .with_context(|| format!("Could not read token file {}.", token_file.display()))?
    };

    let token = FloxhubToken::new(contents.trim().to_string())
        .context("The provided token is not a valid FloxHub token.")?;

    if token.is_expired() {
        bail!("The provided token is expired.\nObtain a fresh token from FloxHub and try again.");
    }

    complete_login(flox, token, insecure_storage, once, storage_pref)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use flox_config::FLOX_CONFIG_FILE;
    use flox_rust_sdk::flox::test_helpers::{create_test_token, flox_instance};
    use floxhub_client::token_test_helpers::FAKE_EXPIRED_TOKEN;

    use super::*;

    /// A token-file login persists through the credential stores like an
    /// interactive one: with the OS keyring disabled it falls back to the
    /// plaintext config file rather than writing plain text unconditionally.
    #[test]
    fn login_with_token_file_stores_valid_token() {
        temp_env::with_var("_FLOX_DISABLE_KEYRING", Some("true"), || {
            let (mut flox, _temp_dir) = flox_instance();
            let token = create_test_token("test-user");
            let token_file = flox.temp_dir.join("token");
            fs::write(&token_file, format!("{}\n", token.secret())).unwrap();

            let handle = login_with_token_file(
                &mut flox,
                &token_file,
                false,
                false,
                TokenStorageMode::Keyring,
            )
            .unwrap();

            assert_eq!(handle, "test-user");
            let config_contents =
                fs::read_to_string(flox.config_dir.join(FLOX_CONFIG_FILE)).unwrap();
            assert_eq!(
                config_contents,
                format!("floxhub_token = \"{}\"\n", token.secret())
            );
            let AuthContext::Auth0(Some(stored)) = &flox.auth_context else {
                panic!("expected an Auth0 auth context with a token");
            };
            assert_eq!(stored.secret(), token.secret());
        });
    }

    #[test]
    fn login_with_token_file_rejects_missing_file() {
        let (mut flox, _temp_dir) = flox_instance();
        let missing = flox.temp_dir.join("nonexistent");

        let err =
            login_with_token_file(&mut flox, &missing, false, false, TokenStorageMode::Keyring)
                .unwrap_err();

        assert_eq!(
            err.to_string(),
            format!("Could not read token file {}.", missing.display())
        );
        assert!(!flox.config_dir.join(FLOX_CONFIG_FILE).exists());
    }

    #[test]
    fn login_with_token_file_rejects_malformed_token() {
        let (mut flox, _temp_dir) = flox_instance();
        let token_file = flox.temp_dir.join("token");
        fs::write(&token_file, "not-a-jwt").unwrap();

        let err = login_with_token_file(
            &mut flox,
            &token_file,
            false,
            false,
            TokenStorageMode::Keyring,
        )
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "The provided token is not a valid FloxHub token."
        );
        assert!(!flox.config_dir.join(FLOX_CONFIG_FILE).exists());
    }

    #[test]
    fn login_with_token_file_rejects_expired_token() {
        let (mut flox, _temp_dir) = flox_instance();
        let token_file = flox.temp_dir.join("token");
        fs::write(&token_file, FAKE_EXPIRED_TOKEN).unwrap();

        let err = login_with_token_file(
            &mut flox,
            &token_file,
            false,
            false,
            TokenStorageMode::Keyring,
        )
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "The provided token is expired.\nObtain a fresh token from FloxHub and try again."
        );
        assert!(!flox.config_dir.join(FLOX_CONFIG_FILE).exists());
    }
}
