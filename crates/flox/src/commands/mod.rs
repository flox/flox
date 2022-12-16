mod channel;
mod environment;
mod general;
mod package;

use std::{env, fs};
use std::{os::unix::process, str::FromStr};

use anyhow::Result;
use bpaf::{command, construct, Bpaf, Parser};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::prelude::{Channel, ChannelRegistry};
use log::warn;
use tempfile::TempDir;

use crate::utils::{init::init_access_tokens, init::init_channels, init::init_git_conf};
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
    pub async fn handle(self, config: crate::config::Config) -> Result<()> {
        // prepare a temp dir for the run:
        let process_dir = config.flox.cache_dir.join("process");
        tokio::fs::create_dir_all(&process_dir).await?;

        // `temp_dir` will automatically be removed from disk when the function returns
        let temp_dir = TempDir::new_in(process_dir)?;
        let temp_dir_path = temp_dir.path().to_owned();

        init_git_conf(temp_dir.path()).await?;

        let channels = init_channels()?;

        let access_tokens = init_access_tokens(&config.nix.access_tokens)?;

        let netrc_file = dirs::home_dir()
            .expect("User must have a home directory")
            .join(".netrc");

        let flox = Flox {
            collect_metrics: config.flox.allow_telemetry.unwrap_or_default(),
            cache_dir: config.flox.cache_dir.clone(),
            data_dir: config.flox.data_dir.clone(),
            config_dir: config.flox.config_dir.clone(),
            channels,
            access_tokens,
            netrc_file,
            temp_dir: temp_dir_path.clone(),
            system: env!("NIX_TARGET_SYSTEM").to_string(),
        };

        // in debug mode keep the tempdir to reproduce nix commands
        if self.debug {
            let _ = temp_dir.into_path();
        }

        // Ensure cursor doesn't remain hidden if a prompt gets cancelled
        // Upstream issue is https://github.com/mitsuhiko/dialoguer/issues/77 - though considered intentional behavior
        ctrlc::set_handler(move || {
            if dialoguer::console::user_attended_stderr() {
                dialoguer::console::Term::stderr()
                    .show_cursor()
                    .expect("Failed to re-show hidden cursor before exit");
            }

            // in case of SIG* the drop handler of temp_dir will not be called
            // if we are not in debugging mode, drop the tempdir manually
            if !self.debug {
                let _ = fs::remove_dir_all(&temp_dir_path);
            }
        })?;

        match self.command {
            Commands::Package(ref package) => package.handle(config, flox).await?,
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
