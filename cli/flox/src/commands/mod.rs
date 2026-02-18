mod activate;
mod auth;
mod build;
mod check_for_upgrades;
mod containerize;
mod delete;
mod edit;
mod envs;
mod exit;
mod gc;
mod general;
mod generations;
mod include;
mod init;
mod install;
mod list;
mod lock_manifest;
mod path_hash;
mod publish;
mod pull;
mod push;
mod search;
mod services;
mod show;
mod uninstall;
mod upgrade;
mod upload;

use std::fmt::Display;
use std::path::PathBuf;
use std::str::FromStr;
use std::{env, fmt, mem};

use anyhow::{Context, Result, anyhow, bail};
use bpaf::{Args, Bpaf, ParseFailure, Parser, ShellComp};
use flox_core::data::environment_ref::{self, DEFAULT_NAME, RemoteEnvironmentRef};
use flox_manifest::{Manifest, TypedOnly};
use flox_rust_sdk::flox::{
    DEFAULT_FLOXHUB_URL,
    FLOX_VERSION,
    Flox,
    Floxhub,
    FloxhubToken,
    FloxhubTokenError,
};
use flox_rust_sdk::models::env_registry;
use flox_rust_sdk::models::env_registry::{ENV_REGISTRY_FILENAME, EnvRegistry};
use flox_rust_sdk::models::environment::generations::GenerationId;
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
use flox_rust_sdk::models::environment::{
    ConcreteEnvironment,
    DOT_FLOX,
    DotFlox,
    EnvironmentError,
    ManagedPointer,
    UninitializedEnvironment,
    find_dot_flox,
    open_path,
};
use indoc::{formatdoc, indoc};
use tempfile::TempDir;
use thiserror::Error;
use toml_edit::visit_mut::VisitMut;
use toml_edit::{Item, Key, KeyMut, Value};
use tracing::{debug, info};
use url::Url;
use xdg::BaseDirectories;

use self::envs::DisplayEnvironments;
use crate::commands::general::update_config;
use crate::config::{
    Config,
    EnvironmentTrust,
    FLOX_CONFIG_FILE,
    FLOX_DIR_NAME,
    FLOX_DISABLE_METRICS_VAR,
};
use crate::utils::active_environments::{
    ActiveEnvironments,
    activated_environments,
    last_activated_environment,
};
use crate::utils::dialog::{Dialog, Select};
use crate::utils::errors::display_chain;
use crate::utils::init::{
    init_catalog_client,
    init_telemetry_uuid,
    telemetry_opt_out_needs_migration,
};
use crate::utils::message;
use crate::utils::metrics::{AWSDatalakeConnection, Client, Hub, METRICS_UUID_FILE_NAME};
use crate::utils::update_notifications::UpdateNotification;

const SHELL_COMPLETION_DIR: ShellComp = ShellComp::Dir { mask: None };
const SHELL_COMPLETION_FILE: ShellComp = ShellComp::File { mask: None };

