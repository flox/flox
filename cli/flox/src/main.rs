use std::env;
use std::fmt::{Debug, Display};
use std::process::ExitCode;

use anyhow::{Context, Result};
use bpaf::{Args, Parser};
use commands::{EnvironmentSelectError, FloxArgs, FloxCli, Prefix, Version};
use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::models::environment::managed_environment::ManagedEnvironmentError;
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironmentError;
use flox_rust_sdk::models::environment::EnvironmentError;
use flox_rust_sdk::providers::services::ServiceError;
use log::{debug, warn};
use utils::errors::format_service_error;
use utils::init::{init_logger, init_sentry};
use utils::{message, populate_default_nix_env_vars};

use crate::utils::errors::{
    format_environment_select_error,
    format_error,
    format_managed_error,
    format_remote_error,
};
use crate::utils::metrics::Hub;

mod commands;
mod config;
mod utils;

async fn run(args: FloxArgs) -> Result<()> {
    init_logger(Some(args.verbosity));
    set_parent_process_id();
    populate_default_nix_env_vars();
    let config = config::Config::parse()?;
    let uuid = utils::metrics::read_metrics_uuid(&config)
        .map(|u| Some(u.to_string()))
        .unwrap_or(None);
    sentry::configure_scope(|scope| {
        scope.set_user(Some(sentry::User {
            id: uuid,
            ..Default::default()
        }));
    });
    args.handle(config).await?;
    Ok(())
}

fn main() -> ExitCode {
    // Avoid SIGPIPE from killing the process
    reset_sigpipe();

    // initialize logger with "best guess" defaults
    // updating the logger conf is cheap, so we reinitialize whenever we get more information
    init_logger(None);

    // Quit early if `--prefix` is present
    if Prefix::check() {
        println!(env!("out"));
        return ExitCode::from(0);
    }

    // Quit early if `--version` is present
    if Version::check() {
        println!("{}", *FLOX_VERSION);
        return ExitCode::from(0);
    }

    // Parse verbosity flags to affect help message/parse errors
    let verbosity = {
        let verbosity_parser = commands::verbosity();
        let other_parser = bpaf::any("_", Some::<String>).many();

        bpaf::construct!(verbosity_parser, other_parser)
            .map(|(v, _)| v)
            .to_options()
            .run_inner(Args::current_args())
            .unwrap_or_default()
    };

    init_logger(Some(verbosity));

    if let Err(err) = set_user() {
        message::error(err.to_string());
        return ExitCode::from(1);
    }

    let disable_metrics = config::Config::parse()
        .unwrap_or_default()
        .flox
        .disable_metrics;

    // Sentry client must be initialized before starting an async runtime or spawning threads
    // https://docs.sentry.io/platforms/rust/#async-main-function
    let _sentry_guard = (!disable_metrics).then(init_sentry);
    let _metrics_guard = Hub::global().try_guard().ok();

    // Pass down the verbosity level to all pkgdb calls
    unsafe {
        std::env::set_var("_FLOX_PKGDB_VERBOSITY", format!("{}", verbosity.to_i32()));
    }
    debug!("set _FLOX_PKGDB_VERBOSITY={}", verbosity.to_i32());

    // Run the argument parser
    //
    // Pass through Completion "failure"; In completion mode this needs to be printed as is
    // to work with the shell completion frontends
    //
    // Pass through Stdout failure; This represents `--help`
    // todo: just `run()` the parser? Unless we still need to control which std{err/out} to use
    let args = commands::flox_cli().run_inner(Args::current_args());

    if let Some(parse_err) = args.as_ref().err() {
        match parse_err {
            bpaf::ParseFailure::Stdout(m, _) => {
                print!("{m:80}");
                return ExitCode::from(0);
            },
            bpaf::ParseFailure::Stderr(m) => {
                message::error(format!("{m:80}"));
                return ExitCode::from(1);
            },
            bpaf::ParseFailure::Completion(c) => {
                print!("{c}");
                return ExitCode::from(0);
            },
        }
    }

    // Errors handled above
    let FloxCli(args) = args.unwrap();

    // Runtime creates our SIGINT/Ctrl-C handler, so care must be taken to drop it last
    let runtime = tokio::runtime::Runtime::new().unwrap();

    // Run flox. Print errors and exit with status 1 on failure
    let exit_code = match runtime.block_on(run(args)) {
        Ok(()) => ExitCode::from(0),

        Err(e) => {
            // Do not print any error
            if e.is::<Exit>() {
                return e.downcast_ref::<Exit>().unwrap().0;
            }

            let message = e
                .downcast_ref::<EnvironmentError>()
                .map(format_error)
                .or_else(|| {
                    e.downcast_ref::<ManagedEnvironmentError>()
                        .map(format_managed_error)
                })
                .or_else(|| {
                    e.downcast_ref::<RemoteEnvironmentError>()
                        .map(format_remote_error)
                })
                .or_else(|| {
                    e.downcast_ref::<EnvironmentSelectError>()
                        .map(format_environment_select_error)
                })
                .or_else(|| e.downcast_ref::<ServiceError>().map(format_service_error));

            if let Some(message) = message {
                message::error(message);
                return ExitCode::from(1);
            }

            // unknown errors are printed with an error trace
            let err_str = e
                .chain()
                .skip(1)
                .fold(e.to_string(), |acc, cause| format!("{acc}: {cause}"));

            message::error(err_str);

            ExitCode::from(1)
        },
    };

    drop(_metrics_guard);
    drop(_sentry_guard);

    exit_code

    // drop(runtime) should implicitly be last
}

/// Error to exit without printing an error message
#[derive(Debug)]
struct Exit(ExitCode);
impl Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <Self as Debug>::fmt(self, f)
    }
}
impl std::error::Error for Exit {}

/// Resets the `$USER`/`HOME` variables to match `euid`
///
/// Files written by `sudo flox ...` / `su`,
/// may write into your user's home (instead of /root).
/// Resetting `$USER`/`$HOME` will solve that.
fn set_user() -> Result<()> {
    {
        if let Some(effective_user) = nix::unistd::User::from_uid(nix::unistd::geteuid())? {
            // TODO: warn if variable is empty?
            if env::var("USER").unwrap_or_default() != effective_user.name {
                unsafe {
                    env::set_var("USER", effective_user.name);
                    env::set_var("HOME", effective_user.dir);
                }
            }
        } else {
            // Corporate LDAP environments rely on finding nss_ldap in
            // ld.so.cache *or* by configuring nscd to perform the LDAP
            // lookups instead. The Nix version of glibc has been modified
            // to disable ld.so.cache, so if nscd isn't configured to do
            // this then ldap access to the passwd map will not work.
            // Bottom line - don't abort if we cannot find a passwd
            // entry for the euid, but do warn because it's very
            // likely to cause problems at some point.
            warn!(
                "cannot determine effective uid - continuing as user '{}'",
                env::var("USER").context("Could not read '$USER' variable")?
            );
        };
        Ok(())
    }
}

/// Stores the PID of the executing shell as shells do not set $SHELL
/// when they are launched.
///
/// $FLOX_PARENT_PID is used when launching sub-shells to ensure users
/// keep running their chosen shell.
fn set_parent_process_id() {
    let ppid = nix::unistd::getppid();
    unsafe {
        env::set_var("FLOX_PARENT_PID", ppid.to_string());
    }
}

/// Avoid SIGPIPE from killing the process
///
/// SECURITY: This is safe because we are setting the signal handler to the default
fn reset_sigpipe() {
    unsafe {
        nix::libc::signal(nix::libc::SIGPIPE, nix::libc::SIG_DFL);
    }
}
