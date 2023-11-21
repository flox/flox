mod auth;
mod environment;
mod general;
mod search;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{anyhow, Context, Result};
use bpaf::{Args, Bpaf, Parser};
use flox_rust_sdk::flox::{Flox, FLOX_VERSION};
use flox_rust_sdk::models::environment::managed_environment::ManagedEnvironment;
use flox_rust_sdk::models::environment::path_environment::{Original, PathEnvironment};
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
use flox_rust_sdk::models::environment::{
    Environment,
    EnvironmentPointer,
    DOT_FLOX,
    FLOX_ACTIVE_ENVIRONMENTS_VAR,
};
use flox_rust_sdk::models::environment_ref;
use flox_rust_sdk::nix::command_line::NixCommandLine;
use indoc::{formatdoc, indoc};
use log::{debug, info, warn};
use once_cell::sync::Lazy;
use tempfile::TempDir;
use toml_edit::Key;

use self::environment::hacky_environment_description;
use crate::config::{Config, FLOX_CONFIG_FILE};
use crate::utils::dialog::{Dialog, Select};
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

        let floxhub_host = std::env::var("__FLOX_FLOXHUB_URL")
            .map(|env_set_host|{
                warn!("Using {env_set_host} as floxhub host");
                warn!("`$__FLOX_FLOXHUB_URL` is used for testing purposes only, alternative floxhub hosts are not yet supported!");
                env_set_host
            })
            .unwrap_or_else(|_| "https://git.hub.flox.dev".to_string());

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
            floxhub_host,
        };

        let channels = init_channels(BTreeMap::new())?;

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

            std::process::exit(130);
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
///flox is a virtual environment and package manager all in one. With flox you create development environments that layer and replace dependencies just where it matters, making them portable across the full software lifecycle.

