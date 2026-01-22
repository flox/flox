use std::path::PathBuf;
use std::str::FromStr;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use serde_with::DeserializeFromStr;
use thiserror::Error;
use url::Url;

use crate::data::FloxVersion;
pub use crate::models::environment_ref::{self, *};
use crate::providers::{catalog, flake_installable_locker};

pub const FLOX_VERSION_VAR: &str = "FLOX_VERSION";

/// The Flox version, this is crate is part of.
/// This is _also_ used to determine the version of the CLI.
/// The version is determined by the following rules:
/// 1. `github:flox/flox#flox`, provides a wrapper that sets `FLOX_VERSION`.
///    This is the main production artifact and canonical version.
/// 2. Our `just` targets will set `FLOX_VERSION` using the current git tag,
///    so `just` builds will have the correct updated version _with_ git metadata.
/// 3. `cargo build` when run outside of `just` will fallback to `0.0.0-dirty`.
///    This is the version that also local IDEs / rust-analyzer will use.
///    However, binaries built this way may fail to run in some cases,
///    e.g. `containerize` on macos which relies on the flox version.
pub static FLOX_VERSION_STRING: LazyLock<String> = LazyLock::new(|| {
    // Runtime provided version,
    // i.e. the flox cli wrapper of the nix built production flox package.
    if let Ok(version) = std::env::var(FLOX_VERSION_VAR) {
        return version;
    };

    // Buildtime provided version, i.e. `just build-flox`.
    // Macro requires string literal rather than const.
    if let Some(version) = option_env!("FLOX_VERSION") {
        return version.to_string();
    }

    // Fallback to dev version, to allow building without just,
    // and default configurations in IDEs.
    "0.0.0-dirty".to_string()
});
pub static FLOX_VERSION: LazyLock<FloxVersion> = LazyLock::new(|| {
    let Ok(version) = (*FLOX_VERSION_STRING).parse() else {
        // Production builds won't panic since we run `flox --version` in pkgs/flox/default.nix.
        panic!(
            "Version '{version}' can not be parsed",
            version = *FLOX_VERSION_STRING
        )
    };
    version
});
pub static FLOX_SENTRY_ENV: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("FLOX_SENTRY_ENV").ok());

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

    /// Token to authenticate with FloxHub.
    /// It's usually populated from the config during [Flox] initialization.
    /// Checking for [None] can be used to check if the use is logged in.
    pub floxhub_token: Option<FloxhubToken>,

    pub catalog_client: catalog::Client,
    pub installable_locker: flake_installable_locker::InstallableLockerImpl,

    /// Feature flags
    pub features: Features,

    pub verbosity: i32,
}

impl Flox {}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
pub struct Features {
    #[serde(default)]
    pub upload: bool,
    #[serde(default)]
    pub qa: bool,
    #[serde(default)]
    pub outputs: bool,
}

pub static DEFAULT_FLOXHUB_URL: LazyLock<Url> =
    LazyLock::new(|| Url::parse("https://hub.flox.dev").unwrap());

/// Assertions about the owner of this token
#[derive(Debug, Clone, Deserialize)]
struct FloxTokenClaims {
    /// The FloxHub handle of the user this token belongs to
    #[serde(rename = "https://flox.dev/handle")]
    handle: String,
    /// The expiration time of the token (Unix timestamp)
    exp: usize,
}

/// A token authenticating a user with FloxHub
#[derive(Debug, Clone, DeserializeFromStr)]
pub struct FloxhubToken {
    /// The entire token as a string
    token: String,
    /// Assertions about the identity of the token's owner
    token_data: FloxTokenClaims,
}

impl FloxhubToken {
    /// Create a new floxhub token from a string
    pub fn new(token: String) -> Result<Self, FloxhubTokenError> {
        token.parse()
    }

    /// Return the token as a string
    pub fn secret(&self) -> &str {
        &self.token
    }

    /// Return the handle of the user the token belongs to
    pub fn handle(&self) -> &str {
        &self.token_data.handle
    }

    /// Returns whether the token has expired by checking the `exp` claim
    /// against the current time.
    pub fn is_expired(&self) -> bool {
        let now = {
            let start = std::time::SystemTime::now();
            start
                .duration_since(std::time::UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs() as usize
        };
        self.token_data.exp < now
    }
}

impl Serialize for FloxhubToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.token.serialize(serializer)
    }
}

impl FromStr for FloxhubToken {
    type Err = FloxhubTokenError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Client side we don't need to verify the signature,
        // as all priviledged access is guarded server side.
        // We still decode the token to extract claims like handle and expiration.

        let token = jsonwebtoken::dangerous::insecure_decode::<FloxTokenClaims>(s)
            .map_err(FloxhubTokenError::InvalidToken)?;

        Ok(FloxhubToken {
            token: s.to_string(),
            token_data: token.claims,
        })
    }
}

