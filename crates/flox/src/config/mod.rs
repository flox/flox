use std::collections::HashMap;
use std::path::PathBuf;
use std::{env, fs};

use anyhow::{Context, Result};
use config::{Config as HierarchicalConfig, Environment};
use flox_rust_sdk::prelude::Stability;
use itertools::{Either, Itertools};
use log::{debug, trace};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use tempfile::PersistError;
use thiserror::Error;
use toml_edit::{Document, Item, Key, Table, TableLike};
use xdg::BaseDirectories;

/// Name of flox managed directories (config, data, cache)
const FLOX_DIR_NAME: &'_ str = "flox";
const FLOX_SH_PATH: &'_ str = env!("FLOX_SH_PATH");

#[derive(Clone, Debug, Deserialize, Default, Serialize)]
pub struct Config {
    /// flox configuration options
    #[serde(default, flatten)]
    pub flox: FloxConfig,

    /// nix configuration options
    #[serde(default)]
    pub nix: NixConfig,

    /// github configuration options
    #[serde(default)]
    pub github: GithubConfig,

    #[serde(default)]
    pub features: HashMap<features::Feature, features::Impl>,
}

// TODO: move to flox_sdk?
/// Describes the Configuration for the flox library
#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct FloxConfig {
    #[serde(default)]
    pub disable_metrics: bool,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    #[serde(default)]
    pub stability: Stability,

    pub default_substituter: String, // Todo: use Url type?

    #[serde(flatten)]
    pub instance: InstanceConfig,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct InstanceConfig {
    pub git_base_url: String, // Todo: use Url type?
}

// TODO: move to runix?
/// Describes the nix config under flox
#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct NixConfig {
    pub access_tokens: HashMap<String, String>,
}

/// Describes the github config under flox
#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct GithubConfig {}
pub mod features;

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
    #[error(transparent)]
    Persist(#[from] PersistError),
}

impl Config {
    /// Creates a raw [Config] object and caches it for the lifetime of the program
    fn raw_config<'a>() -> Result<&'a HierarchicalConfig> {
        static INSTANCE: OnceCell<HierarchicalConfig> = OnceCell::new();
        INSTANCE.get_or_try_init(|| {
            let flox_dirs = BaseDirectories::with_prefix(FLOX_DIR_NAME)?;

            let cache_dir = flox_dirs.get_cache_home();
            let data_dir = flox_dirs.get_data_home();
            let config_dir = match env::var("FLOX_CONFIG_HOME") {
                Ok(v) => {
                    debug!("`$FLOX_CONFIG_HOME` set: {v}");
                    fs::create_dir_all(&v)
                        .context(format!("Could not create config directory: {v:?}"))?;
                    v.into()
                },
                Err(_) => {
                    let config_dir = flox_dirs.get_config_home();
                    debug!("`$FLOX_CONFIG_HOME` not set, using {config_dir:?}");
                    fs::create_dir_all(&config_dir)
                        .context(format!("Could not create config directory: {config_dir:?}"))?;
                    debug!("`FLOX_CONFIG_HOME` not set, using {config_dir:?}");
                    let config_dir = config_dir
                        .canonicalize()
                        .context("Could not canonicalize  conifig directory '{config_dir:?}'")?;
                    env::set_var("FLOX_CONFIG_HOME", &config_dir);
                    config_dir
                },
            };

            let mut builder = HierarchicalConfig::builder()
                .set_default("default_substituter", "https://cache.floxdev.com/")?
                .set_default("git_base_url", "https://github.com/")?
                .set_default("cache_dir", cache_dir.to_str().unwrap())?
                .set_default("data_dir", data_dir.to_str().unwrap())?
                // config dir is added to the config for completeness, the config file cannot chenge the config dir
                .set_default("config_dir", config_dir.to_str().unwrap())?;

            // read from (flox-bash) installation
            builder = builder.add_source(
                config::File::from(PathBuf::from("/etc").join("flox.toml"))
                    .format(config::FileFormat::Toml)
                    .required(false),
            );

            // read from (flox-bash) installation
            builder = builder.add_source(
                config::File::from(PathBuf::from(FLOX_SH_PATH).join("etc").join("flox.toml"))
                    .format(config::FileFormat::Toml),
            );

            // look for files in XDG_CONFIG_DIRS locations
            for file in flox_dirs.find_config_files("flox.toml") {
                builder =
                    builder.add_source(config::File::from(file).format(config::FileFormat::Toml));
            }

            // Add explicit FLOX_CONFIG_HOME file last
            builder = builder.add_source(
                config::File::from(config_dir.join("flox.toml"))
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
        })
    }

    /// Creates a [Config] from the environment and config file
    pub fn parse() -> Result<Config> {
        let final_config = Self::raw_config()?;
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
        let document: toml_edit::Document = toml_edit::ser::to_document(self)?;

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
            Some(content) => content.parse::<Document>()?,
            None => Document::new(),
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
    fn test_read_flattened() {
        let mut config = Config::default();
        config.flox.instance.git_base_url = "hello".to_string();
        assert!(matches!(
            config.get(&Key::parse("flox.instance.git_base_url").unwrap()),
            Err(ReadWriteError::InvalidKey(_))
        ));
        assert_eq!(
            config.get(&Key::parse("git_base_url").unwrap()).unwrap(),
            "\"hello\"".to_string()
        );
    }

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
                ("FLOX_GIT_BASE_URL", Some("hello")),
            ],
            || {
                env::set_var("FLOX_GIT_BASE_URL", "hello");
                let config = Config::parse().unwrap();
                assert_eq!(
                    config.get(&Key::parse("git_base_url").unwrap()).unwrap(),
                    "\"hello\"".to_string()
                );
                env::remove_var("FLOX_CONFIG_HOME");
            },
        );
    }

    #[test]
    fn test_writing_value() {
        let config_content =
            Config::write_to(None, &Key::parse("git_base_url").unwrap(), Some("hello")).unwrap();
        assert_eq!(config_content, indoc! {"
            git_base_url = \"hello\"
            "})
    }
    #[test]
    fn test_appending_value() {
        let config_before = indoc! {"
        git_base_url = \"hello\"
        "};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("stability").unwrap(),
            Some("stable"),
        )
        .unwrap();
        assert_eq!(config_content, indoc! {"
        git_base_url = \"hello\"
        stability = \"stable\"
        "});
    }

    #[test]
    fn test_appending_value_keep_comment() {
        let config_before = indoc! {"
        # my git base url is friendly, see:
        git_base_url = \"hello\"
        "};

        let config_content = Config::write_to(
            Some(config_before.to_string()),
            &Key::parse("stability").unwrap(),
            Some("stable"),
        )
        .unwrap();
        assert_eq!(config_content, indoc! {"
        # my git base url is friendly, see:
        git_base_url = \"hello\"
        stability = \"stable\"
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
