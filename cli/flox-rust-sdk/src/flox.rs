use std::path::PathBuf;
use std::str::FromStr;

use jsonwebtoken::{DecodingKey, Validation};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_with::DeserializeFromStr;
use thiserror::Error;
use url::Url;

pub use crate::models::environment_ref::{self, *};
use crate::models::search::SearchStrategy;
use crate::providers::{catalog, flox_cpp_utils};

pub static FLOX_VERSION: Lazy<String> =
    Lazy::new(|| std::env::var("FLOX_VERSION").unwrap_or(env!("FLOX_VERSION").to_string()));
pub static FLOX_SENTRY_ENV: Lazy<Option<String>> =
    Lazy::new(|| std::env::var("FLOX_SENTRY_ENV").ok());

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
    pub temp_dir: PathBuf,

    pub system: String,

    pub uuid: uuid::Uuid,

    pub floxhub: Floxhub,

    /// Token to authenticate with FloxHub.
    /// It's usually populated from the config during [Flox] initialization.
    /// Checking for [None] can be used to check if the use is logged in.
    pub floxhub_token: Option<FloxhubToken>,

    pub catalog_client: Option<catalog::Client>,
    pub installable_locker: flox_cpp_utils::InstallableLockerImpl,

    /// Feature flags
    pub features: Features,
}

impl Flox {}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Features {
    /// Which matching logic to use when searching for packages
    #[serde(default)]
    pub search_strategy: SearchStrategy,
    #[serde(default)]
    pub use_catalog: UseCatalog,

    #[serde(default)]
    pub build: bool,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, derive_more::Deref)]
pub struct UseCatalog(bool);

impl Default for UseCatalog {
    fn default() -> Self {
        UseCatalog(true)
    }
}

pub static DEFAULT_FLOXHUB_URL: Lazy<Url> =
    Lazy::new(|| Url::parse("https://hub.flox.dev").unwrap());

#[derive(Debug, Clone, Deserialize)]
struct FloxTokenClaims {
    #[serde(rename = "https://flox.dev/handle")]
    handle: String,
}

#[derive(Debug, Clone, DeserializeFromStr)]
pub struct FloxhubToken {
    token: String,
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
        let mut validation = Validation::default();
        // we're neither creating or verifying the token on the client side
        validation.insecure_disable_signature_validation();
        validation.validate_aud = false;
        let token =
            jsonwebtoken::decode::<FloxTokenClaims>(s, &DecodingKey::from_secret(&[]), &validation)
                .map_err(|e| match e.kind() {
                    jsonwebtoken::errors::ErrorKind::ExpiredSignature => FloxhubTokenError::Expired,
                    _ => FloxhubTokenError::InvalidToken(e),
                })?;

        Ok(FloxhubToken {
            token: s.to_string(),
            token_data: token.claims,
        })
    }
}

#[derive(Debug, Clone, Error, Eq, PartialEq)]
pub enum FloxhubTokenError {
    #[error("token expired")]
    Expired,

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

    use tempfile::{tempdir_in, TempDir};

    use self::catalog::MockClient;
    use super::*;
    use crate::models::environment::{global_manifest_path, init_global_manifest};
    use crate::providers::git::{GitCommandProvider, GitProvider};

    pub fn flox_instance() -> (Flox, TempDir) {
        flox_instance_with_optional_floxhub_and_client(None, true)
    }

    /// If owner is None, no mock FloxHub is setup.
    /// If it is Some, a mock FloxHub with a repo for that owner will be setup,
    /// but no other owners will work.
    pub fn flox_instance_with_optional_floxhub_and_client(
        owner: Option<&EnvironmentOwner>,
        use_client: bool,
    ) -> (Flox, TempDir) {
        // Use /tmp instead of std::env::temp_dir since we store sockets in cache_dir,
        // and std::env::temp_dir may return a path that is too long.
        let tempdir_handle = tempfile::tempdir_in(PathBuf::from("/tmp")).unwrap();

        let cache_dir = tempdir_handle.path().join("caches");
        let data_dir = tempdir_handle.path().join(".local/share/flox");
        let temp_dir = tempdir_handle.path().join("temp");
        let config_dir = tempdir_handle.path().join("config");

        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();

        let git_url_override = owner.map(|owner| {
            let mock_floxhub_git_dir = tempdir_in(&temp_dir).unwrap().into_path();
            let floxmeta_dir = mock_floxhub_git_dir.join(owner.as_str()).join("floxmeta");
            fs::create_dir_all(&floxmeta_dir).unwrap();
            GitCommandProvider::init(floxmeta_dir, true).unwrap();
            Url::from_directory_path(mock_floxhub_git_dir).unwrap()
        });

        let flox = Flox {
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            cache_dir,
            data_dir,
            temp_dir,
            config_dir,
            uuid: Default::default(),
            floxhub: Floxhub::new(
                Url::from_str("https://hub.flox.dev").unwrap(),
                git_url_override,
            )
            .unwrap(),
            floxhub_token: None,
            catalog_client: if use_client {
                Some(MockClient::default().into())
            } else {
                None
            },
            installable_locker: Default::default(),
            features: Default::default(),
        };

        init_global_manifest(&global_manifest_path(&flox)).unwrap();

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
    const FAKE_TOKEN: &str= "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjk5OTk5OTk5OTl9.6-nbzFzQEjEX7dfWZFLE-I_qW2N_-9W2HFzzfsquI74";

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
    const FAKE_EXPIRED_TOKEN: &str= "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjE3MDQwNjM2MDB9.-5VCofPtmYQuvh21EV1nEJhTFV_URkRP0WFu4QDPFxY";

    #[tokio::test]
    async fn test_get_username() {
        let token = FloxhubToken::new(FAKE_TOKEN.to_string()).unwrap();
        assert_eq!(token.handle(), "test");
    }

    #[tokio::test]
    async fn test_detect_expired() {
        let token_error =
            FloxhubToken::new(FAKE_EXPIRED_TOKEN.to_string()).expect_err("Token should be expired");
        assert_eq!(token_error, FloxhubTokenError::Expired);
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
