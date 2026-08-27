use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use chrono::offset::Utc;
use chrono::{DateTime, Duration};
use flox_config::{Config, FLOX_CONFIG_FILE, TokenStorageMode};
use flox_events::{EventKind, EventsHub};
use flox_rust_sdk::flox::{FLOX_VERSION, Flox};
use floxhub_client::{AuthContext, AuthFailure, DiscoveredLoginConfig, discover_login_config};
use indoc::{formatdoc, indoc};
use oauth2::basic::{
    BasicClient,
    BasicErrorResponse,
    BasicRevocationErrorResponse,
    BasicTokenIntrospectionResponse,
    BasicTokenResponse,
};
use oauth2::{
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

#[derive(Default, Clone, Serialize)]
pub struct Credential {
    pub token: String,
    /// Wall-clock expiry derived from the token response's `expires_in`,
    /// which RFC 6749 makes optional; the token's own `exp` claim and /me
    /// are the authorities on expiry, so absence is not an error.
    pub expiry: Option<String>,
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credential")
            .field("token", &"***")
            .field("expiry", &self.expiry)
            .finish()
    }
}

// The device flow never touches the authorization endpoint, so the client
// carries no auth URL.
type ConfiguredClient<
    HasAuthUrl = EndpointNotSet,
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

/// Audience sent with the device-code request when the deployment serves no
/// login configuration — the value the SaaS Auth0 tenant expects, matching
/// the compiled-in endpoints used on that same path.
const FALLBACK_AUDIENCE: &str = "https://hub.flox.dev/api";

/// Where `flox auth login` runs the device grant: endpoints, client id, and
/// the audience to send (none when the deployment's document omits it).
#[derive(Debug, Clone, PartialEq)]
struct LoginConfig {
    device_auth_url: DeviceAuthorizationUrl,
    token_url: TokenUrl,
    client_id: ClientId,
    audience: Option<String>,
}

impl LoginConfig {
    fn oauth_client(&self) -> ConfiguredClient {
        BasicClient::new(self.client_id.clone())
            .set_token_uri(self.token_url.clone())
            .set_device_authorization_url(self.device_auth_url.clone())
    }
}

/// Resolve each login value by precedence: an explicit `_FLOX_OAUTH_*`
/// environment override, then the deployment's discovered document, then the
/// compiled-in constants. Env vars on top preserves the debugging escape
/// hatch, at the accepted cost that a stale override silently beats a
/// correct advertisement.
///
/// The audience has no override variable: a discovered document decides it
/// (absent member means send none — Dex has no audience concept), and only
/// the no-document path sends the compiled-in Auth0 value.
fn merge_login_config(discovered: Option<DiscoveredLoginConfig>) -> Result<LoginConfig> {
    let (device_endpoint, token_endpoint, client_id, audience) = match discovered {
        Some(discovered) => (
            Some(discovered.device_authorization_endpoint.to_string()),
            Some(discovered.token_endpoint.to_string()),
            Some(discovered.client_id),
            discovered.audience,
        ),
        None => (None, None, None, Some(FALLBACK_AUDIENCE.to_string())),
    };

    let device_auth_url = DeviceAuthorizationUrl::new(
        std::env::var("_FLOX_OAUTH_DEVICE_AUTH_URL")
            .ok()
            .or(device_endpoint)
            .unwrap_or_else(|| env!("OAUTH_DEVICE_AUTH_URL").to_string()),
    )
    .context("Invalid device auth url")?;
    let token_url = TokenUrl::new(
        std::env::var("_FLOX_OAUTH_TOKEN_URL")
            .ok()
            .or(token_endpoint)
            .unwrap_or_else(|| env!("OAUTH_TOKEN_URL").to_string()),
    )
    .context("Invalid token url")?;
    let client_id = ClientId::new(
        std::env::var("_FLOX_OAUTH_CLIENT_ID")
            .ok()
            .or(client_id)
            .unwrap_or_else(|| env!("OAUTH_CLIENT_ID").to_string()),
    );

    Ok(LoginConfig {
        device_auth_url,
        token_url,
        client_id,
        audience,
    })
}

