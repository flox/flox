mod channel;
mod environment;
mod general;
mod package;

use std::{env, fs};

use anyhow::{Context, Result};
use bpaf::{Args, Bpaf, Parser};
use flox_rust_sdk::flox::{Flox, DEFAULT_OWNER, FLOX_VERSION};
use flox_rust_sdk::models::floxmeta::{Floxmeta, GetFloxmetaError};
use flox_rust_sdk::nix::command_line::NixCommandLine;
use flox_rust_sdk::providers::git::GitCommandProvider;
use indoc::{formatdoc, indoc};
use log::{debug, info};
use once_cell::sync::Lazy;
use tempfile::TempDir;
use toml_edit::Key;

use self::package::{Parseable, Run, WithPassthru};
use crate::config::{Config, FLOX_CONFIG_FILE};
use crate::utils::init::{
    init_access_tokens,
    init_channels,
    init_git_conf,
    init_telemetry,
    init_uuid,
    telemetry_opt_out_needs_migration,
};
use crate::utils::metrics::METRICS_UUID_FILE_NAME;

static FLOX_WELCOME_MESSAGE: Lazy<String> = Lazy::new(|| {
    formatdoc! {r#"
    flox version {FLOX_VERSION}

    Usage: flox OPTIONS (init|activate|search|install|...) [--help]

    Use "flox --help" for full list of commands and more information

    First time? Create an environment with "flox init"
"#}
});

static ADDITIONAL_COMMANDS: &str = indoc! {"
    build, upgrade, import, export, config, wipe-history, subscribe, unsubscribe,

    channels, history, print-dev-env, shell
"};

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
        #[bpaf(short('v'), long("verbose"), req_flag(()), many, map(vec_len))]
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
    #[bpaf(long, req_flag(()), many, map(vec_not_empty))]
    pub debug: bool,

    #[bpaf(external(commands), optional)]
    command: Option<Commands>,
}

impl FloxArgs {
    /// Initialize the command line by creating an initial FloxBuilder
    pub async fn handle(self, mut config: crate::config::Config) -> Result<()> {
        // Given no command, skip initialization and print welcome message
        if self.command.is_none() {
            println!("{}", &*FLOX_WELCOME_MESSAGE);
            return Ok(());
        }

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

        // migrate metrics denial
        // metrics could be turned off by writing an empty UUID file
        // this branch migrates empty files to a config value in the user's flox.toml
        // and deletes the now defunct empty file
        if telemetry_opt_out_needs_migration(&config.flox.data_dir, &config.flox.cache_dir).await? {
            info!("Migrating previous telemetry opt out to user config");
            // update current run time config
            config.flox.disable_metrics = true;

            // update persistent config file
            Config::write_to_in(
                config.flox.config_dir.join(FLOX_CONFIG_FILE),
                &temp_dir,
                &[Key::new("disable_metrics")],
                Some(true),
            )?;

            // remove marker uuid file
            tokio::fs::remove_file(&config.flox.data_dir.join(METRICS_UUID_FILE_NAME)).await?;
        }

        if !config.flox.disable_metrics {
            init_telemetry(&config.flox.data_dir, &config.flox.cache_dir).await?;
        } else {
            debug!("Metrics collection disabled");
            env::set_var("FLOX_DISABLE_METRICS", "true");
        }

        let access_tokens = init_access_tokens(&config.nix.access_tokens)?;

        let netrc_file = dirs::home_dir()
            .expect("User must have a home directory")
            .join(".netrc");

        let boostrap_flox = Flox {
            cache_dir: config.flox.cache_dir.clone(),
            data_dir: config.flox.data_dir.clone(),
            config_dir: config.flox.config_dir.clone(),
            channels: Default::default(),
            access_tokens,
            netrc_file,
            temp_dir: temp_dir_path.clone(),
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            uuid: init_uuid(&config.flox.data_dir).await?,
        };

        let floxmeta = match boostrap_flox
            .floxmeta::<GitCommandProvider>(DEFAULT_OWNER)
            .await
        {
            Ok(floxmeta) => floxmeta,
            Err(GetFloxmetaError::NotFound(_)) => {
                Floxmeta::create_floxmeta(&boostrap_flox, DEFAULT_OWNER)
                    .await
                    .context("Could not create 'floxmeta'")?
            },
            Err(e) => Err(e).context("Could not read 'floxmeta'")?,
        };

        //  Floxmeta::create_floxmeta creates an intial user_meta
        let user_meta = floxmeta
            .user_meta()
            .await
            .context("Could not get user metadata")?;

        let user_channels = user_meta.channels.unwrap_or_default();
        let channels = init_channels(user_channels)?;

        let flox = Flox {
            channels,
            ..boostrap_flox
        };

        // Set the global Nix config via the environment variables in flox.default_args so that
        // subprocesses called by `flox` (e.g. `parser-util`) can inherit them.
        flox.nix::<NixCommandLine>(vec![]).export_env_vars();

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

        // command handled above
        match self.command.unwrap() {
            Commands::Development(group) => group.handle(config, flox).await?,
            Commands::Sharing(group) => group.handle(config, flox).await?,
            Commands::Additional(group) => group.handle(config, flox).await?,
            Commands::Internal(group) => group.handle(config, flox).await?,
        }
        Ok(())
    }
}

#[allow(clippy::large_enum_variant)] // there's only a single instance of this enum
#[derive(Bpaf, Clone)]
enum Commands {
    Development(#[bpaf(external(local_development_commands))] LocalDevelopmentCommands),
    Sharing(#[bpaf(external(sharing_commands))] SharingCommands),
    Additional(#[bpaf(external(additional_commands))] AdditionalCommands),
    Internal(#[bpaf(external(internal_commands))] InternalCommands),
}

/// Local Development Commands
#[derive(Bpaf, Clone)]
enum LocalDevelopmentCommands {
    /// Create an environment in the current directory
    #[bpaf(command, short('i'), long("create"), short('c'))]
    Init(#[bpaf(external(environment::init))] environment::Init),
    /// Activate environment
    #[bpaf(command, short('a'), long("develop"))]
    Activate(#[bpaf(external(environment::activate))] environment::Activate),
    /// Search packages in subscribed channels
    #[bpaf(command)]
    Search(#[bpaf(external(channel::search))] channel::Search),
    /// Install a package into an environment
    #[bpaf(command, short('i'))]
    Install(#[bpaf(external(environment::install))] environment::Install),
    /// Uninstall installed packages from an environment
    #[bpaf(command, short('u'), long("remove"), long("rm"))]
    Uninstall(#[bpaf(external(environment::uninstall))] environment::Uninstall),
    /// Edit declarative environment configuration
    #[bpaf(command, short('e'))]
    Edit(#[bpaf(external(environment::edit))] environment::Edit),
    /// Run app from current project
    #[bpaf(command)]
    Run(#[bpaf(external(WithPassthru::parse))] WithPassthru<Run>),
    /// List (status?) packages installed in an environment
    #[bpaf(command)]
    List(#[bpaf(external(environment::list))] environment::List),
    /// Access to the nix CLI
    #[bpaf(command)]
    Nix(#[bpaf(external(general::parse_nix_passthru))] general::WrappedNix),
    /// Delete an environment
    #[bpaf(command)]
    Delete(#[bpaf(external(environment::delete))] environment::Delete),
}

impl LocalDevelopmentCommands {
    async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            LocalDevelopmentCommands::Init(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Activate(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Edit(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Install(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Uninstall(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::List(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Nix(args) => args.handle(config, flox).await?,
            LocalDevelopmentCommands::Search(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Run(args) => args.handle(config, flox).await?,
            LocalDevelopmentCommands::Delete(args) => args.handle(flox).await?,
        }
        Ok(())
    }
}

/// Sharing Commands
#[derive(Bpaf, Clone)]
enum SharingCommands {
    /// Send environment to flox hub
    #[bpaf(command)]
    Push(#[bpaf(external(environment::push))] environment::Push),
    #[bpaf(command)]
    /// Pull environment from flox hub
    Pull(#[bpaf(external(environment::pull))] environment::Pull),
    /// Containerize an environment
    #[bpaf(command)]
    Containerize(#[bpaf(external(WithPassthru::parse))] WithPassthru<package::Containerize>),
}
impl SharingCommands {
    async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            SharingCommands::Push(args) => args.handle(flox).await?,
            SharingCommands::Pull(args) => args.handle(flox).await?,
            SharingCommands::Containerize(args) => args.handle(config, flox).await?,
        }
        Ok(())
    }
}

/// Additional Commands. Use "flox COMMAND --help" for more info
#[derive(Bpaf, Clone)]
enum AdditionalCommands {
    Documentation(
        #[bpaf(external(AdditionalCommands::documentation))] AdditionalCommandsDocumentation,
    ),
    #[bpaf(command, hide)]
    Build(#[bpaf(external(WithPassthru::parse))] WithPassthru<package::Build>),
    #[bpaf(command, hide)]
    Upgrade(#[bpaf(external(environment::upgrade))] environment::Upgrade),
    #[bpaf(command, hide)]
    Import(#[bpaf(external(environment::import))] environment::Import),
    #[bpaf(command, hide)]
    Export(#[bpaf(external(environment::export))] environment::Export),
    #[bpaf(command, hide)]
    Config(#[bpaf(external(general::config_args))] general::ConfigArgs),
    #[bpaf(command("wipe-history"), hide)]
    WipeHistory(#[bpaf(external(environment::wipe_history))] environment::WipeHistory),
    #[bpaf(command, hide)]
    Subscribe(#[bpaf(external(channel::subscribe))] channel::Subscribe),
    #[bpaf(command, hide)]
    Unsubscribe(#[bpaf(external(channel::unsubscribe))] channel::Unsubscribe),
    #[bpaf(command, hide)]
    Channels(#[bpaf(external(channel::channels))] channel::Channels),
    #[bpaf(command, hide)]
    History(#[bpaf(external(environment::history))] environment::History),
    #[bpaf(command, hide)]
    PrintDevEnv(#[bpaf(external(WithPassthru::parse))] WithPassthru<package::PrintDevEnv>),
    #[bpaf(command, hide)]
    Shell(#[bpaf(external(WithPassthru::parse))] WithPassthru<package::Shell>),
}

impl AdditionalCommands {
    fn documentation() -> impl Parser<AdditionalCommandsDocumentation> {
        bpaf::literal(ADDITIONAL_COMMANDS)
            .hide_usage()
            .map(|_| AdditionalCommandsDocumentation)
    }

    async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            AdditionalCommands::Documentation(args) => args.handle(),
            AdditionalCommands::Build(args) => args.handle(config, flox).await?,
            AdditionalCommands::Upgrade(args) => args.handle(flox).await?,
            AdditionalCommands::Import(args) => args.handle(flox).await?,
            AdditionalCommands::Export(args) => args.handle(flox).await?,
            AdditionalCommands::Config(args) => args.handle(config, flox).await?,
            AdditionalCommands::WipeHistory(args) => args.handle(flox).await?,
            AdditionalCommands::Subscribe(args) => args.handle(flox).await?,
            AdditionalCommands::Unsubscribe(args) => args.handle(flox).await?,
            AdditionalCommands::Channels(args) => args.handle(flox)?,
            AdditionalCommands::History(args) => args.handle(flox).await?,
            AdditionalCommands::PrintDevEnv(args) => args.handle(config, flox).await?,
            AdditionalCommands::Shell(args) => args.handle(config, flox).await?,
        }
        Ok(())
    }
}

#[derive(Clone)]
struct AdditionalCommandsDocumentation;
impl AdditionalCommandsDocumentation {
    fn handle(self) {
        println!("🥚");
    }
}

/// Additional Commands. Use "flox COMMAND --help" for more info
#[derive(Bpaf, Clone)]
#[bpaf(hide)]
enum InternalCommands {
    #[bpaf(command("reset-metrics"))]
    ResetMetrics(#[bpaf(external(general::reset_metrics))] general::ResetMetrics),
    #[bpaf(command)]
    Generations(#[bpaf(external(environment::generations))] environment::Generations),
    #[bpaf(command("switch-generation"))]
    SwitchGeneration(
        #[bpaf(external(environment::switch_generation))] environment::SwitchGeneration,
    ),
    #[bpaf(command)]
    Rollback(#[bpaf(external(environment::rollback))] environment::Rollback),
    /// List all available environments
    ///
    /// Aliases:
    ///   environments, envs
    #[bpaf(command, long("environments"))]
    Envs(#[bpaf(external(environment::envs))] environment::Envs),
    #[bpaf(command)]
    Git(#[bpaf(external(environment::git))] environment::Git),
    #[bpaf(command("init-package"))]
    InitPackage(#[bpaf(external(WithPassthru::parse))] WithPassthru<package::InitPackage>),
    #[bpaf(command)]
    Publish(#[bpaf(external(package::publish))] package::Publish),
    #[bpaf(command)]
    Bundle(#[bpaf(external(WithPassthru::parse))] WithPassthru<package::Bundle>),
    #[bpaf(command)]
    Flake(#[bpaf(external(WithPassthru::parse))] WithPassthru<package::Flake>),
    #[bpaf(command)]
    Eval(#[bpaf(external(WithPassthru::parse))] WithPassthru<package::Eval>),
}

impl InternalCommands {
    async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            InternalCommands::ResetMetrics(args) => args.handle(config, flox).await?,
            InternalCommands::Generations(args) => args.handle(flox).await?,
            InternalCommands::SwitchGeneration(args) => args.handle(flox).await?,
            InternalCommands::Rollback(args) => args.handle(flox).await?,
            InternalCommands::Envs(args) => args.handle(flox).await?,
            InternalCommands::Git(args) => args.handle(flox).await?,
            InternalCommands::InitPackage(args) => args.handle(flox).await?,
            InternalCommands::Publish(args) => args.handle(config, flox).await?,
            InternalCommands::Bundle(args) => args.handle(config, flox).await?,
            InternalCommands::Flake(args) => args.handle(config, flox).await?,
            InternalCommands::Eval(args) => args.handle(config, flox).await?,
        }
        Ok(())
    }
}

/// Special command to check for the presence of the `--prefix` flag.
///
/// With `--prefix` the application will print the prefix of the program
/// and quit early.
#[derive(Bpaf, Default)]
pub struct Prefix {
    #[bpaf(long)]
    prefix: bool,
    #[bpaf(any("REST", Some), many)]
    _catchall: Vec<String>,
}

impl Prefix {
    /// Parses to [Self] and extract the `--prefix` flag
    pub fn check() -> bool {
        prefix()
            .to_options()
            .run_inner(Args::current_args())
            .unwrap_or_default()
            .prefix
    }
}

/// Special command to check for the presence of the `--bash-passthru`
///
/// With `--bash-passthru`,
/// all arguments to `flox` are passed to `flox-bash`
#[derive(Bpaf, Default, Debug)]
pub struct BashPassthru {
    #[bpaf(long("bash-passthru"))]
    do_passthru: bool,

    #[bpaf(any("REST", Some), many)]
    flox_args: Vec<String>,
}

impl BashPassthru {
    /// Parses to [Self] and extract the `--bash-passthru` flag
    /// returning a list of the remaining arguments if given.
    pub fn check() -> Option<Vec<String>> {
        let passtrhu = bash_passthru()
            .to_options()
            .run_inner(Args::current_args())
            .unwrap_or_default();

        if passtrhu.do_passthru {
            return Some(passtrhu.flox_args);
        }

        None
    }
}

pub fn not_help(s: String) -> Option<String> {
    if s == "--help" || s == "-h" {
        None
    } else {
        Some(s)
    }
}
