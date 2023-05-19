use std::env;
use std::ffi::OsString;
use std::fmt::{Debug, Display};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{ExitCode, ExitStatus};

use anyhow::{anyhow, Context, Result};
use bpaf::Parser;
use commands::{BashPassthru, FloxArgs, Prefix};
use flox_rust_sdk::environment::default_nix_subprocess_env;
use itertools::Itertools;
use log::{debug, error, warn};
use tokio::process::Command;
use utils::init::init_logger;

mod build;
mod commands;
mod config;
mod utils;

use flox_rust_sdk::flox::{Flox, FLOX_SH};

async fn run(args: FloxArgs) -> Result<()> {
    init_logger(Some(args.verbosity.clone()), Some(args.debug));
    set_user()?;
    set_parent_process_id();
    args.handle(config::Config::parse()?).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    init_logger(None, None);

    // redirect to flox early if `--bash-passthru` is present
    if let Some(args) = BashPassthru::check() {
        return run_in_flox(None, &args)
            .await
            .unwrap_or_else(|run_error| {
                error!("Error: {:?}", run_error);
                1u8
            })
            .into();
    }

    // Quit early if `--prefix` is present
    if Prefix::check() {
        println!(env!("out"));
        return ExitCode::from(0);
    }

    let (verbosity, debug) = {
        let verbosity_parser = commands::verbosity();
        let debug_parser = bpaf::long("debug").switch();
        let other_parser = bpaf::any::<String>("ANY").many();

        bpaf::construct!(verbosity_parser, debug_parser, other_parser)
            .map(|(v, d, _)| (v, d))
            .to_options()
            .try_run()
            .unwrap_or_default()
    };
    init_logger(Some(verbosity), Some(debug));

    let args = commands::flox_args().try_run().map_err(|err| match err {
        bpaf::ParseFailure::Stdout(_) => err,
        bpaf::ParseFailure::Stderr(message) => {
            let mut help_args = env::args_os()
                .skip(1)
                .take_while(|arg| arg != "")
                .collect_vec();
            help_args.push(OsString::from("--help".to_string()));
            let failure = commands::flox_args()
                .run_inner(help_args[..].into())
                .err()
                .unwrap();
            match failure {
                bpaf::ParseFailure::Stdout(ref e) | bpaf::ParseFailure::Stderr(ref e) => {
                    bpaf::ParseFailure::Stderr(format!("{message}\n\n{e}"))
                },
            }
        },
    });

    if let Some(parse_err) = args.as_ref().err() {
        match parse_err {
            bpaf::ParseFailure::Stdout(m) => {
                print!("{m}");
                return ExitCode::from(0);
            },
            bpaf::ParseFailure::Stderr(m) => {
                error!("{m}");
                return ExitCode::from(1);
            },
        }
    }
    let args = args.unwrap();

    match run(args).await {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            // Do not print any error if caused by wrapped flox (sh)
            if e.is::<FloxShellErrorCode>() {
                return e.downcast_ref::<FloxShellErrorCode>().unwrap().0;
            }

            error!("{:?}", anyhow!(e));

            ExitCode::from(1)
        },
    }
}

#[derive(Debug)]
struct FloxShellErrorCode(ExitCode);
impl Display for FloxShellErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <Self as Debug>::fmt(self, f)
    }
}
impl std::error::Error for FloxShellErrorCode {}

pub async fn flox_forward(flox: &Flox) -> Result<()> {
    let result = run_in_flox(Some(flox), &env::args_os().collect::<Vec<_>>()[1..]).await?;
    if !ExitStatus::from_raw(result as i32).success() {
        Err(FloxShellErrorCode(ExitCode::from(result)))?
    }

    Ok(())
}

#[allow(clippy::bool_to_int_with_if)]

pub async fn run_in_flox(
    _flox: Option<&Flox>,
    args: &[impl AsRef<std::ffi::OsStr> + Debug],
) -> Result<u8> {
    debug!("Running in flox with arguments: {:?}", args);

    let flox_bin = std::env::var("FLOX_BASH_PREFIX")
        .map_or(Path::new(FLOX_SH).to_path_buf(), |prefix| {
            Path::new(&prefix).join("bin").join("flox")
        });

    let status = Command::new(&flox_bin)
        .args(args)
        .envs(&default_nix_subprocess_env())
        .spawn()
        .expect("failed to spawn flox")
        .wait()
        .await
        .context("fatal: failed executing {flox_bin}")?;

    let code = status.code().unwrap_or_else(|| {
        status
            .signal()
            .expect("Process terminated by unknown means")
    }) as u8;

    Ok(code)
}

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