pub async fn authorize(
    client: ConfiguredClient,
    audience: Option<&str>,
    floxhub_url: &Url,
) -> Result<Credential> {
    let http_client = reqwest::ClientBuilder::new()
        .redirect(redirect::Policy::none())
        .user_agent(format!("flox-cli/{}", &*FLOX_VERSION))
        .build()
        .expect("Failed to build OAuth HTTP client");

    let mut request = client
        .exchange_device_code()
        .add_scope(Scope::new("openid".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .add_scope(Scope::new("email".to_string()));
    // An Auth0 extension parameter; a document without the member — or a
    // Dex issuer — gets a bare RFC 8628 request.
    if let Some(audience) = audience {
        request = request.add_extra_param("audience".to_string(), audience.to_string());
    }
    let details: StandardDeviceAuthorizationResponse = request
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
        expiry: token
            .expires_in()
            .map(|expires_in| calculate_expiry(expires_in.as_secs() as i64)),
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
                        )
                        .await?;
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
            Auth::Status => {
                let span = tracing::info_span!("status");
                let _guard = span.enter();
                // Resolve the identity before probing the credential source.
                // The startup resolver may already have read the keyring; this
                // guard avoids an *additional* keyring read (and a possible
                // unlock prompt) during source probing when the user is not
                // logged in.
                match flox.get_identity().await {
                    Ok(Some(identity)) => {
                        message::plain(format!(
                            "You are logged in as {} on {}",
                            identity.handle,
                            flox.floxhub.base_url()
                        ));
                    },
                    Ok(None) => {
                        message::warning(
                            "Found a FloxHub token but could not reach FloxHub to verify it.",
                        );
                        return Err(Exit(1).into());
                    },
                    Err(AuthFailure::TokenExpired) => {
                        message::warning("Your FloxHub token is expired or has been revoked.");
                        return Err(Exit(1).into());
                    },
                    Err(_) => {
                        message::warning("You are not currently logged in to FloxHub.");
                        return Err(Exit(1).into());
                    },
                }

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

                // Any bearer credential prints — Auth0-shaped, bare, or
                // opaque alike. Kerberos carries no token, so it reports
                // as not logged in here.
                let Some(secret) = flox.auth_context.token_secret() else {
                    message::warning("You are not currently logged in to FloxHub.");
                    return Err(Exit(1).into());
                };

                println!("{secret}");
                Ok(())
            },
        }
    }
}

/// run the login flow
///
/// * updates the config file with the received token
/// * updates the floxhub_token field in the config struct
// TODO: `flox auth login` is currently OAuth-specific. It should be abstracted
// to handle different auth methods — for Kerberos, it should print a warning
// that login is not needed (Kerberos authentication is handled externally via
// `kinit`).
pub async fn login_flox(
    flox: &mut Flox,
    insecure_storage: bool,
    once: bool,
    storage_pref: TokenStorageMode,
) -> Result<String> {
    // Checked before discovery so a non-interactive invocation fails fast
    // instead of probing the network first.
    if !Dialog::can_prompt() {
        bail!("Cannot prompt for user input")
    }

    let floxhub_url = flox.floxhub.base_url();
    let discovered = discover_login_config(floxhub_url)
        .await
        .with_context(|| format!("Could not discover the login configuration for {floxhub_url}"))?;
    let login_config = merge_login_config(discovered)?;
    debug!(?login_config, "resolved login configuration");

    let cred = authorize(
        login_config.oauth_client(),
        login_config.audience.as_deref(),
        floxhub_url,
    )
    .await
    .context("Could not authorize via oauth")?;

    debug!("Credentials received: {cred:#?}");

    // Route by what the issuer minted: an Auth0-shaped JWT answers its
    // handle locally, a bare JWT or opaque token resolves it from /me. An
    // unresolvable identity fails the login — it exists to produce a usable
    // credential, and every handle consumer needs the same server.
    let auth_context = AuthContext::new_from_token(Some(&cred.token));
    let _ = flox.set_auth_context(auth_context.clone());
    let handle = match flox.get_identity().await {
        Ok(Some(identity)) => identity.handle,
        Ok(None) => bail!(indoc! {"
            Could not reach FloxHub to verify the login.
            Try again."
        }),
        Err(_) => bail!(indoc! {"
            FloxHub rejected the login token.
            Try again."
        }),
    };

    complete_login(
        flox,
        auth_context,
        handle,
        insecure_storage,
        once,
        storage_pref,
    )
}

