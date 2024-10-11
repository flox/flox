use std::fmt::Display;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;
use clap::Args;
use fslock::LockFile;
use indoc::indoc;
use time::{Duration, OffsetDateTime};

use crate::activations::{self, Activations};
use crate::Error;

#[derive(Debug, Args)]
pub struct StartOrAttachArgs {
    #[arg(help = "The PID of the shell registering interest in the activation.")]
    #[arg(short, long, value_name = "PID")]
    pub pid: u32,
    #[arg(help = "The path to the .flox directory for the environment.")]
    #[arg(short, long, value_name = "PATH")]
    pub flox_env: PathBuf,
    #[arg(help = "The store path of the rendered environment for this activation.")]
    #[arg(short, long, value_name = "PATH")]
    pub store_path: String,
}

impl StartOrAttachArgs {
    pub(crate) fn handle(self, runtime_dir: PathBuf) -> Result<(), anyhow::Error> {
        let mut retries = 3;

        loop {
            let result = self.handle_inner(&runtime_dir, attach, start, std::io::stdout());

            let Err(err) = result else {
                break;
            };

            if let Some(restartable_failure) = err.downcast_ref::<RestartableFailure>() {
                eprintln!("{}", restartable_failure);
                retries -= 1;
                if retries == 0 {
                    return Err(err);
                }
                eprintln!("Retrying ...");
                continue;
            }

            return Err(err);
        }

        Ok(())
    }

