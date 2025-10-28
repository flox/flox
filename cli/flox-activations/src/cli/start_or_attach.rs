use std::fmt::Display;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Args;
use flox_core::activations::{self, Activations};
use fslock::LockFile;
use indoc::indoc;
use log::debug;
use time::{Duration, OffsetDateTime};

use crate::Error;

#[derive(Debug, Args)]
pub struct StartOrAttachArgs {
    #[arg(help = "The PID of the shell registering interest in the activation.")]
    #[arg(short, long, value_name = "PID")]
    pub pid: i32,
    #[arg(help = "The path to the activation symlink for the environment.")]
    #[arg(short, long, value_name = "PATH")]
    pub flox_env: PathBuf,
    #[arg(help = "The store path of the rendered environment for this activation.")]
    #[arg(short, long, value_name = "PATH")]
    pub store_path: String,
    /// The path to the runtime directory keeping activation data.
    #[arg(long, value_name = "PATH")]
    pub runtime_dir: PathBuf,
}

impl StartOrAttachArgs {
    // Returns activation_id for use in tests
    pub fn handle(self) -> Result<(bool, PathBuf, String), anyhow::Error> {
        let mut retries = 3;

        loop {
            let result = self.handle_inner(&self.runtime_dir, attach, start);

            let Err(err) = result else {
                let (attach, activation_state_dir, activation_id) = result?;
                return Ok((attach, activation_state_dir, activation_id));
            };

            if let Some(restartable_failure) = err.downcast_ref::<RestartableFailure>() {
                debug!("{restartable_failure}");
                retries -= 1;
                if retries == 0 {
                    return Err(err);
                }
                debug!("Retrying ...");
                continue;
            }

            return Err(err);
        }
    }

    // Returns activation_id for use in tests
    pub fn handle_inner(
        &self,
        runtime_dir: &Path,
        attach_fn: impl FnOnce(&Path, LockFile, &str, i32) -> Result<(), Error>,
        start_fn: impl FnOnce(
            Activations,
            PathBuf,
            &Path,
            &PathBuf,
            fslock::LockFile,
            &str,
            i32,
        ) -> Result<String, Error>,
    ) -> Result<(bool, PathBuf, String), Error> {
        let activations_json_path = activations::activations_json_path(runtime_dir, &self.flox_env);

        debug!("Reading activations from {:?}", activations_json_path);
        let (activations, lock) = activations::read_activations_json(&activations_json_path)?;
        let mut activations = activations
            .map(|a| a.check_version())
            .transpose()?
            .unwrap_or_default();

        // Registry logic following the 4-box pattern from activate-architecture-refactor.mmd:
        // n111: exists? - Check if activation exists for store_path
        // n112: active and env up to date? - Prune dead PIDs, check if still active
        // n113: ready to attach? - Check if ready
        // n114: do_start() - Start new activation

        // n111: Check if activation exists for this store_path
        let (activation_id, attaching) = match activations.activation_for_store_path(&self.store_path)
        {
            Some(activation) => {
                let activation_id = activation.id();
                debug!(
                    "Activation {} exists for store_path {}",
                    activation_id, self.store_path
                );

                // n112: Check if activation is "active and env up to date"
                // This means: prune dead PIDs and check if any PIDs remain
                let activation = activations
                    .activation_for_store_path_mut(&self.store_path)
                    .expect("activation disappeared");

                let pids_removed = activation.remove_terminated_pids();
                if pids_removed {
                    debug!("Removed terminated PIDs from activation {}", activation_id);
                }

                // If no PIDs remain after pruning, the activation is not active
                if activation.attached_pids().is_empty() {
                    debug!(
                        "Activation {} has no active PIDs, will start new activation",
                        activation_id
                    );
                    // Remove the dead activation
                    activations.remove_activation(&activation_id);
                    // Write the pruned activations back
                    activations::write_activations_json(&activations, &activations_json_path, lock)?;

                    // n114: Start new activation (acquire new lock)
                    let (_, lock) = activations::read_activations_json(&activations_json_path)?;
                    let id = start_fn(
                        activations,
                        activations_json_path,
                        runtime_dir,
                        &self.flox_env,
                        lock,
                        &self.store_path,
                        self.pid,
                    )?;
                    (id, false)
                } else {
                    // Activation is active, write back the pruned activations
                    // This consumes the lock, so we need to acquire a new one for attach_fn
                    activations::write_activations_json(&activations, &activations_json_path, lock)?;

                    // n113: Ready to attach? (handled in attach_fn)
                    // Acquire new lock for attach operation (attach_fn will drop it immediately)
                    let (_, lock) = activations::read_activations_json(&activations_json_path)?;
                    attach_fn(&activations_json_path, lock, &self.store_path, self.pid)?;
                    (activation_id, true)
                }
            },
            // n111: No activation exists
            None => {
                debug!("No activation exists for store_path {}", self.store_path);
                // n114: Start new activation
                let id = start_fn(
                    activations,
                    activations_json_path,
                    runtime_dir,
                    &self.flox_env,
                    lock,
                    &self.store_path,
                    self.pid,
                )?;
                (id, false)
            },
        };

        let activation_state_dir =
            activations::activation_state_dir_path(runtime_dir, &self.flox_env, &activation_id)?;

        Ok((attaching, activation_state_dir, activation_id))
    }
}

