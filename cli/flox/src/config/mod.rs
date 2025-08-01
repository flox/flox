use std::collections::HashMap;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{Context, Result};
use config::{Config as HierarchicalConfig, Environment};
use flox_rust_sdk::flox::{EnvironmentRef, Features};
use flox_rust_sdk::models::search::SearchLimit;
use itertools::{Either, Itertools};
use serde::{Deserialize, Serialize};
use tempfile::PersistError;
use thiserror::Error;
use toml_edit::{DocumentMut, Item, Key, Table, TableLike};
use tracing::{debug, trace};
use url::Url;
use xdg::BaseDirectories;

/// Name of flox managed directories (config, data, cache)
pub const FLOX_DIR_NAME: &str = "flox";
const FLOX_CONFIG_DIR_VAR: &str = "FLOX_CONFIG_DIR";
pub const FLOX_CONFIG_FILE: &str = "flox.toml";

pub const FLOX_DISABLE_METRICS_VAR: &str = "FLOX_DISABLE_METRICS";

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct Config {
    /// flox configuration options
    #[serde(default, flatten)]
    pub flox: FloxConfig,

    /// Feature flags are set from config but more commonly controlled by
    /// `FLOX_FEATURES_` environment variables.
    ///
    /// Accessing from `flox_rust_sdk::flox::Flox.features` should be preferred
    /// over `flox::config::Config.features` if both are available.
    #[serde(default)]
    #[deprecated(note = "Access `flox_rust_sdk::flox::Flox.features` instead")]
    pub features: Option<Features>,
}

// TODO: move to flox_sdk?
/// Describes the Configuration for the flox library
#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct FloxConfig {
    /// Disable collecting and sending usage metrics
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
    ///
    /// Note: This does _not_ use [flox_rust_sdk::flox::FloxhubToken] because
    /// parsing the token -- and thus parsing the config -- fails if the token is expired.
    /// Instead parse as String (or some specialized enum to support keychains in the future)
    /// and then validate the token as we build the [flox_rust_sdk::flox::Flox] instance.
    pub floxhub_token: Option<String>,

    /// How many items `flox search` should show by default
    pub search_limit: SearchLimit,

    /// Remote environments that are trusted for activation
    #[serde(default)]
    pub trusted_environments: HashMap<EnvironmentRef, EnvironmentTrust>,

    /// The URL of the FloxHub instance to use
    pub floxhub_url: Option<Url>,

    /// The URL of the catalog instance to use
    // Using a URL here adds an extra trailing slash,
    // so just use a String.
    pub catalog_url: Option<String>,

    /// Rule whether to change the shell prompt in activated environments.
    /// Deprecated in favor of set_prompt and hide_default_prompt.
    pub shell_prompt: Option<EnvironmentPromptConfig>,

    /// Set shell prompt when activating an environment
    pub set_prompt: Option<bool>,

    /// Hide environments named 'default' from the shell prompt
    pub hide_default_prompt: Option<bool>,

    /// Print notification if upgrades are available on `flox activate`.
    /// The notification message is:
    /// ```
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
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

impl Display for InstallerChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallerChannel::Stable => write!(f, "stable"),
            InstallerChannel::Nightly => write!(f, "nightly"),
            InstallerChannel::Qa => write!(f, "qa"),
        }
    }
}

