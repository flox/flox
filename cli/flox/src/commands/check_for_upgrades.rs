use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::SystemTime;

use anyhow::{bail, Context, Result};
use bpaf::{Bpaf, Parser};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::providers::catalog::{self, CatalogQoS};
use flox_rust_sdk::providers::upgrade_checks::{UpgradeInformation, UpgradeInformationGuard};
use flox_rust_sdk::utils::CommandExt;
use serde::de::DeserializeOwned;
use time::{Duration, OffsetDateTime};
use tracing::{debug, info_span, instrument};

use super::UninitializedEnvironment;
use crate::subcommand_metric;

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
        // to do som in a more scoped way and have the changes be reverted at some point [1].
        // For now, in this command we're modifying the `Flox` object only once
        // and for the rest of the command's (short) lifetime.
        // A possible future improvement was sketched out in the comment above [1].
        //
        // [1]: <https://github.com/flox/flox/pull/2658#discussion_r1932362747>
        if let catalog::Client::Catalog(ref mut catalog_client) = flox.catalog_client {
            catalog_client.update_config(|config| {
                let (qos_key, qos_value) = CatalogQoS::Background.as_header_pair();
                config.extra_headers.insert(qos_key, qos_value);
            });
        }

        self.check_for_upgrades(&flox)?;
        Ok(())
    }

    fn check_for_upgrades(self, flox: &Flox) -> Result<ExitBranch> {
        let mut environment = self.environment.into_concrete_environment(flox)?;

        let upgrade_information = UpgradeInformationGuard::read_in(environment.cache_path()?)?;

        // Return if previous information
        // - exists &&
        // - targets the current lockfile &&
        // - has recently been fetched
        // Otherwise, run a dry-upgrade tof he environment and store the new information
        if let Some(info) = upgrade_information.info() {
            let environment_lockfile = environment.lockfile(flox)?;

            let is_information_for_current_lockfile =
                info.result.old_lockfile == Some(environment_lockfile);
            let is_checked_recently = (OffsetDateTime::now_utc() - info.last_checked)
                < Duration::seconds(self.check_timeout);

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
                old_info.last_checked = OffsetDateTime::now_utc();
            },
            Some(_) | None => {
                debug!(diff = ?result.diff(), "Upgrading information with new result.");

                let new_info = UpgradeInformation {
                    last_checked: OffsetDateTime::now_utc(),
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
///
/// ## SAFETY:
///
/// [pre_exec](std::os::unix::process::CommandExt::pre_exec)
/// is unsafe because it runs in an environment atypical for Rust,
/// where many guarantees provided by the rust ownership model
/// do not necessarily hold.
/// It is strongly recommended to limit the scope to `pre_exec`.
/// Here we limit the scope of the `pre_exec` call to closing (duplicated) file descriptors
/// and detaching the process from the parent process group.
/// Closing file descriptors _before_ exec'ing is considerably safer
/// than closing them in the child process, which may have already opened its own unknown descriptors.
/// Likewise, detaching the process from the parent process group
/// applies only to the subprocess and only when spawning the process as a background process.
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
    let log_file = File::create(upgrade_check_log)?;
    let log_file_fd = log_file.as_raw_fd();
    command.stderr(log_file);
    command.stdout(Stdio::null());
    command.stdin(Stdio::null());

    let keep_fds = [log_file_fd];

    // Close all additional file descriptors except the log file
    // and detach the process from the parent process group.
    // See the SAFETY section above for more information on the safety of this operation.
    unsafe {
        use std::os::unix::process::CommandExt as _;
        command.pre_exec(move || {
            close_fds::CloseFdsBuilder::new()
                .keep_fds(&keep_fds)
                .cloexecfrom(3);

            // Detach the process from the parent process group
            // so that it wont receive signals from the parent
            nix::unistd::setsid()?;
            Ok(())
        });
    }

    command.display();
    debug!(cmd=%command.display(), "Spawning check-for-upgrades process in background");

    // continue in the background
    let _child = command
        .spawn()
        .context("Failed to spawn 'check-for-upgrades' process")?;

    Ok(())
}

#[cfg(test)]
mod tests {

    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment_from_env_files;
    use flox_rust_sdk::models::environment::{ConcreteEnvironment, UpgradeResult};
    use flox_rust_sdk::providers::catalog::{Client, MockClient, GENERATED_DATA};

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
            result: UpgradeResult {
                old_lockfile: Some(environment.lockfile(&flox).unwrap()),
                new_lockfile: environment.lockfile(&flox).unwrap(),
                store_path: None,
            },
        });
        locked.commit().unwrap();

        let serialized = UninitializedEnvironment::from_concrete_environment(
            &ConcreteEnvironment::Path(environment),
        )
        .unwrap();

        // Check for upgrades with a timeout of u64::MAX
        // to ensure that the fake upgrade information is always considered recent
        let command = CheckForUpgrades {
            check_timeout: i64::MAX,
            environment: serialized,
        };

        let exit_branch = command.check_for_upgrades(&flox).unwrap();

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

        let serialized = UninitializedEnvironment::from_concrete_environment(
            &ConcreteEnvironment::Path(environment),
        )
        .unwrap();

        let command = CheckForUpgrades {
            check_timeout: 0,
            environment: serialized,
        };

        let exit_branch = command.check_for_upgrades(&flox).unwrap();

        assert_eq!(exit_branch, ExitBranch::LockTaken);
    }

    #[test]
    fn checks_if_not_recently_checked() {
        let (mut flox, _tempdir) = flox_instance();

        let environment =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));

        // required to read the upgrade information after being moved in the following line.
        let cache_path = environment.cache_path().unwrap();

        let serialized = UninitializedEnvironment::from_concrete_environment(
            &ConcreteEnvironment::Path(environment),
        )
        .unwrap();

        let command = CheckForUpgrades {
            check_timeout: 0,
            environment: serialized,
        };

        // provide a mock response from the catalog client
        // in this case an older [sic] version of the hello package,
        // which should trigger an upgrade.
        flox.catalog_client = Client::Mock(
            MockClient::new(Some(GENERATED_DATA.join("resolve/old_hello.json"))).unwrap(),
        );

        let exit_branch = command.check_for_upgrades(&flox).unwrap();

        assert_eq!(exit_branch, ExitBranch::Checked);

        // assert that the upgrade information was stored
        let upgrade_information = UpgradeInformationGuard::read_in(cache_path).unwrap();

        assert!(upgrade_information.info().is_some());
        let info = upgrade_information.info().as_ref().unwrap();
        assert!(info.result.old_lockfile.is_some());
        assert_ne!(
            &info.result.new_lockfile,
            info.result.old_lockfile.as_ref().unwrap()
        );
    }
}
