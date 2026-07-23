use std::path::PathBuf;
use std::str::FromStr;
use std::sync::LazyLock;

use flox_core::features::Features;
use flox_core::floxhub::Floxhub;
use flox_core::vars::FLOX_VERSION_STRING;
pub use floxhub_client::{
    AccessToken,
    AuthContext,
    AuthFailure,
    FloxhubToken,
    FloxhubTokenError,
    UserIdentity,
};
use floxhub_client::{FloxhubClient, FloxhubClientError, IdentityError};
use url::Url;
use uuid::Uuid;

use crate::data::FloxVersion;
use crate::providers::flake_installable_locker;

pub static FLOX_VERSION: LazyLock<FloxVersion> = LazyLock::new(|| {
    let Ok(version) = (*FLOX_VERSION_STRING).parse() else {
        // Production builds won't panic since we run `flox --version` in pkgs/flox/default.nix.
        panic!(
            "Version '{version}' cannot be parsed",
            version = *FLOX_VERSION_STRING
        )
    };
    version
});

/// The main API struct for our flox implementation
///
/// A [Flox] instance serves as the context for nix invocations
/// and possibly other tools such as git.
/// As a CLI application one invocation of `flox` would run on the same instance
/// but may call different methods.
///
/// [Flox] will provide a preconfigured instance of the Nix API.
/// By default this nix API uses the nix CLI.
/// Preconfiguration includes environment variables and flox specific arguments.
#[derive(Debug)]
pub struct Flox {
    /// The directory pointing to the users flox configuration
    ///
    /// TODO: set a default in the lib or CLI?
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub temp_dir: PathBuf,
    pub runtime_dir: PathBuf,

    pub system: String,
    pub system_user_name: String,
    pub system_hostname: String,
    // The command used to run flox
    pub argv: Vec<String>,

    pub floxhub: Floxhub,

    /// The current authentication credential.
    pub auth_context: AuthContext,

    /// Shared HTTP client for both the catalog and factory API surfaces.
    pub floxhub_client: FloxhubClient,

    pub installable_locker: flake_installable_locker::InstallableLockerImpl,

    /// Feature flags
    pub features: Features,

    pub verbosity: i32,

    /// Device UUID for telemetry correlation.
    /// None when metrics are disabled.
    pub metrics_device_uuid: Option<Uuid>,
}

impl Flox {
    /// Set a new token and rebuild the credential to reflect it.
    ///
    /// Note: when using Kerberos authentication, the token is stored but has
    /// no effect on the credential — Kerberos does not use FloxHub tokens.
    pub fn set_auth_context(
        &mut self,
        auth_context: AuthContext,
    ) -> Result<(), FloxhubClientError> {
        self.auth_context = auth_context.clone();
        self.floxhub_client.update_config(|config| {
            config.auth_context = auth_context;
        })?;
        Ok(())
    }

    /// The identity behind the current credential — the one uniform way to
    /// answer "who is authenticated": JWT claims for Auth0, `GET
    /// /api/v1/accounts/me` for a personal access token (a successful
    /// resolution is cached for the process), and the principal for
    /// Kerberos.
    ///
    /// - `Ok(Some(identity))` — authenticated. Expiry is reported *in* the
    ///   identity ([`UserIdentity::is_expired`]), not as a failure — what
    ///   expiry means is each caller's decision.
    /// - `Ok(None)` — the identity is unknown: there is a credential, but it
    ///   could not be verified (e.g. FloxHub was unreachable). Typically not
    ///   fatal: the server stays the authority for whether the credential
    ///   actually authenticates requests, so callers usually degrade rather
    ///   than block.
    /// - `Err(failure)` — affirmatively unauthenticated: no credential
    ///   ([`AuthFailure::NotLoggedIn`]), no Kerberos ticket
    ///   ([`AuthFailure::NoKerberosTicket`]), or the server rejected the
    ///   token ([`AuthFailure::TokenExpired`]).
    pub async fn get_identity(&self) -> Result<Option<UserIdentity>, AuthFailure> {
        match &self.auth_context {
            AuthContext::Auth0(Some(token)) => Ok(Some(UserIdentity {
                handle: token.handle().to_string(),
                expires_at: Some(token.expires_at()),
            })),
            AuthContext::Auth0(None) => Err(AuthFailure::NotLoggedIn),
            AuthContext::AccessToken(token) => {
                match self.floxhub_client.resolve_identity(token).await {
                    Ok(identity) => Ok(Some(identity)),
                    Err(IdentityError::Unauthorized) => Err(AuthFailure::TokenExpired),
                    Err(_) => Ok(None),
                }
            },
            AuthContext::Kerberos(Some(material)) => Ok(Some(UserIdentity {
                handle: material.principal.clone(),
                expires_at: None,
            })),
            AuthContext::Kerberos(None) => Err(AuthFailure::NoKerberosTicket),
        }
    }
}

