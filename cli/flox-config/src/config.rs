use std::collections::HashMap;
use std::fmt::Display;
use std::num::NonZeroU8;
use std::path::{Path, PathBuf};

use anyhow::Result;
use flox_core::activate::context::AutoActivateFishMode;
use flox_core::data::environment_ref::RemoteEnvironmentRef;
use flox_core::features::Features;
use serde::{Deserialize, Serialize};
use toml_edit::Key;
use url::Url;
use xdg::BaseDirectories;

use crate::write::ReadWriteError;
use crate::{load, write};

/// Name of flox managed directories (config, data, cache)
pub const FLOX_DIR_NAME: &str = "flox";
pub const FLOX_CONFIG_FILE: &str = "flox.toml";

/// How many items `flox search` should show by default.
///
/// Mirrors `floxhub_client::SearchLimit`.
pub type SearchLimit = NonZeroU8;

/// Authentication mode for FloxHub.
///
/// Consumers match on this directly to build an `AuthContext` via
/// `AuthContext::new_from_token` / `AuthContext::new_kerberos`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthnMode {
    /// Token authentication
    Token,
    /// Kerberos authentication
    Kerberos,
}

/// Where `flox auth login` stores the FloxHub token.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TokenStorageMode {
    /// Store the token in the OS-native keyring (default).
    #[default]
    Keyring,
    /// Store the token in plain text in flox.toml.
    Plaintext,
}

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct Config {
    /// flox configuration options
    #[serde(default, flatten)]
    pub flox: FloxConfig,

    /// Feature flags are set from config but more commonly controlled by
    /// `FLOX_FEATURES_` environment variables.
    ///
    /// Accessing from `flox_rust_sdk::flox::Flox.features` should be preferred
    /// over `Config.features` if both are available.
    #[serde(default)]
    #[deprecated(note = "Access `flox_rust_sdk::flox::Flox.features` instead")]
    pub features: Option<Features>,
}

