use std::io::Read;
use std::path::PathBuf;

use derive_more::Constructor;
use jsonwebtoken::{DecodingKey, Validation};
use log::info;
use once_cell::sync::Lazy;
use runix::arguments::common::NixCommonArgs;
use runix::arguments::config::NixConfigArgs;
use runix::command_line::{DefaultArgs, NixCommandLine};
use runix::installable::{AttrPath, FlakeAttribute};
use runix::NixBackend;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::environment::{self, default_nix_subprocess_env};
pub use crate::models::environment_ref::{self, *};

pub static FLOX_VERSION: Lazy<String> =
    Lazy::new(|| std::env::var("FLOX_VERSION").unwrap_or(env!("FLOX_VERSION").to_string()));

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

    /// access tokens injected in nix.conf
    ///
    /// Use [Vec] to preserve original ordering
    pub access_tokens: Vec<(String, String)>,
    pub netrc_file: PathBuf,

    pub system: String,

    pub uuid: uuid::Uuid,

    pub floxhub: Floxhub,

    /// Token to authenticate with floxhub.
    /// It's usually populated from the config during [Flox] initialization.
    /// Checking for [None] can be used to check if the use is logged in.
    pub floxhub_token: Option<FloxhubToken>,
}

pub trait FloxNixApi: NixBackend {
    fn new(flox: &Flox, default_nix_args: DefaultArgs) -> Self;
}

impl FloxNixApi for NixCommandLine {
    fn new(_: &Flox, default_nix_args: DefaultArgs) -> NixCommandLine {
        NixCommandLine {
            nix_bin: Some(environment::NIX_BIN.to_string()),
            defaults: default_nix_args,
        }
    }
}

#[derive(Debug, Constructor)]
pub struct ResolvedInstallableMatch {
    pub flakeref: String,
    pub prefix: String,
    pub system: Option<String>,
    pub explicit_system: bool,
    pub key: Vec<String>,
    pub description: Option<String>,
}

impl ResolvedInstallableMatch {
    pub fn flake_attribute(self) -> FlakeAttribute {
        // Join the prefix and key into a safe attrpath, adding the associated system if present
        let attr_path = {
            let mut builder = AttrPath::default();
            // enforce exact attr path (<flakeref>#.<attrpath>)
            builder.push_attr("").unwrap();
            builder.push_attr(&self.prefix).unwrap();
            if let Some(ref system) = self.system {
                builder.push_attr(system).unwrap();
            }

            // Build the multi-part key into a Nix-safe single string
            for key in self.key {
                builder.push_attr(&key).unwrap();
            }
            builder
        };

        FlakeAttribute {
            flakeref: self.flakeref.parse().unwrap(),
            attr_path,
            outputs: Default::default(),
        }
    }
}

impl Flox {
    /// Produce a new Nix Backend
    ///
    /// This method performs backend independent configuration of nix
    /// and passes itself and the default config to the constructor of the Nix Backend
    ///
    /// The constructor will perform backend specific configuration measures
    /// and return a fresh initialized backend.
    pub fn nix<Nix: FloxNixApi>(&self, mut caller_extra_args: Vec<String>) -> Nix {
        use std::io::Write;
        use std::os::unix::prelude::OpenOptionsExt;

        let environment = {
            let config = NixConfigArgs {
                accept_flake_config: true.into(),
                warn_dirty: false.into(),
                extra_experimental_features: ["nix-command", "flakes"]
                    .map(String::from)
                    .to_vec()
                    .into(),
                extra_substituters: ["https://cache.floxdev.com"]
                    .map(String::from)
                    .to_vec()
                    .into(),
                extra_trusted_public_keys: [
                    "flox-store-public-0:8c/B+kjIaQ+BloCmNkRUKwaVPFWkriSAd0JJvuDu4F0=",
                ]
                .map(String::from)
                .to_vec()
                .into(),
                extra_access_tokens: self.access_tokens.clone().into(),
                flake_registry: None,
                netrc_file: Some(self.netrc_file.clone().into()),
                connect_timeout: 5.into(),
                ..Default::default()
            };

            let nix_config = format!(
                "# Automatically generated - do not edit.\n{}\n",
                config.to_config_string()
            );

            // Write nix.conf file if it does not exist or has changed
            let global_config_file_path = self.config_dir.join("nix.conf");
            if !global_config_file_path.exists() || {
                let mut contents = String::new();
                std::fs::File::open(&global_config_file_path)
                    .unwrap()
                    .read_to_string(&mut contents)
                    .unwrap();

                contents != nix_config
            } {
                let temp_config_file_path = self.temp_dir.join("nix.conf");

                std::fs::File::options()
                    .mode(0o600)
                    .create_new(true)
                    .write(true)
                    .open(&temp_config_file_path)
                    .unwrap()
                    .write_all(nix_config.as_bytes())
                    .unwrap();

                info!("Updating nix.conf: {global_config_file_path:?}");
                std::fs::rename(temp_config_file_path, &global_config_file_path).unwrap()
            }

            let mut env = default_nix_subprocess_env();
            let _ = env.insert(
                "NIX_USER_CONF_FILES".to_string(),
                global_config_file_path.to_string_lossy().to_string(),
            );
            env
        };

        #[allow(clippy::needless_update)]
        let common_args = NixCommonArgs {
            ..Default::default()
        };

        let mut extra_args = vec!["--quiet".to_string(), "--quiet".to_string()];
        extra_args.append(&mut caller_extra_args);

        let default_nix_args = DefaultArgs {
            environment,
            common_args,
            extra_args,
            ..Default::default()
        };

        Nix::new(self, default_nix_args)
    }
}