/// Finish a login with a fresh, validated token, shared by the interactive
/// and token-file flows: store the token in the OS keyring (falling back to
/// the plaintext config file, or forced there by `--insecure-storage`), set
/// the auth context, and report where the credential landed.
fn complete_login(
    flox: &mut Flox,
    auth_context: AuthContext,
    handle: String,
    insecure_storage: bool,
    once: bool,
    storage_pref: TokenStorageMode,
) -> Result<String> {
    let secret = auth_context
        .token_secret()
        .expect("login completes with a bearer credential")
        .to_string();

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
        .persist_login_token(&secret, target)
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

    if let Err(err) = EventsHub::global().record_event_with_auth_subject(
        EventKind::CliAuthenticated {},
        flox.auth_context.user_subject(),
    ) {
        debug!(error = %err, "Failed to record v2 cli.authenticated event");
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
/// * validates the token and rejects expired (or, for a server-verified
///   token, revoked and invalid) tokens
/// * stores the token like an interactive login (OS keyring, plaintext
///   fallback, or forced plaintext via `--insecure-storage`)
/// * updates the auth context of the [Flox] instance
///
/// [AuthContext::new_from_token] routes by what the credential's claims
/// answer locally: Auth0-shaped JWTs identify themselves, bare JWTs and
/// opaque tokens resolve their identity via `/me`. No token is rejected
/// locally — validity is the server's call.
pub async fn login_with_token_file(
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

    let secret = contents.trim();

    let auth_context = AuthContext::new_from_token(Some(secret));
    let _ = flox.set_auth_context(auth_context.clone());

    // Validates locally for a JWT; via /me for a personal access token,
    // where a 401 covers expired and revoked tokens alike.
    // Elsewhere an unresolvable identity degrades to the UNKNOWN handle, but
    // login exists to verify-and-store the credential — an unverifiable
    // token is a failure here, not a success.
    let handle = match flox.get_identity().await {
        Ok(Some(identity)) if identity.is_expired() => bail!(indoc! {"
            The provided token is expired.
            Obtain a fresh token from FloxHub and try again."
        }),
        Ok(Some(identity)) => identity.handle,
        // Login exists to verify-and-store the credential — an unknown
        // identity is a failure here, not a success.
        Ok(None) => bail!(indoc! {"
            Could not reach FloxHub to verify the token.
            Try again."
        }),
        // The server rejected the token — invalid, expired, and revoked
        // alike; nothing distinguishes them locally now that any string
        // parses as a credential.
        Err(_) => bail!(indoc! {"
            FloxHub rejected the provided token: it is invalid, expired, or revoked.
            Obtain a fresh token from FloxHub and try again."
        }),
    };

    complete_login(
        flox,
        auth_context,
        handle,
        insecure_storage,
        once,
        storage_pref,
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use flox_config::FLOX_CONFIG_FILE;
    use flox_events::{EventsBuffer, EventsClient, SharedMetadataTemplate};
    use flox_rust_sdk::flox::FloxhubToken;
    use flox_rust_sdk::flox::test_helpers::{create_test_token, flox_instance};
    use floxhub_client::test_helpers::{FAKE_EXPIRED_TOKEN, FAKE_TOKEN_WITH_SUB};
    use httpmock::MockServer;
    use serial_test::serial;
    use uuid::Uuid;

    use super::*;

    #[test]
    fn credential_debug_redacts_token() {
        let credential = Credential {
            token: "synthetic-token".to_string(),
            expiry: Some("2026-08-13T12:00:00Z".to_string()),
        };

        assert_eq!(format!("{credential:#?}"), indoc! {r#"
                Credential {
                    token: "***",
                    expiry: Some(
                        "2026-08-13T12:00:00Z",
                    ),
                }"#});
    }

    struct EventsClientReset(Option<EventsClient>);

    impl Drop for EventsClientReset {
        fn drop(&mut self) {
            EventsHub::global().clear_client();
            if let Some(previous) = self.0.take() {
                EventsHub::global().set_client(previous);
            }
        }
    }

    fn install_events_client(flox: &Flox) -> EventsClientReset {
        let client = EventsClient::new(
            Uuid::new_v4(),
            &flox.data_dir,
            "",
            "",
            Uuid::new_v4(),
            Some("previous-subject".to_string()),
            SharedMetadataTemplate {
                flox_version: "0.0.0-test".to_string(),
                os_family: None,
                os_family_release: None,
                os: None,
                os_version: None,
                os_platform_version: None,
                shell: None,
                architecture: None,
                empty_flags: vec![],
                invocation_sources: vec![],
            },
        );
        EventsClientReset(EventsHub::global().set_client(client))
    }

    /// Point the Flox instance's client at a mock `/me` server.
    fn override_client(flox: &mut Flox, server: &MockServer) {
        flox.floxhub_client = floxhub_client::FloxhubClient::new(
            floxhub_client::client::test_helpers::client_config(&server.base_url()),
        )
        .unwrap();
    }

    const OAUTH_OVERRIDE_VARS: [&str; 3] = [
        "_FLOX_OAUTH_DEVICE_AUTH_URL",
        "_FLOX_OAUTH_TOKEN_URL",
        "_FLOX_OAUTH_CLIENT_ID",
    ];

    /// Run `f` with every `_FLOX_OAUTH_*` override unset.
    fn without_oauth_overrides<R>(f: impl FnOnce() -> R) -> R {
        temp_env::with_vars(OAUTH_OVERRIDE_VARS.map(|var| (var, None::<&str>)), f)
    }

    fn discovered_dex_config() -> DiscoveredLoginConfig {
        DiscoveredLoginConfig {
            client_id: "floxhub-cli".to_string(),
            audience: None,
            device_authorization_endpoint: "https://floxhub.example/dex/device/code"
                .parse()
                .unwrap(),
            token_endpoint: "https://floxhub.example/dex/token".parse().unwrap(),
        }
    }

    #[test]
    fn merge_login_config_falls_back_to_compiled_constants() {
        without_oauth_overrides(|| {
            let config = merge_login_config(None).unwrap();
            assert_eq!(config, LoginConfig {
                device_auth_url: DeviceAuthorizationUrl::new(
                    env!("OAUTH_DEVICE_AUTH_URL").to_string()
                )
                .unwrap(),
                token_url: TokenUrl::new(env!("OAUTH_TOKEN_URL").to_string()).unwrap(),
                client_id: ClientId::new(env!("OAUTH_CLIENT_ID").to_string()),
                audience: Some(FALLBACK_AUDIENCE.to_string()),
            });
        });
    }

    /// A discovered document supplies every value, and its missing audience
    /// member means "send none" — not the compiled-in Auth0 audience.
    #[test]
    fn merge_login_config_prefers_the_discovered_document() {
        without_oauth_overrides(|| {
            let config = merge_login_config(Some(discovered_dex_config())).unwrap();
            assert_eq!(config, LoginConfig {
                device_auth_url: DeviceAuthorizationUrl::new(
                    "https://floxhub.example/dex/device/code".to_string()
                )
                .unwrap(),
                token_url: TokenUrl::new("https://floxhub.example/dex/token".to_string()).unwrap(),
                client_id: ClientId::new("floxhub-cli".to_string()),
                audience: None,
            });
        });
    }

    #[test]
    fn merge_login_config_carries_the_document_audience() {
        without_oauth_overrides(|| {
            let discovered = DiscoveredLoginConfig {
                audience: Some("https://hub.preview.example/api".to_string()),
                ..discovered_dex_config()
            };
            let config = merge_login_config(Some(discovered)).unwrap();
            assert_eq!(
                config.audience,
                Some("https://hub.preview.example/api".to_string())
            );
        });
    }

    #[test]
    fn merge_login_config_prefers_env_overrides_over_the_document() {
        temp_env::with_vars(
            [
                (
                    "_FLOX_OAUTH_DEVICE_AUTH_URL",
                    Some("https://override.example/device"),
                ),
                (
                    "_FLOX_OAUTH_TOKEN_URL",
                    Some("https://override.example/token"),
                ),
                ("_FLOX_OAUTH_CLIENT_ID", Some("override-client")),
            ],
            || {
                let config = merge_login_config(Some(discovered_dex_config())).unwrap();
                assert_eq!(config, LoginConfig {
                    device_auth_url: DeviceAuthorizationUrl::new(
                        "https://override.example/device".to_string()
                    )
                    .unwrap(),
                    token_url: TokenUrl::new("https://override.example/token".to_string()).unwrap(),
                    client_id: ClientId::new("override-client".to_string()),
                    // The audience has no override variable; the document
                    // still decides it.
                    audience: None,
                });
            },
        );
    }

    /// A token-file login persists through the credential stores like an
    /// interactive one: with the OS keyring disabled it falls back to the
    /// plaintext config file rather than writing plain text unconditionally.
    #[tokio::test]
    async fn login_with_token_file_stores_valid_token() {
        temp_env::async_with_vars([("_FLOX_DISABLE_KEYRING", Some("true"))], async {
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
            .await
            .unwrap();

            assert_eq!(handle, "test-user");
            let config_contents =
                fs::read_to_string(flox.config_dir.join(FLOX_CONFIG_FILE)).unwrap();
            let config: toml::Table = toml::from_str(&config_contents).unwrap();
            assert_eq!(
                config["floxhub_token"].as_str(),
                Some(token.secret()),
                "the config stores exactly the provided token"
            );
            let AuthContext::Auth0(Some(stored)) = &flox.auth_context else {
                panic!("expected an Auth0 auth context with a token");
            };
            assert_eq!(stored.secret(), token.secret());
        })
        .await;
    }

    #[test]
    #[serial(global_events_client)]
    fn complete_login_records_fresh_auth0_subject_and_omits_pat_subject() {
        temp_env::with_var("_FLOX_DISABLE_KEYRING", Some("true"), || {
            let (mut flox, _temp_dir) = flox_instance();
            let _reset = install_events_client(&flox);

            complete_login(
                &mut flox,
                AuthContext::Auth0(Some(
                    FloxhubToken::new(FAKE_TOKEN_WITH_SUB.to_string()).expect("token parses"),
                )),
                "test".to_string(),
                false,
                false,
                TokenStorageMode::Keyring,
            )
            .expect("Auth0 login completes");
            complete_login(
                &mut flox,
                AuthContext::new_from_token(Some("flox_pat_secret")),
                "pat-user".to_string(),
                false,
                false,
                TokenStorageMode::Keyring,
            )
            .expect("PAT login completes");

            let buffer = EventsBuffer::read(&flox.data_dir).expect("read events");
            let recorded: Vec<_> = buffer
                .iter()
                .map(|event| (event.auth_subject.as_deref(), &event.kind))
                .collect();
            assert_eq!(recorded, vec![
                (Some("github|424242"), &EventKind::CliAuthenticated {},),
                (None, &EventKind::CliAuthenticated {}),
            ]);
        });
    }

    #[tokio::test]
    #[serial(global_events_client)]
    async fn failed_login_records_no_authenticated_event() {
        let (mut flox, _temp_dir) = flox_instance();
        let _reset = install_events_client(&flox);
        let token_file = flox.temp_dir.join("token");
        fs::write(&token_file, "not-a-jwt").unwrap();

        login_with_token_file(
            &mut flox,
            &token_file,
            false,
            false,
            TokenStorageMode::Keyring,
        )
        .await
        .expect_err("malformed token fails");

        let buffer = EventsBuffer::read(&flox.data_dir).expect("read events");
        assert_eq!(buffer.iter().count(), 0);
    }

    #[tokio::test]
    async fn login_with_token_file_rejects_missing_file() {
        let (mut flox, _temp_dir) = flox_instance();
        let missing = flox.temp_dir.join("nonexistent");

        let err =
            login_with_token_file(&mut flox, &missing, false, false, TokenStorageMode::Keyring)
                .await
                .unwrap_err();

        assert_eq!(
            err.to_string(),
            format!("Could not read token file {}.", missing.display())
        );
        assert!(!flox.config_dir.join(FLOX_CONFIG_FILE).exists());
    }

    /// A non-JWT token can no longer be rejected locally — it may be an
    /// issuer's opaque access token — so it is verified via /me like a PAT,
    /// and the server's rejection fails the login.
    #[tokio::test(flavor = "multi_thread")]
    async fn login_with_token_file_rejects_opaque_token_the_server_rejects() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/accounts/api/v1/accounts/me");
            then.status(401);
        });

        let (mut flox, _temp_dir) = flox_instance();
        override_client(&mut flox, &server);
        let token_file = flox.temp_dir.join("token");
        fs::write(&token_file, "not-a-jwt").unwrap();

        let err = login_with_token_file(
            &mut flox,
            &token_file,
            false,
            false,
            TokenStorageMode::Keyring,
        )
        .await
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "FloxHub rejected the provided token: it is invalid, expired, or revoked.\nObtain a fresh token from FloxHub and try again."
        );
        assert!(!flox.config_dir.join(FLOX_CONFIG_FILE).exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn login_with_token_file_stores_pat_and_resolves_handle() {
        temp_env::async_with_vars([("_FLOX_DISABLE_KEYRING", Some("true"))], async {
            let server = MockServer::start();
            server.mock(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/accounts/api/v1/accounts/me")
                    .header("authorization", "bearer flox_pat_secret");
                then.status(200).json_body(serde_json::json!({
                    "user_id": "pat|1",
                    "handle": "pat-user",
                    "expires_at": null,
                }));
            });

            let (mut flox, _temp_dir) = flox_instance();
            override_client(&mut flox, &server);
            let token_file = flox.temp_dir.join("token");
            fs::write(&token_file, "flox_pat_secret\n").unwrap();

            let handle = login_with_token_file(
                &mut flox,
                &token_file,
                false,
                false,
                TokenStorageMode::Keyring,
            )
            .await
            .unwrap();

            assert_eq!(handle, "pat-user");
            let config_contents =
                fs::read_to_string(flox.config_dir.join(FLOX_CONFIG_FILE)).unwrap();
            let config: toml::Table = toml::from_str(&config_contents).unwrap();
            assert_eq!(
                config["floxhub_token"].as_str(),
                Some("flox_pat_secret"),
                "the config stores exactly the provided token"
            );
            assert!(matches!(&flox.auth_context, AuthContext::AccessToken(_)));
        })
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn login_with_token_file_rejects_unverifiable_pat() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/accounts/api/v1/accounts/me");
            then.status(500);
        });

        let (mut flox, _temp_dir) = flox_instance();
        override_client(&mut flox, &server);
        let token_file = flox.temp_dir.join("token");
        fs::write(&token_file, "flox_pat_unverifiable").unwrap();

        let err = login_with_token_file(
            &mut flox,
            &token_file,
            false,
            false,
            TokenStorageMode::Keyring,
        )
        .await
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "Could not reach FloxHub to verify the token.\nTry again."
        );
        assert!(!flox.config_dir.join(FLOX_CONFIG_FILE).exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn login_with_token_file_rejects_revoked_pat() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/accounts/api/v1/accounts/me");
            then.status(401);
        });

        let (mut flox, _temp_dir) = flox_instance();
        override_client(&mut flox, &server);
        let token_file = flox.temp_dir.join("token");
        fs::write(&token_file, "flox_pat_revoked").unwrap();

        let err = login_with_token_file(
            &mut flox,
            &token_file,
            false,
            false,
            TokenStorageMode::Keyring,
        )
        .await
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "FloxHub rejected the provided token: it is invalid, expired, or revoked.\nObtain a fresh token from FloxHub and try again."
        );
        assert!(!flox.config_dir.join(FLOX_CONFIG_FILE).exists());
    }

    #[tokio::test]
    async fn login_with_token_file_rejects_expired_token() {
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
        .await
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "The provided token is expired.\nObtain a fresh token from FloxHub and try again."
        );
        assert!(!flox.config_dir.join(FLOX_CONFIG_FILE).exists());
    }
}