pub mod test_helpers {
    use std::fs;

    use flox_core::data::environment_ref::EnvironmentOwner;
    use tempfile::{TempDir, tempdir_in};

    use super::*;
    use crate::providers::catalog::test_helpers::UNIT_TEST_GENERATED;
    use crate::providers::flake_installable_locker::{
        InstallableLockerImpl,
        InstallableLockerMock,
    };
    use crate::providers::git::{GitCommandProvider, GitProvider};

    pub fn create_test_token(handle: &str) -> FloxhubToken {
        let my_claims = serde_json::json!({
        "https://flox.dev/handle": handle,
        "exp": 9999999999_i64
        });

        // my_claims is a struct that implements Serialize
        // This will create a JWT using HS256 as algorithm
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &my_claims,
            &jsonwebtoken::EncodingKey::from_secret("secret".as_ref()),
        )
        .unwrap();

        FloxhubToken::from_str(&token).unwrap()
    }

    /// Set a pre-existing token on a [Flox] instance and rebuild the auth
    /// strategy so that `auth_context.handle()` and friends see it immediately.
    pub fn set_test_token(flox: &mut Flox, token: FloxhubToken) {
        let _ = flox.set_auth_context(AuthContext::Auth0(Some(token)));
    }

    /// Set up test authentication on a [Flox] instance.
    ///
    /// Creates a test token for the given handle, sets it on the instance,
    /// and rebuilds the auth strategy so that `auth_context.handle()` and friends
    /// see the token immediately.
    pub fn set_test_auth(flox: &mut Flox, handle: &str) {
        set_test_token(flox, create_test_token(handle));
    }

    /// Describes which test user to load:
    /// - One that has an existing personal catalog and access to other test
    ///   catalogs.
    /// - No access to org catalogs, and a personal catalog that doesn't exist
    ///   yet.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum PublishTestUser {
        WithCatalogs,
        NoCatalogs,
    }

    /// Look up a token from the test-users file by handle.
    ///
    /// Panics if the handle is not found or the file cannot be read.
    pub fn test_token_for_handle(handle: &str) -> FloxhubToken {
        let test_user_file_path = UNIT_TEST_GENERATED
            .parent()
            .unwrap()
            .join("floxhub_test_users.json");
        let contents =
            std::fs::read_to_string(test_user_file_path).expect("couldn't open test user file");
        let json: serde_json::Value =
            serde_json::from_str(&contents).expect("couldn't parse test user file");
        let user = json
            .as_array()
            .expect("test user file is not an array")
            .iter()
            .find(|obj| obj.get("handle").and_then(|h| h.as_str()) == Some(handle))
            .unwrap_or_else(|| panic!("handle '{handle}' not found in test user file"));
        // Distinguish a missing handle from a found handle that lacks a token
        // rather than reporting both as "not found".
        let token = user
            .get("token")
            .unwrap_or_else(|| panic!("test user '{handle}' has no 'token' field"))
            .as_str()
            .expect("test user token is not a string")
            .to_string();
        // Parse the token to extract claims (including exp) from the JWT
        FloxhubToken::from_str(&token).expect("couldn't parse test user token")
    }

    pub fn test_token_from_floxhub_test_users_file(user: PublishTestUser) -> FloxhubToken {
        let handle = match user {
            PublishTestUser::WithCatalogs => "test1",
            PublishTestUser::NoCatalogs => "test_user_no_catalogs",
        };
        test_token_for_handle(handle)
    }

    pub fn flox_instance() -> (Flox, TempDir) {
        flox_instance_with_optional_floxhub(None)
    }

    /// If owner is None, no mock FloxHub is setup.
    /// If it is Some, a mock FloxHub with a repo for that owner will be setup,
    /// but no other owners will work.
    pub fn flox_instance_with_optional_floxhub(
        owner: Option<&EnvironmentOwner>,
    ) -> (Flox, TempDir) {
        // Use /tmp instead of std::env::temp_dir since we store sockets in runtime_dir,
        // and std::env::temp_dir may return a path that is too long.
        let tempdir_handle = tempfile::tempdir_in(PathBuf::from("/tmp")).unwrap();

        let cache_dir = tempdir_handle.path().join("caches");
        let data_dir = tempdir_handle.path().join(".local/share/flox");
        let state_dir = tempdir_handle.path().join(".local/state/flox");
        let temp_dir = tempdir_handle.path().join("temp");
        let config_dir = tempdir_handle.path().join("config");
        let runtime_dir = tempdir_handle.path().join("run");

        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&runtime_dir).unwrap();

        let git_url_override = owner.map(|owner| {
            let mock_floxhub_git_dir = tempdir_in(&temp_dir).unwrap().keep();
            let floxmeta_dir = mock_floxhub_git_dir.join(owner.as_str()).join("floxmeta");
            fs::create_dir_all(&floxmeta_dir).unwrap();
            GitCommandProvider::init(floxmeta_dir, true).unwrap();
            Url::from_directory_path(mock_floxhub_git_dir).unwrap()
        });

        let auth_context = AuthContext::new_from_token(None).expect("no token to parse");

        let flox = Flox {
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            system_user_name: "its-a-me-mario".to_string(),
            system_hostname: "mushroom-kingdom".to_string(),
            argv: vec![],
            cache_dir,
            data_dir,
            state_dir,
            temp_dir,
            config_dir,
            runtime_dir,
            floxhub: Floxhub::new(
                Url::from_str("https://hub.flox.dev").unwrap(),
                None,
                git_url_override,
            )
            .unwrap(),
            auth_context,
            floxhub_client: floxhub_client::client::test_helpers::new_noop(),
            installable_locker: InstallableLockerImpl::Mock(InstallableLockerMock::new()),
            features: Default::default(),
            verbosity: 0,
            metrics_device_uuid: None,
        };

        (flox, tempdir_handle)
    }
}

