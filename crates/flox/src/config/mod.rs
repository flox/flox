use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};
use config::{Config as HierarchicalConfig, Environment};
use flox_rust_sdk::prelude::Stability;
use itertools::{Either, Itertools};
use log::debug;
use once_cell::sync::OnceCell;
use serde::Deserialize;
use xdg::BaseDirectories;

/// Name of flox managed directories (config, data, cache)
const FLOX_DIR_NAME: &'_ str = "flox-preview";

#[derive(Clone, Debug, Deserialize, Default)]
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
#[derive(Clone, Debug, Deserialize, Default)]
pub struct FloxConfig {
    #[serde(default)]
    pub disable_telemetry: bool,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
    #[serde(default)]
    pub stability: Stability,
}

// TODO: move to runix?
/// Describes the nix config under flox
#[derive(Clone, Debug, Deserialize, Default)]
pub struct NixConfig {
    pub access_tokens: HashMap<String, String>,
}

/// Describes the github config under flox
#[derive(Clone, Debug, Deserialize, Default)]
pub struct GithubConfig {}
pub mod features;

impl Config {
    /// Creates a raw [Config] object and caches it for the lifetime of the program
    fn raw_config<'a>() -> Result<&'a HierarchicalConfig> {
        static INSTANCE: OnceCell<HierarchicalConfig> = OnceCell::new();
        INSTANCE.get_or_try_init(|| {
            let flox_dirs = BaseDirectories::with_prefix(FLOX_DIR_NAME)?;

            let cache_dir = flox_dirs.get_cache_home();
            let data_dir = flox_dirs.get_data_home();
            let config_dir = match env::var("FLOX_PREVIEW_CONFIG_DIR") {
                Ok(v) => v.into(),
                Err(_) => {
                    let config_dir = flox_dirs.get_config_home();
                    debug!("`FLOX_PREVIEW_CONFIG_DIR` not set, using {config_dir:?}");
                    config_dir
                },
            };

            let builder = HierarchicalConfig::builder()
                .set_default("cache_dir", cache_dir.to_str().unwrap())?
                .set_default("data_dir", data_dir.to_str().unwrap())?
                // config dir is added to the config for completenes, the config file cannot chenge the config dir
                .set_default("config_dir", config_dir.to_str().unwrap())?
                .add_source(
                    config::File::with_name("flox")
                        .required(false),
                );

            let mut flox_envs = env::vars()
                .filter_map(|(k, v)| k.strip_prefix("FLOX_PREVIEW_").map(|k| (k.to_owned(), v)))
                .collect::<Vec<_>>();

            let builder = builder
                .add_source(mk_environment(&mut flox_envs, "NIX"))
                .add_source(mk_environment(&mut flox_envs, "GITHUB"))
                .add_source(mk_environment(&mut flox_envs, "FEATURES"))
                .add_source(Environment::default().source(Some(HashMap::from_iter(flox_envs))));

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
        .source(Some(prefixed_envs));
    *envs = flox_envs;
    environment
}