#[derive(Debug, Clone, Error, Eq, PartialEq)]
pub enum FloxhubTokenError {
    #[error("invalid token")]
    InvalidToken(#[source] jsonwebtoken::errors::Error),
}

#[derive(Debug, Clone)]
pub struct Floxhub {
    base_url: Url,
    git_url: Url,
    git_url_overridden: bool,
}

impl Floxhub {
    pub fn new(base_url: Url, git_url_override: Option<Url>) -> Result<Self, FloxhubError> {
        let git_url_overridden = git_url_override.is_some();
        let git_url = match git_url_override {
            Some(git_url_override) => git_url_override,
            None => Self::derive_git_url(&base_url)?,
        };
        Ok(Floxhub {
            base_url,
            git_url,
            git_url_overridden,
        })
    }

    /// Return the base url of the FloxHub instance
    /// might change to a more specific url in the future
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub fn git_url_override(&self) -> Option<&Url> {
        self.git_url_overridden.then_some(&self.git_url)
    }

    /// Return the url of the FloxHub git interface
    ///
    /// If the environment variable `_FLOX_FLOXHUB_GIT_URL` is set,
    /// it will be used instead of the derived FloxHub host.
    /// This is useful for testing FloxHub locally.
    pub fn git_url(&self) -> &Url {
        &self.git_url
    }

    fn derive_git_url(base_url: &Url) -> Result<Url, FloxhubError> {
        let mut git_url = base_url.clone();
        let host = git_url
            .host_str()
            .ok_or(FloxhubError::NoHost(base_url.to_string()))?;
        let without_hub = host
            .strip_prefix("hub.")
            .ok_or(FloxhubError::NoHubPrefix(base_url.to_string()))?;
        let with_api_prefix = format!("api.{}", without_hub);
        git_url
            .set_host(Some(&with_api_prefix))
            .map_err(|e| FloxhubError::InvalidFloxhubBaseUrl(with_api_prefix, e))?;
        git_url.set_path("git");
        Ok(git_url)
    }
}

#[derive(Error, Debug)]
pub enum FloxhubError {
    #[error("Invalid FloxHub URL: '{0}'. URL must contain a host.")]
    NoHost(String),
    #[error("Invalid FloxHub URL: '{0}'. URL must begin with 'hub.'")]
    NoHubPrefix(String),
    #[error("Couldn't set git URL host to '{0}'")]
    InvalidFloxhubBaseUrl(String, #[source] url::ParseError),
}

pub mod test_helpers {
    use std::fs;

    use tempfile::{TempDir, tempdir_in};

    use self::catalog::MockClient;
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

    pub fn test_token_from_floxhub_test_users_file(user: PublishTestUser) -> FloxhubToken {
        let idx = match user {
            PublishTestUser::WithCatalogs => 0,
            PublishTestUser::NoCatalogs => 1,
        };
        let test_user_file_path = UNIT_TEST_GENERATED
            .parent()
            .unwrap()
            .join("floxhub_test_users.json");
        let contents =
            std::fs::read_to_string(test_user_file_path).expect("couldn't open test user file");
        let json: serde_json::Value =
            serde_json::from_str(&contents).expect("couldn't parse test user file");
        let token = json
            .get(idx)
            .and_then(|obj| obj.get("token"))
            .expect("couldn't extract token from test user file")
            .as_str()
            .unwrap()
            .to_string();
        // Parse the token to extract claims (including exp) from the JWT
        FloxhubToken::from_str(&token).expect("couldn't parse test user token")
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
                git_url_override,
            )
            .unwrap(),
            floxhub_token: None,
            catalog_client: MockClient::default().into(),
            installable_locker: InstallableLockerImpl::Mock(InstallableLockerMock::new()),
            features: Default::default(),
            verbosity: 0,
        };

        (flox, tempdir_handle)
    }
}

#[cfg(test)]
pub mod tests {
    use std::str::FromStr;

    use super::*;

    /// A fake FloxHub token
    ///
    /// {
    ///  "typ": "JWT",
    ///  "alg": "HS256"
    /// }
    /// .
    /// {
    ///   "https://flox.dev/handle": "test"
    ///   "exp": 9999999999,                // 2286-11-20T17:46:39+00:00
    /// }
    /// .
    /// AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
    const FAKE_TOKEN: &str = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjk5OTk5OTk5OTl9.6-nbzFzQEjEX7dfWZFLE-I_qW2N_-9W2HFzzfsquI74";

    /// A fake floxhub token, that is expired
    ///
    /// {
    ///  "typ": "JWT",
    ///  "alg": "HS256"
    /// }
    /// .
    /// {
    ///   "https://flox.dev/handle": "test"
    ///   "exp": 1704063600,                // 2024-01-01T00:00:00+00:00
    /// }
    /// .
    /// AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
    const FAKE_EXPIRED_TOKEN: &str = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjE3MDQwNjM2MDB9.-5VCofPtmYQuvh21EV1nEJhTFV_URkRP0WFu4QDPFxY";

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

    #[test]
    fn test_derive_git_url() {
        assert_eq!(
            Floxhub::derive_git_url(&Url::from_str("https://hub.flox.dev").unwrap()).unwrap(),
            Url::from_str("https://api.flox.dev/git").unwrap()
        );
    }

    #[test]
    fn test_derive_git_url_dev() {
        assert_eq!(
            Floxhub::derive_git_url(&Url::from_str("https://hub.preview.flox.dev").unwrap())
                .unwrap(),
            Url::from_str("https://api.preview.flox.dev/git").unwrap()
        );
    }
}
