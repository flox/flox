use std::env;
use std::fmt::{Debug, Display};
use std::process::ExitCode;

use anyhow::{anyhow, Context, Result};
use bpaf::{Args, Parser};
use commands::{FloxArgs, FloxCli, Prefix, Version};
use flox_rust_sdk::models::environment::init_global_manifest;
use log::{error, warn};
use utils::init::init_logger;

mod build;
mod commands;
mod config;
mod utils;

async fn run(args: FloxArgs) -> Result<()> {
    init_logger(Some(args.verbosity.clone()), Some(args.debug));
    set_user()?;
    set_parent_process_id();
    let config = config::Config::parse()?;
    init_global_manifest(&config.flox.config_dir.join("global-manifest.toml"))?;
    args.handle(config).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    // initialize logger with "best guess" defaults
    // updating the logger conf is cheap, so we reinitialize whenever we get more information
    init_logger(None, None);

    // Quit early if `--prefix` is present
    if Prefix::check() {
        println!(env!("out"));
        return ExitCode::from(0);
    }

    // Quit early if `--version` is present
    if Version::check() {
        println!("Version: {}", env!("FLOX_VERSION"));
        return ExitCode::from(0);
    }

    // Parse verbosity flags to affect help message/parse errors
    let (verbosity, debug) = {
        let verbosity_parser = commands::verbosity();
        let debug_parser = bpaf::long("debug").switch();
        let other_parser = bpaf::any("ANY", Some::<String>).many();

        bpaf::construct!(verbosity_parser, debug_parser, other_parser)
            .map(|(v, d, _)| (v, d))
            .to_options()
            .run_inner(Args::current_args())
            .unwrap_or_default()
    };
    init_logger(Some(verbosity), Some(debug));

    // Run the argument parser
    //
    // Pass through Completion "failure"; In completion mode this needs to be printed as is
    // to work with the shell completion frontends
    //
    // Pass through Stdout failure; This represents `--help`
    let args = commands::flox_cli().run_inner(Args::current_args());

    if let Some(parse_err) = args.as_ref().err() {
        match parse_err {
            bpaf::ParseFailure::Stdout(m, _) => {
                print!("{m}");
                return ExitCode::from(0);
            },
            bpaf::ParseFailure::Stderr(m) => {
                error!("{m}");
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
            // Do not print any error if caused by wrapped flox (sh)
            if e.is::<FloxShellErrorCode>() {
                return e.downcast_ref::<FloxShellErrorCode>().unwrap().0;
            }

            error!("{:?}", anyhow!(e));

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
