mod auth;
mod environment;
mod general;
mod search;

use std::{env, fs};

use anyhow::{Context, Result};
use bpaf::{Args, Bpaf, Parser};
use flox_rust_sdk::flox::{Flox, DEFAULT_OWNER, FLOX_VERSION};
use flox_rust_sdk::models::floxmeta::{Floxmeta, GetFloxmetaError};
use flox_rust_sdk::nix::command_line::NixCommandLine;
use indoc::{formatdoc, indoc};
use log::{debug, info};
use once_cell::sync::Lazy;
use tempfile::TempDir;
use toml_edit::Key;

use crate::config::{Config, FLOX_CONFIG_FILE};
use crate::utils::init::{
    init_access_tokens,
    init_channels,
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
    upgrade, config, wipe-history, history
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
pub struct FloxArgs {
    /// Verbose mode.
    ///
    /// Invoke multiple times for increasing detail.
    #[bpaf(external, fallback(Default::default()))]
    pub verbosity: Verbosity,

    /// Debug mode.
    #[bpaf(long, req_flag(()), many, map(vec_not_empty))]
    pub debug: bool,

    /// Print the version of the program
    #[allow(dead_code)] // fake arg, `--version` is checked for separately (see [Version])
    #[bpaf(long)]
    version: bool,

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
            floxhub_token: config.flox.floxhub_token.clone(),
        };

        // TODO: revisit this when we discussed floxmeta's role to contribute to config/channels
        // region: revisit reg. channels
        let floxmeta = match boostrap_flox.floxmeta(DEFAULT_OWNER) {
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
            .context("Could not get user metadata")?;

        let user_channels = user_meta.channels.unwrap_or_default();
        let channels = init_channels(user_channels)?;

        let flox = Flox {
            channels,
            ..boostrap_flox
        };

        // endregion: revisit reg. channels

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
    #[bpaf(command, long("create"))]
    Init(#[bpaf(external(environment::init))] environment::Init),
    /// Activate environment
    #[bpaf(command, long("develop"))]
    Activate(#[bpaf(external(environment::activate))] environment::Activate),
    /// Search for packages to install
    #[bpaf(command)]
    Search(#[bpaf(external(search::search))] search::Search),
    /// Show detailed information about a single package
    #[bpaf(command, long("show"))]
    Show(#[bpaf(external(search::show))] search::Show),
    /// Install a package into an environment
    #[bpaf(command)]
    Install(#[bpaf(external(environment::install))] environment::Install),
    /// Uninstall installed packages from an environment
    #[bpaf(command, long("remove"), long("rm"))]
    Uninstall(#[bpaf(external(environment::uninstall))] environment::Uninstall),
    /// Edit declarative environment configuration
    #[bpaf(command)]
    Edit(#[bpaf(external(environment::edit))] environment::Edit),
    /// List packages installed in an environment
    #[bpaf(command)]
    List(#[bpaf(external(environment::list))] environment::List),
    /// Delete an environment
    #[bpaf(command, long("destroy"))]
    Delete(#[bpaf(external(environment::delete))] environment::Delete),
}

impl LocalDevelopmentCommands {
    async fn handle(self, _config: Config, flox: Flox) -> Result<()> {
        match self {
            LocalDevelopmentCommands::Init(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Activate(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Edit(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Install(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Uninstall(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::List(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Search(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Show(args) => args.handle(flox).await?,
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
    Containerize(#[bpaf(external(environment::containerize))] environment::Containerize),
}
impl SharingCommands {
    async fn handle(self, _config: Config, flox: Flox) -> Result<()> {
        match self {
            SharingCommands::Push(args) => args.handle(flox).await?,
            SharingCommands::Pull(args) => args.handle(flox).await?,
            SharingCommands::Containerize(args) => args.handle(flox).await?,
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
    Upgrade(#[bpaf(external(environment::upgrade))] environment::Upgrade),
    #[bpaf(command, hide)]
    Config(#[bpaf(external(general::config_args))] general::ConfigArgs),
    #[bpaf(command("wipe-history"), hide)]
    WipeHistory(#[bpaf(external(environment::wipe_history))] environment::WipeHistory),
    #[bpaf(command, hide)]
    History(#[bpaf(external(environment::history))] environment::History),
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
            AdditionalCommands::Upgrade(args) => args.handle(flox).await?,
            AdditionalCommands::Config(args) => args.handle(config, flox).await?,
            AdditionalCommands::WipeHistory(args) => args.handle(flox).await?,
            AdditionalCommands::History(args) => args.handle(flox).await?,
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
    #[bpaf(command)]
    Auth(#[bpaf(external(general::auth))] general::Auth),
    ///Auth2
    #[bpaf(command)]
    Auth2(#[bpaf(external(auth::auth2))] auth::Auth2),
}

impl InternalCommands {
    async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            InternalCommands::ResetMetrics(args) => args.handle(config, flox).await?,
            InternalCommands::Generations(args) => args.handle(flox).await?,
            InternalCommands::SwitchGeneration(args) => args.handle(flox).await?,
            InternalCommands::Rollback(args) => args.handle(flox).await?,
            InternalCommands::Auth(args) => args.handle(config, flox).await?,
            InternalCommands::Auth2(args) => args.handle(config, flox).await?,
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

/// Fake argument used to parse `--version` separately
///
/// bpaf allows `flox --invalid option --version`
/// (https://github.com/pacak/bpaf/issues/288) but common utilities,
/// such as git always require correct arguments even in the presence of
/// short circuiting flags such as `--version`
#[derive(Bpaf, Default)]
pub struct Version(#[bpaf(long("version"))] bool);

impl Version {
    /// Parses to [Self] and extract the `--version` flag
    pub fn check() -> bool {
        bpaf::construct!(version(), flox_args())
            .to_options()
            .run_inner(bpaf::Args::current_args())
            .map(|(v, _)| v)
            .unwrap_or_default()
            .0
    }
}

pub fn not_help(s: String) -> Option<String> {
    if s == "--help" || s == "-h" {
        None
    } else {
        Some(s)
    }
}
