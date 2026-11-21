use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use bpaf::{Bpaf, Parser};
use flox_core::log_file_format_upgrade_check;
use flox_manifest::lockfile::Lockfile;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{
    ConcreteEnvironment,
    Environment,
    EnvironmentError,
    UpgradeResult,
};
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

#[derive(Debug)]
enum ExitBranch {
    LockTaken,
    AlreadyChecked,
    Checked,
    /// The dry upgrade failed and the failure was recorded.
    CheckFailed(EnvironmentError),
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

        check_for_upgrades_and_remote_state(
            &flox,
            &mut environment,
            Duration::seconds(self.check_timeout),
        )
    }
}

/// Check for package upgrades and, unless the check was skipped, fetch the
/// environment's remote state.
///
/// The remote fetch is throttled by the upgrade check's cooldown,
/// so it runs whenever the check runs, including when the check fails.
/// A failed check's error is returned after the fetch, so it reaches the
/// check's log file; a fetch error in that case is only logged.
fn check_for_upgrades_and_remote_state(
    flox: &Flox,
    environment: &mut ConcreteEnvironment,
    timeout: Duration,
) -> Result<()> {
    match check_for_package_upgrades(flox, environment, timeout)? {
        ExitBranch::LockTaken | ExitBranch::AlreadyChecked => Ok(()),
        ExitBranch::Checked => Ok(update_remote_environment_state(flox, environment)?),
        ExitBranch::CheckFailed(check_error) => {
            if let Err(fetch_error) = update_remote_environment_state(flox, environment) {
                debug!(error = ?fetch_error, "Failed to fetch remote environment state");
            }
            Err(check_error.into())
        },
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

    let current_lockfile: Lockfile = environment.lockfile(flox)?.into();

    // When logged out, skip the catalog resolve entirely and record a no-op
    // result instead. old_lockfile == new_lockfile means diff() is empty,
    // so no false upgrade notification is shown. Recording last_checked here
    // throttles update_remote_environment_state on the same schedule as the
    // authenticated path, so logged-out users are not forced to fetch remote
    // state on every activation (DEV-324).
    let upgrade_result = if flox.auth_context.is_unauthenticated() {
        debug!("Not logged in; skipping package upgrade resolve.");
        Ok(UpgradeResult {
            old_lockfile: Some(current_lockfile.clone()),
            new_lockfile: current_lockfile.clone(),
            store_path: None,
        })
    } else {
        info_span!("check-upgrade", progress = "Performing dry upgrade")
            .entered()
            .in_scope(|| environment.dry_upgrade(flox, &[]))
    };

    // A failure is recorded too, so that a check that keeps failing, such as
    // a resolve the catalog refuses, reruns once per `timeout` rather than on
    // every activation.
    let (new_info, exit_branch) = match upgrade_result {
        Ok(upgrade_result) => (
            UpgradeInformation {
                last_checked: OffsetDateTime::now_utc(),
                upgrade_result,
                check_failed: false,
            },
            ExitBranch::Checked,
        ),
        Err(err) => (
            UpgradeInformation::failed_check(current_lockfile),
            ExitBranch::CheckFailed(err),
        ),
    };

    let _ = locked.info_mut().insert(new_info);

    locked.commit()?;

    Ok(exit_branch)
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

    use flox_core::data::environment_ref::EnvironmentOwner;
    use flox_rust_sdk::flox::test_helpers::{
        flox_instance,
        flox_instance_with_optional_floxhub,
        set_test_auth,
    };
    use flox_rust_sdk::models::environment::UpgradeResult;
    use flox_rust_sdk::models::environment::floxmeta_branch::BranchOrd;
    use flox_rust_sdk::models::environment::generations::GenerationsExt;
    use flox_rust_sdk::models::environment::managed_environment::ManagedEnvironment;
    use flox_rust_sdk::models::environment::managed_environment::test_helpers::mock_managed_environment_from_env_files;
    use flox_rust_sdk::models::environment::path_environment::PathEnvironment;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::{
        new_path_environment,
        new_path_environment_from_env_files,
    };
    use flox_rust_sdk::providers::catalog::test_helpers::catalog_replay_client;
    use flox_test_utils::GENERATED_DATA;
    use flox_test_utils::manifests::HELLO;
    use floxhub_client::FloxhubClient;
    use floxhub_client::client::test_helpers::client_config;
    use httpmock::{Mock, MockServer};

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
            check_failed: false,
        });
        locked.commit().unwrap();

        let exit_branch =
            check_for_package_upgrades(&flox, &mut environment.into(), Duration::MAX).unwrap();

        assert!(
            matches!(exit_branch, ExitBranch::AlreadyChecked),
            "{exit_branch:?}"
        );
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

        assert!(
            matches!(exit_branch, ExitBranch::LockTaken),
            "{exit_branch:?}"
        );
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

        // flox_instance() gives a logged-out instance; the dry_upgrade path
        // requires catalog access, so authenticate before running the check.
        set_test_auth(&mut flox, "test");

        // required to read the upgrade information after being moved in the following line.
        let cache_path = environment.cache_path().unwrap();

        // provide a mock response from the catalog client
        // in this case an older [sic] version of the hello package,
        // which should trigger an upgrade.
        flox.floxhub_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/old_hello.yaml")).await;

        let exit_branch =
            check_for_package_upgrades(&flox, &mut environment.into(), Duration::MIN).unwrap();

        assert!(
            matches!(exit_branch, ExitBranch::Checked),
            "{exit_branch:?}"
        );

        // assert that the upgrade information was stored
        let upgrade_information = UpgradeInformationGuard::read_in(cache_path).unwrap();

        assert!(upgrade_information.info().is_some());
        let info = upgrade_information.info().as_ref().unwrap();
        assert!(!info.check_failed);
        assert!(info.upgrade_result.old_lockfile.is_some());
        assert_ne!(
            &info.upgrade_result.new_lockfile,
            info.upgrade_result.old_lockfile.as_ref().unwrap()
        );
    }

    /// Refuse every resolve, as the catalog does for callers it does not accept.
    fn refuse_resolves(server: &MockServer) -> Mock<'_> {
        server.mock(|when, then| {
            when.method("POST").path("/api/v1/catalog/resolve");
            then.status(401)
                .json_body(serde_json::json!({"detail": "Log in to FloxHub"}));
        })
    }

    /// A failed check replaces the previous result and starts the cooldown,
    /// so activations neither report the earlier upgrades nor rerun the dry
    /// upgrade.
    #[cfg_attr(
        all(target_os = "macos", target_arch = "x86_64"),
        ignore = "catalog recordings don't cover x86_64-darwin"
    )]
    #[tokio::test(flavor = "multi_thread")]
    async fn failed_check_is_recorded_and_throttles() {
        let (mut flox, _tempdir) = flox_instance();

        let mut environment = new_path_environment(&flox, HELLO);
        flox.floxhub_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;
        environment.lockfile(&flox).unwrap();
        let cache_path = environment.cache_path().unwrap();
        let open = |flox: &Flox| -> ConcreteEnvironment {
            PathEnvironment::open(flox, environment.pointer.clone(), environment.path.clone())
                .unwrap()
                .into()
        };

        set_test_auth(&mut flox, "test");

        // A successful check that finds upgrades, for the failure to replace.
        flox.floxhub_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/old_hello.yaml")).await;
        let exit_branch =
            check_for_package_upgrades(&flox, &mut open(&flox), Duration::ZERO).unwrap();
        assert!(
            matches!(exit_branch, ExitBranch::Checked),
            "{exit_branch:?}"
        );
        let guard = UpgradeInformationGuard::read_in(&cache_path).unwrap();
        let upgrades_available = guard.info().clone().unwrap();
        assert!(!upgrades_available.upgrade_result.diff().is_empty());

        let server = MockServer::start_async().await;
        let resolve = refuse_resolves(&server);
        flox.floxhub_client = FloxhubClient::new(client_config(&server.base_url())).unwrap();

        let exit_branch =
            check_for_package_upgrades(&flox, &mut open(&flox), Duration::ZERO).unwrap();
        assert!(
            matches!(exit_branch, ExitBranch::CheckFailed(_)),
            "{exit_branch:?}"
        );
        resolve.assert_calls(1);

        let lockfile: Lockfile = open(&flox).lockfile(&flox).unwrap().into();
        let guard = UpgradeInformationGuard::read_in(&cache_path).unwrap();
        let info = guard.info().clone().unwrap();
        assert_eq!(info, UpgradeInformation {
            last_checked: info.last_checked,
            ..UpgradeInformation::failed_check(lockfile)
        });

        let exit_branch =
            check_for_package_upgrades(&flox, &mut open(&flox), Duration::MAX).unwrap();
        assert!(
            matches!(exit_branch, ExitBranch::AlreadyChecked),
            "{exit_branch:?}"
        );
        resolve.assert_calls(1);
    }

    /// The remote fetch shares the upgrade check's cooldown, so a failed check
    /// must still fetch, or a generation pushed from elsewhere goes unreported
    /// until the cooldown ends.
    #[tokio::test(flavor = "multi_thread")]
    async fn failed_check_still_fetches_remote_state() {
        let owner: EnvironmentOwner = "owner".parse().unwrap();
        let (mut flox, _tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let environment = mock_managed_environment_from_env_files(
            &flox,
            GENERATED_DATA.join("envs/hello"),
            owner,
        );

        // Push from a second machine. Its own clone of the owner's floxmeta
        // lives in its data dir, so this machine sees the push only by fetching.
        let this_machine_data_dir =
            std::mem::replace(&mut flox.data_dir, flox.temp_dir.join("other-machine-data"));
        let other_project = flox.temp_dir.join("other-machine-project");
        std::fs::create_dir_all(&other_project).unwrap();
        let mut other_machine_environment =
            ManagedEnvironment::open(&flox, environment.pointer().clone(), &other_project, None)
                .unwrap();
        other_machine_environment
            .edit(&flox, "version = 1\n".to_string())
            .unwrap();
        other_machine_environment.push(&flox, false).unwrap();
        flox.data_dir = this_machine_data_dir;
        assert_eq!(environment.compare_remote().unwrap(), BranchOrd::Equal);

        set_test_auth(&mut flox, "test");
        let server = MockServer::start_async().await;
        let resolve = refuse_resolves(&server);
        flox.floxhub_client = FloxhubClient::new(client_config(&server.base_url())).unwrap();

        let mut environment = ConcreteEnvironment::Managed(environment);
        let result = check_for_upgrades_and_remote_state(&flox, &mut environment, Duration::MAX);
        assert!(result.is_err(), "{result:?}");
        resolve.assert_calls(1);

        let ConcreteEnvironment::Managed(environment) = environment else {
            unreachable!()
        };
        assert_eq!(environment.compare_remote().unwrap(), BranchOrd::Behind);
    }

    /// When logged out, `check_for_package_upgrades` records a no-op
    /// `UpgradeInformation` (old_lockfile == new_lockfile, no store_path) so
    /// that the throttle fires on subsequent calls just as it would for an
    /// authenticated user — preventing `update_remote_environment_state` from
    /// running on every activation (DEV-324).
    #[test]
    fn logged_out_records_noop_check_and_throttles() {
        // flox_instance() sets AuthContext::new_from_token(None) → Auth0(None),
        // which is_unauthenticated() == true — no catalog replay client needed
        // because no resolve occurs.
        let (flox, _tempdir) = flox_instance();

        let env = new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        let cache_path = env.cache_path().unwrap();

        // Capture the path and pointer so we can re-open the same environment
        // for the throttle assertion below without needing Clone on PathEnvironment.
        let dot_flox_path = env.path.clone();
        let pointer = env.pointer.clone();

        // First call: should record a no-op check and return Checked.
        let exit_branch =
            check_for_package_upgrades(&flox, &mut env.into(), Duration::MAX).unwrap();
        assert!(
            matches!(exit_branch, ExitBranch::Checked),
            "{exit_branch:?}"
        );

        // Verify the written UpgradeInformation contains old == new (no false
        // upgrade notification) and no store_path (no build occurred).
        let guard = UpgradeInformationGuard::read_in(cache_path).unwrap();
        let info = guard
            .info()
            .as_ref()
            .expect("upgrade information was written");
        assert_eq!(
            info.upgrade_result.old_lockfile.as_ref(),
            Some(&info.upgrade_result.new_lockfile),
            "old_lockfile should equal new_lockfile — diff() must be empty"
        );
        assert!(
            info.upgrade_result.store_path.is_none(),
            "no build should have occurred"
        );

        // Re-open the same environment; second call must be throttled (AlreadyChecked),
        // proving update_remote_environment_state is no longer invoked on every
        // activation for logged-out users.
        let env2 = PathEnvironment::open(&flox, pointer, dot_flox_path).unwrap();
        let exit_branch2 =
            check_for_package_upgrades(&flox, &mut env2.into(), Duration::MAX).unwrap();
        assert!(
            matches!(exit_branch2, ExitBranch::AlreadyChecked),
            "second call should be throttled: {exit_branch2:?}"
        );
    }
}
