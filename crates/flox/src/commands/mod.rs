mod channel;
mod environment;
mod general;
mod package;

use std::str::FromStr;
use std::{env, fs};

use anyhow::Result;
use bpaf::{Bpaf, Parser};
use flox_rust_sdk::flox::{Flox, FLOX_VERSION};
use flox_rust_sdk::prelude::Channel;
use tempfile::TempDir;

use self::channel::ChannelCommands;
use self::environment::EnvironmentCommands;
use self::general::GeneralCommands;
use self::package::interface;
use crate::utils::init::{
    init_access_tokens,
    init_channels,
    init_git_conf,
    init_telemetry_consent,
    init_uuid,
};

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

impl Default for Verbosity {
    fn default() -> Self {
        Verbosity::Verbose(0)
    }
}

#[derive(Bpaf)]
#[bpaf(options, version(FLOX_VERSION))]
pub struct FloxArgs {
    /// Verbose mode.
    ///
    /// Invoke multiple times for increasing detail.
    #[bpaf(external, fallback(Default::default()))]
    pub verbosity: Verbosity,

    /// Debug mode.
    #[bpaf(long, switch, many, map(vec_not_empty))]
    pub debug: bool,

    #[bpaf(external(commands))]
    command: Commands,
}

impl FloxArgs {
    /// Initialize the command line by creating an initial FloxBuilder
    pub async fn handle(self, mut config: crate::config::Config) -> Result<()> {
        // ensure xdg dirs exist
        tokio::fs::create_dir_all(&config.flox.config_dir).await?;
        tokio::fs::create_dir_all(&config.flox.data_dir).await?;

        // prepare a temp dir for the run:
        let process_dir = config.flox.cache_dir.join("process");
        tokio::fs::create_dir_all(&process_dir).await?;

        // `temp_dir` will automatically be removed from disk when the function returns
        let temp_dir = TempDir::new_in(process_dir)?;
        let temp_dir_path = temp_dir.path().to_owned();

        init_git_conf(temp_dir.path(), &config.flox.config_dir).await?;

        // disabling telemetry will work regardless
        // but we don't want to give users who disabled it the prompt
        if !config.flox.disable_telemetry {
            init_telemetry_consent(&config.flox.data_dir, &config.flox.cache_dir).await?;
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
        if self.debug || matches!(self.verbosity, Verbosity::Verbose(1..)) {
            let _ = temp_dir.into_path();
        }

        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.unwrap();
            // in case of SIG* the drop handler of temp_dir will not be called
            // if we are not in debugging mode, drop the tempdir manually
            if !self.debug || !matches!(self.verbosity, Verbosity::Verbose(1..)) {
                let _ = fs::remove_dir_all(&temp_dir_path);
            }
        });

        match self.command {
            Commands::Package { options, command } => {
                // Resolve stability from flag or config (which reads environment variables).
                // If the stability is set by a flag, modify STABILITY env variable to match
                // the set stability.
                // Flox invocations in a child process will inherit hence inherit the stability.

                // mutability, meh
                config.flox.stability = {
                    if let Some(ref stability) = options.stability {
                        env::set_var("FLOX_STABILITY", stability.to_string());
                        stability.clone()
                    } else {
                        config.flox.stability
                    }
                };

                let mut flox = flox;
                // more mutable state hurray :/
                flox.channels.register_channel(
                    "nixpkgs",
                    Channel::from_str(&format!("github:flox/nixpkgs/{}", config.flox.stability))?,
                );
                command.handle(config, flox).await?
            },
            Commands::Environment(ref environment) => environment.handle(flox).await?,
            Commands::Channel(ref channel) => channel.handle(flox).await?,
            Commands::General(ref general) => general.handle(config, flox).await?,
        }

        Ok(())
    }
}

/// Transparent separation of different categories of commands
#[derive(Bpaf, Clone)]
pub enum Commands {
    Package {
        #[bpaf(external(package::package_args), group_help("Development Options"))]
        options: package::PackageArgs,

        #[bpaf(external(package::interface::package_commands))]
        #[bpaf(group_help("Development Commands"))]
        command: interface::PackageCommands,
    },

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

/// Special command to check for the presence of the `--prefix` flag.
///
/// With `--prefix` the application will print the prefix of the program
/// and quit early.
#[derive(Bpaf, Default)]
pub struct Prefix {
    #[bpaf(long)]
    prefix: bool,
    #[bpaf(any, many)]
    _catchall: Vec<String>,
}

impl Prefix {
    /// Parses to [Self] and extract the `--prefix` flag
    pub fn check() -> bool {
        prefix().to_options().try_run().unwrap_or_default().prefix
    }
}
