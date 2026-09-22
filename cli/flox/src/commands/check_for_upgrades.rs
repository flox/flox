use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use bpaf::{Bpaf, Parser};
use flox_core::log_file_format_upgrade_check;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment, EnvironmentError};
use flox_rust_sdk::providers::catalog::CatalogQoS;
use flox_rust_sdk::providers::upgrade_checks::{UpgradeInformation, UpgradeInformationGuard};
use serde::de::DeserializeOwned;
use time::{Duration, OffsetDateTime};
use tracing::{debug, info_span, instrument};

use super::UninitializedEnvironment;
use crate::subcommand_metric;
use crate::utils::detached::{DetachedCommand, LogFile};

/// By default check once a day
const DEFAULT_TIMEOUT_SECONDS: i64 = 24 * 60 * 60;

#[derive(Bpaf, Clone)]
pub struct CheckForUpgrades {
    /// Skip checking for upgrade if checked less <timeout> seconds ago
    #[bpaf(long, argument("seconds"), fallback(DEFAULT_TIMEOUT_SECONDS))]
    check_timeout: i64,

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
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        subcommand_metric!("check-upgrade");

        // For catalog requests made by this command, set the QoS to background.
        // Eventually we might want to prioritize these requests differently,
        // since they are not as time-sensitive as the ones actively made by the user.
        //
        // @billlevine brought up that if we start mutating the catalog client
        // or `Flox` object in multiple places, it would be preferable
        // to do some in a more scoped way and have the changes be reverted at some point [1].
        // For now, in this command we're modifying the `Flox` object only once
        // and for the rest of the command's (short) lifetime.
        // A possible future improvement was sketched out in the comment above [1].
        //
        // [1]: <https://github.com/flox/flox/pull/2658#discussion_r1932362747>
        // Update the shared client's extra headers to set background QoS.
        flox.floxhub_client.update_config(|config| {
            let (qos_key, qos_value) = CatalogQoS::Background.as_header_pair();
            config.extra_headers.insert(qos_key, qos_value);
        })?;

        let mut environment = self.environment.into_concrete_environment(&flox, None)?;
        let check_exit_branch = check_for_package_upgrades(
            &flox,
            &mut environment,
            Duration::seconds(self.check_timeout),
        )?;
        match check_exit_branch {
            ExitBranch::Checked => update_remote_environment_state(&flox, &environment)?,
            // `check_exit_branch` determined,
            // that we are already concurrently checking for updates (LockTaken)
            // or we have `AlreadyChecked` for updates recently (within self.check_timeout)
            // and there have been no local changes (to the lockfile).
            //
            // Use either case, use this to throttle environment fetches.
            ExitBranch::LockTaken | ExitBranch::AlreadyChecked => {},
        }

        Ok(())
    }
}

fn check_for_package_upgrades(
    flox: &Flox,
    environment: &mut ConcreteEnvironment,
    timeout: Duration,
) -> Result<ExitBranch> {
    let upgrade_information = UpgradeInformationGuard::read_in(environment.cache_path()?)?;

    // Return if previous information
    // - exists &&
    // - targets the current lockfile &&
    // - has recently been fetched
    // Otherwise, run a dry-upgrade of the environment and store the new information
    if let Some(info) = upgrade_information.info() {
        let environment_lockfile = environment.lockfile(flox)?.into();

        let is_information_for_current_lockfile =
            info.upgrade_result.old_lockfile == Some(environment_lockfile);
        let is_checked_recently = (OffsetDateTime::now_utc() - info.last_checked) < timeout;

        if is_information_for_current_lockfile && is_checked_recently {
            debug!("Recently checked for upgrades. Skipping.");
            return Ok(ExitBranch::AlreadyChecked);
        }
    }

    let Ok(mut locked) = upgrade_information.lock_if_unlocked()? else {
        debug!("Lock already taken. Skipping.");
        return Ok(ExitBranch::LockTaken);
    };

    let upgrade_result = info_span!("check-upgrade", progress = "Performing dry upgrade")
        .entered()
        .in_scope(|| environment.dry_upgrade(flox, &[]))?;

    let new_info = UpgradeInformation {
        last_checked: OffsetDateTime::now_utc(),
        upgrade_result,
    };

    let _ = locked.info_mut().insert(new_info);

    locked.commit()?;

    Ok(ExitBranch::Checked)
}

/// Fetch remote state for FloxHub environments,
/// so remote updates are visible and can be picked up by activate messaging.
fn update_remote_environment_state(
    flox: &Flox,
    environment: &ConcreteEnvironment,
) -> Result<(), EnvironmentError> {
    match environment {
        ConcreteEnvironment::Path(_) => Ok(()),
        ConcreteEnvironment::Managed(managed_environment) => {
            Ok(managed_environment.fetch_remote_state(flox)?)
        },
        ConcreteEnvironment::Remote(remote_environment) => {
            Ok(remote_environment.fetch_remote_state(flox)?)
        },
    }
}

