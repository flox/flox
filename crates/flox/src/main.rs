use std::env;
use std::ffi::OsString;
use std::fmt::{Debug, Display};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{ExitCode, ExitStatus};

use anyhow::{anyhow, Context, Result};
use bpaf::{Args, Doc, Parser};
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
    // initialize logger with "best guess" defaults
    // updating the logger conf is cheap, so we reinitialize whenever we get more information
    init_logger(None, None);

    // redirect to flox early if `--bash-passthru` is present
    if let Some(args) = BashPassthru::check() {
        let bash_command = run_in_flox(None, &args).await;

        match bash_command {
            // If calling the bash command caused an error, print the error and exit with status 1
            Err(run_error) => {
                error!("Error: {:?}", run_error);
                return ExitCode::from(1);
            },
            // If _calling the bash command was successful, exit with its exit code
            Ok(exit_code) => return ExitCode::from(exit_code),
        };
    }

    // Quit early if `--prefix` is present
    if Prefix::check() {
        println!(env!("out"));
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
    let args = commands::flox_args()
        .run_inner(Args::current_args())
        .map_err(|err| match err {
            bpaf::ParseFailure::Completion(c) => bpaf::ParseFailure::Completion(c),
            bpaf::ParseFailure::Stdout(_, _) => err,
            bpaf::ParseFailure::Stderr(mut message) => {
                let mut help_args = env::args_os()
                    .skip(1)
                    .take_while(|arg| arg != "")
                    .collect_vec();
                help_args.push(OsString::from("--help".to_string()));
                let failure = commands::flox_args()
                    .run_inner(&help_args[..])
                    .err()
                    .unwrap();
                match failure {
                    bpaf::ParseFailure::Stdout(ref e, _) | bpaf::ParseFailure::Stderr(ref e) => {
                        message.doc(&Doc::from("\n"));
                        message.doc(e);

                        bpaf::ParseFailure::Stderr(message)
                    },
                    _ => todo!(),
                }
            },
        });

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
    let args = args.unwrap();

    // Run flox. Print errors and exit with status 1 on failure
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