#[cfg(test)]
pub mod tests {
    use floxhub_client::test_helpers::{FAKE_EXPIRED_TOKEN, FAKE_TOKEN};

    use super::test_helpers::flox_instance;
    use super::*;

    #[tokio::test]
    async fn test_get_username() {
        let token = FloxhubToken::new(FAKE_TOKEN.to_string()).unwrap();
        assert_eq!(token.handle(), "test");
    }

    #[tokio::test]
    async fn test_detect_expired() {
        let token =
            FloxhubToken::new(FAKE_EXPIRED_TOKEN.to_string()).expect("Expired token should parse");
        assert!(token.is_expired(), "Token should be marked as expired");
        assert_eq!(token.handle(), "test", "Handle should still be accessible");
        assert!(
            !token.secret().is_empty(),
            "secret() should still return the token string for expired tokens"
        );
    }

    #[tokio::test]
    async fn get_identity_without_token_is_not_logged_in() {
        let (flox, _temp_dir) = flox_instance();
        assert!(matches!(
            flox.get_identity().await,
            Err(AuthFailure::NotLoggedIn)
        ));
    }

    #[tokio::test]
    async fn get_identity_derives_jwt_identity_from_claims() {
        let (mut flox, _temp_dir) = flox_instance();
        let token: FloxhubToken = FAKE_TOKEN.parse().unwrap();
        let expires_at = token.expires_at();
        let _ = flox.set_auth_context(AuthContext::Auth0(Some(token)));

        assert_eq!(
            flox.get_identity().await.unwrap(),
            Some(UserIdentity {
                handle: "test".to_string(),
                expires_at: Some(expires_at),
            })
        );
    }

    #[tokio::test]
    async fn get_identity_reports_expiry_in_the_identity() {
        let (mut flox, _temp_dir) = flox_instance();
        let _ = flox.set_auth_context(AuthContext::Auth0(Some(
            FAKE_EXPIRED_TOKEN.parse().unwrap(),
        )));

        let identity = flox.get_identity().await.unwrap().unwrap();
        assert_eq!(identity.handle, "test");
        assert!(identity.is_expired(), "expiry is data, not an error");
    }
}