static FLOX_DESCRIPTION: &'_ str = indoc! {"
    Flox is a virtual environment and package manager all in one.\n\n

    With Flox you create environments that layer and replace dependencies just where it matters,
    making them portable across the full software lifecycle."
};

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
    pub fn to_i32(self) -> i32 {
        match self {
            Verbosity::Quiet => -1,
            Verbosity::Verbose(n) => n
                .try_into()
                .expect("If you passed -v enough times to overflow an i32, I'm impressed"),
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
        // ensure xdg dirs exist
        tokio::fs::create_dir_all(&config.flox.config_dir).await?;
        tokio::fs::create_dir_all(&config.flox.data_dir).await?;
        let flox_dirs = BaseDirectories::with_prefix(FLOX_DIR_NAME);
        // runtime_dir is used for socket paths,
        // so we have to try to keep it short.
        // See comment on services_socket_path for more
        let runtime_dir = match flox_dirs.get_runtime_directory() {
            Ok(runtime_dir) => runtime_dir.to_path_buf(),
            Err(_) => config.flox.cache_dir.join("run"),
        };
        tokio::fs::create_dir_all(&runtime_dir).await?;

        // prepare a temp dir for the run:
        let process_dir = config.flox.cache_dir.join("process");
        tokio::fs::create_dir_all(&process_dir).await?;

        // `temp_dir` will automatically be removed from disk when the function returns
        let temp_dir = TempDir::new_in(process_dir)?;

        let update_channel = config.flox.installer_channel.clone();

        // Given no command, skip initialization and print welcome message
        if self.command.is_none() {
            let envs = env_registry::read_environment_registry(
                config.flox.data_dir.join(ENV_REGISTRY_FILENAME),
            )?
            .unwrap_or_default();
            let active_environments = activated_environments();
            print_welcome_message(envs, active_environments);
            UpdateNotification::check_for_and_print_update_notification(
                &config.flox.cache_dir,
                &update_channel,
            )
            .await;
            return Ok(());
        }

        let cache_dir = config.flox.cache_dir.clone();

        let check_for_update_handle = {
            let update_channel = update_channel.clone();
            tokio::spawn(async move {
                UpdateNotification::check_for_update(cache_dir, &update_channel).await
            })
        };

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
                &[Key::new("disable_metrics")],
                Some(true),
            )?;

            // remove marker uuid file
            tokio::fs::remove_file(&config.flox.data_dir.join(METRICS_UUID_FILE_NAME)).await?;
        }

        if !config.flox.disable_metrics {
            debug!("Metrics collection enabled");

            init_telemetry_uuid(&config.flox.data_dir, &config.flox.cache_dir)?;

            let connection = AWSDatalakeConnection::default();
            let client = Client::new_with_config(&config, connection)?;
            Hub::global().set_client(client);
        } else {
            debug!("Metrics collection disabled");
            unsafe {
                env::set_var(FLOX_DISABLE_METRICS_VAR, "true");
            }
        }

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
            .and_then(|s| {
                if s.is_empty() {
                    None
                } else {
                    Some(FloxhubToken::from_str(s))
                }
            })
            .transpose();

        let floxhub_token = match floxhub_token {
            Err(FloxhubTokenError::Expired) => {
                message::warning("Your FloxHub token has expired. You may need to log in again.");
                if let Err(e) =
                    update_config(&config.flox.config_dir, "floxhub_token", None::<String>)
                {
                    debug!("Could not remove token from user config: {e}");
                }
                None
            },
            Err(FloxhubTokenError::InvalidToken(token_error)) => {
                message::error(formatdoc! {"
                    Your FloxHub token is invalid: {token_error}
                    You may need to log in again.
                "});
                if let Err(e) =
                    update_config(&config.flox.config_dir, "floxhub_token", None::<String>)
                {
                    debug!("Could not remove token from user config: {e}");
                }
                None
            },
            Ok(token) => token,
        };

        let catalog_client = init_catalog_client(&config)?;

        // we already make sure $USER corresponds to **euid** earlier on oin the process.
        let system_user_name =
            std::env::var("USER").context("could not determine username from $USER")?;
        let system_hostname = sys_info::hostname().context("could not determine hostname")?;
        let argv = std::env::args().collect();

        let flox = Flox {
            cache_dir: config.flox.cache_dir.clone(),
            data_dir: config.flox.data_dir.clone(),
            state_dir: config.flox.state_dir.clone(),
            config_dir: config.flox.config_dir.clone(),
            runtime_dir,
            temp_dir: temp_dir.path().to_path_buf(),
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            system_user_name,
            system_hostname,
            argv,
            floxhub_token,
            floxhub,
            catalog_client,
            installable_locker: Default::default(),
            #[allow(deprecated, reason = "This should be the only internal use")]
            features: config.features.unwrap_or_default(),
            verbosity: self.verbosity.to_i32(),
        };
        debug!(
            configured = ?flox.features,
            "feature flags"
        );

        let signal_handler = async { tokio::signal::ctrl_c().await.unwrap() };
        let keep_tempfiles = config.flox.keep_tempdir.unwrap_or_default();

        let cli_worker = async move {
            // command handled above
            let result = match self.command.unwrap() {
                Commands::Help(group) => {
                    group.handle();
                    Ok(())
                },
                Commands::Manage(args) => args.handle(flox).await,
                Commands::Use(args) => args.handle(config, flox).await,
                Commands::Discover(args) => args.handle(config, flox).await,
                Commands::Modify(args) => args.handle(config, flox).await,
                Commands::Share(args) => args.handle(config, flox).await,
                Commands::Admin(args) => args.handle(config, flox).await,
                Commands::Internal(args) => args.handle(flox).await,
            };

            // This will print the update notification after output from a successful
            // command but before an error is printed for an unsuccessful command.
            // That's a bit weird,
            // but I'm not sure it's worth a refactor.
            match check_for_update_handle.await {
                Ok(update_notification) => {
                    UpdateNotification::handle_update_result(update_notification, &update_channel);
                },
                Err(e) => debug!("Failed to check for CLI update: {}", display_chain(&e)),
            }

            result
        };

        // Wait for either an interrupting signal or completion of the cli work
        let result = tokio::task::LocalSet::new()
            .run_until(async {
                tokio::select! {
                    _ = tokio::task::spawn_local(signal_handler) => {
                        // TODO:
                        // For now we rely on subprocesses to inherit `flox` process group
                        // and thus being sent ctrl_c signals in sync with flox itself.
                        // If we do need more control here,
                        // we can find process children and propagate signals manually.
                        Err(anyhow!("user interrupted process"))
                    }
                    result = tokio::task::spawn_local(cli_worker) => result?
                }
            })
            .await;

        // Remove tempdirs
        if (self.debug || matches!(self.verbosity, Verbosity::Verbose(1..))) && keep_tempfiles {
            debug!(temp_dir = ?temp_dir.path(), "leaving process tempdir in place");
            let _ = temp_dir.keep();
        } else {
            debug!(temp_dir = ?temp_dir.path(), "removing process tempdir");
            drop(temp_dir);
        }

        result
    }
}