/// Error returned by [`Config::get()`]
#[derive(Debug, Error)]
pub enum ReadWriteError {
    #[error("Invalid config key: '{}'",
         _0.iter()
         .map(|key| key.display_repr()
         .into_owned())
         .collect_vec()
         .join("."))]
    InvalidKey(Vec<Key>),
    #[error("Config key '{}' not in user configuration", _0.iter().map(|key| key.display_repr().into_owned()).collect_vec().join("."))]
    NotAUserValue(Vec<Key>),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    TomlEdit(#[from] toml_edit::TomlError),
    #[error(transparent)]
    TomlSer(#[from] toml_edit::ser::Error),
    #[error(transparent)]
    TomlDe(#[from] toml_edit::de::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("Could not read config file: {0}")]
    ReadConfig(std::io::Error),
    #[error("Could not write config file: {0}")]
    WriteConfig(std::io::Error),
    #[error(transparent)]
    Persist(#[from] PersistError),
}

/// Locates the system wide flox config dir.
///
/// By default that is `/etc`.
/// The directory can be overridden by the user via the environment variable
/// `$FLOX_SYSTEM_CONFIG_DIR`.
///
/// IFF `$FLOX_SYSTEM_CONFIG_DIR` is set to an empty string, system config will be ignored.
fn locate_system_config_dir() -> Result<Option<PathBuf>> {
    // TODO: make constant
    match env::var("FLOX_SYSTEM_CONFIG_DIR").ok() {
        Some(path) if path.is_empty() => Ok(None),
        Some(path) => Ok(Some(path.into())),
        None => Ok(Some(PathBuf::from("/etc"))),
    }
}

/// Locates the user specific config dir.
///
/// The implementation probes for config files in relevant xdg dirs in order:
/// $XDG_CONFIG_HOME/flox/flox.toml
/// for dir in $XDG_CONFIG_DIRS: $dir/flox/flox.toml
///
/// The first occurrence determines the config_dir.
/// If no config file was found, this function defaults to $XDG_CONFIG_HOME/flox.
///
/// IFF $FLOX_CONFIG_DIR is set to a non-empty string,
/// its value will be used as the user config dir instead.
fn locate_user_config_dir(flox_dirs: &BaseDirectories) -> Result<PathBuf> {
    let user_config_dir = env::var(FLOX_CONFIG_DIR_VAR).ok();

    let config_dir = match user_config_dir {
        Some(path) if !path.is_empty() => {
            debug!(?path, "Global config directory overridden");
            fs::create_dir_all(&path)
                .context(format!("Could not create config directory: {path:?}"))?;
            path.into()
        },
        None | Some(_ /* empty */) => {
            let config_dir = if let Some(existing_xdg_based_config) =
                // Look for flox/flox.toml in $XDG_CONFIG_HOME and then $XDG_CONFIG_DIRS
                flox_dirs.find_config_file(FLOX_CONFIG_FILE)
            {
                debug!(file=?existing_xdg_based_config, "found existing config file");
                existing_xdg_based_config
                    .parent()
                    .expect("filename is always appended to a directory")
                    .to_path_buf()
            } else {
                debug!("no user config file found");
                // fall back to `XDG_CONFIG_HOME/flox`
                flox_dirs.get_config_home()
            };

            fs::create_dir_all(&config_dir)
                .context(format!("Could not create config directory: {config_dir:?}"))?;
            let config_dir = config_dir
                .canonicalize()
                .context("Could not canonicalize config directory '{config_dir:?}'")?;

            // Allow subshells to find the same config dir.
            // TODO: decide if its worth modifying the env for this.
            // SAFTEY: config initially read when there is no concurrent access to env variables.
            unsafe {
                env::set_var(FLOX_CONFIG_DIR_VAR, &config_dir);
            }
            config_dir
        },
    };

    Ok(config_dir)
}

/// Reads a [HierarchicalConfig] from an optional system config file,
/// a user config file and environment variables.
fn raw_config_from_parts(
    flox_dirs: &BaseDirectories,
    user_config_dir: &Path,
    system_config_dir: Option<&Path>,
    env: impl IntoIterator<Item = (String, String)>,
) -> Result<HierarchicalConfig> {
    let cache_dir = flox_dirs.get_cache_home();
    let data_dir = flox_dirs.get_data_home();
    let state_dir = flox_dirs.get_state_home();

    let config_dir = user_config_dir;

    let mut builder = HierarchicalConfig::builder()
        .set_default("default_substituter", "https://cache.floxdev.com/")?
        .set_default("cache_dir", cache_dir.to_str().unwrap())?
        .set_default("data_dir", data_dir.to_str().unwrap())?
        .set_default("state_dir", state_dir.to_str().unwrap())?
        // Config dir is added to the config for completeness;
        // the config file cannot change the config dir.
        .set_override("config_dir", config_dir.to_str().unwrap())?;
    // Read System Config first
    if let Some(system_config_dir) = system_config_dir {
        builder = builder.add_source(
            config::File::from(system_config_dir.join(FLOX_CONFIG_FILE))
                .format(config::FileFormat::Toml)
                .required(false),
        );
    };

    // Read User Config
    builder = builder.add_source(
        config::File::from(config_dir.join(FLOX_CONFIG_FILE))
            .format(config::FileFormat::Toml)
            .required(false),
    );

    // Override via env variables
    let builder = {
        let mut flox_envs = env
            .into_iter()
            .filter_map(|(k, v)| k.strip_prefix("FLOX_").map(|k| (k.to_owned(), v)))
            .collect::<Vec<_>>();
        builder
            .add_source(mk_environment(&mut flox_envs, "NIX"))
            .add_source(mk_environment(&mut flox_envs, "GITHUB"))
            .add_source(mk_environment(&mut flox_envs, "FEATURES"))
            .add_source(
                Environment::default()
                    .source(Some(HashMap::from_iter(flox_envs)))
                    .try_parsing(true),
            )
    };

    let final_config = builder.build()?;
    Ok(final_config)
}

impl Config {
    fn parse_with(
        flox_dirs: &BaseDirectories,
        user_config_dir: &Path,
        system_config_dir: Option<&Path>,
        env: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Config> {
        let final_config =
            raw_config_from_parts(flox_dirs, user_config_dir, system_config_dir, env)?;

        let cli_config: Config = final_config
            .to_owned()
            .try_deserialize()
            .context("Could not parse config")?;
        Ok(cli_config)
    }

    /// Creates a [Config] from the environment and config files
    pub fn parse() -> Result<Config> {
        let base_directories = BaseDirectories::with_prefix(FLOX_DIR_NAME)?;
        Self::parse_with(
            &base_directories,
            &locate_user_config_dir(&base_directories)?,
            locate_system_config_dir()?.as_deref(),
            env::vars(),
        )
    }

    /// get a value from the config
    ///
    /// **intended for human consumption/intospection of config only**
    ///
    /// Values in the context should be read from the [Config] type instead!
    pub fn get(&self, path: &[Key]) -> Result<String, ReadWriteError> {
        let document: toml_edit::DocumentMut = toml_edit::ser::to_document(self)?;

        if path.is_empty() {
            return Ok(document.to_string());
        }

        let mut cfg = document.as_table() as &dyn TableLike;

        let (key, parents) = path.split_last().unwrap();

        for (n, segment) in parents.iter().enumerate() {
            let maybe_value = cfg.get(segment).and_then(|item| item.as_table_like());

            match maybe_value {
                Some(v) => cfg = v,
                None => {
                    Err(ReadWriteError::InvalidKey(path[..=n].to_vec()))?;
                },
            }
        }

        let value = cfg
            .get(key.as_ref())
            .ok_or(ReadWriteError::InvalidKey(path.to_vec()))?;

        Ok(value.to_string())
    }

    /// Append or update a key value paring in the toml representation of a partial config
    ///
    /// Validate using [Self]
    pub fn write_to<V: Serialize>(
        config_file: Option<String>,
        path: &[Key],
        value: Option<V>,
    ) -> Result<String, ReadWriteError> {
        let mut validation_document = toml_edit::ser::to_document(&Config::default())?;

        let mut document = match config_file {
            Some(content) => content.parse::<DocumentMut>()?,
            None => DocumentMut::new(),
        };

        let (mut handle, mut validation) =
            (document.as_table_mut(), validation_document.as_table_mut());

        let (key, parents) = path.split_last().unwrap();

        for segment in parents {
            trace!("stepping into path segment {}", segment);

            if !handle.contains_table(segment) {
                handle.insert(segment, Item::Table(Table::new()));
            }
            if !validation.contains_table(segment) {
                validation.insert(segment, Item::Table(Table::new()));
            }

            handle = handle.get_mut(segment).unwrap().as_table_mut().unwrap();
            validation = validation.get_mut(segment).unwrap().as_table_mut().unwrap();
        }

        trace!("write value for key '{}'", key.display_repr());

        match value {
            None => {
                let _ = handle
                    .remove(key.as_ref())
                    .ok_or(ReadWriteError::NotAUserValue(path.to_vec()))?;
            },
            Some(ref value) => {
                for handle in [handle, validation] {
                    handle.insert(
                        key.as_ref(),
                        Item::Value(value.serialize(toml_edit::ser::ValueSerializer::default())?),
                    );
                }
                trace!("try parsing the new virtual config (validation)");
                let validation_config: Config = toml_edit::de::from_document(validation_document)?;

                validation_config.get(path)?;
            },
        }

        Ok(document.to_string())
    }

    pub fn write_to_in<V: Serialize>(
        config_file_path: impl AsRef<Path>,
        temp_dir: impl AsRef<Path>,
        query: &[Key],
        value: Option<V>,
    ) -> Result<(), ReadWriteError> {
        let config_file_contents = match fs::read_to_string(&config_file_path) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!(
                    "No existing user config file found in {:?}, creating it now",
                    config_file_path.as_ref()
                );
                Ok(None)
            },
            Err(e) => Err(e),
        }
        .map_err(ReadWriteError::ReadConfig)?;

        let config_file_contents = Self::write_to(config_file_contents, query, value)?;

        let tempfile = tempfile::Builder::new().tempfile_in(temp_dir)?;
        fs::write(&tempfile, config_file_contents).map_err(ReadWriteError::WriteConfig)?;
        tempfile.persist(config_file_path)?;

        Ok(())
    }
}

fn mk_environment(envs: &mut Vec<(String, String)>, prefix: &str) -> Environment {
    let (prefixed_envs, flox_envs): (HashMap<String, String>, Vec<(String, String)>) = envs
        .iter()
        .partition_map(|(k, v)| match k.strip_prefix(&format!("{prefix}_")) {
            Some(suffix) => Either::Left((format!("{prefix}#{suffix}"), v.to_owned())),
            None => Either::Right((k.to_owned(), v.to_owned())),
        });
    let environment = Environment::with_prefix(prefix)
        .keep_prefix(true)
        .separator("#")
        .source(Some(prefixed_envs))
        .try_parsing(true);
    *envs = flox_envs;
    environment
}

#[cfg(test)]
mod tests {

    use std::num::NonZero;

    use indoc::{formatdoc, indoc};
    use proptest::prelude::*;

    // TODO: update the `xdg` crate and build `BaseDirectories` by hand with known (test) paths
    fn mock_flox_dirs() -> BaseDirectories {
        BaseDirectories::new().unwrap()
    }

    use super::*;

    #[test]
    fn test_read_bool() {
        let mut config = Config::default();
        config.flox.disable_metrics = true;
        assert_eq!(
            config.get(&Key::parse("disable_metrics").unwrap()).unwrap(),
            "true".to_string()
        );
    }

    #[test]
    fn test_set_by_env() {
        let tempdir = tempfile::tempdir().unwrap();
        temp_env::with_vars(
            [
                (
                    "HOME",
                    Some(tempdir.path().as_os_str().to_string_lossy().as_ref()),
                ),
                ("FLOX_FLOXHUB_URL", Some("https://example.com")),
            ],
            || {
                // SAFETY: env already isolated from concurrent access via `temp_env`
                unsafe {
                    env::set_var("FLOX_FLOXHUB_URL", "https://example.com");
                }
                let config = Config::parse().unwrap();
                assert_eq!(
                    config.get(&Key::parse("floxhub_url").unwrap()).unwrap(),
                    "\"https://example.com/\"".to_string()
                );
                unsafe { env::remove_var(FLOX_CONFIG_DIR_VAR) };
            },
        );
    }

    #[test]
    fn set_by_system_config() {
        let user_config_dir = tempfile::tempdir().unwrap();
        let system_config_dir = tempfile::tempdir().unwrap();

        let floxhub_url = "https://testhub.flox.dev";
        let disable_metrics = true;
        let search_limit = NonZero::new(42).unwrap();

        fs::write(
            system_config_dir.path().join(FLOX_CONFIG_FILE),
            formatdoc! {"
            floxhub_url = '{floxhub_url}'
            disable_metrics = {disable_metrics}
            search_limit = {search_limit}
        "},
        )
        .unwrap();

        // todo: fix mocks to avoid pulling in user config
        fs::write(user_config_dir.path().join(FLOX_CONFIG_FILE), "").unwrap();

        let config = Config::parse_with(
            &mock_flox_dirs(),
            user_config_dir.path(),
            Some(system_config_dir.path()),
            [],
        )
        .unwrap();

        assert_eq!(config.flox.floxhub_url, Some(floxhub_url.parse().unwrap()));
        assert!(config.flox.disable_metrics);
        assert_eq!(config.flox.search_limit, Some(search_limit));
    }

    #[test]
    fn user_config_overrides_system() {
        let user_config_dir = tempfile::tempdir().unwrap();
        let system_config_dir = tempfile::tempdir().unwrap();

        let floxhub_url = "https://testhub.flox.dev";
        let disable_metrics = true;
        let search_limit = NonZero::new(42).unwrap();

        fs::write(user_config_dir.path().join(FLOX_CONFIG_FILE), formatdoc! {"
            floxhub_url = '{floxhub_url}'
            disable_metrics = {disable_metrics}
            search_limit = {search_limit}
        "})
        .unwrap();

        fs::write(
            system_config_dir.path().join(FLOX_CONFIG_FILE),
            formatdoc! {"
            floxhub_url = 'https://system.flox.dev'
            disable_metrics = false
            search_limit = 24
        "},
        )
        .unwrap();

        let config = Config::parse_with(
            &mock_flox_dirs(),
            user_config_dir.path(),
            Some(system_config_dir.path()),
            [],
        )
        .unwrap();

        assert_eq!(config.flox.floxhub_url, Some(floxhub_url.parse().unwrap()));
        assert!(config.flox.disable_metrics);
        assert_eq!(config.flox.search_limit, Some(search_limit));
    }

    #[test]
    fn env_overrides_user_and_system() {
        let user_config_dir = tempfile::tempdir().unwrap();
        let system_config_dir = tempfile::tempdir().unwrap();

        let floxhub_url = "https://env.flox.dev";
        let disable_metrics = true;
        let search_limit = NonZero::new(42).unwrap();

        fs::write(user_config_dir.path().join(FLOX_CONFIG_FILE), formatdoc! {"
            floxhub_url = 'https://user.flox.dev'
            disable_metrics = false
            search_limit = 12
        "})
        .unwrap();

        fs::write(
            system_config_dir.path().join(FLOX_CONFIG_FILE),
            formatdoc! {"
            floxhub_url = 'https://system.flox.dev'
            disable_metrics = false
            search_limit = 24
        "},
        )
        .unwrap();

        let env = [
            ("FLOX_FLOXHUB_URL".into(), floxhub_url.to_owned()),
            ("FLOX_DISABLE_METRICS".into(), format!("{disable_metrics}")),
            ("FLOX_SEARCH_LIMIT".into(), format!("{search_limit}")),
        ];

        let config = Config::parse_with(
            &mock_flox_dirs(),
            user_config_dir.path(),
            Some(system_config_dir.path()),
            env,
        )
        .unwrap();

        assert_eq!(config.flox.floxhub_url, Some(floxhub_url.parse().unwrap()));
        assert!(config.flox.disable_metrics);
        assert_eq!(config.flox.search_limit, Some(search_limit));
    }

    #[test]
    fn test_writing_value() {
        let config_content = Config::write_to(
            None,
            &Key::parse("floxhub_url").unwrap(),
            Some("https://example.com"),
        )
        .unwrap();
        assert_eq!(config_content, indoc! {"
            floxhub_url = \"https://example.com\"
            "})
    }
    #[test]
    fn test_appending_value() {
        let config_before = indoc! {"
        floxhub_url = \"hello\"
        "};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("disable_metrics").unwrap(),
            Some(true),
        )
        .unwrap();
        assert_eq!(config_content, indoc! {"
        floxhub_url = \"hello\"
        disable_metrics = true
        "});
    }

    #[test]
    fn test_appending_value_keep_comment() {
        let config_before = indoc! {"
        # my FloxHub url is friendly, see:
        floxhub_url = \"hello\"
        "};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("disable_metrics").unwrap(),
            Some(true),
        )
        .unwrap();
        assert_eq!(config_content, indoc! {"
        # my FloxHub url is friendly, see:
        floxhub_url = \"hello\"
        disable_metrics = true
        "});
    }

    #[test]
    fn test_writing_bool() {
        let config_content =
            Config::write_to(None, &Key::parse("disable_metrics").unwrap(), Some(true)).unwrap();
        assert_eq!(config_content, indoc! {"
        disable_metrics = true
        "});
    }

    #[test]
    fn test_writing_invalid() {
        let config_content =
            Config::write_to(None, &Key::parse("does_not_exist").unwrap(), Some("true"));
        assert!(matches!(config_content, Err(ReadWriteError::InvalidKey(_))));
    }

    #[test]
    fn test_remove() {
        let config_before = indoc! {"
        # my git base url is friendly, see:
        git_base_url = \"hello\"
        "};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("git_base_url").unwrap(),
            None::<()>,
        )
        .unwrap();
        assert_eq!(config_content, indoc! {""});
    }

    #[test]
    fn test_remove_invalid() {
        let config_before = indoc! {"
        # my git base url is friendly, see:
        git_base_url = \"hello\"
        "};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("invalid").unwrap(),
            None::<()>,
        );
        assert!(matches!(
            config_content,
            Err(ReadWriteError::NotAUserValue(_))
        ));
    }

    #[test]
    fn test_remove_not_present() {
        let config_before = indoc! {""};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("git_base_url").unwrap(),
            None::<()>,
        );
        assert!(matches!(
            config_content,
            Err(ReadWriteError::NotAUserValue(_))
        ));
    }

    #[test]
    fn test_remove_nested_not_present() {
        let config_before = indoc! {""};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("trusted_environments.\"foo/bar\"").unwrap(),
            None::<()>,
        );
        assert!(matches!(
            config_content,
            Err(ReadWriteError::NotAUserValue(_))
        ));
    }

    #[test]
    fn test_remove_nested() {
        let config_before = indoc! {"
        [trusted_environments]
        \"foo/bar\" = \"baz\"
        "};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("trusted_environments.\"foo/bar\"").unwrap(),
            None::<()>,
        )
        .unwrap();
        assert_eq!(config_content, indoc! {"
        [trusted_environments]
        "});
    }

    proptest! {
        #[test]
        fn installer_channel_display_matches_serialized(channel in any::<InstallerChannel>()) {
            let display_quoted = format!("\"{}\"", channel);
            let serialized = serde_json::to_string(&channel).unwrap();
            prop_assert_eq!(display_quoted, serialized);
        }
    }
}