/// Local Development Commands
#[derive(Bpaf, Clone)]
enum LocalDevelopmentCommands {
    /// Create an environment in the current directory
    #[bpaf(command, long("create"))]
    Init(#[bpaf(external(environment::init))] environment::Init),
    /// Enter the environment
    #[bpaf(command, long("develop"))]
    Activate(#[bpaf(external(environment::activate))] environment::Activate),
    /// Search for system or library packages to install
    #[bpaf(command)]
    Search(#[bpaf(external(search::search))] search::Search),
    /// Show details about a single package
    #[bpaf(command, long("show"))]
    Show(#[bpaf(external(search::show))] search::Show),
    /// Install a package into an environment
    #[bpaf(command)]
    Install(#[bpaf(external(environment::install))] environment::Install),
    /// Uninstall installed packages from an environment
    #[bpaf(command, long("remove"), long("rm"))]
    Uninstall(#[bpaf(external(environment::uninstall))] environment::Uninstall),
    /// Edit declarative environment configuration file
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

#[derive(Debug, Default, Bpaf, Clone)]
pub enum EnvironmentSelect {
    Dir(
        /// Path containing a .flox/ directory
        #[bpaf(long("dir"), short('d'), argument("path"))]
        PathBuf,
    ),
    Remote(
        /// A remote environment on floxhub
        #[bpaf(long("remote"), short('r'), argument("owner/name"))]
        environment_ref::EnvironmentRef,
    ),
    #[default]
    Unspecified,
}

impl EnvironmentSelect {
    /// Open a concrete environment, not detecting the currently active
    /// environment.
    /// 
    /// Use this method for commands like `activate` that shouldn't change
    /// behavior based on whether an environment is already active. For example,
    /// `flox activate` should never re-activate the last activated environment;
    /// it should default to an environment in the current directory.
    pub fn to_concrete_environment(
        &self,
        flox: &Flox,
    ) -> Result<ConcreteEnvironment> {
        match self {
            EnvironmentSelect::Dir(path) => open_path(flox, path),
            // TODO: needs design - do we want to search up?
            EnvironmentSelect::Unspecified => {
                let current_dir = env::current_dir().context("could not get current directory")?;
                let maybe_current_pointer = EnvironmentPointer::open(&current_dir);
                maybe_current_pointer
                    .map(|current_dir_pointer| {
                        open_env_pointer(flox, &current_dir, current_dir_pointer)
                    })
                    .context(format!("No environment found in {current_dir:?}"))?
            },
            EnvironmentSelect::Remote(_) => todo!(),
        }
    }

    /// Open a concrete environment, detecting the currently active environment.
    /// 
    /// Use this method for commands like `install` that should use the
    /// currently activated environment. For example, `flox install` should
    /// install to the last activated environment if there isn't an environment
    /// in the current directory.
    pub fn detect_concrete_environment(
        &self,
        flox: &Flox,
        message: &str,
    ) -> Result<ConcreteEnvironment> {
        match self {
            EnvironmentSelect::Dir(path) => open_path(flox, path),
            // If the user doesn't specify an environment, check if there's an
            // already activated environment or an environment in the current
            // directory.
            // TODO: needs design - do we want to search up?
            EnvironmentSelect::Unspecified => detect_environment(flox, message),
            EnvironmentSelect::Remote(_) => todo!(),
        }
    }
}

pub fn detect_environment(flox: &Flox, message: &str) -> Result<ConcreteEnvironment> {
    let current_dir = env::current_dir().context("could not get current directory")?;
    let maybe_current_pointer = EnvironmentPointer::open(&current_dir);
    let maybe_activated = last_activated_environment();
    match (maybe_activated, maybe_current_pointer) {
        (Some(activated), Ok(current_dir_pointer)) => {
            if activated == current_dir {
                open_env_pointer(flox, &current_dir, current_dir_pointer)
            } else {
                let activated_pointer = EnvironmentPointer::open(&activated)?;
                let message = format!("Do you want to {message} the current directory's flox environment or the current active flox environment?");
                let current_description =
                    hacky_environment_description(&current_dir, &current_dir_pointer)?;
                let activated_description =
                    hacky_environment_description(&activated, &activated_pointer)?;
                if Dialog::can_prompt() {
                    let dialog = Dialog {
                        message: &message,
                        help_message: None,
                        typed: Select {
                            options: vec![
                                format!(
                                    "current directory's flox environment [{current_description}]",
                                ),
                                format!(
                                    "current active flox environment [{activated_description}]",
                                ),
                            ],
                        },
                    };
                    let (index, _) = dialog.raw_prompt()?;
                    match index {
                        0 => open_env_pointer(flox, &current_dir, current_dir_pointer),
                        1 => open_env_pointer(flox, &activated, activated_pointer),
                        _ => unreachable!(),
                    }
                } else {
                    Err(anyhow!("can't determine whether to use {current_description} or {activated_description}; specify an environment using --dir or --remote"))?
                }
            }
        },
        (Some(activated), Err(_)) => open_path(flox, &activated),
        (None, Ok(current_dir_pointer)) => {
            open_env_pointer(flox, &current_dir, current_dir_pointer)
        },
        (None, Err(e)) => Err(e).context(format!("No environment found in {current_dir:?}"))?,
    }
}

/// Open an environment defined in `{path}/.flox`
fn open_path(flox: &Flox, path: &PathBuf) -> Result<ConcreteEnvironment> {
    let pointer = EnvironmentPointer::open(path)
        .with_context(|| format!("No environment found in {path:?}"))?;

    open_env_pointer(flox, path, pointer)
}

/// Open an environment defined in `{path}/.flox` with an already parsed pointer
///
/// This is used directly when the env pointer was read previously during detection.
fn open_env_pointer(
    flox: &Flox,
    path: &Path,
    pointer: EnvironmentPointer,
) -> Result<ConcreteEnvironment> {
    let dot_flox_path = path.join(DOT_FLOX);
    let env = match pointer {
        EnvironmentPointer::Path(path_pointer) => {
            debug!("detected concrete environment type: path");
            ConcreteEnvironment::Path(PathEnvironment::open(
                path_pointer,
                dot_flox_path,
                &flox.temp_dir,
            )?)
        },
        EnvironmentPointer::Managed(managed_pointer) => {
            debug!("detected concrete environment type: managed");
            let env = ManagedEnvironment::open(flox, managed_pointer, dot_flox_path)?;
            ConcreteEnvironment::Managed(env)
        },
    };
    Ok(env)
}

/// The various ways in which an environment can be referred to
pub enum ConcreteEnvironment {
    /// Container for [PathEnvironment]
    Path(PathEnvironment<Original>),
    /// Container for [ManagedEnvironment]
    #[allow(unused)] // pending implementation of ManagedEnvironment
    Managed(ManagedEnvironment),
    /// Container for [RemoteEnvironment]
    #[allow(unused)] // pending implementation of RemoteEnvironment
    Remote(RemoteEnvironment),
}

impl ConcreteEnvironment {
    fn into_dyn_environment(self) -> Box<dyn Environment> {
        match self {
            ConcreteEnvironment::Path(path_env) => Box::new(path_env),
            ConcreteEnvironment::Managed(managed_env) => Box::new(managed_env),
            ConcreteEnvironment::Remote(remote_env) => Box::new(remote_env),
        }
    }
}

/// Determine the path to most recently activated environment.
///
/// When inside a `flox activate` shell, flox stores the path to the
/// activated environment in `$FLOX_ACTIVE_ENVIRONMENTS`. Environments which
/// are activated while in a `flox activate` shell, are prepended - the most
/// recently activated environment is the _first in the list of environments.
/// TODO: how will we handle remote environments?
fn last_activated_environment() -> Option<PathBuf> {
    activated_environments().into_iter().next()
}

/// Return paths to all activated environments.
fn activated_environments() -> Vec<PathBuf> {
    env::var(FLOX_ACTIVE_ENVIRONMENTS_VAR)
        .map(|active| env::split_paths(&active).collect())
        .unwrap_or(vec![])
}
