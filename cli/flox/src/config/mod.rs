use std::collections::HashMap;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::{env, fs};

use anyhow::{Context, Result};
use config::{Config as HierarchicalConfig, Environment};
use flox_rust_sdk::flox::EnvironmentRef;
use flox_rust_sdk::models::search::SearchLimit;
use itertools::{Either, Itertools};
use log::{debug, trace};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use tempfile::PersistError;
use thiserror::Error;
use toml_edit::{DocumentMut, Item, Key, Table, TableLike};
use url::Url;
use xdg::BaseDirectories;

/// Name of flox managed directories (config, data, cache)
const FLOX_DIR_NAME: &str = "flox";
const FLOX_CONFIG_DIR_VAR: &str = "FLOX_CONFIG_DIR";
pub const FLOX_CONFIG_FILE: &str = "flox.toml";

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct Config {
    /// flox configuration options
    #[serde(default, flatten)]
    pub flox: FloxConfig,

    /// nix configuration options
    #[serde(default)]
    pub nix: Option<NixConfig>,

    /// Feature flags are set from config but more commonly controlled by
    /// `FLOX_FEATURES_` environment variables.
    ///
    /// Accessing from `flox_rust_sdk::flox::Flox.features` should be prefered
    /// over `flox::config::Config.features` if both are available.
    #[serde(default)]
    pub features: Option<flox_rust_sdk::flox::Features>,
}

// TODO: move to flox_sdk?
/// Describes the Configuration for the flox library
#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct FloxConfig {
    /// Disable collecting and sending usage metrics
    #[serde(default)]
    pub disable_metrics: bool,
    /// Directory where flox should store ephemeral data (default:
    /// `$XDG_CACHE_HOME/flox`)
    pub cache_dir: PathBuf,
    /// Directory where flox should store persistent data (default:
    /// `$XDG_DATA_HOME/flox`)
    pub data_dir: PathBuf,
    /// Directory where flox should load its configuration file (default:
    /// `$XDG_CONFIG_HOME/flox`)
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

    /// Rule whether to change the shell prompt in activated environments
    pub shell_prompt: Option<EnvironmentPromptConfig>,
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

// TODO: move to runix?
/// Describes the nix config under flox
#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct NixConfig {
    pub access_tokens: HashMap<String, String>,
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

