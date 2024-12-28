use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use anyhow::{bail, Context, Result};
use bpaf::{Bpaf, Parser};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::providers::upgrade_checks::{UpgradeInformation, UpgradeInformationGuard};
use flox_rust_sdk::utils::CommandExt;
use serde::de::DeserializeOwned;
use tracing::{debug, info_span, instrument};

use super::UninitializedEnvironment;
use crate::subcommand_metric;

/// By default check once a day
const DEFAULT_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;

#[derive(Bpaf, Clone)]
pub struct CheckForUpgrades {
    /// Skip checking for upgrade if checked less <timeout> seconds ago
    #[bpaf(long, argument("seconds"), fallback(DEFAULT_TIMEOUT_SECONDS))]
    check_timeout: u64,

    #[bpaf(external(parse_uninitialized_environment_json))]
    environment: UninitializedEnvironment,
}

fn parse_uninitialized_environment_json<T: DeserializeOwned>() -> impl Parser<T> {
    bpaf::positional("environment")
        .help("JSON representation of an uninitialized environment to be checked")
        .parse(|string: String| serde_json::from_str::<T>(&string))
}

#[derive(Debug, PartialEq)]
enum ExitBranch {
    LockTaken,
    AlreadyChecked,
    Checked,
}

impl CheckForUpgrades {
    #[instrument(name = "check-upgrade", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("check-upgrade");
        self.check_for_upgrades(&flox)?;
        Ok(())
    }

    fn check_for_upgrades(self, flox: &Flox) -> Result<ExitBranch> {
        let mut environment = self.environment.into_concrete_environment(flox)?;

        let upgrade_information =
            UpgradeInformationGuard::for_environment(&flox.cache_dir, environment.dot_flox_path())?;

        // Return if previous information
        // - exists &&
        // - targets the current lockfile &&
        // - has recently been fetched
        // Otherwise, run a dry-upgrade tof he environment and store the new information
        if let Some(info) = upgrade_information.info() {
            let environment_lockfile = environment.lockfile(flox)?;

            let is_information_for_current_lockfile =
                info.result.old_lockfile == Some(environment_lockfile);
            let is_checked_recently =
                info.last_checked.elapsed().unwrap().as_secs() <= self.check_timeout;

            if is_information_for_current_lockfile && is_checked_recently {
                debug!("Recently checked for upgrades. Skipping.");
                return Ok(ExitBranch::AlreadyChecked);
            }
        }

        let Ok(mut locked) = upgrade_information.lock_if_unlocked()? else {
            debug!("Lock already taken. Skipping.");
            return Ok(ExitBranch::LockTaken);
        };

        let result = info_span!("check-upgrade", progress = "Performing dry upgrade")
            .entered()
            .in_scope(|| environment.dry_upgrade(flox, &[]))?;

        let info = locked.info_mut();

        match info.as_mut() {
            // Check if the resolution didn't change,
            // only update the last checked timestamp
            Some(old_info) if old_info.result.new_lockfile == result.new_lockfile => {
                debug!("Resolution didn't change. Updating last checked timestamp.");
                old_info.last_checked = SystemTime::now()
            },
            Some(_) | None => {
                debug!(diff = ?result.diff(), "Upgrading information with new result.");

                let new_info = UpgradeInformation {
                    last_checked: SystemTime::now(),
                    result,
                };
                let _ = info.insert(new_info);
            },
        };

        locked.commit()?;

        Ok(ExitBranch::Checked)
    }
}

/// Spawn a new `flox check-for-upgrades` process in the background,
/// and redirect its logs to a log file.
///
/// The process will live on after the parent process exits.
/// When multiple processes are spawned, e.g. due to multiple successive activations,
/// one (usually the first) process will grab a lock on the upgrade information file,
/// and the others will exit early.
pub fn spawn_detached_check_for_upgrades_process(
    environment: &UninitializedEnvironment,
    self_executable: Option<PathBuf>,
    log_dir: &Path,
    check_timeout: Option<u64>,
) -> Result<()> {
    // Get the path to the current executable
    let self_executable = match self_executable {
        Some(path) => path,
        None if cfg!(test) => {
            bail!("self_executable must be provided in tests")
        },
        // SECURITY:
        // This is safe because the flox executable path
        // is at an immutable nix store path.
        None => std::env::current_exe()?,
    };

    let environment_json = serde_json::to_string(&environment)?;

    let mut command = Command::new(self_executable);
    command.arg("check-for-upgrades");
    command.arg(environment_json);

    if let Some(timeout) = check_timeout {
        command.arg("--check-timeout").arg(timeout.to_string());
    };

    command.arg("-vv"); // enable debug logging

    // Redirect logs to a file
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("now is after UNIX EPOCH")
        .as_secs();
    let upgrade_check_log = log_dir.join(format!("upgrade-check.{}.log", timestamp));

    debug!(
        log_file=?upgrade_check_log,
        "Logging upgrade check output to file, and redirecting std{{in,out}} to /dev/null"
    );

    fs::create_dir_all(log_dir)?;
    let file = File::create(upgrade_check_log)?;
    command.stderr(file);
    command.stdout(Stdio::null());
    command.stdin(Stdio::null());

    command.display();
    debug!(cmd=%command.display(), "Spawning check-for-upgrades process in background");

    // continue in the background
    let _child = command
        .spawn()
        .context("Failed to spawn 'check-for-upgrades' process")?;

    Ok(())
}