/// Print general welcome message with short usage instructions
/// and give hints for creating and activating environments.
/// List active environments if any are active.
fn print_welcome_message(envs: EnvRegistry, active_environments: ActiveEnvironments) {
    let welcome_message = {
        let version = FLOX_VERSION.to_string();
        formatdoc! {r#"
            flox version {version}

            Usage: flox OPTIONS (init|activate|search|install|...) [--help]

            Use 'flox --help' for full list of commands and more information
        "#}
    };

    message::plain(welcome_message);

    // print trailer message
    // - if no environments are known to Flox yet, hint at creating one
    // - if no environments are active, hint at activating one
    // - if environments are active, list them

    if envs.entries.is_empty() {
        message::plain("First time? Create an environment with 'flox init'\n");
        return;
    }

    if active_environments.last_active().is_none() {
        message::plain("No active environments. Use 'flox envs' to list all environments.\n");
    } else {
        message::created("Active environments:");
        let envs = indent::indent_all_by(
            2,
            DisplayEnvironments::new(active_environments.iter(), true).to_string(),
        );
        // We should use message::plain once bold formatting is fixed in
        // tracing-subscriber
        // https://github.com/tokio-rs/tracing/issues/3369
        eprintln!("{envs}");
    }
}

#[allow(clippy::large_enum_variant)] // there's only a single instance of this enum
#[derive(Bpaf, Clone)]
enum Commands {
    /// Prints help information
    #[bpaf(command, hide)]
    Help(#[bpaf(external(help))] Help),

    Manage(#[bpaf(external(manage_commands))] ManageCommands),
    Use(#[bpaf(external(use_commands))] UseCommands),
    Discover(#[bpaf(external(discover_commands))] DiscoverCommands),
    Modify(#[bpaf(external(modify_commands))] ModifyCommands),
    Share(#[bpaf(external(share_commands))] ShareCommands),
    Admin(#[bpaf(external(admin_commands))] AdminCommands),

    Internal(#[bpaf(external(internal_commands))] InternalCommands),
}

#[derive(Debug, Bpaf, Clone)]
struct Help {
    /// Command to show help for
    #[bpaf(positional("cmd"))]
    cmd: Option<String>,
}

/// Force `--help` output for `flox` with a given command
pub fn display_help(cmd: Option<String>) {
    let mut args = Vec::from_iter(cmd.as_deref());
    args.push("--help");

    match flox_cli().run_inner(&*args) {
        Ok(_) => unreachable!(),
        Err(ParseFailure::Completion(comp)) => print!("{comp:80}"),
        Err(ParseFailure::Stdout(doc, _)) => message::plain(format!("{doc:80}")),
        Err(ParseFailure::Stderr(err)) => message::error(err),
    }
}
impl Help {
    fn handle(self) {
        display_help(self.cmd);
    }
}

/// Manage environments
#[derive(Bpaf, Clone)]
enum ManageCommands {
    /// Create an environment in the current directory
    #[bpaf(
        command,
        long("create"),
        footer("Run 'man flox-init' for more details.")
    )]
    Init(#[bpaf(external(init::init))] init::Init),

    /// Show active and available environments
    #[bpaf(command, footer("Run 'man flox-envs' for more details."))]
    Envs(#[bpaf(external(envs::envs))] envs::Envs),

    /// Delete an environment
    #[bpaf(
        command,
        long("destroy"),
        footer("Run 'man flox-delete' for more details.")
    )]
    Delete(#[bpaf(external(delete::delete))] delete::Delete),
}

impl ManageCommands {
    async fn handle(self, flox: Flox) -> Result<()> {
        match self {
            ManageCommands::Init(args) => args.handle(flox).await?,
            ManageCommands::Envs(args) => args.handle(flox)?,
            ManageCommands::Delete(args) => args.handle(flox).await?,
        }
        Ok(())
    }
}

/// Use environments
#[derive(Bpaf, Clone)]
enum UseCommands {
    /// Enter the environment, type 'exit' to leave
    #[bpaf(
        command,
        long("develop"),
        header(indoc! {"
            When called with no arguments 'flox activate' will look for a '.flox' directory
            in the current directory. Calling 'flox activate' in your home directory will
            activate a default environment. Environments in other directories and FloxHub
            environments are activated with the '-d' and '-r' flags respectively.
        "}),
        footer("Run 'man flox-activate' for more details.")
    )]
    Activate(#[bpaf(external(activate::activate))] activate::Activate),

    /// Manage services in an environment
    #[bpaf(command)]
    Services(
        #[bpaf(
            external(services::services_commands),
            fallback(services::ServicesCommands::Help)
        )]
        services::ServicesCommands,
    ),
}

impl UseCommands {
    async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            UseCommands::Activate(args) => args.handle(config, flox).await?,
            UseCommands::Services(args) => args.handle(config, flox).await?,
        }
        Ok(())
    }
}

/// Discover packages
#[derive(Bpaf, Clone)]
enum DiscoverCommands {
    /// Search for system or library packages to install
    #[bpaf(command, footer("Run 'man flox-search' for more details."))]
    Search(#[bpaf(external(search::search))] search::Search),

    /// Show details about a single package
    #[bpaf(command, long("show"), footer("Run 'man flox-show' for more details."))]
    Show(#[bpaf(external(show::show))] show::Show),
}

impl DiscoverCommands {
    async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            DiscoverCommands::Search(args) => args.handle(config, flox).await?,
            DiscoverCommands::Show(args) => args.handle(flox).await?,
        }
        Ok(())
    }
}

/// Modify environments
#[derive(Bpaf, Clone)]
enum ModifyCommands {
    /// Install packages into an environment
    #[bpaf(
        command,
        short('i'),
        footer("Run 'man flox-install' for more details.")
    )]
    Install(#[bpaf(external(install::install))] install::Install),

    /// List packages installed in an environment
    #[bpaf(
        command,
        long("list"),
        short('l'),
        footer("Run 'man flox-list' for more details.")
    )]
    List(#[bpaf(external(list::list))] list::List),

    /// Edit declarative environment configuration file
    #[bpaf(command, footer("Run 'man flox-edit' for more details."))]
    Edit(#[bpaf(external(edit::edit))] edit::Edit),

    /// Compose environments together
    #[bpaf(command)]
    Include(
        #[bpaf(
            external(include::include_commands),
            fallback(include::IncludeCommands::Help)
        )]
        include::IncludeCommands,
    ),

    /// Upgrade packages in an environment
    #[bpaf(command, long("update"), footer("Run 'man flox-upgrade' for more details."), header(indoc! {"
        When no arguments are specified,
        all packages in the environment are upgraded if possible.
        A package is upgraded if its version, build configuration,
        or dependency graph changes.\n\n

        Packages to upgrade can be specified by group name.
        Packages without a specified pkg-group in the manifest
        are placed in a group named 'toplevel'.
        The packages in that group can be upgraded without updating any other groups
        by passing 'toplevel' as the group name.\n\n

        A single package can only be specified to upgrade by ID
        if it is not in a group with any other packages.
    "}))]
    Upgrade(#[bpaf(external(upgrade::upgrade))] upgrade::Upgrade),

    /// Uninstall installed packages from an environment
    #[bpaf(
        command,
        long("remove"),
        long("rm"),
        footer("Run 'man flox-uninstall' for more details.")
    )]
    Uninstall(#[bpaf(external(uninstall::uninstall))] uninstall::Uninstall),

    /// Version control for environments pushed to FloxHub
    #[bpaf(command, long("generations"), long("generation"))]
    Generations(
        #[bpaf(
            external(generations::generations_commands),
            fallback(generations::GenerationsCommands::Help)
        )]
        generations::GenerationsCommands,
    ),
}

