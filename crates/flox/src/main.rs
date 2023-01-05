use self::config::{Feature, Impl};
use anyhow::{Context, Result};
use commands::FloxArgs;
use flox_rust_sdk::environment::default_nix_subprocess_env;
use fslock::LockFile;
use log::{debug, error, warn};
use serde_json::json;
use std::env;
use std::fmt::{Debug, Display};
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::{ExitCode, ExitStatus};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use utils::init::init_logger;
use utils::metrics::{METRICS_LOCK_FILE_NAME, METRICS_UUID_FILE_NAME};

use tokio::process::Command;

mod build;
mod commands;
mod config;
mod utils;

use flox_rust_sdk::flox::{Flox, FLOX_SH};

async fn run(args: FloxArgs) -> Result<()> {
    init_logger(args.verbosity.clone(), args.debug);
    set_user()?;
    args.handle(config::Config::parse()?).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = commands::flox_args().run();
    let debug = args.debug;

    match run(args).await {
        Ok(()) => ExitCode::from(0),
        Err(e) => {
            // Do not print any error if caused by wrapped flox (sh)
            if e.is::<FloxShellErrorCode>() {
                return e.downcast_ref::<FloxShellErrorCode>().unwrap().0;
            }
            if debug {
                error!("{:#?}", e);
            } else {
                error!("{}", e);
            }
            ExitCode::from(1)
        }
    }
}

pub fn should_flox_forward(f: Feature) -> Result<bool> {
    if f.implementation()? == Impl::Bash {
        let env_name = format!(
            "FLOX_PREVIEW_FEATURES_{}",
            serde_variant::to_variant_name(&f)?.to_uppercase()
        );
        debug!("`{env_name}` unset or not \"rust\", falling back to legacy flox");
        Ok(true)
    } else {
        Ok(false)
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

async fn sync_bash_metrics_consent(data_dir: &Path, cache_dir: &Path) -> Result<()> {
    let mut metrics_lock = LockFile::open(&cache_dir.join(METRICS_LOCK_FILE_NAME))?;
    tokio::task::spawn_blocking(move || metrics_lock.lock()).await??;

    let uuid_path = data_dir.join(METRICS_UUID_FILE_NAME);

    let metrics_enabled = match tokio::fs::File::open(&uuid_path).await {
        Ok(mut f) => {
            let mut uuid_str = String::new();
            f.read_to_string(&mut uuid_str).await?;
            !uuid_str.trim().is_empty()
        }
        Err(err) => match err.kind() {
            std::io::ErrorKind::NotFound => {
                // Consent hasn't been determined yet, so there is nothing to sync
                return Ok(());
            }
            _ => return Err(err.into()),
        },
    };

    let bash_config_home = dirs::config_dir().context("Unable to find config dir")?;
    let bash_user_meta_path = bash_config_home.join("flox").join("floxUserMeta.json");

    let mut bash_user_meta_file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&bash_user_meta_path)
        .await
        .context("Unable to open bash flox meta")?;

    let mut json: serde_json::Value = {
        let mut bash_user_meta_json = String::new();
        bash_user_meta_file
            .read_to_string(&mut bash_user_meta_json)
            .await
            .context("Unable to read bash flox meta")?;

        serde_json::from_str(&bash_user_meta_json).context("Unable to parse bash flox meta")?
    };

    json["floxMetricsConsent"] = json!(if metrics_enabled { 1 } else { 0 });

    let bash_user_meta_json = serde_json::to_string_pretty(&json)
        .context("Failed to serialize modified bash flox meta")?;

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
            if env::var("USER")? != effective_user.name {
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
                env::var("USER")?
            );
        };
        Ok(())
    }
}
