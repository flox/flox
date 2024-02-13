use std::env;
use std::fmt::{Debug, Display};
use std::process::ExitCode;

use anyhow::{Context, Result};
use bpaf::{Args, Parser};
use commands::{FloxArgs, FloxCli, Prefix, Version};
use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::models::environment::managed_environment::ManagedEnvironmentError;
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironmentError;
use flox_rust_sdk::models::environment::{init_global_manifest, EnvironmentError2};
use log::{debug, warn};
use tracing;
use utils::init::init_logger;
use utils::message;

use crate::utils::errors::{format_error, format_managed_error, format_remote_error};

mod build;
mod commands;
mod config;
mod utils;

#[tracing::instrument]
async fn run(args: FloxArgs) -> Result<()> {
    init_logger(Some(args.verbosity.clone()));
    set_user()?;
    set_parent_process_id();
    let config = config::Config::parse()?;
    init_global_manifest(&config.flox.config_dir.join("global-manifest.toml"))?;
    args.handle(config).await?;
    Ok(())
}

fn main() -> ExitCode {
    let sentry_dns = std::env::var("SENTRY_DSN");
    let _sentry;

    if sentry_dns.is_ok() {
        _sentry = sentry::init((sentry_dns.unwrap(), sentry::ClientOptions {
            // https://docs.sentry.io/platforms/rust/configuration/releases/
            // TODO: should we maybe just use commit hash
            release: sentry::release_name!(),

            // https://docs.sentry.io/platforms/rust/configuration/environments/
            // TODO: need to set this to respective channel: nightly, ...
            // eg. environment: std::env::var("SENTRY_ENV").unwrap_or_default("development").into(),
            environment: Some("development".into()),

            // certain personally identifiable information (PII) are added
            send_default_pii: true,

            // Enable debug mode when needed
            debug: true,

            // To set a uniform sample rate
            // https://docs.sentry.io/platforms/rust/performance/
            traces_sample_rate: 1.0,

            ..Default::default()
        }));
    }

    // TODO: configure user
    //sentry::configure_scope(|scope| {
    //    scope.set_user(Some(sentry::User {
    //        email: Some("jane.doe@example.com".to_owned()),
    //        ..Default::default()
    //    }));
    //});

    let exit_code = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { amain().await });

    return exit_code;
}

//#[tokio::main]
async fn amain() -> ExitCode {
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
        println!("Version: {}", *FLOX_VERSION);
        return ExitCode::from(0);
    }

    // Parse verbosity flags to affect help message/parse errors
    let (verbosity, _debug) = {
        let verbosity_parser = commands::verbosity();
        let debug_parser = bpaf::long("debug").switch();
        let other_parser = bpaf::any("ANY", Some::<String>).many();

        bpaf::construct!(verbosity_parser, debug_parser, other_parser)
            .map(|(v, d, _)| (v, d))
            .to_options()
            .run_inner(Args::current_args())
            .unwrap_or_default()
    };
    // Pass down the verbosity level to all pkgdb calls
    std::env::set_var(
        "_FLOX_PKGDB_VERBOSITY",
        format!("{}", verbosity.to_pkgdb_verbosity_level()),
    );
    init_logger(Some(verbosity.clone()));
    debug!(
        "set _FLOX_PKGDB_VERBOSITY={}",
        verbosity.to_pkgdb_verbosity_level()
    );

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
                print!("{m}");
                return ExitCode::from(0);
            },
            bpaf::ParseFailure::Stderr(m) => {
                message::error(m);
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

    // Run flox. Print errors and exit with status 1 on failure
    let exit_code = match run(args).await {
        Ok(()) => ExitCode::from(0),

        Err(e) => {
            // todo: figure out how to deal with context, properly
            debug!("{:#}", e);

            // Do not print any error if caused by wrapped flox (sh)
            if e.is::<FloxShellErrorCode>() {
                return e.downcast_ref::<FloxShellErrorCode>().unwrap().0;
            }

            if let Some(e) = e.downcast_ref::<EnvironmentError2>() {
                message::error(format_error(e));
                return ExitCode::from(1);
            }

            if let Some(e) = e.downcast_ref::<ManagedEnvironmentError>() {
                message::error(format_managed_error(e));
                return ExitCode::from(1);
            }

            if let Some(e) = e.downcast_ref::<RemoteEnvironmentError>() {
                message::error(format_remote_error(e));
                return ExitCode::from(1);
            }

            let err_str = e
                .chain()
                .skip(1)
                .fold(e.to_string(), |acc, cause| format!("{}: {}", acc, cause));

            message::error(err_str);

            ExitCode::from(1)
        },
    };
    utils::init::flush_logger();
    exit_code
}

#[derive(Debug)]
struct FloxShellErrorCode(ExitCode);
impl Display for FloxShellErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <Self as Debug>::fmt(self, f)
    }
}
impl std::error::Error for FloxShellErrorCode {}

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
                env::set_var("USER", effective_user.name);
                env::set_var("HOME", effective_user.dir);
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
    env::set_var("FLOX_PARENT_PID", ppid.to_string());
}
