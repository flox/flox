use std::{env, path::PathBuf};

use anyhow::{Error, Result};
use config::{Config, Environment};
use log::info;
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct CliConfig {
    #[serde(flatten)]
    enable: CliEnable,
    flox: FloxConfig,
    nix: NixConfig,
    github: GithubConfig,
}

// TODO: move to flox_sdk?
/// Describes the Configuration for the flox library
#[derive(Debug, Deserialize, Default)]
pub struct FloxConfig {}

// TODO: move to runix?
/// Describes the nix config under flox
#[derive(Debug, Deserialize, Default)]
pub struct NixConfig {}

/// Describes the github config under flox
#[derive(Debug, Deserialize, Default)]
pub struct GithubConfig {}

/// controls wheter flox-preview is enabled or not
#[derive(Debug, Deserialize, Default)]
pub struct CliEnable {
    enable: bool,
}

impl CliConfig {
    /// Creates a raw [Config] object
    pub fn config() -> Result<Config> {
        let config_dir = env::var("FLOX_PREVIEW_CONFIG_DIR")
            .map(Into::<PathBuf>::into)
            .or_else::<Error, _>(|e| {
                info!("`FLOX_PREVIEW_CONFIG_DIR` not set");
                let config_dir = dirs::config_dir().unwrap();
                Ok(config_dir.join("flox-preview"))
            })?;

        let builder = Config::builder()
            .add_source(
                config::File::with_name(config_dir.join("flox").to_str().unwrap()).required(false),
            )
            .add_source(Environment::with_prefix("FLOX_PREVIEW"));
        let final_config = builder.build()?;
        Ok(final_config)
    }

    /// Creates a [CliConfig] from the environment and config file
    pub fn parse() -> Result<CliConfig> {
        let final_config = Self::config()?;
        let cli_confg = final_config.try_deserialize()?;
        Ok(cli_confg)
    }

    /// Extracts the enable option
    pub fn enabled() -> Result<bool> {
        let enabled: CliEnable = Self::config()?.try_deserialize()?;
        Ok(enabled.enable)
    }
}

// struct FloxConfiguration {
//     defaultFlake: String
// }

// impl FloxConfiguration {

// }