/// Spawn a detached `flox check-for-upgrades` process in the background.
///
/// The process outlives the parent. When several are spawned by successive
/// activations, one grabs the upgrade-information file lock and the rest exit
/// early.
///
/// The process-group detach, fd close, stdio redirection, and env propagation
/// are the shared mechanism in [`DetachedCommand`]; this function only builds
/// the argument list and the per-invocation log name. The log name embeds a
/// timestamp so the activations executive's `gc_logs_per_process` keeps the
/// last N and prunes the rest.
pub fn spawn_detached_check_for_upgrades_process(
    environment: &UninitializedEnvironment,
    self_executable: Option<PathBuf>,
    log_dir: &Path,
    check_timeout: Option<u64>,
) -> Result<()> {
    let environment_json = serde_json::to_string(&environment)?;

    let mut args = vec!["check-for-upgrades".to_string(), environment_json];
    if let Some(timeout) = check_timeout {
        args.push("--check-timeout".to_string());
        args.push(timeout.to_string());
    }
    args.push("-vv".to_string()); // enable debug logging

    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .expect("now is after UNIX EPOCH")
        .as_secs();
    let log_name = log_file_format_upgrade_check(timestamp);

    DetachedCommand {
        args: &args,
        log_file: LogFile::PerInvocation(log_name),
        log_dir,
    }
    .spawn(self_executable)
    .context("Failed to spawn 'check-for-upgrades' process")
}

#[cfg(test)]
mod tests {

    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::UpgradeResult;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::{
        new_path_environment,
        new_path_environment_from_env_files,
    };
    use flox_rust_sdk::providers::catalog::test_helpers::catalog_replay_client;
    use flox_test_utils::GENERATED_DATA;
    use flox_test_utils::manifests::HELLO;

    use super::*;

    #[test]
    fn skips_if_recently_checked() {
        let (flox, _tempdir) = flox_instance();

        let mut environment =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));

        let upgrade_information =
            UpgradeInformationGuard::read_in(environment.cache_path().unwrap()).unwrap();

        // Create a fake upgrade information based on the current lockfile
        // and mark it as checked recently (now)
        let mut locked = upgrade_information.lock_if_unlocked().unwrap().unwrap();
        let _ = locked.info_mut().insert(UpgradeInformation {
            last_checked: OffsetDateTime::now_utc(),
            upgrade_result: UpgradeResult {
                old_lockfile: Some(environment.lockfile(&flox).unwrap().into()),
                new_lockfile: environment.lockfile(&flox).unwrap().into(),
                store_path: None,
            },
        });
        locked.commit().unwrap();

        let exit_branch =
            check_for_package_upgrades(&flox, &mut environment.into(), Duration::MAX).unwrap();

        assert_eq!(exit_branch, ExitBranch::AlreadyChecked);
    }

    #[test]
    fn skips_if_lock_taken() {
        let (flox, _tempdir) = flox_instance();

        let environment =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));

        let upgrade_information =
            UpgradeInformationGuard::read_in(environment.cache_path().unwrap()).unwrap();

        // Simulate a lock being taken by another process (i.e. `_locked` is not dropped)
        // A separate test in the SDK checks that `lock_if_unlocked` does not block.
        let _locked = upgrade_information.lock_if_unlocked().unwrap().unwrap();

        let exit_branch =
            check_for_package_upgrades(&flox, &mut environment.into(), Duration::MIN).unwrap();

        assert_eq!(exit_branch, ExitBranch::LockTaken);
    }

    #[cfg_attr(
        all(target_os = "macos", target_arch = "x86_64"),
        ignore = "catalog recordings don't cover x86_64-darwin"
    )]
    #[tokio::test(flavor = "multi_thread")]
    async fn checks_if_not_recently_checked() {
        let (mut flox, _tempdir) = flox_instance();

        // Build the env from a pinned manifest and lock it from the recording
        // instead of the generated env fixture (whose manifest doesn't pin
        // systems).
        let mut environment = new_path_environment(&flox, HELLO);
        flox.floxhub_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;
        environment.lockfile(&flox).unwrap();

        // required to read the upgrade information after being moved in the following line.
        let cache_path = environment.cache_path().unwrap();

        // provide a mock response from the catalog client
        // in this case an older [sic] version of the hello package,
        // which should trigger an upgrade.
        flox.floxhub_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/old_hello.yaml")).await;

        let exit_branch =
            check_for_package_upgrades(&flox, &mut environment.into(), Duration::MIN).unwrap();

        assert_eq!(exit_branch, ExitBranch::Checked);

        // assert that the upgrade information was stored
        let upgrade_information = UpgradeInformationGuard::read_in(cache_path).unwrap();

        assert!(upgrade_information.info().is_some());
        let info = upgrade_information.info().as_ref().unwrap();
        assert!(info.upgrade_result.old_lockfile.is_some());
        assert_ne!(
            &info.upgrade_result.new_lockfile,
            info.upgrade_result.old_lockfile.as_ref().unwrap()
        );
    }
}
