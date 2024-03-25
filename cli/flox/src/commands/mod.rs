mod auth;
mod environment;
mod general;
mod init;
mod search;

use std::collections::VecDeque;
use std::fmt::Display;
use std::path::PathBuf;
use std::str::FromStr;
use std::{env, fmt, fs, mem};

use anyhow::{bail, Context, Result};
use bpaf::{Args, Bpaf, ParseFailure, Parser};
use flox_rust_sdk::flox::{
    EnvironmentRef,
    Flox,
    Floxhub,
    FloxhubToken,
    FloxhubTokenError,
    DEFAULT_FLOXHUB_URL,
    DEFAULT_NAME,
    FLOX_VERSION,
};
use flox_rust_sdk::models::environment::managed_environment::ManagedEnvironment;
use flox_rust_sdk::models::environment::path_environment::PathEnvironment;
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
use flox_rust_sdk::models::environment::{
    find_dot_flox,
    DotFlox,
    Environment,
    EnvironmentError2,
    EnvironmentPointer,
    ManagedPointer,
    DOT_FLOX,
    FLOX_ACTIVE_ENVIRONMENTS_VAR,
};
use flox_rust_sdk::models::environment_ref;
use indoc::{formatdoc, indoc};
use log::{debug, info};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;
use thiserror::Error;
use toml_edit::Key;
use url::Url;

use crate::commands::general::update_config;
use crate::config::{Config, EnvironmentTrust, FLOX_CONFIG_FILE};
use crate::utils::dialog::{Dialog, Select};
use crate::utils::init::{
    init_access_tokens,
    init_telemetry,
    init_uuid,
    telemetry_opt_out_needs_migration,
};
use crate::utils::message;
use crate::utils::metrics::METRICS_UUID_FILE_NAME;

static FLOX_DESCRIPTION: &'_ str = indoc! {"
    flox is a virtual environment and package manager all in one.\n\n

    With flox you create environments that layer and replace dependencies just where it matters,
    making them portable across the full software lifecycle."
};