pub static DEFAULT_FLOXHUB_URL: Lazy<Url> =
    Lazy::new(|| Url::parse("https://hub.flox.dev").unwrap());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloxhubToken(String);

impl AsRef<str> for FloxhubToken {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FloxhubToken {
    /// Create a new floxhub token from a string
    pub fn new(token: String) -> Self {
        FloxhubToken(token)
    }

    /// Return the token as a string
    pub fn secret(&self) -> &str {
        &self.0
    }

    pub fn handle(&self) -> Result<String, FloxhubTokenError> {
        #[derive(Debug, Deserialize)]
        struct Claims {
            #[serde(rename = "https://flox.dev/handle")]
            handle: String,
        }

        let mut validation = Validation::default();
        // we're neither creating or verifying the token on the client side
        validation.insecure_disable_signature_validation();
        validation.validate_aud = false;
        let token =
            jsonwebtoken::decode::<Claims>(&self.0, &DecodingKey::from_secret(&[]), &validation)
                .map_err(FloxhubTokenError::InvalidToken)?;
        Ok(token.claims.handle)
    }
}

#[derive(Debug, Clone, Error)]
pub enum FloxhubTokenError {
    #[error("invalid token")]
    InvalidToken(#[source] jsonwebtoken::errors::Error),
}

#[derive(Debug, Clone)]
pub struct Floxhub {
    base_url: Url,
    git_url_override: Option<Url>,
}

impl Floxhub {
    pub fn new(base_url: Url) -> Self {
        Floxhub {
            base_url,
            git_url_override: None,
        }
    }

    pub fn set_git_url_override(&mut self, git_url_override: Url) -> &mut Self {
        self.git_url_override = Some(git_url_override);
        self
    }

    /// Return the base url of the floxhub instance
    /// might change to a more specific url in the future
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub fn git_url_override(&self) -> Option<&Url> {
        self.git_url_override.as_ref()
    }

    /// Return the url of the floxhub git interface
    ///
    /// If the environment variable `_FLOX_FLOXHUB_GIT_URL` is set,
    /// it will be used instead of the derived floxhub host.
    /// This is useful for testing floxhub locally.
    pub fn git_url(&self) -> Result<Url, url::ParseError> {
        if let Some(ref url) = self.git_url_override {
            return Ok(url.clone());
        }

        let mut url = self.base_url.clone();
        let host = url.host_str().unwrap();
        url.set_host(Some(&format!("git.{}", host)))?;

        Ok(url)
    }
}

#[cfg(test)]
pub mod tests {
    use std::str::FromStr;

    use tempfile::TempDir;

    use super::*;

    /// A fake floxhub token
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

    pub fn flox_instance() -> (Flox, TempDir) {
        let tempdir_handle = tempfile::tempdir_in(std::env::temp_dir()).unwrap();

        let cache_dir = tempdir_handle.path().join("caches");
        let data_dir = tempdir_handle.path().join(".local/share/flox");
        let temp_dir = tempdir_handle.path().join("temp");
        let config_dir = tempdir_handle.path().join("config");

        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();

        let flox = Flox {
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            cache_dir,
            data_dir,
            temp_dir,
            config_dir,
            access_tokens: Default::default(),
            netrc_file: Default::default(),
            uuid: Default::default(),
            floxhub: Floxhub::new(Url::from_str("https://hub.flox.dev").unwrap()),
            floxhub_token: None,
        };

        init_global_manifest(&global_manifest_path(&flox)).unwrap();

        (flox, tempdir_handle)
    }

    #[test]
    fn test_resolved_installable_match_to_installable() {
        let resolved = ResolvedInstallableMatch::new(
            "github:flox/flox".to_string(),
            "packages".to_string(),
            Some("aarch64-darwin".to_string()),
            false,
            vec!["flox".to_string()],
            None,
        );
        assert_eq!(
            FlakeAttribute::from_str("github:flox/flox#.packages.aarch64-darwin.flox").unwrap(),
            resolved.flake_attribute(),
        );
    }

    use crate::models::environment::{global_manifest_path, init_global_manifest};

    #[tokio::test]
    async fn test_get_username() {
        let token = FloxhubToken::new(FAKE_TOKEN.to_string());
        assert_eq!(token.handle().unwrap(), "test");
    }
}
