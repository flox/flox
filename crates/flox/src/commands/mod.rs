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

use crate::FLOX_VERSION;

use self::channel::ChannelArgs;
use self::environment::EnvironmentArgs;
use self::general::GeneralArgs;
use self::package::PackageArgs;

#[derive(Bpaf)]
#[bpaf(options, version(FLOX_VERSION))]
pub struct FloxArgs {
    verbose: bool,

    debug: bool,

    #[bpaf(external(commands))]
    command: Commands,

    #[bpaf(positional)]
    nix_args: Vec<String>,
}

impl FloxArgs {
    /// Initialize the command line by creating an initial FloxBuilder
    pub async fn handle(&self, config: crate::config::Config) -> Result<()> {
        // prepare a temp dir for the run:
        let process_dir = config.flox.cache_dir.join("process");
        tokio::fs::create_dir_all(&process_dir).await?;

        // `temp_dir` will automatically be removed from disk when the function returns
        let temp_dir = TempDir::new_in(process_dir)?;

        let mut channels = ChannelRegistry::default();
        channels.register_channel("flox", Channel::from_str("github:flox/floxpkgs")?);
        channels.register_channel("nixpkgs", Channel::from_str("github:flox/nixpkgs/stable")?);

        // generate these dynamically based on <?>
        channels.register_channel(
            "nixpkgs-stable",
            Channel::from_str("github:flox/nixpkgs/stable")?,
        );
        channels.register_channel(
            "nixpkgs-staging",
            Channel::from_str("github:flox/nixpkgs/staging")?,
        );
        channels.register_channel(
            "nixpkgs-unstable",
            Channel::from_str("github:flox/nixpkgs/unstable")?,
        );

        let flox = Flox {
            collect_metrics: config.flox.allow_telemetry.unwrap_or_default(),
            cache_dir: config.flox.cache_dir,
            data_dir: config.flox.data_dir,
            config_dir: config.flox.config_dir,
            channels: channels,
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
        #[bpaf(external(package::package_args))]
        #[bpaf(group_help("Development Commands"))]
        PackageArgs,
    ),
    Environment(
        #[bpaf(external(environment::environment_args))]
        #[bpaf(group_help("Environment Commands"))]
        EnvironmentArgs,
    ),
    Channel(
        #[bpaf(external(channel::channel_args))]
        #[bpaf(group_help("Channel Commands"))]
        ChannelArgs,
    ),
    General(
        #[bpaf(external(general::general_args))]
        #[bpaf(group_help("General Commands"))]
        GeneralArgs,
    ),
}