    fn handle_inner(
        &self,
        runtime_dir: &Path,
        attach_fn: impl FnOnce(&Path, LockFile, &str, u32) -> Result<(), Error>,
        start_fn: impl FnOnce(
            Activations,
            PathBuf,
            &Path,
            &PathBuf,
            fslock::LockFile,
            &str,
            u32,
        ) -> Result<String, Error>,
        mut output: impl Write,
    ) -> Result<(), Error> {
        let activations_json_path =
            activations::activations_json_path(runtime_dir, &self.flox_env)?;

        let (activations, lock) = activations::read_activations_json(&activations_json_path)?;
        let activations = activations.unwrap_or_default();

        let (activation_id, attaching) =
            match activations.activation_for_store_path(&self.store_path) {
                Some(activation) => {
                    attach_fn(&activations_json_path, lock, &self.store_path, self.pid)?;
                    (activation.id(), true)
                },
                None => {
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

        writeln!(&mut output, "_FLOX_ATTACH={attaching}")?;
        writeln!(
            &mut output,
            "_FLOX_ACTIVATION_STATE_DIR={}",
            activations::activation_state_dir_path(runtime_dir, &self.flox_env, &activation_id)?
                .display()
        )?;
        writeln!(&mut output, "_FLOX_ACTIVATION_ID={activation_id}")?;

        Ok(())
    }
}

fn attach(
    activations_json_path: &Path,
    lock: fslock::LockFile,
    store_path: &str,
    pid: u32,
) -> Result<(), Error> {
    // Drop the lock to allow the activation to be updated by other processes
    drop(lock);

    let attach_expiration = OffsetDateTime::now_utc() + Duration::seconds(10);
    wait_for_activation_ready_and_attach_pid(
        activations_json_path,
        store_path,
        attach_expiration,
        pid,
    )?;
    Ok(())
}

fn start(
    mut activations: Activations,
    activations_json_path: PathBuf,
    runtime_dir: &Path,
    flox_env: &PathBuf,
    lock: fslock::LockFile,
    store_path: &str,
    pid: u32,
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
fn wait_for_activation_ready_and_attach_pid(
    activations_json_path: &Path,
    store_path: &str,
    attach_expiration: OffsetDateTime,
    attaching_pid: u32,
) -> Result<(), anyhow::Error> {
    loop {
        let ready = check_for_activation_ready_and_attach_pid(
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

fn check_for_activation_ready_and_attach_pid(
    activations_json_path: &Path,
    store_path: &str,
    attaching_pid: u32,
    attach_expiration: OffsetDateTime,
    now: OffsetDateTime,
) -> Result<bool, anyhow::Error> {
    let (activations, lock) = activations::read_activations_json(activations_json_path)?;
    let Some(mut activations) = activations else {
        anyhow::bail!("Expected an existing activations.json file");
    };

    let activation = activations
        .activation_for_store_path_mut(store_path)
        .context("Prior activation of the environment completed before it could be attached to.")
        .map_err(RestartableFailure)?;

    if activation.ready() {
        activation.attach_pid(attaching_pid, None);
        activations::write_activations_json(&activations, activations_json_path, lock)?;
        return Ok(true);
    }

    if !activation.startup_process_running() {
        // TODO: clean out old activation of store_path
        // Or we may need to do that in activation_for_store_path()
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
        let pid = std::process::id();

        let id = write_activations(&runtime_dir, &flox_env, |activations| {
            activations.create_activation(store_path, pid).unwrap().id()
        });

        let args = StartOrAttachArgs {
            pid,
            flox_env: flox_env.clone(),
            store_path: store_path.to_string(),
        };

        let mut output = Vec::new();

        args.handle_inner(
            runtime_dir.path(),
            |_, _, _, _| Ok(()),
            |_, _, _, _, _, _, _| panic!("start should not be called"),
            &mut output,
        )
        .expect("handle_inner should succeed");

        let output = String::from_utf8(output).unwrap();

        assert!(output.contains("_FLOX_ATTACH=true"));
        assert!(output.contains(&format!(
            "_FLOX_ACTIVATION_STATE_DIR={}",
            activations::activation_state_dir_path(&runtime_dir, flox_env, &id)
                .unwrap()
                .display()
        )));
        assert!(output.contains(&format!("_FLOX_ACTIVATION_ID={}", id)));
    }

    #[test]
    fn start_if_activation_does_not_exist() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let flox_env = PathBuf::from("/path/to/floxenv");
        let store_path = "/store/path";

        // The PID of the current process, guaranteed to be running
        let pid = std::process::id();

        write_activations(&runtime_dir, &flox_env, |_| {});

        let args = StartOrAttachArgs {
            pid,
            flox_env: flox_env.clone(),
            store_path: store_path.to_string(),
        };

        let mut output = Vec::new();

        let id = "1".to_string();
        args.handle_inner(
            runtime_dir.path(),
            |_, _, _, _| panic!("attach should not be called"),
            |_, _, _, _, _, _, _| Ok(id.clone()),
            &mut output,
        )
        .expect("handle_inner should succeed");

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("_FLOX_ATTACH=false"));
        assert!(output.contains(&format!(
            "_FLOX_ACTIVATION_STATE_DIR={}",
            activations::activation_state_dir_path(&runtime_dir, flox_env, &id)
                .unwrap()
                .display()
        )));
        assert!(output.contains(&format!("_FLOX_ACTIVATION_ID={}", id)));
    }

    #[test]
    fn check_for_activation_not_ready() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let flox_env = PathBuf::from("/path/to/floxenv");
        let store_path = "/store/path";

        // The PID of the current process, guaranteed to be running
        let pid = std::process::id();
        let attaching_pid = 5678;

        let now = OffsetDateTime::now_utc();
        let attach_expiration = now + Duration::seconds(10);

        let _ = write_activations(&runtime_dir, &flox_env, |activations| {
            activations.create_activation(store_path, pid).unwrap().id()
        });

        let activations_json_path =
            activations::activations_json_path(&runtime_dir, &flox_env).unwrap();

        let ready = check_for_activation_ready_and_attach_pid(
            &activations_json_path,
            store_path,
            attaching_pid,
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
        let pid = std::process::id();
        let attaching_pid = 5678;

        let now = OffsetDateTime::now_utc();
        let attach_expiration = now + Duration::seconds(10);

        let _ = write_activations(&runtime_dir, &flox_env, |activations| {
            let activation = activations.create_activation(store_path, pid).unwrap();
            activation.set_ready();
            activation.id()
        });

        let activations_json_path =
            activations::activations_json_path(&runtime_dir, &flox_env).unwrap();

        let ready = check_for_activation_ready_and_attach_pid(
            &activations_json_path,
            store_path,
            attaching_pid,
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

    fn make_unused_pid() -> u32 {
        let pid = std::process::id();
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

        let activations_json_path =
            activations::activations_json_path(&runtime_dir, &flox_env).unwrap();

        let result = check_for_activation_ready_and_attach_pid(
            &activations_json_path,
            store_path,
            attaching_pid,
            attach_expiration,
            now,
        );

        assert!(
            result.is_err(),
            "check_for_activation_ready_and_attach_pid should fail"
        );
    }

    #[test]
    fn check_for_activation_fails_if_starting_process_timeout_expires() {
        let runtime_dir = tempfile::tempdir().unwrap();
        let flox_env = PathBuf::from("/path/to/floxenv");
        let store_path = "/store/path";

        // The PID of the current process, guaranteed to be running
        let pid = std::process::id();
        let attaching_pid = 5678;

        let now = OffsetDateTime::now_utc();
        // Set the expiration to be in the past
        let attach_expiration = now - Duration::seconds(10);

        let _ = write_activations(&runtime_dir, &flox_env, |activations| {
            let activation = activations.create_activation(store_path, pid).unwrap();
            activation.id()
        });

        let activations_json_path =
            activations::activations_json_path(&runtime_dir, &flox_env).unwrap();

        let result = check_for_activation_ready_and_attach_pid(
            &activations_json_path,
            store_path,
            attaching_pid,
            attach_expiration,
            now,
        );

        assert!(
            result.is_err(),
            "check_for_activation_ready_and_attach_pid should fail"
        );
    }
}
