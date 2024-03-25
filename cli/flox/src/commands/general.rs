use std::io;
use std::path::Path;

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use fslock::LockFile;
use indoc::indoc;
use serde::Serialize;
use tokio::fs;
use toml_edit::Key;
use tracing::instrument;

use crate::config::{Config, ReadWriteError, FLOX_CONFIG_FILE};
use crate::subcommand_metric;
use crate::utils::message;
use crate::utils::metrics::{
    METRICS_EVENTS_FILE_NAME,
    METRICS_LOCK_FILE_NAME,
    METRICS_UUID_FILE_NAME,
};

// Reset the metrics queue (if any), reset metrics ID, and re-prompt for consent
#[derive(Bpaf, Clone)]
pub struct ResetMetrics {}
impl ResetMetrics {
    #[instrument(name = "reset-metrics", skip_all)]
    pub async fn handle(self, _config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("reset-metrics");
        let mut metrics_lock = LockFile::open(&flox.cache_dir.join(METRICS_LOCK_FILE_NAME))?;
        tokio::task::spawn_blocking(move || metrics_lock.lock()).await??;

        if let Err(err) =
            tokio::fs::remove_file(flox.cache_dir.join(METRICS_EVENTS_FILE_NAME)).await
        {
            match err.kind() {
                std::io::ErrorKind::NotFound => {},
                _ => Err(err)?,
            }
        }

        if let Err(err) = tokio::fs::remove_file(flox.data_dir.join(METRICS_UUID_FILE_NAME)).await {
            match err.kind() {
                std::io::ErrorKind::NotFound => {},
                _ => Err(err)?,
            }
        }

        let notice = indoc! {"
            Sucessfully reset telemetry ID for this machine!

            A new ID will be assigned next time you use flox.

            The collection of metrics can be disabled in the following ways:

                environment: FLOX_DISABLE_METRICS=true
                user-wide: flox config --set-bool disable_metrics true
                system-wide: update /etc/flox.toml as described in flox(1)
        "};

        message::plain(notice);
        Ok(())
    }
}

#[derive(Bpaf, Clone)]
#[bpaf(fallback(ConfigArgs::List))]
pub enum ConfigArgs {
    /// List the current values of all options
    #[bpaf(short, long)]
    List,
    /// Reset all options to their default values without further confirmation
    #[bpaf(short, long)]
    Reset,
    /// Set a config value
    Set(#[bpaf(external(config_set))] ConfigSet),
    /// Set a numeric config value
    SetNumber(#[bpaf(external(config_set_number))] ConfigSetNumber),
    /// Set a boolean config value
    SetBool(#[bpaf(external(config_set_bool))] ConfigSetBool),
    /// Delete a config value
    Delete(#[bpaf(external(config_delete))] ConfigDelete),
}

impl ConfigArgs {
    /// handle config flags like commands
    #[instrument(name = "config", skip_all)]
    pub async fn handle(&self, config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("config");
        match self {
            ConfigArgs::List => println!("{}", config.get(&[])?),
            ConfigArgs::Reset => {
                match fs::remove_file(&flox.config_dir.join(FLOX_CONFIG_FILE)).await {
                    Err(err) if err.kind() != io::ErrorKind::NotFound => {
                        Err(err).context("Could not reset config file")?
                    },
                    _ => (),
                }
            },
            ConfigArgs::Set(ConfigSet { key, value, .. }) => {
                update_config(&flox.config_dir, &flox.temp_dir, key, Some(value))?
            },
            ConfigArgs::SetNumber(ConfigSetNumber { key, value, .. }) => {
                update_config(&flox.config_dir, &flox.temp_dir, key, Some(value))?
            },
            ConfigArgs::SetBool(ConfigSetBool { key, value, .. }) => {
                update_config(&flox.config_dir, &flox.temp_dir, key, Some(value))?
            },
            ConfigArgs::Delete(ConfigDelete { key, .. }) => {
                update_config::<()>(&flox.config_dir, &flox.temp_dir, key, None)?
            },
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Bpaf)]
#[bpaf(adjacent)]
pub struct ConfigSet {
    /// set <key> to <string>
    #[allow(unused)]
    set: (),
    /// Configuration key
    #[bpaf(positional("key"))]
    key: String,
    /// Configuration value (string)
    #[bpaf(positional("string"))]
    value: String,
}

#[derive(Debug, Clone, Bpaf)]
#[bpaf(adjacent)]
#[allow(unused)]
pub struct ConfigSetNumber {
    /// Set <key> to <number>
    #[bpaf(long("set-number"))]
    set_number: (),
    /// Configuration key
    #[bpaf(positional("key"))]
    key: String,
    /// Configuration value (i32)
    #[bpaf(positional("number"))]
    value: i32,
}

#[derive(Debug, Clone, Bpaf)]
#[bpaf(adjacent)]
#[allow(unused)]
pub struct ConfigSetBool {
    /// Set <key> to <bool>
    #[bpaf(long("set-bool"))]
    set_bool: (),
    /// Configuration key
    #[bpaf(positional("key"))]
    key: String,
    /// Configuration value (bool)
    #[bpaf(positional("bool"))]
    value: bool,
}

#[derive(Debug, Clone, Bpaf)]
#[allow(unused)]
pub struct ConfigDelete {
    /// Delete config key
    #[bpaf(long("delete"), argument("key"))]
    key: String,
}

/// wrapper around [Config::write_to]
pub(super) fn update_config<V: Serialize>(
    config_dir: &Path,
    temp_dir: &Path,
    key: impl AsRef<str>,
    value: Option<V>,
) -> Result<()> {
    let query = Key::parse(key.as_ref()).context("Could not parse key")?;

    let config_file_path = config_dir.join(FLOX_CONFIG_FILE);

    match Config::write_to_in(config_file_path, temp_dir, &query, value) {
                err @ Err(ReadWriteError::ReadConfig(_)) => err.context("Could not read current config file.\nPlease verify the format or reset using `flox config --reset`")?,
                err @ Err(_) => err?,
                Ok(()) => ()
            }
    Ok(())
}
