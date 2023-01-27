use std::env;
use std::ffi::OsString;
use std::fmt::{Debug, Display};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{ExitCode, ExitStatus};

use anyhow::{anyhow, Context, Result};
use bpaf::Parser;
use commands::{FloxArgs, Prefix};
use flox_rust_sdk::environment::default_nix_subprocess_env;
use fslock::LockFile;
use itertools::Itertools;
use log::{debug, error, warn};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::process::Command;
use utils::init::init_logger;
use utils::metrics::{METRICS_LOCK_FILE_NAME, METRICS_UUID_FILE_NAME};

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
    // Quit early if `--prefix` is present
    if Prefix::check() {
        println!(env!("out"));
        return ExitCode::from(0);
    }

    init_logger(None, None);
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
    let result = run_in_flox(flox, &env::args_os().collect::<Vec<_>>()[1..]).await?;
    if !result.success() {
        Err(FloxShellErrorCode(ExitCode::from(
            result.code().unwrap_or_else(|| {
                result
                    .signal()
                    .expect("Process terminated by unknown means")
            }) as u8,
        )))?;
    }
    Ok(())
}

#[allow(clippy::bool_to_int_with_if)]
async fn sync_bash_metrics_consent(data_dir: &Path, cache_dir: &Path) -> Result<()> {
    let mut metrics_lock = LockFile::open(&cache_dir.join(METRICS_LOCK_FILE_NAME))?;
    tokio::task::spawn_blocking(move || metrics_lock.lock()).await??;

    let uuid_path = data_dir.join(METRICS_UUID_FILE_NAME);

    let metrics_enabled = match tokio::fs::File::open(&uuid_path).await {
        Ok(mut f) => {
            let mut uuid_str = String::new();
            f.read_to_string(&mut uuid_str).await?;
            !uuid_str.trim().is_empty()
        },
        Err(err) => match err.kind() {
            std::io::ErrorKind::NotFound => {
                // Consent hasn't been determined yet, so there is nothing to sync
                return Ok(());
            },
            _ => return Err(err.into()),
        },
    };

    let bash_flox_dirs =
        xdg::BaseDirectories::with_prefix("flox").context("Unable to find config dir")?;
    let bash_config_home = bash_flox_dirs.get_config_home();
    let bash_user_meta_path = bash_config_home.join("floxUserMeta.json");

    tokio::fs::create_dir_all(bash_config_home).await?;

    let mut bash_user_meta_file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(&bash_user_meta_path)
        .await
        .context(format!(
            "Unable to open (rw) legacy config file at {bash_user_meta_path:?}"
        ))?;

    let mut json: serde_json::Value = {
        let mut bash_user_meta_json = String::new();
        bash_user_meta_file
            .read_to_string(&mut bash_user_meta_json)
            .await
            .context("Unable to read bash flox meta")?;

        if bash_user_meta_json.is_empty() {
            json!({})
        } else {
            serde_json::from_str(&bash_user_meta_json).context("Unable to parse bash flox meta")?
        }
    };

    json["floxMetricsConsent"] = json!(if metrics_enabled { 1 } else { 0 });
    json["version"] = json!(1);

    let mut bash_user_meta_json = serde_json::to_string_pretty(&json)
        .context("Failed to serialize modified bash flox meta")?;
    bash_user_meta_json.push('\n');

    bash_user_meta_file.set_len(0).await?;
    bash_user_meta_file.rewind().await?;
    bash_user_meta_file
        .write_all(bash_user_meta_json.as_bytes())
        .await
        .context("Unable to write modified bash flox meta")?;

    Ok(())
}

pub async fn run_in_flox(
    flox: &Flox,
    args: &[impl AsRef<std::ffi::OsStr> + Debug],
) -> Result<ExitStatus> {
    debug!("Running in flox with arguments: {:?}", args);

    sync_bash_metrics_consent(&flox.data_dir, &flox.cache_dir).await?;

    let status = Command::new(FLOX_SH)
        .args(args)
        .envs(&default_nix_subprocess_env())
        .spawn()
        .expect("failed to spawn flox")
        .wait()
        .await?;

    Ok(status)
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