impl Config {
    /// Creates a raw [Config] object and caches it for the lifetime of the program
    fn raw_config(mut reload: bool) -> Result<HierarchicalConfig> {
        static INSTANCE: OnceCell<Mutex<HierarchicalConfig>> = OnceCell::new();

        debug!(
            "reading raw config (initialized: {initialized}, reload: {reload})",
            initialized = INSTANCE.get().is_some()
        );

        fn read_raw_cofig() -> Result<HierarchicalConfig> {
            let flox_dirs = BaseDirectories::with_prefix(FLOX_DIR_NAME)?;

            let cache_dir = flox_dirs.get_cache_home();
            let data_dir = flox_dirs.get_data_home();

            let config_dir = match env::var(FLOX_CONFIG_DIR_VAR) {
                Ok(v) => {
                    debug!("`${FLOX_CONFIG_DIR_VAR}` set: {v}");
                    fs::create_dir_all(&v)
                        .context(format!("Could not create config directory: {v:?}"))?;
                    v.into()
                },
                Err(_) => {
                    let config_dir = flox_dirs.get_config_home();
                    debug!("`${FLOX_CONFIG_DIR_VAR}` not set, using {config_dir:?}");
                    fs::create_dir_all(&config_dir)
                        .context(format!("Could not create config directory: {config_dir:?}"))?;
                    let config_dir = config_dir
                        .canonicalize()
                        .context("Could not canonicalize config directory '{config_dir:?}'")?;
                    env::set_var(FLOX_CONFIG_DIR_VAR, &config_dir);
                    config_dir
                },
            };

            let mut builder = HierarchicalConfig::builder()
                .set_default("default_substituter", "https://cache.floxdev.com/")?
                .set_default("cache_dir", cache_dir.to_str().unwrap())?
                .set_default("data_dir", data_dir.to_str().unwrap())?
                // Config dir is added to the config for completeness;
                // the config file cannot change the config dir.
                .set_override("config_dir", config_dir.to_str().unwrap())?;

            // read from /etc
            builder = builder.add_source(
                config::File::from(PathBuf::from("/etc").join(FLOX_CONFIG_FILE))
                    .format(config::FileFormat::Toml)
                    .required(false),
            );

            // look for files in XDG_CONFIG_DIRS locations
            for file in flox_dirs.find_config_files(FLOX_CONFIG_FILE) {
                builder =
                    builder.add_source(config::File::from(file).format(config::FileFormat::Toml));
            }

            // Add explicit FLOX_CONFIG_DIR file last
            builder = builder.add_source(
                config::File::from(config_dir.join(FLOX_CONFIG_FILE))
                    .format(config::FileFormat::Toml)
                    .required(false),
            );

            // override via env variables
            let mut flox_envs = env::vars()
                .filter_map(|(k, v)| k.strip_prefix("FLOX_").map(|k| (k.to_owned(), v)))
                .collect::<Vec<_>>();

            let builder = builder
                .add_source(mk_environment(&mut flox_envs, "NIX"))
                .add_source(mk_environment(&mut flox_envs, "GITHUB"))
                .add_source(mk_environment(&mut flox_envs, "FEATURES"))
                .add_source(
                    Environment::default()
                        .source(Some(HashMap::from_iter(flox_envs)))
                        .try_parsing(true),
                );

            let final_config = builder.build()?;
            Ok(final_config)
        }

        let instance = INSTANCE.get_or_try_init(|| {
            // If we are initializing the config for the first time,
            // we don't need to reload right after
            reload = false;
            let config = read_raw_cofig()?;

            Ok::<_, anyhow::Error>(Mutex::new(config))
        })?;

        let mut config_guard = instance.lock().expect("config mutex poisoned");
        if reload {
            *config_guard = read_raw_cofig()?;
        }

        return Ok(config_guard.deref().clone());
    }

    /// Creates a [Config] from the environment and config file
    ///
    /// When running in tests, the config is reloaded on every call.
    pub fn parse() -> Result<Config> {
        #[cfg(test)]
        let reload = true;

        #[cfg(not(test))]
        let reload = false;

        let final_config = Self::raw_config(reload)?;
        let cli_confg: Config = final_config
            .to_owned()
            .try_deserialize()
            .context("Could not parse config")?;
        Ok(cli_confg)
    }

    /// get a value from the config
    ///
    /// **intended for human consumtion/intospection of config only**
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

    /// Append or update a key value parin in the toml representation of a partial config
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

    use indoc::indoc;

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
                env::set_var("FLOX_FLOXHUB_URL", "https://example.com");
                let config = Config::parse().unwrap();
                assert_eq!(
                    config.get(&Key::parse("floxhub_url").unwrap()).unwrap(),
                    "\"https://example.com/\"".to_string()
                );
                env::remove_var(FLOX_CONFIG_DIR_VAR);
            },
        );
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
    fn test_writing_nested() {
        let config_content = Config::write_to(
            None,
            &Key::parse("nix.access_tokens.\"github.com\"").unwrap(),
            Some("ghp_my_access_token"),
        )
        .unwrap();
        assert_eq!(config_content, indoc! {"
        [nix]

        [nix.access_tokens]
        \"github.com\" = \"ghp_my_access_token\"
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
            &Key::parse("nix.access_tokens.\"github.com\"").unwrap(),
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
        [nix]

        [nix.access_tokens]
        \"github.com\" = \"ghp_my_access_token\"
        "};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("nix.access_tokens.\"github.com\"").unwrap(),
            None::<()>,
        )
        .unwrap();
        assert_eq!(config_content, indoc! {"
        [nix]

        [nix.access_tokens]
        "});
    }
}