/// Describes the Configuration for the flox library
#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct FloxConfig {
    /// Disable collecting and sending usage metrics
    /// Prefer `Flox.metrics_device_uuid.is_some()` if both are available.
    #[serde(default)]
    pub disable_metrics: bool,
    /// Directory where flox should store ephemeral data (default:
    /// `$XDG_CACHE_HOME/flox` e.g. `~/.cache/flox`)
    pub cache_dir: PathBuf,
    /// Directory where flox should store persistent data (default:
    /// `$XDG_DATA_HOME/flox` e.g. `~/.local/share/flox`)
    pub data_dir: PathBuf,
    /// Directory where flox should store data that's not critical but also
    /// shouldn't be able to be freely deleted like data in the cache directory.
    /// (default: `$XDG_STATE_HOME/flox` e.g. `~/.local/state/flox`)
    pub state_dir: PathBuf,
    /// Directory where flox should load its configuration file (default:
    /// `$XDG_CONFIG_HOME/flox` e.g. `~/.config/flox`)
    pub config_dir: PathBuf,

    /// Token to authenticate on FloxHub
    // Note: This does _not_ use `flox_rust_sdk::flox::FloxhubToken` because
    // parsing the token -- and thus parsing the config -- fails if the token is expired.
    // Instead parse as String (or some specialized enum to support keychains in the future)
    // and then validate the token as we build the `flox_rust_sdk::flox::Flox` instance.
    pub floxhub_token: Option<String>,

    /// How many items `flox search` should show by default
    pub search_limit: Option<SearchLimit>,

    /// Remote environments that are trusted for activation
    #[serde(default)]
    pub trusted_environments: HashMap<RemoteEnvironmentRef, EnvironmentTrust>,

    /// The URL of the FloxHub instance to use
    pub floxhub_url: Option<Url>,

    /// The URL of the catalog instance to use
    // TODO: hide this as an internal switch,
    // api/catalog urls should be derived from floxhub_url.
    // Manually setting derived urls should be an internal knob (see _FLOX_FLOXHUB_GIT_URL).
    pub catalog_url: Option<Url>,

    /// Authentication mode for FloxHub.
    /// Unset means the consumer's compiled-in default.
    pub floxhub_authn_mode: Option<AuthnMode>,

    /// Where new FloxHub tokens are stored: the OS keyring (default) or plain
    /// text in flox.toml. Set to `plaintext` by
    /// `flox auth login --insecure-storage`; cleared with
    /// `flox config --delete floxhub_token_storage`.
    #[serde(default)]
    pub floxhub_token_storage: TokenStorageMode,

    /// Rule whether to change the shell prompt in activated environments.
    ///
    /// Deprecated in favor of set_prompt and hide_default_prompt.
    pub shell_prompt: Option<EnvironmentPromptConfig>,

    /// Set shell prompt when activating an environment
    pub set_prompt: Option<bool>,

    /// Hide environments named 'default' from the shell prompt
    pub hide_default_prompt: Option<bool>,

    /// Print notification if upgrades are available on `flox activate`.
    /// The notification message is:
    ///
    /// ```text
    /// Upgrades are available for packages in 'environment-name'.
    /// Use 'flox upgrade --dry-run' for details.
    /// ```
    ///
    /// (default: true)
    pub upgrade_notifications: Option<bool>,

    /// Configuration for 'flox publish'.
    pub publish: Option<PublishConfig>,

    /// Release channel to use when checking for updates to Flox.
    pub installer_channel: Option<InstallerChannel>,

    /// Flox creates a single tempdir for each process in
    /// `$FLOX_CACHE_HOME/process`.
    /// Flox will delete this tempdir upon conclusion of the process
    /// unless `keep_tempdir == true` AND verbose logs are enabled.
    pub keep_tempdir: Option<bool>,

    /// Whether to automatically activate environments.
    /// Possible values: `prompt` (default), `allowlist`, `disabled`.
    pub auto_activate: Option<AutoActivate>,

    /// Controls how the fish shell hook responds to directory changes.
    /// Possible values: `eval_on_arrow` (default), `eval_after_arrow`, `disable_arrow`.
    pub auto_activate_fish_mode: Option<AutoActivateFishMode>,

    /// Per-directory auto-activation preferences.
    /// Maps absolute paths to explicit allow/deny decisions.
    #[serde(default)]
    pub auto_activate_environments: HashMap<PathBuf, AutoActivationPreference>,

    /// Don't setup the Flox prompt hook as part of activation.
    /// This disables auto-activation as well as features like `flox deactivate`
    /// without `--print-script` (default: false)
    pub disable_hook: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvironmentTrust {
    Trust,
    Deny,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnvironmentPromptConfig {
    /// Change the shell prompt to show all active environments
    ShowAll,
    /// Do not change the shell prompt
    HideAll,
    /// Change the shell prompt to show the active environments,
    /// but omit 'default' environments
    HideDefault,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PublishConfig {
    /// Default path of the signing key used by 'flox publish'
    pub signing_private_key: Option<PathBuf>,
}

/// Channels must match: https://downloads.flox.dev/?prefix=by-env/
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
#[serde(rename_all = "lowercase")]
pub enum InstallerChannel {
    #[default]
    Stable,
    Nightly,
    Qa,
}

/// Whether to automatically activate environments
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AutoActivate {
    /// Only auto-activate environments the user has already allowed (via the
    /// prompt or `flox activate allow`). Walking past an unregistered `.flox`
    /// does nothing — no prompt.
    #[serde(alias = "allowed")]
    Allowlist,
    /// Auto-activate allowed environments, and prompt before activating an
    /// environment that has not yet been allowed or denied.
    #[default]
    Prompt,
    /// Auto-activation off entirely: never activate (not even allowed
    /// environments) and never prompt.
    Disabled,
}

/// Auto-activation preference for a specific directory
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AutoActivationPreference {
    Allow,
    Deny,
}

impl Display for InstallerChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallerChannel::Stable => write!(f, "stable"),
            InstallerChannel::Nightly => write!(f, "nightly"),
            InstallerChannel::Qa => write!(f, "qa"),
        }
    }
}

impl Config {
    /// Creates a [Config] from the environment and config files
    pub fn parse() -> Result<Config> {
        load::parse()
    }

    /// Like [Config::parse], with explicit directories and environment.
    pub fn parse_with(
        flox_dirs: &BaseDirectories,
        user_config_dir: &Path,
        system_config_dir: Option<&Path>,
        env: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Config> {
        load::parse_with(flox_dirs, user_config_dir, system_config_dir, env)
    }

    /// get a value from the config
    ///
    /// **intended for human consumption/introspection of config only**
    ///
    /// Values in the context should be read from the [Config] type instead!
    pub fn get_verbatim(&self, path: &[Key]) -> Result<String, ReadWriteError> {
        write::get(self, path)
    }

    /// Append or update a key value paring in the toml representation of a partial config
    ///
    /// Validate using [Self]
    pub fn write_to<V: Serialize>(
        config_file: Option<String>,
        path: &[Key],
        value: Option<V>,
    ) -> Result<String, ReadWriteError> {
        write::write_to(config_file, path, value)
    }

    pub fn write_to_in<V: Serialize>(
        config_file_path: impl AsRef<Path>,
        query: &[Key],
        value: Option<V>,
    ) -> Result<(), ReadWriteError> {
        write::write_to_in(config_file_path, query, value)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn installer_channel_display_matches_serialized(channel in any::<InstallerChannel>()) {
            let display_quoted = format!("\"{}\"", channel);
            let serialized = serde_json::to_string(&channel).unwrap();
            prop_assert_eq!(display_quoted, serialized);
        }
    }
}
