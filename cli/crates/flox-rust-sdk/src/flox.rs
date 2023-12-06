use std::collections::{BTreeSet, HashMap};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use derive_more::Constructor;
use indoc::indoc;
use log::{debug, info, warn};
use once_cell::sync::Lazy;
use reqwest;
use reqwest::header::USER_AGENT;
use runix::arguments::common::NixCommonArgs;
use runix::arguments::config::NixConfigArgs;
use runix::arguments::flake::{FlakeArgs, OverrideInput};
use runix::arguments::{EvalArgs, NixArgs};
use runix::command::{Eval, FlakeMetadata};
use runix::command_line::{DefaultArgs, NixCommandLine};
use runix::flake_ref::path::PathRef;
use runix::installable::{AttrPath, FlakeAttribute};
use runix::{NixBackend, RunJson};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::environment::{self, default_nix_subprocess_env};
use crate::models::channels::ChannelRegistry;
pub use crate::models::environment_ref::{self, *};
use crate::models::flake_ref::FlakeRef;
pub use crate::models::flox_installable::*;
use crate::models::floxmeta::{Floxmeta, GetFloxmetaError};
use crate::models::root::transaction::ReadOnly;
use crate::models::root::{self, Root};
use crate::providers::git::GitProvider;

static INPUT_CHARS: Lazy<Vec<char>> = Lazy::new(|| ('a'..='t').collect());

pub const FLOX_VERSION: &str = env!("FLOX_VERSION");

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
#[derive(Debug, Default)]
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

    pub channels: ChannelRegistry,

    pub system: String,

    pub uuid: uuid::Uuid,

    pub floxhub_token: Option<String>,
    pub floxhub_host: String,
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

/// Typed matching installable outputted by our Nix evaluation
#[derive(Deserialize, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
struct InstallableEvalQueryEntry {
    system: Option<String>,
    explicit_system: bool,
    prefix: String,
    key: Vec<String>,
    input: String,
    description: Option<String>,
}

#[derive(Error, Debug)]
pub enum ResolveFloxInstallableError<Nix: FloxNixApi>
where
    Eval: RunJson<Nix>,
{
    #[error("Error checking for installable matches: {0}")]
    Eval(<Eval as RunJson<Nix>>::JsonError),
    #[error("Error parsing installable eval output: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Typed output of our Nix evaluation to find matching installables
type InstallableEvalQueryOut = BTreeSet<InstallableEvalQueryEntry>;

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
    pub fn resource<X>(&self, x: X) -> Root<root::Closed<X>> {
        Root::closed(self, x)
    }

    // TODO: revisit this when we discussed floxmeta's role to contribute to config/channels
    //       flox.floxmeta is referring to the legacy floxmeta implementation
    //       and is currently only used by the CLI to read the channels from the users floxmain.
    //
    //       N.B.: Decide whether we want to keep the `Flox.<model>` API
    //       to create instances of subsystem models
    // region: revisit reg. channels
    pub fn floxmeta(&self, owner: &str) -> Result<Floxmeta<ReadOnly>, GetFloxmetaError> {
        Floxmeta::get_floxmeta(self, owner)
    }

    // endregion: revisit reg. channels

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
            // Write registry file if it does not exist or has changed
            let global_registry_file = self.config_dir.join("floxFlakeRegistry.json");
            let registry_content = serde_json::to_string_pretty(&self.channels).unwrap();
            if !global_registry_file.exists() || {
                let contents: ChannelRegistry =
                    serde_json::from_reader(std::fs::File::open(&global_registry_file).unwrap())
                        .expect("Invalid registry file");

                contents != self.channels
            } {
                let temp_registry_file = self.temp_dir.join("registry.json");

                std::fs::File::options()
                    .mode(0o600)
                    .create_new(true)
                    .write(true)
                    .open(&temp_registry_file)
                    .unwrap()
                    .write_all(registry_content.as_bytes())
                    .unwrap();

                debug!("Updating flake registry: {global_registry_file:?}");
                std::fs::rename(temp_registry_file, &global_registry_file).unwrap();
            }

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
                flake_registry: Some(global_registry_file.into()),
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

/// Requires login with auth0 with "openid" and "profile" scopes
/// https://auth0.com/docs/scopes/current/oidc-scopes
/// See also: `authenticate` in `flox/src/commands/auth.rs` where we set the scopes
#[derive(Debug, Serialize, Deserialize)]
struct Auth0User {
    /// full name of the user
    name: String,
    /// nickname of the user (e.g. github username)
    nickname: String,
}

pub struct Auth0Client {
    base_url: String,
    oauth_token: String,
}

impl Auth0Client {
    pub fn new(base_url: String, oauth_token: String) -> Self {
        Auth0Client {
            base_url,
            oauth_token,
        }
    }

    pub async fn get_username(&self) -> Result<String, reqwest::Error> {
        let url = format!("{}/userinfo", self.base_url);
        let client = reqwest::Client::new();
        let request = client
            .get(url)
            .header(USER_AGENT, "flox cli")
            .bearer_auth(&self.oauth_token);

        let response = request.send().await?;

        if response.status().is_success() {
            let user: Auth0User = response.json().await?;
            Ok(user.nickname)
        } else {
            Err(response.error_for_status().unwrap_err())
        }
    }
}

#[cfg(test)]
pub mod tests {
    use reqwest::header::AUTHORIZATION;
    use tempfile::TempDir;

    use super::*;

    pub fn flox_instance() -> (Flox, TempDir) {
        let tempdir_handle = tempfile::tempdir_in(std::env::temp_dir()).unwrap();

        let cache_dir = tempdir_handle.path().join("caches");
        let data_dir = tempdir_handle.path().join(".local/share/flox");
        let temp_dir = tempdir_handle.path().join("temp");
        let config_dir = tempdir_handle.path().join("config");

        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();

        let mut channels = ChannelRegistry::default();
        channels.register_channel("flox", "github:flox/floxpkgs/master".parse().unwrap());

        let flox = Flox {
            system: "aarch64-darwin".to_string(),
            cache_dir,
            data_dir,
            temp_dir,
            config_dir,
            channels,
            ..Default::default()
        };

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

    use mockito;

    use crate::flox::Auth0Client;

    #[tokio::test]
    async fn test_get_username() {
        let mock_response = serde_json::json!({
            "nickname": "exampleuser",
            "name": "Example User",
        });
        let mut server = mockito::Server::new();
        let mock_server_url = server.url();
        let mock_server = server
            .mock("GET", "/userinfo")
            .match_header(AUTHORIZATION.as_str(), "Bearer your_oauth_token")
            .match_header(USER_AGENT.as_str(), "flox cli")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_response.to_string())
            .create();

        let github_client = Auth0Client::new(mock_server_url, "your_oauth_token".to_string());

        let username = github_client.get_username().await.unwrap();
        assert_eq!(username, "exampleuser".to_string());

        mock_server.assert();
    }
}
