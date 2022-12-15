mod channel;
mod environment;
mod general;
mod package;

use std::{os::unix::process, str::FromStr};

use anyhow::Result;
use bpaf::{command, construct, Bpaf, Parser};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::prelude::{Channel, ChannelRegistry};
use tempfile::TempDir;

use crate::utils::init_channels;
use flox_rust_sdk::flox::{FLOX_SH, FLOX_VERSION};

use self::channel::{ChannelArgs, ChannelCommands};
use self::environment::{EnvironmentArgs, EnvironmentCommands};
use self::general::{GeneralArgs, GeneralCommands};
use self::package::{PackageArgs, PackageCommands};

#[derive(Bpaf)]
#[bpaf(options, version(FLOX_VERSION))]
pub struct FloxArgs {
    /// Verbose mode.
    ///
    /// Invoke multiple times for increasing detail.
    verbose: bool,

    /// Debug mode.
    ///
    /// Invoke multiple times for increasing detail.
    debug: bool,

    #[bpaf(external(commands))]
    command: Commands,
}

impl FloxArgs {
    /// Initialize the command line by creating an initial FloxBuilder
    pub async fn handle(&self, config: crate::config::Config) -> Result<()> {
        // prepare a temp dir for the run:
        let process_dir = config.flox.cache_dir.join("process");
        tokio::fs::create_dir_all(&process_dir).await?;

        // `temp_dir` will automatically be removed from disk when the function returns
        let temp_dir = TempDir::new_in(process_dir)?;

        let channels = init_channels()?;

        let flox = Flox {
            collect_metrics: config.flox.allow_telemetry.unwrap_or_default(),
            cache_dir: config.flox.cache_dir,
            data_dir: config.flox.data_dir,
            config_dir: config.flox.config_dir,
            channels,
            temp_dir: temp_dir.path().to_path_buf(),
            system: env!("NIX_TARGET_SYSTEM").to_string(),
        };

        match self.command {
            Commands::Package(ref package) => package.handle(flox).await?,
            Commands::Environment(ref environment) => environment.handle(flox).await?,
            Commands::Channel(ref channel) => channel.handle(flox).await?,
            Commands::General(ref general) => general.handle(flox).await?,
        }
        Ok(())
    }
}

/// Transparent separation of different categories of commands
#[derive(Bpaf)]
pub enum Commands {
    Package(
        #[bpaf(external(package::package_commands))]
        #[bpaf(group_help("Development Commands"))]
        PackageCommands,
    ),
    Environment(
        #[bpaf(external(environment::environment_commands))]
        #[bpaf(group_help("Environment Commands"))]
        EnvironmentCommands,
    ),
    Channel(
        #[bpaf(external(channel::channel_commands))]
        #[bpaf(group_help("Channel Commands"))]
        ChannelCommands,
    ),
    General(
        #[bpaf(external(general::general_commands))]
        #[bpaf(group_help("General Commands"))]
        GeneralCommands,
    ),
}