static FLOX_WELCOME_MESSAGE: Lazy<String> = Lazy::new(|| {
    let version = FLOX_VERSION.to_string();
    formatdoc! {r#"
    flox version {version}

    Usage: flox OPTIONS (init|activate|search|install|...) [--help]

    Use `flox --help` for full list of commands and more information

    First time? Create an environment with `flox init`
"#}
});

const ADDITIONAL_COMMANDS: &str = indoc! {"
    update, upgrade, config, auth
"};

fn vec_len<T>(x: Vec<T>) -> usize {
    Vec::len(&x)
}

fn vec_not_empty<T>(x: Vec<T>) -> bool {
    !x.is_empty()
}

#[derive(Bpaf, Clone, Copy, Debug)]
pub enum Verbosity {
    Verbose(
        /// Increase logging verbosity
        ///
        /// Invoke multiple times for increasing detail.
        #[bpaf(short('v'), long("verbose"), req_flag(()), many, map(vec_len))]
        usize,
    ),

    /// Silence logs except for errors
    #[bpaf(short, long)]
    Quiet,
}

impl Verbosity {
    pub fn to_pkgdb_verbosity_level(self) -> usize {
        match self {
            Verbosity::Quiet => 0,
            Verbosity::Verbose(n) => n,
        }
    }
}

impl Default for Verbosity {
    fn default() -> Self {
        Verbosity::Verbose(0)
    }
}

#[derive(Bpaf)]
#[bpaf(
    options,
    descr(FLOX_DESCRIPTION),
    footer("Run 'man flox' for more details.")
)]
pub struct FloxCli(#[bpaf(external(flox_args))] pub FloxArgs);

/// Main flox args parser
///
/// This struct is used to parse the command line arguments
/// and allows to be composed with other parsers.
///
/// To parse the flox CLI, use [`FloxCli`] instead using [`flox_cli()`].
#[derive(Debug, Bpaf)]
#[bpaf(ignore_rustdoc)] // we don't want this struct to be interpreted as a group
pub struct FloxArgs {
    /// Verbose mode
    ///
    /// Invoke multiple times for increasing detail.
    #[bpaf(external, fallback(Default::default()))]
    pub verbosity: Verbosity,

    /// Debug mode
    #[bpaf(long, req_flag(()), many, map(vec_not_empty), hide)]
    pub debug: bool,

    /// Print the version of the program
    #[allow(dead_code)] // fake arg, `--version` is checked for separately (see [Version])
    #[bpaf(long, short('V'))]
    version: bool,

    #[bpaf(external(commands), optional)]
    command: Option<Commands>,
}

impl fmt::Debug for Commands {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Command")
    }
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

        let access_tokens = init_access_tokens(
            config
                .nix
                .as_ref()
                .map(|nix_config| &nix_config.access_tokens),
        )?;

        let netrc_file = dirs::home_dir()
            .expect("User must have a home directory")
            .join(".netrc");

        let git_url_override = {
            if let Ok(env_set_host) = std::env::var("_FLOX_FLOXHUB_GIT_URL") {
                message::warning(formatdoc! {"
                    Using {env_set_host} as FloxHub host
                    '$_FLOX_FLOXHUB_GIT_URL' is used for testing purposes only,
                    alternative FloxHub hosts are not yet supported!
                "});
                Some(Url::parse(&env_set_host)?)
            } else {
                None
            }
        };

        let floxhub = Floxhub::new(
            config
                .flox
                .floxhub_url
                .clone()
                .unwrap_or_else(|| DEFAULT_FLOXHUB_URL.clone()),
            git_url_override,
        )?;

        let floxhub_token = config
            .flox
            .floxhub_token
            .as_deref()
            .map(FloxhubToken::from_str)
            .transpose();

        let floxhub_token = match floxhub_token {
            Err(FloxhubTokenError::Expired) => {
                message::warning("Your FloxHub token has expired. You may need to log in again.");
                if let Err(e) = update_config(
                    &config.flox.config_dir,
                    &temp_dir_path,
                    "floxhub_token",
                    None::<String>,
                ) {
                    log::debug!("Could not remove token from user config: {e}");
                }
                None
            },
            Err(FloxhubTokenError::InvalidToken(token_error)) => {
                message::error(formatdoc! {"
                    Your FloxHub token is invalid: {token_error}
                    You may need to log in again.
                "});
                if let Err(e) = update_config(
                    &config.flox.config_dir,
                    &temp_dir_path,
                    "floxhub_token",
                    None::<String>,
                ) {
                    log::debug!("Could not remove token from user config: {e}");
                }
                None
            },
            Ok(token) => token,
        };

        let flox = Flox {
            cache_dir: config.flox.cache_dir.clone(),
            data_dir: config.flox.data_dir.clone(),
            config_dir: config.flox.config_dir.clone(),
            access_tokens,
            netrc_file,
            temp_dir: temp_dir_path.clone(),
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            uuid: init_uuid(&config.flox.data_dir).await?,
            floxhub_token,
            floxhub,
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

            std::process::exit(130);
        });

        // command handled above
        match self.command.unwrap() {
            Commands::Help(group) => group.handle(),
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
    /// Prints help information
    #[bpaf(command, hide)]
    Help(#[bpaf(external(help))] Help),
    Development(#[bpaf(external(local_development_commands))] LocalDevelopmentCommands),
    Sharing(#[bpaf(external(sharing_commands))] SharingCommands),
    Additional(#[bpaf(external(additional_commands))] AdditionalCommands),
    Internal(#[bpaf(external(internal_commands))] InternalCommands),
}

#[derive(Debug, Bpaf, Clone)]
struct Help {
    /// Command to show help for
    #[bpaf(positional("cmd"))]
    cmd: Option<String>,
}
impl Help {
    fn handle(self) {
        let mut args = Vec::from_iter(self.cmd.as_deref());
        args.push("--help");

        // todo: just `run()` this -- we might not need the expl;icit error handling anymore
        match flox_cli().run_inner(&*args) {
            Ok(_) => unreachable!(),
            Err(ParseFailure::Completion(comp)) => print!("{comp:80}"),
            Err(ParseFailure::Stdout(doc, _)) => message::plain(format!("{doc:80}")),
            Err(ParseFailure::Stderr(err)) => message::error(err),
        }
    }
}

/// Local Development Commands
#[derive(Bpaf, Clone)]
enum LocalDevelopmentCommands {
    /// Create an environment in the current directory
    #[bpaf(
        command,
        long("create"),
        footer("Run 'man flox-init' for more details.")
    )]
    Init(#[bpaf(external(init::init))] init::Init),
    /// Enter the environment, type 'exit' to leave
    #[bpaf(
        command,
        long("develop"),
        footer("Run 'man flox-activate' for more details.")
    )]
    Activate(#[bpaf(external(environment::activate))] environment::Activate),
    /// Search for system or library packages to install
    #[bpaf(command, footer("Run 'man flox-search' for more details."))]
    Search(#[bpaf(external(search::search))] search::Search),
    /// Show details about a single package
    #[bpaf(command, long("show"), footer("Run 'man flox-show' for more details."))]
    Show(#[bpaf(external(search::show))] search::Show),
    /// Install packages into an environment
    #[bpaf(
        command,
        short('i'),
        footer("Run 'man flox-install' for more details.")
    )]
    Install(#[bpaf(external(environment::install))] environment::Install),
    /// Uninstall installed packages from an environment
    #[bpaf(
        command,
        long("remove"),
        long("rm"),
        footer("Run 'man flox-uninstall' for more details.")
    )]
    Uninstall(#[bpaf(external(environment::uninstall))] environment::Uninstall),
    /// Edit declarative environment configuration file
    #[bpaf(command, footer("Run 'man flox-edit' for more details."))]
    Edit(#[bpaf(external(environment::edit))] environment::Edit),
    /// List packages installed in an environment
    #[bpaf(command, footer("Run 'man flox-list' for more details."))]
    List(#[bpaf(external(environment::list))] environment::List),
    /// Delete an environment
    #[bpaf(
        command,
        long("destroy"),
        footer("Run 'man flox-delete' for more details.")
    )]
    Delete(#[bpaf(external(environment::delete))] environment::Delete),
}

impl LocalDevelopmentCommands {
    async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            LocalDevelopmentCommands::Init(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Activate(args) => args.handle(config, flox).await?,
            LocalDevelopmentCommands::Edit(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Install(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Uninstall(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::List(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Search(args) => args.handle(config, flox).await?,
            LocalDevelopmentCommands::Show(args) => args.handle(flox).await?,
            LocalDevelopmentCommands::Delete(args) => args.handle(flox).await?,
        }
        Ok(())
    }
}

/// Sharing Commands
#[derive(Bpaf, Clone)]
enum SharingCommands {
    /// Send an environment to FloxHub
    #[bpaf(command, footer("Run 'man flox-push' for more details."))]
    Push(#[bpaf(external(environment::push))] environment::Push),
    /// Pull an environment from FloxHub
    #[bpaf(command, footer("Run 'man flox-pull' for more details."))]
    Pull(#[bpaf(external(environment::pull))] environment::Pull),
    /// Containerize an environment
    #[bpaf(
        command,
        hide,
        footer("Run 'man flox-containerize' for more details."),
        header("This command is experimental and its behaviour is subject to change")
    )]
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
    /// Update environment's base catalog or the global base catalog
    #[bpaf(command, hide, footer("Run 'man flox-update' for more details."))]
    Update(#[bpaf(external(environment::update))] environment::Update),
    /// Upgrade packages in an environment
    #[bpaf(command, hide, footer("Run 'man flox-upgrade' for more details."), header(indoc! {"
        When no arguments are specified, all packages in the environment are upgraded.\n\n

        Packages to upgrade can be specified by either group name, or, if a package is
        not in a group with any other packages, it may be specified by ID. If the
        specified argument is both a group name and a package ID, the group is
        upgraded.\n\n

        Packages without a specified group in the manifest are placed in a group
        named 'toplevel'.
        The packages in that group can be upgraded without updating any other
        groups by passing 'toplevel' as the group name.
    "}))]
    Upgrade(#[bpaf(external(environment::upgrade))] environment::Upgrade),
    /// View and set configuration options
    #[bpaf(command, hide, footer("Run 'man flox-config' for more details."))]
    Config(#[bpaf(external(general::config_args))] general::ConfigArgs),
    /// Delete builds of non-current versions of an environment
    #[bpaf(command("wipe-history"), hide)]
    WipeHistory(#[bpaf(external(environment::wipe_history))] environment::WipeHistory),
    /// Show all versions of an environment
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
            AdditionalCommands::Update(args) => args.handle(flox).await?,
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
    /// Reset the metrics queue (if any), reset metrics ID, and re-prompt for consent
    #[bpaf(command("reset-metrics"))]
    ResetMetrics(#[bpaf(external(general::reset_metrics))] general::ResetMetrics),
    /// List environment generations with contents
    #[bpaf(command)]
    Generations(#[bpaf(external(environment::generations))] environment::Generations),
    /// Switch to a specific generation of an environment
    #[bpaf(command("switch-generation"))]
    SwitchGeneration(
        #[bpaf(external(environment::switch_generation))] environment::SwitchGeneration,
    ),
    /// Rollback to the previous generation of an environment
    #[bpaf(command)]
    Rollback(#[bpaf(external(environment::rollback))] environment::Rollback),
    /// FloxHub authentication commands
    #[bpaf(command, footer("Run 'man flox-auth' for more details."))]
    Auth(#[bpaf(external(auth::auth))] auth::Auth),
}

impl InternalCommands {
    async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            InternalCommands::ResetMetrics(args) => args.handle(config, flox).await?,
            InternalCommands::Generations(args) => args.handle(flox).await?,
            InternalCommands::SwitchGeneration(args) => args.handle(flox).await?,
            InternalCommands::Rollback(args) => args.handle(flox).await?,
            InternalCommands::Auth(args) => args.handle(config, flox).await?,
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
pub struct Version(#[bpaf(short('V'), long("version"))] bool);

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

#[derive(Debug, Default, Bpaf, Clone)]
pub enum EnvironmentSelect {
    Dir(
        /// Path containing a .flox/ directory
        #[bpaf(long("dir"), short('d'), argument("path"))]
        PathBuf,
    ),
    Remote(
        /// A remote environment on FloxHub
        #[bpaf(long("remote"), short('r'), argument("owner>/<name"))]
        environment_ref::EnvironmentRef,
    ),
    #[default]
    #[bpaf(hide)]
    Unspecified,
}

#[derive(Debug, Error)]
pub enum EnvironmentSelectError {
    #[error(transparent)]
    Environment(#[from] EnvironmentError2),
    #[error("Did not find an environment in the current directory.")]
    EnvNotFoundInCurrentDirectory,
    #[error(transparent)]
    Anyhow(#[from] anyhow::Error),
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
    ) -> Result<ConcreteEnvironment, EnvironmentSelectError> {
        match self {
            EnvironmentSelect::Dir(path) => Ok(open_path(flox, path)?),
            EnvironmentSelect::Unspecified => {
                let current_dir = env::current_dir().context("could not get current directory")?;
                let maybe_found_environment = find_dot_flox(&current_dir)?;
                match maybe_found_environment {
                    Some(found) => {
                        Ok(UninitializedEnvironment::DotFlox(found)
                            .into_concrete_environment(flox)?)
                    },
                    None => Err(EnvironmentSelectError::EnvNotFoundInCurrentDirectory)?,
                }
            },
            EnvironmentSelect::Remote(env_ref) => {
                let pointer = ManagedPointer::new(
                    env_ref.owner().clone(),
                    env_ref.name().clone(),
                    &flox.floxhub,
                );

                let env = RemoteEnvironment::new(flox, pointer).map_err(anyhow::Error::new)?;
                Ok(ConcreteEnvironment::Remote(env))
            },
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
    ) -> Result<ConcreteEnvironment, EnvironmentSelectError> {
        match self {
            EnvironmentSelect::Dir(path) => Ok(open_path(flox, path)?),
            // If the user doesn't specify an environment, check if there's an
            // already activated environment or an environment in the current
            // directory.
            EnvironmentSelect::Unspecified => match detect_environment(message)? {
                Some(env) => Ok(env.into_concrete_environment(flox)?),
                None => Err(EnvironmentSelectError::EnvNotFoundInCurrentDirectory)?,
            },
            EnvironmentSelect::Remote(env_ref) => {
                let pointer = ManagedPointer::new(
                    env_ref.owner().clone(),
                    env_ref.name().clone(),
                    &flox.floxhub,
                );

                let env = RemoteEnvironment::new(flox, pointer).map_err(anyhow::Error::new)?;
                Ok(ConcreteEnvironment::Remote(env))
            },
        }
    }
}

/// Determine what environment a flox command should use.
///
/// - Look in current directory and search upwards from the current directory if
///   inside a git repo.
/// - Check if there's an already activated environment.
/// - Prompt if both are true.
pub fn detect_environment(
    message: &str,
) -> Result<Option<UninitializedEnvironment>, EnvironmentSelectError> {
    let current_dir = env::current_dir().context("could not get current directory")?;
    let maybe_activated = last_activated_environment();
    let maybe_found_environment = find_dot_flox(&current_dir)?;

    let found = match (maybe_activated, maybe_found_environment) {
        (
            Some(ref activated @ UninitializedEnvironment::DotFlox(DotFlox { ref path, .. })),
            Some(found),
        ) if path == &found.path => Some(activated.clone()),

        // If both a 'default' environment is activated and an environment is
        // found in the current directory or git repo, prefer the detected one.
        (Some(activated), Some(detected))
            if activated.pointer().name().as_ref() == DEFAULT_NAME =>
        {
            Some(UninitializedEnvironment::DotFlox(detected))
        },

        // If there's both an activated environment and an environment in the
        // current directory or git repo, prompt for which to use.
        (Some(activated_env), Some(found)) => {
            let type_of_directory = if found.path == current_dir {
                "current directory's flox environment"
            } else {
                "flox environment detected in git repo"
            };
            let message = format!("Do you want to {message} the {type_of_directory} or the current active flox environment?");
            let found = UninitializedEnvironment::DotFlox(found);

            if !Dialog::can_prompt() {
                debug!("No TTY detected, using the environment {found:?} found in the current directory or an ancestor directory");
                return Ok(Some(found));
            }

            let dialog = Dialog {
                message: &message,
                help_message: None,
                typed: Select {
                    options: vec![
                        format!("{type_of_directory} [{found}]",),
                        format!("current active flox environment [{activated_env}]",),
                    ],
                },
            };
            let (index, _) = dialog.raw_prompt().map_err(anyhow::Error::new)?;
            match index {
                0 => Some(found),
                1 => Some(activated_env),
                _ => unreachable!(),
            }
        },
        (Some(activated_env), None) => Some(activated_env),
        (None, Some(found)) => Some(UninitializedEnvironment::DotFlox(found)),
        (None, None) => None,
    };
    Ok(found)
}

/// Open an environment defined in `{path}/.flox`
fn open_path(flox: &Flox, path: &PathBuf) -> Result<ConcreteEnvironment, EnvironmentError2> {
    DotFlox::open(path)
        .map(UninitializedEnvironment::DotFlox)?
        .into_concrete_environment(flox)
}

/// The various ways in which an environment can be referred to
pub enum ConcreteEnvironment {
    /// Container for [PathEnvironment]
    Path(PathEnvironment),
    /// Container for [ManagedEnvironment]
    #[allow(unused)] // pending implementation of ManagedEnvironment
    Managed(ManagedEnvironment),
    /// Container for [RemoteEnvironment]
    #[allow(unused)] // pending implementation of RemoteEnvironment
    Remote(RemoteEnvironment),
}

impl ConcreteEnvironment {
    pub fn into_dyn_environment(self) -> Box<dyn Environment> {
        match self {
            ConcreteEnvironment::Path(path_env) => Box::new(path_env),
            ConcreteEnvironment::Managed(managed_env) => Box::new(managed_env),
            ConcreteEnvironment::Remote(remote_env) => Box::new(remote_env),
        }
    }

    pub fn dyn_environment_ref_mut(&mut self) -> &mut dyn Environment {
        match self {
            ConcreteEnvironment::Path(path_env) => path_env,
            ConcreteEnvironment::Managed(managed_env) => managed_env,
            ConcreteEnvironment::Remote(remote_env) => remote_env,
        }
    }
}

/// An environment descriptor of an environment that can be (re)opened,
/// i.e. to install packages into it.
///
/// Unlike [ConcreteEnvironment], this type does not hold a concrete instance any environment,
/// but rather fully qualified metadata to create an instance from.
///
/// * for [PathEnvironment] and [ManagedEnvironment] that's the path to their `.flox` and `.flox/pointer.json`
/// * for [RemoteEnvironment] that's the [ManagedPointer] to the remote environment
///
/// Serialized as is into [FLOX_ACTIVE_ENVIRONMENTS_VAR] to be able to reopen environments.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
#[serde(rename_all = "kebab-case")]
pub enum UninitializedEnvironment {
    /// Container for "local" environments pointed to by [DotFlox]
    DotFlox(DotFlox),
    /// Container for [RemoteEnvironment]
    Remote(ManagedPointer),
}

impl UninitializedEnvironment {
    pub fn from_concrete_environment(env: &ConcreteEnvironment) -> Result<Self> {
        match env {
            ConcreteEnvironment::Path(path_env) => {
                let pointer = path_env.pointer.clone().into();
                Ok(Self::DotFlox(DotFlox {
                    path: path_env.parent_path().unwrap(),
                    pointer,
                }))
            },
            ConcreteEnvironment::Managed(managed_env) => {
                let pointer = managed_env.pointer().clone().into();
                Ok(Self::DotFlox(DotFlox {
                    path: managed_env.parent_path().unwrap(),
                    pointer,
                }))
            },
            ConcreteEnvironment::Remote(remote_env) => {
                let env_ref = remote_env.pointer().clone();
                Ok(Self::Remote(env_ref))
            },
        }
    }

    /// Open the contained environment and return a [ConcreteEnvironment]
    ///
    /// This function will fail if the contained environment is not available or invalid
    pub fn into_concrete_environment(
        self,
        flox: &Flox,
    ) -> Result<ConcreteEnvironment, EnvironmentError2> {
        match self {
            UninitializedEnvironment::DotFlox(dot_flox) => {
                let dot_flox_path = dot_flox.path.join(DOT_FLOX);
                let env = match dot_flox.pointer {
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
            },
            UninitializedEnvironment::Remote(pointer) => {
                let env = RemoteEnvironment::new(flox, pointer)?;
                Ok(ConcreteEnvironment::Remote(env))
            },
        }
    }

    fn pointer(&self) -> EnvironmentPointer {
        match self {
            UninitializedEnvironment::DotFlox(DotFlox { pointer, .. }) => pointer.clone(),
            UninitializedEnvironment::Remote(pointer) => {
                EnvironmentPointer::Managed(pointer.clone())
            },
        }
    }
}

impl Display for UninitializedEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UninitializedEnvironment::DotFlox(DotFlox {
                path,
                pointer: EnvironmentPointer::Managed(managed_pointer),
            }) => {
                write!(
                    f,
                    "{}/{} at {}",
                    managed_pointer.owner,
                    managed_pointer.name,
                    path.to_string_lossy(),
                )
            },
            UninitializedEnvironment::DotFlox(DotFlox {
                path,
                pointer: EnvironmentPointer::Path(path_pointer),
            }) => {
                write!(f, "{} at {}", path_pointer.name, path.to_string_lossy())
            },
            UninitializedEnvironment::Remote(pointer) => {
                write!(f, "{}/{} (remote)", pointer.owner, pointer.name,)
            },
        }
    }
}

/// A list of environments that are currently active
/// (i.e. have been activated with `flox activate`)
///
/// When inside a `flox activate` shell,
/// flox stores [UninitializedEnvironment] metadata to (re)open the activated environment
/// in `$FLOX_ACTIVE_ENVIRONMENTS`.
///
/// Environments which are activated while in a `flox activate` shell, are prepended
/// -> the most recently activated environment is the _first_ in the list of environments.
///
/// Internally this is implemented through a [VecDeque] which is serialized to JSON.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActiveEnvironments(VecDeque<UninitializedEnvironment>);

impl FromStr for ActiveEnvironments {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Ok(Self(VecDeque::new()));
        }
        serde_json::from_str(s).map(Self)
    }
}

impl ActiveEnvironments {
    /// Read the last active environment
    pub fn last_active(&self) -> Option<UninitializedEnvironment> {
        self.0.front().cloned()
    }

    /// Set the last active environment
    pub fn set_last_active(&mut self, env: UninitializedEnvironment) {
        self.0.push_front(env);
    }

    pub fn is_active(&self, env: &UninitializedEnvironment) -> bool {
        self.0.contains(env)
    }
}

impl Display for ActiveEnvironments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let result = if f.alternate() {
            serde_json::to_string_pretty(&self)
        } else {
            serde_json::to_string(&self)
        };
        let data = match result {
            Ok(data) => data,
            Err(e) => {
                debug!("Could not serialize active environments: {e}");
                return Err(fmt::Error);
            },
        };

        f.write_str(&data)
    }
}

impl IntoIterator for ActiveEnvironments {
    type IntoIter = std::collections::vec_deque::IntoIter<Self::Item>;
    type Item = UninitializedEnvironment;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// Determine the most recently activated environment [ActiveEnvironment].
fn last_activated_environment() -> Option<UninitializedEnvironment> {
    activated_environments().last_active()
}

/// Read [ActiveEnvironments] from the process environment [FLOX_ACTIVE_ENVIRONMENTS_VAR]
fn activated_environments() -> ActiveEnvironments {
    let flox_active_environments_var: String =
        env::var(FLOX_ACTIVE_ENVIRONMENTS_VAR).unwrap_or_default();

    match ActiveEnvironments::from_str(&flox_active_environments_var) {
        Ok(active_environments) => active_environments,
        Err(e) => {
            message::error(format!(
                "Could not parse _FLOX_ACTIVE_ENVIRONMENTS -- using defaults: {}",
                e
            ));
            ActiveEnvironments::default()
        },
    }
}

/// Check whether the given [EnvironmentRef] is trusted.
///
/// If not, prompt the user to trust or deny abort or ask again.
///
/// This function returns [`Ok`] if the environment is trusted
/// and a formatted error message if not.
pub(super) async fn ensure_environment_trust(
    config: &mut Config,
    flox: &Flox,
    environment: &RemoteEnvironment,
) -> Result<()> {
    let env_ref = EnvironmentRef::new_from_parts(environment.owner().clone(), environment.name());

    let trust = config.flox.trusted_environments.get(&env_ref);

    // Official Flox environments are trusted by default
    // Only applies to the current flox owned FloxHub,
    // so this rule might need to be revisited in the future.
    if env_ref.owner().as_str() == "flox" {
        debug!("Official Flox environment {env_ref} is trusted by default");
        return Ok(());
    }

    if let Some(ref token) = flox.floxhub_token {
        if token.handle() == env_ref.owner().as_str() {
            debug!("environment {env_ref} is trusted by token");
            return Ok(());
        }
    }

    if matches!(trust, Some(EnvironmentTrust::Trust)) {
        debug!("environment {env_ref} is trusted by config");
        return Ok(());
    }

    if matches!(trust, Some(EnvironmentTrust::Deny)) {
        debug!("environment {env_ref} is denied by config");

        let message = formatdoc! {"
            Environment {env_ref} is not trusted.

            Run 'flox config --set trusted_environments.{env_ref} trust' to trust it."};
        bail!("{message}");
    }

    #[derive(Debug, PartialEq)]
    enum Choices {
        Trust,
        Deny,
        TrustTemporarily,
        Abort,
        ShowConfig,
    }

    #[derive(Debug, derive_more::AsRef)]
    struct Choice<K: fmt::Display, V: PartialEq>(K, #[as_ref] V);
    impl<K: fmt::Display, V: PartialEq> Display for Choice<K, V> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            self.0.fmt(f)
        }
    }
    impl<K: fmt::Display, V: PartialEq> PartialEq for Choice<K, V> {
        fn eq(&self, other: &Self) -> bool {
            self.1 == other.1
        }
    }

    let message = formatdoc! {"
        Environment {env_ref} is not trusted.

        flox environments do not run in a sandbox.
        Activation hooks can run arbitrary code on your machine.
        Thus, environments need to be trusted to be activated."};

    if Dialog::can_prompt() {
        message::warning(message);
    } else {
        bail!("{message}")
    }

    loop {
        let message = format!("Do you trust {env_ref}?", env_ref = env_ref);
        let choice = Dialog {
            message: &message,
            help_message: None,
            typed: Select {
                options: vec![
                    Choice("Do not trust, ask again next time", Choices::Abort),
                    Choice("Do not trust, save choice", Choices::Deny),
                    Choice("Trust, ask again next time", Choices::TrustTemporarily),
                    Choice("Trust, save choice", Choices::Trust),
                    Choice("Show the manifest", Choices::ShowConfig),
                ],
            },
        }
        .prompt()
        .await?;

        debug!("user chose: {choice:?}");

        match choice.as_ref() {
            Choices::Trust => {
                update_config(
                    &flox.config_dir,
                    &flox.temp_dir,
                    format!("trusted_environments.'{}'", env_ref),
                    Some(EnvironmentTrust::Trust),
                )
                .context("Could not write token to config")?;
                let _ = mem::replace(config, Config::parse()?);
                info!("Trusted environment {env_ref} (saved choice)",);
                return Ok(());
            },
            Choices::Deny => {
                update_config(
                    &flox.config_dir,
                    &flox.temp_dir,
                    format!("trusted_environments.'{}'", env_ref),
                    Some(EnvironmentTrust::Deny),
                )
                .context("Could not write token to config")?;
                let _ = mem::replace(config, Config::parse()?);
                bail!("Denied {env_ref} (saved choice).");
            },
            Choices::TrustTemporarily => {
                info!("Trusted environment {env_ref} (temporary)");
                return Ok(());
            },
            Choices::Abort => bail!("Denied {env_ref} (temporary)"),
            Choices::ShowConfig => eprintln!("{}", environment.manifest_content(flox)?),
        }
    }
}

/// Ensure a floxhub_token is present
///
/// If the token is not present and we can prompt the user,
/// run the login flow ([auth::login_flox]).
pub(super) async fn ensure_floxhub_token(flox: &mut Flox) -> Result<()> {
    match flox.floxhub_token {
        Some(ref token) => {
            log::debug!("floxhub token is present; logged in as {}", token.handle());
        },
        None if !Dialog::can_prompt() => {
            log::debug!("floxhub token is not present; can not prompt user");
            let message = formatdoc! {"
                You are not logged in to FloxHub.

                Can not automatically login to FloxHub in non-interactive context.

                To login you can either
                * login to FloxHub with 'flox auth login',
                * set the 'floxhub_token' field to '<your token>' in your config
                * set the '$FLOX_FLOXHUB_TOKEN=<your_token>' environment variable."
            };
            bail!(message);
        },
        None => {
            log::debug!("floxhub token is not present; prompting user");

            message::plain("You are not logged in to FloxHub. Logging in...");
            auth::login_flox(flox).await?;
        },
    };

    Ok(())
}

pub fn environment_description(environment: &ConcreteEnvironment) -> Result<String> {
    Ok(UninitializedEnvironment::from_concrete_environment(environment)?.to_string())
}

#[cfg(test)]
mod tests {

    use flox_rust_sdk::flox::EnvironmentName;
    use flox_rust_sdk::models::environment::PathPointer;

    use super::*;

    /// is_active() behaves as expected when using set_last_active()
    #[test]
    fn test_is_active() {
        let env1 = UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::new(),
            pointer: EnvironmentPointer::Path(PathPointer::new(
                EnvironmentName::from_str("env1").unwrap(),
            )),
        });
        let env2 = UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::new(),
            pointer: EnvironmentPointer::Path(PathPointer::new(
                EnvironmentName::from_str("env2").unwrap(),
            )),
        });

        let mut active = ActiveEnvironments::default();
        active.set_last_active(env1.clone());

        assert!(active.is_active(&env1));
        assert!(!active.is_active(&env2));
    }

    /// Simulate setting an active environment in one flox invocation and then
    /// checking if it's active in a second.
    #[test]
    fn test_is_active_round_trip_from_env() {
        let uninitialized = UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::new(),
            pointer: EnvironmentPointer::Path(PathPointer::new(
                EnvironmentName::from_str("test").unwrap(),
            )),
        });
        let mut first_active = temp_env::with_var(
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            None::<&str>,
            activated_environments,
        );

        first_active.set_last_active(uninitialized.clone());

        let second_active = temp_env::with_var(
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            Some(first_active.to_string()),
            activated_environments,
        );

        assert!(second_active.is_active(&uninitialized));
    }
}