fn attach(
    activations_json_path: &Path,
    lock: fslock::LockFile,
    store_path: &str,
    pid: i32,
) -> Result<(), Error> {
    // It doesn't really make sense to drop the lock when first attempting to
    // attach since we're about to immediately re-acquire it.
    // But wait_for_activation_ready_and_attach_pid has to poll for a starting
    // activation to complete,
    // so it will have to drop the lock each time it sleeps.
    // It's simpler to make it acquire the lock every time rather than trying to
    // special case the first iteration.
    drop(lock);

    let attach_expiration = OffsetDateTime::now_utc() + Duration::seconds(10);
    wait_for_activation_ready_and_optionally_attach_pid(
        activations_json_path,
        store_path,
        attach_expiration,
        Some(pid),
    )?;
    Ok(())
}

/// Starts a new activation by creating a new activation
/// (with ready == `false` and attached pids == [`pid`]),
/// creating a state dir for the activation, and updating
/// the `activations.json` file on disk.
fn start(
    mut activations: Activations,
    activations_json_path: PathBuf,
    runtime_dir: &Path,
    flox_env: &PathBuf,
    lock: fslock::LockFile,
    store_path: &str,
    pid: i32,
) -> Result<String, anyhow::Error> {
    let activation_id = activations.create_activation(store_path, pid)?.id();
    // The activation script will assume this directory exists
    fs::create_dir_all(activations::activation_state_dir_path(
        runtime_dir,
        flox_env,
        &activation_id,
    )?)?;

    activations::write_activations_json(&activations, &activations_json_path, lock)?;
    Ok(activation_id)
}

