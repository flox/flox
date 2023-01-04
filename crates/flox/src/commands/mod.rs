mod channel;
mod environment;
mod general;
mod package;

use std::{env, fs};

use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tempfile::TempDir;

use crate::utils::init::{
    init_access_tokens, init_channels, init_git_conf, init_telemetry_consent, init_uuid,
};
use flox_rust_sdk::flox::FLOX_VERSION;

use self::channel::ChannelCommands;
use self::environment::EnvironmentCommands;
use self::general::GeneralCommands;
use self::package::PackageCommands;

fn vec_len<T>(x: Vec<T>) -> usize {
    Vec::len(&x)
}

fn vec_not_empty<T>(x: Vec<T>) -> bool {
    !x.is_empty()
}

#[derive(Bpaf, Clone, Debug)]
pub enum Verbosity {
    Verbose(
        /// Verbose mode.
        ///
        /// Invoke multiple times for increasing detail.
        #[bpaf(short('v'), long("verbose"), switch, many, map(vec_len))]
        usize,
    ),

    #[bpaf(short, long)]
    Quiet,
}

#[derive(Bpaf)]
#[bpaf(options, version(FLOX_VERSION))]
pub struct FloxArgs {
    /// Verbose mode.
    ///
    /// Invoke multiple times for increasing detail.
    #[bpaf(external, fallback(Verbosity::Verbose(0)))]
    pub verbosity: Verbosity,

    /// Debug mode.
    #[bpaf(short, long, switch, many, map(vec_not_empty))]
    pub debug: bool,

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

        // disabling telemetry will work regardless
        // but we don't want to give users who disabled it the prompt
        if !config.flox.disable_telemetry {
            init_telemetry_consent(&config.flox.data_dir).await?;
        }

        let channels = init_channels()?;

        let access_tokens = init_access_tokens(&config.nix.access_tokens)?;

        let netrc_file = dirs::home_dir()
            .expect("User must have a home directory")
            .join(".netrc");

        let flox = Flox {
            cache_dir: config.flox.cache_dir.clone(),
            data_dir: config.flox.data_dir.clone(),
            config_dir: config.flox.config_dir.clone(),
            channels,
            access_tokens,
            netrc_file,
            temp_dir: temp_dir_path.clone(),
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            uuid: init_uuid(&config.flox.data_dir).await?,
        };

        // in debug mode keep the tempdir to reproduce nix commands
        if self.debug {
            let _ = temp_dir.into_path();
        }

        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.unwrap();
            // in case of SIG* the drop handler of temp_dir will not be called
            // if we are not in debugging mode, drop the tempdir manually
            if !self.debug {
                let _ = fs::remove_dir_all(&temp_dir_path);
            }
        });

        match self.command {
            Commands::Package(ref package) => package.handle(config, flox).await?,
            Commands::Environment(ref environment) => environment.handle(flox).await?,
            Commands::Channel(ref channel) => channel.handle(flox).await?,
            Commands::General(ref general) => general.handle(flox).await?,
            Commands::Prefix => println!(env!("out")),
        }

        Ok(())
    }
}

/// Transparent separation of different categories of commands
#[derive(Bpaf, Clone)]
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
    /// For development only
    #[bpaf(hide)]
    Prefix,
}
