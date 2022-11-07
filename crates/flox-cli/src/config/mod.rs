use std::{env, path::PathBuf};

use anyhow::{Context, Error, Result};
use config::{Config as HierarchicalConfig, Environment};
use log::info;
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    /// flox configuration options
    #[serde(default)]
    pub flox: FloxConfig,

    /// nix configuration options
    #[serde(default)]
    pub nix: NixConfig,

    /// github configuration options
    #[serde(default)]
    pub github: GithubConfig,
}

// TODO: move to flox_sdk?
/// Describes the Configuration for the flox library
#[derive(Debug, Deserialize, Default)]
pub struct FloxConfig {
    pub allow_telemetry: Option<bool>,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub config_dir: PathBuf,
}

// TODO: move to runix?
/// Describes the nix config under flox
#[derive(Debug, Deserialize, Default)]
pub struct NixConfig {}

/// Describes the github config under flox
#[derive(Debug, Deserialize, Default)]
pub struct GithubConfig {}

/// controls wheter flox-preview is enabled or not
#[derive(Debug, Deserialize, Default)]
pub struct EnablePreview {
    #[serde(default)]
    enable: bool,
}

impl Config {
    /// Creates a raw [Config] object
    fn raw_config() -> Result<HierarchicalConfig> {
        let cache_dir = dirs::cache_dir().unwrap().join("flox-preview");
        let data_dir = dirs::data_dir().unwrap().join("flox-preview");
        let config_dir = match env::var("FLOX_PREVIEW_CONFIG_DIR") {
            Ok(v) => v.into(),
            Err(_) => {
                info!("`FLOX_PREVIEW_CONFIG_DIR` not set");
                let config_dir = dirs::config_dir().unwrap();
                config_dir.join("flox-preview")
            }
        };

        let builder = HierarchicalConfig::builder()
            .set_default("flox.cache_dir", cache_dir.to_str().unwrap())?
            .set_default("flox.data_dir", data_dir.to_str().unwrap())?
            .set_default("flox.config_dir", config_dir.to_str().unwrap())?
            .add_source(
                config::File::with_name(config_dir.join("flox").to_str().unwrap()).required(false),
            )
            .add_source(Environment::with_prefix("FLOX_PREVIEW"));
        let final_config = builder.build()?;
        Ok(final_config)
    }

    /// Creates a [CliConfig] from the environment and config file
    pub fn parse() -> Result<Config> {
        let final_config = Self::raw_config()?;
        let cli_confg = final_config
            .try_deserialize()
            .context("Could not parse config")?;
        Ok(cli_confg)
    }

    /// Reuses the same hierarchical config (config dir < ENV) but
    /// only reads into an [EnablePreview] object.
    ///
    /// Preview config files may be invalid as long as the
    /// CLI is in passthrough mode.
    pub fn preview_enabled() -> Result<bool> {
        let enabled = Self::raw_config()?
            .try_deserialize::<EnablePreview>()?
            .enable;
        Ok(enabled)
    }
}