/// Wait for the activation with the given ID to become ready.
/// I.e if an activation is being started already, wait for it to become ready,
/// then _attach_ the given PID to it.
/// If the activation is not ready within the given timeout,
/// exit with an error.
/// If the activation startup process fails, exit with an error.
/// In either case, the activation can likely just be restarted.
pub fn wait_for_activation_ready_and_optionally_attach_pid(
    activations_json_path: &Path,
    store_path: &str,
    attach_expiration: OffsetDateTime,
    attaching_pid: Option<i32>,
) -> Result<(), anyhow::Error> {
    loop {
        let ready = check_for_activation_ready_and_optionally_attach_pid(
            activations_json_path,
            store_path,
            attaching_pid,
            attach_expiration,
            OffsetDateTime::now_utc(),
        )?;

        if ready {
            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(())
}

fn check_for_activation_ready_and_optionally_attach_pid(
    activations_json_path: &Path,
    store_path: &str,
    attaching_pid: Option<i32>,
    attach_expiration: OffsetDateTime,
    now: OffsetDateTime,
) -> Result<bool, anyhow::Error> {
    let (activations, lock) = activations::read_activations_json(activations_json_path)?;
    let Some(activations) = activations else {
        anyhow::bail!("Expected an existing activations.json file");
    };

    let mut activations = activations.check_version()?;

    let activation = activations
        .activation_for_store_path_mut(store_path)
        .context("Prior activation of the environment completed before it could be attached to.")
        .map_err(RestartableFailure)?;

    if activation.ready() {
        if let Some(attaching_pid) = attaching_pid {
            activation.attach_pid(attaching_pid, None);
            activations::write_activations_json(&activations, activations_json_path, lock)?;
        };
        return Ok(true);
    }

    if !activation.startup_process_running() {
        // Remove the deadlock so that a retry can proceed with a new start.
        let id = activation.id();
        activations.remove_activation(id);
        activations::write_activations_json(&activations, activations_json_path, lock)?;
        return Err(RestartableFailure(anyhow::anyhow!(indoc! {"
            Prior activation of the environment failed to start, or completed.
        "}))
        .into());
    }

    if now > attach_expiration {
        anyhow::bail!(indoc! {"
            Timed out waiting for a prior activation of the environment
            to complete startup hooks.

            Try again after the previous activation of the environment has completed.
        "});
    }
    Ok(false)
}

#[derive(Debug)]
struct RestartableFailure(anyhow::Error);
impl std::error::Error for RestartableFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}
impl Display for RestartableFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::test::{read_activations, write_activations};

    #[test]
    fn attach_if_activation_exists() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let flox_env = PathBuf::from("/path/to/floxenv");
        let store_path = "/store/path";

        // The PID of the current process, guaranteed to be running
        let pid = nix::unistd::getpid().as_raw();

        let id = write_activations(&runtime_dir, &flox_env, |activations| {
            activations.create_activation(store_path, pid).unwrap().id()
        });

        let args = StartOrAttachArgs {
            pid,
            flox_env: flox_env.clone(),
            store_path: store_path.to_string(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };

        let (attaching, activation_state_dir, activation_id) = args
            .handle_inner(
                runtime_dir.path(),
                |_, _, _, _| Ok(()),
                |_, _, _, _, _, _, _| panic!("start should not be called"),
            )
            .expect("handle_inner should succeed");

        assert!(attaching, "should be attaching");
        assert_eq!(
            activation_state_dir,
            activations::activation_state_dir_path(&runtime_dir, flox_env, &id).unwrap()
        );
        assert_eq!(activation_id, id);
    }

    #[test]
    fn start_if_activation_does_not_exist() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let flox_env = PathBuf::from("/path/to/floxenv");
        let store_path = "/store/path";

        // The PID of the current process, guaranteed to be running
        let pid = nix::unistd::getpid().as_raw();

        write_activations(&runtime_dir, &flox_env, |_| {});

        let args = StartOrAttachArgs {
            pid,
            flox_env: flox_env.clone(),
            store_path: store_path.to_string(),
            runtime_dir: runtime_dir.path().to_path_buf(),
        };

        let id = "1".to_string();
        let (attaching, activation_state_dir, activation_id) = args
            .handle_inner(
                runtime_dir.path(),
                |_, _, _, _| panic!("attach should not be called"),
                |_, _, _, _, _, _, _| Ok(id.clone()),
            )
            .expect("handle_inner should succeed");

        assert!(!attaching, "should not be attaching");
        assert_eq!(
            activation_state_dir,
            activations::activation_state_dir_path(&runtime_dir, flox_env, &id).unwrap()
        );
        assert_eq!(activation_id, id);
    }

    #[test]
    fn check_for_activation_not_ready() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let flox_env = PathBuf::from("/path/to/floxenv");
        let store_path = "/store/path";

        // The PID of the current process, guaranteed to be running
        let pid = nix::unistd::getpid().as_raw();
        let attaching_pid = 5678;

        let now = OffsetDateTime::now_utc();
        let attach_expiration = now + Duration::seconds(10);

        let _ = write_activations(&runtime_dir, &flox_env, |activations| {
            activations.create_activation(store_path, pid).unwrap().id()
        });

        let activations_json_path = activations::activations_json_path(&runtime_dir, &flox_env);

        let ready = check_for_activation_ready_and_optionally_attach_pid(
            &activations_json_path,
            store_path,
            Some(attaching_pid),
            attach_expiration,
            now,
        )
        .expect("check_for_activation_ready_and_attach_pid should succeed with not ready");
        assert!(!ready);
    }

    /// When the activation is ready, the attaching PID should be attached to the activation,
    /// and the return value should be true.
    #[test]
    fn check_for_activation_ready() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let flox_env = PathBuf::from("/path/to/floxenv");
        let store_path = "/store/path";

        // The PID of the current process, guaranteed to be running
        let pid = nix::unistd::getpid().as_raw();
        let attaching_pid = 5678;

        let now = OffsetDateTime::now_utc();
        let attach_expiration = now + Duration::seconds(10);

        let _ = write_activations(&runtime_dir, &flox_env, |activations| {
            let activation = activations.create_activation(store_path, pid).unwrap();
            activation.set_ready();
            activation.id()
        });

        let activations_json_path = activations::activations_json_path(&runtime_dir, &flox_env);

        let ready = check_for_activation_ready_and_optionally_attach_pid(
            &activations_json_path,
            store_path,
            Some(attaching_pid),
            attach_expiration,
            now,
        )
        .expect("check_for_activation_ready_and_attach_pid should succeed with not ready");

        assert!(ready, "Activation should be ready");

        read_activations(runtime_dir, flox_env, |activations| {
            let activation = activations.activation_for_store_path(store_path).unwrap();
            let attached_pid = activation
                .attached_pids()
                .iter()
                .find(|pid| pid.pid == attaching_pid);
            assert!(attached_pid.is_some(), "PID should be attached");
        });
    }

    fn make_unused_pid() -> i32 {
        let pid = nix::unistd::getpid().as_raw();
        pid + 100
    }

    #[test]
    fn check_for_activation_fails_if_starting_process_is_dead() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let flox_env = PathBuf::from("/path/to/floxenv");
        let store_path = "/store/path";

        let pid = make_unused_pid();
        let attaching_pid = 5678;

        let now = OffsetDateTime::now_utc();
        let attach_expiration = now + Duration::seconds(10);

        let _ = write_activations(&runtime_dir, &flox_env, |activations| {
            let activation = activations.create_activation(store_path, pid).unwrap();
            activation.id()
        });

        let activations_json_path = activations::activations_json_path(&runtime_dir, &flox_env);

        let result = check_for_activation_ready_and_optionally_attach_pid(
            &activations_json_path,
            store_path,
            Some(attaching_pid),
            attach_expiration,
            now,
        );

        assert!(
            result.is_err(),
            "check_for_activation_ready_and_attach_pid should fail"
        );
        assert!(
            result
                .unwrap_err()
                .downcast_ref::<RestartableFailure>()
                .is_some(),
            "should return RestartableFailure"
        );
        read_activations(&runtime_dir, &flox_env, |activations| {
            assert!(
                activations.is_empty(),
                "activations should be empty, got: {:?}",
                activations
            );
        });
    }

    #[test]
    fn check_for_activation_fails_if_starting_process_timeout_expires() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let flox_env = PathBuf::from("/path/to/floxenv");
        let store_path = "/store/path";

        // The PID of the current process, guaranteed to be running
        let pid = nix::unistd::getpid().as_raw();
        let attaching_pid = 5678;

        let now = OffsetDateTime::now_utc();
        // Set the expiration to be in the past
        let attach_expiration = now - Duration::seconds(10);

        let _ = write_activations(&runtime_dir, &flox_env, |activations| {
            let activation = activations.create_activation(store_path, pid).unwrap();
            activation.id()
        });

        let activations_json_path = activations::activations_json_path(&runtime_dir, &flox_env);

        let result = check_for_activation_ready_and_optionally_attach_pid(
            &activations_json_path,
            store_path,
            Some(attaching_pid),
            attach_expiration,
            now,
        );

        assert!(
            result.is_err(),
            "check_for_activation_ready_and_attach_pid should fail"
        );
        assert!(
            result
                .unwrap_err()
                .downcast_ref::<RestartableFailure>()
                .is_none(),
            "should return terminal (not Restartable) error"
        );
        read_activations(&runtime_dir, &flox_env, |activations| {
            assert!(!activations.is_empty(), "activations should not be empty",);
        });
    }
}