impl ModifyCommands {
    async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            ModifyCommands::Install(args) => args.handle(flox).await?,
            ModifyCommands::List(args) => args.handle(flox).await?,
            ModifyCommands::Edit(args) => args.handle(flox).await?,
            ModifyCommands::Include(args) => args.handle(flox).await?,
            ModifyCommands::Upgrade(args) => args.handle(flox).await?,
            ModifyCommands::Uninstall(args) => args.handle(flox).await?,
            ModifyCommands::Generations(args) => args.handle(config, flox)?,
        }
        Ok(())
    }
}

/// Share with others
#[derive(Bpaf, Clone)]
enum ShareCommands {
    /// Build packages for Flox
    #[bpaf(
        command,
        header(indoc!{"Build packages from the manifest's 'build' table, Nix
                       expression files in '.flox/pkgs/', or run 'clean'
                       subcommand if specified."}),
        footer("Run 'man flox-build' for more details.")
    )]
    Build(#[bpaf(external(build::build))] build::Build),

    /// Publish packages for Flox
    #[bpaf(
        command,
        header(indoc!{"Publish the specified `<package>` from the environment in `<path>`, uploading
                       artifact metadata and copying the artifacts so that it is available in the
                       Flox Catalog."}),
        footer("Run 'man flox-publish' for more details.")
    )]
    Publish(#[bpaf(external(publish::publish))] publish::Publish),

    /// Send an environment to FloxHub
    #[bpaf(command, footer("Run 'man flox-push' for more details."))]
    Push(#[bpaf(external(push::push))] push::Push),

    /// Pull an environment from FloxHub
    #[bpaf(command, footer("Run 'man flox-pull' for more details."))]
    Pull(#[bpaf(external(pull::pull))] pull::Pull),

    /// Containerize an environment
    #[bpaf(command, footer("Run 'man flox-containerize' for more details."))]
    Containerize(#[bpaf(external(containerize::containerize))] containerize::Containerize),
}

impl ShareCommands {
    async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            ShareCommands::Build(args) => args.handle(flox).await?,
            ShareCommands::Publish(args) => args.handle(config, flox).await?,
            ShareCommands::Push(args) => args.handle(flox).await?,
            ShareCommands::Pull(args) => args.handle(flox).await?,
            ShareCommands::Containerize(args) => args.handle(flox).await?,
        }
        Ok(())
    }
}

/// Administration
#[derive(Bpaf, Clone)]
enum AdminCommands {
    /// FloxHub authentication commands
    #[bpaf(command, footer("Run 'man flox-auth' for more details."))]
    Auth(#[bpaf(external(auth::auth))] auth::Auth),

    /// View and set configuration options
    #[bpaf(command, footer("Run 'man flox-config' for more details."))]
    Config(#[bpaf(external(general::config_args))] general::ConfigArgs),

    /// Garbage collects any data for deleted environments.
    #[bpaf(
        command,
        header(
            "This both deletes data managed by Flox and runs garbage collection on the Nix store."
        ),
        footer("Run 'man flox-gc' for more details.")
    )]
    Gc(#[bpaf(external(gc::gc))] gc::Gc),
}

impl AdminCommands {
    async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        match self {
            AdminCommands::Auth(args) => args.handle(config, flox).await?,
            AdminCommands::Config(args) => args.handle(config, flox).await?,
            AdminCommands::Gc(args) => args.handle(flox)?,
        }
        Ok(())
    }
}

/// Internal commands that aren't documented or supported for public use.
#[derive(Bpaf, Clone)]
#[bpaf(hide)]
enum InternalCommands {
    /// Reset the metrics queue (if any), reset metrics ID, and re-prompt for consent
    #[bpaf(command("reset-metrics"), hide)]
    ResetMetrics(#[bpaf(external(general::reset_metrics))] general::ResetMetrics),

    /// Upload packages
    #[bpaf(command, hide, footer("Run 'man flox-upload' for more details."))]
    Upload(#[bpaf(external(upload::upload))] upload::Upload),

    /// Lock a manifest file
    #[bpaf(command, hide)]
    LockManifest(#[bpaf(external(lock_manifest::lock_manifest))] lock_manifest::LockManifest),

    /// Check for environmet upgrades
    #[bpaf(command, hide)]
    CheckForUpgrades(
        #[bpaf(external(check_for_upgrades::check_for_upgrades))]
        check_for_upgrades::CheckForUpgrades,
    ),

    /// Print information how to exit environment
    #[bpaf(command, long("exit"), long("deactivate"), hide)]
    Exit(#[bpaf(external(exit::exit))] exit::Exit),

    /// Print the hash of a filesystem path. Useful for determining which
    /// activation state directory to look at while debugging.
    #[bpaf(command, long("path-hash"), hide)]
    PathHash(#[bpaf(external(path_hash::path_hash))] path_hash::PathHash),
}

impl InternalCommands {
    async fn handle(self, flox: Flox) -> Result<()> {
        match self {
            InternalCommands::ResetMetrics(args) => args.handle(flox).await?,
            InternalCommands::Upload(args) => args.handle(flox).await?,
            InternalCommands::LockManifest(args) => args.handle(flox).await?,
            InternalCommands::CheckForUpgrades(args) => args.handle(flox).await?,
            InternalCommands::Exit(args) => args.handle(flox)?,
            InternalCommands::PathHash(args) => args.handle()?,
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
        #[bpaf(
            long("dir"),
            short('d'),
            argument("path"),
            complete_shell(SHELL_COMPLETION_DIR)
        )]
        PathBuf,
    ),
    Remote(
        /// A FloxHub environment
        #[bpaf(long("reference"), long("ref"), short('r'), argument("owner>/<name"))]
        environment_ref::RemoteEnvironmentRef,
    ),
    #[default]
    #[bpaf(hide)]
    Unspecified,
}

#[derive(Debug, Default, Bpaf, Clone)]
pub enum DirEnvironmentSelect {
    Dir(
        /// Path containing a .flox/ directory
        #[bpaf(
            long("dir"),
            short('d'),
            argument("path"),
            complete_shell(SHELL_COMPLETION_DIR)
        )]
        PathBuf,
    ),
    #[default]
    #[bpaf(hide)]
    Unspecified,
}

#[derive(Debug, Error)]
pub enum EnvironmentSelectError {
    #[error(transparent)]
    EnvironmentError(#[from] EnvironmentError),
    #[error("Did not find an environment in the current directory.")]
    EnvNotFoundInCurrentDirectory,
    #[error("Remote environments not supported for this operation")]
    RemoteNotSupported,
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
        generation: Option<GenerationId>,
    ) -> Result<ConcreteEnvironment, EnvironmentSelectError> {
        let env = match self {
            EnvironmentSelect::Dir(path) => {
                debug!(
                    path = %path.display(),
                    "getting concrete environment from supplied path"
                );
                open_path(flox, path, generation)?
            },
            EnvironmentSelect::Unspecified => {
                debug!("getting concrete environment without explicit args");
                let current_dir = env::current_dir().context("could not get current directory")?;
                let maybe_found_environment = find_dot_flox(&current_dir)?;
                match maybe_found_environment {
                    Some(found) => UninitializedEnvironment::DotFlox(found)
                        .into_concrete_environment(flox, generation)?,
                    None => return Err(EnvironmentSelectError::EnvNotFoundInCurrentDirectory),
                }
            },
            EnvironmentSelect::Remote(env_ref) => {
                debug!(
                    remote = env_ref.to_string(),
                    "getting concrete environment from remote"
                );
                let pointer = ManagedPointer::new(
                    env_ref.owner().clone(),
                    env_ref.name().clone(),
                    &flox.floxhub,
                );

                let env = RemoteEnvironment::new(flox, pointer, generation)
                    .map_err(anyhow::Error::new)?;
                ConcreteEnvironment::Remote(env)
            },
        };
        Ok(env)
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
        let env = match self {
            EnvironmentSelect::Dir(path) => DirEnvironmentSelect::Dir(path.clone())
                .detect_concrete_environment(flox, message)?,
            EnvironmentSelect::Unspecified => {
                DirEnvironmentSelect::Unspecified.detect_concrete_environment(flox, message)?
            },
            EnvironmentSelect::Remote(env_ref) => {
                let pointer = ManagedPointer::new(
                    env_ref.owner().clone(),
                    env_ref.name().clone(),
                    &flox.floxhub,
                );

                let generation = activated_environments()
                    .is_active_with_generation(&UninitializedEnvironment::Remote(pointer.clone()));

                let env = RemoteEnvironment::new(flox, pointer, generation)
                    .map_err(anyhow::Error::new)?;

                ConcreteEnvironment::Remote(env)
            },
        };
        Ok(env)
    }

    fn to_flags(&self) -> Option<Vec<String>> {
        match self {
            EnvironmentSelect::Dir(path) => {
                Some(vec!["-d".to_string(), path.display().to_string()])
            },
            EnvironmentSelect::Remote(env_ref) => Some(vec!["-r".to_string(), env_ref.to_string()]),
            EnvironmentSelect::Unspecified => None,
        }
    }
}

impl DirEnvironmentSelect {
    pub fn detect_concrete_environment(
        &self,
        flox: &Flox,
        message: &str,
    ) -> Result<ConcreteEnvironment, EnvironmentSelectError> {
        match self {
            DirEnvironmentSelect::Dir(path) => Ok(open_path(flox, path, None)?),
            // If the user doesn't specify an environment, check if there's an
            // already activated environment or an environment in the current
            // directory.
            DirEnvironmentSelect::Unspecified => match detect_environment(message)? {
                Some(UninitializedEnvironment::Remote(_)) => {
                    Err(EnvironmentSelectError::RemoteNotSupported)
                },
                Some(env) => {
                    let generation = activated_environments().is_active_with_generation(&env);
                    Ok(env.into_concrete_environment(flox, generation)?)
                },
                None => Err(EnvironmentSelectError::EnvNotFoundInCurrentDirectory)?,
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

        // If an environment is activated and a 'default' environment is present
        // in the current directory or git repo, prefer the activated one.
        (Some(activated), Some(detected)) if detected.pointer.name().as_ref() == DEFAULT_NAME => {
            Some(activated.clone())
        },

        // If we can't prompt, use the environment found in the current directory or git repo
        (Some(_), Some(found)) if !Dialog::can_prompt() => {
            debug!(
                "No TTY detected, using the environment {found:?} found in the current directory or an ancestor directory"
            );
            Some(UninitializedEnvironment::DotFlox(found))
        },
        // If there's both an activated environment and an environment in the
        // current directory or git repo, prompt for which to use.
        (Some(activated_env), Some(found)) => {
            let found_in_current_dir = found.path == current_dir.join(DOT_FLOX);
            Some(query_which_environment(
                message,
                activated_env,
                found,
                found_in_current_dir,
            )?)
        },
        (Some(activated_env), None) => Some(activated_env),
        (None, Some(found)) => Some(UninitializedEnvironment::DotFlox(found)),
        (None, None) => None,
    };
    Ok(found)
}

/// Helper function for [detect_environment] which handles the user prompt to decide which environment to use for the current operation.
fn query_which_environment(
    message: &str,
    activated_env: UninitializedEnvironment,
    found: DotFlox,
    found_in_current_dir: bool,
) -> Result<UninitializedEnvironment> {
    let type_of_directory = if found_in_current_dir {
        "current directory"
    } else {
        "detected in git repo"
    };
    let found = UninitializedEnvironment::DotFlox(found);

    let message = format!("{message} which environment?");

    let dialog = Dialog {
        message: &message,
        help_message: None,
        typed: Select {
            options: vec![
                format!("{type_of_directory} [{}]", found.bare_description()),
                format!("currently active [{}]", activated_env.bare_description()),
            ],
        },
    };
    let (index, _) = dialog.raw_prompt().map_err(anyhow::Error::new)?;
    match index {
        0 => Ok(found),
        1 => Ok(activated_env),
        _ => unreachable!(),
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
    env_ref: &RemoteEnvironmentRef,
    env_included: bool,
    manifest_contents: &String,
) -> Result<()> {
    let trust = config.flox.trusted_environments.get(env_ref);
    let env_config_key = format!("trusted_environments.{env_ref}");
    let env_prefixed_name = match env_included {
        true => format!("included environment {env_ref}"),
        false => format!("environment {env_ref}"),
    };

    // Official Flox environments are trusted by default
    // Only applies to the current flox owned FloxHub,
    // so this rule might need to be revisited in the future.
    if env_ref.owner().as_str() == "flox" {
        debug!("Official Flox {env_prefixed_name} is trusted by default");
        return Ok(());
    }

    if let Some(ref token) = flox.floxhub_token
        && token.handle() == env_ref.owner().as_str()
    {
        debug!("{env_prefixed_name} is trusted by token");
        return Ok(());
    }

    if matches!(trust, Some(EnvironmentTrust::Trust)) {
        debug!("{env_prefixed_name} is trusted by config");
        return Ok(());
    }

    if matches!(trust, Some(EnvironmentTrust::Deny)) {
        debug!("{env_prefixed_name} is denied by config");

        let message = formatdoc! {"
            The {env_prefixed_name} is not trusted.

            Run 'flox config --set {env_config_key} trust' to trust it."};
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
        The {env_prefixed_name} is not trusted.

        flox environments do not run in a sandbox.
        Activation hooks can run arbitrary code on your machine.
        Thus, environments need to be trusted to be activated."};

    if Dialog::can_prompt() {
        message::warning(message);
    } else {
        bail!("{message}")
    }

    loop {
        let message = format!("Do you trust the {env_prefixed_name}?");
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
                    &env_config_key,
                    Some(EnvironmentTrust::Trust),
                )
                .context("Could not write token to config")?;
                let _ = mem::replace(config, Config::parse()?);
                info!("Trusted {env_prefixed_name} (saved choice)",);
                return Ok(());
            },
            Choices::Deny => {
                update_config(
                    &flox.config_dir,
                    &env_config_key,
                    Some(EnvironmentTrust::Deny),
                )
                .context("Could not write token to config")?;
                let _ = mem::replace(config, Config::parse()?);
                bail!("Denied {env_prefixed_name} (saved choice).");
            },
            Choices::TrustTemporarily => {
                info!("Trusted {env_prefixed_name} (temporary)");
                return Ok(());
            },
            Choices::Abort => bail!("Denied {env_prefixed_name} (temporary)"),
            Choices::ShowConfig => eprintln!("{}", manifest_contents),
        }
    }
}

/// Ensure a floxhub_token is present
///
/// If the token is not present and we can prompt the user,
/// run the login flow ([auth::login_flox]).
pub(super) async fn ensure_floxhub_token(flox: &mut Flox) -> Result<&FloxhubToken> {
    match flox.floxhub_token {
        Some(ref token) => {
            debug!("floxhub token is present; logged in as {}", token.handle());
            Ok(token)
        },
        None if !Dialog::can_prompt() => {
            debug!("floxhub token is not present; can not prompt user");
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
            debug!("floxhub token is not present; prompting user");

            message::plain("You are not logged in to FloxHub. Logging in...");
            let token = auth::login_flox(flox).await?;
            Ok(token)
        },
    }
}

pub fn environment_description(environment: &ConcreteEnvironment) -> Result<String> {
    uninitialized_environment_description(&UninitializedEnvironment::from_concrete_environment(
        environment,
    ))
}

/// The environment description when displayed in messages
/// Use UninitializedEnvironment::bare_description for other cases
pub fn uninitialized_environment_description(
    environment: &UninitializedEnvironment,
) -> Result<String> {
    if let Some(owner) = environment.owner_if_remote() {
        Ok(format!("'{}/{}' (local)", owner, environment.name()))
    } else if let Some(owner) = environment.owner_if_managed() {
        Ok(format!("'{}/{}'", owner, environment.name()))
    } else if is_current_dir(environment).context("couldn't read current directory")? {
        Ok(String::from("in current directory"))
    } else {
        Ok(format!("'{}'", environment.name()))
    }
}

/// Returns true if the environment is in the current directory
fn is_current_dir(environment: &UninitializedEnvironment) -> Result<bool> {
    match environment {
        UninitializedEnvironment::DotFlox(DotFlox { path, .. }) => {
            let current_dir = std::env::current_dir()?;
            let is_current = current_dir.canonicalize()? == path.canonicalize()?;
            Ok(is_current)
        },
        UninitializedEnvironment::Remote(_) => Ok(false),
    }
}

/// Render a merged or included `Manifest` to a string for displaying to the user.
///
/// `Environment::manifest_contents` should be used for non-composition
/// manifests so that it matches what the user has on disk.
fn render_composition_manifest(manifest: &Manifest<TypedOnly>) -> Result<String> {
    // A visitor that converts inline tables to proper tables
    // Nested tables are rendered as `dotted` tables.
    // The default behavior when instantiating with `Visitor::new_for_document`,
    // is to render toplevel tables as non-dotted, sections.
    struct Visitor {
        dotted: bool,
    }
    impl Visitor {
        fn new_for_document() -> Self {
            Visitor { dotted: false }
        }
    }
    impl VisitMut for Visitor {
        fn visit_table_like_kv_mut(&mut self, _key: KeyMut<'_>, node: &mut Item) {
            if let toml_edit::Item::Value(Value::InlineTable(inline_table)) = node {
                let mut table = std::mem::take(inline_table).into_table();
                table.set_implicit(true);
                table.set_dotted(self.dotted);
                toml_edit::visit_mut::visit_table_mut(&mut Visitor { dotted: true }, &mut table);
                *node = toml_edit::Item::Table(table);
            }
        }
    }

    let mut document = toml_edit::ser::to_document(manifest)?;
    toml_edit::visit_mut::visit_document_mut(&mut Visitor::new_for_document(), &mut document);

    Ok(document.to_string())
}
