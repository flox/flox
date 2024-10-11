use std::path::{Path, PathBuf};

use anyhow::Context;
use flox_core::{path_hash, traceable_path, Version};
use fslock::LockFile;
use nix::errno::Errno;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::{Duration, OffsetDateTime};
use tracing::debug;

type Error = anyhow::Error;

/// Deserialized contents of activations.json
///
/// This is the state of the activations of the environments.
/// There is EXACTLY ONE [Activations] file per FLOX_ENV,
/// which may be rendered at different times with different store paths.
/// [Activations::activations] is a list of [Activation]s
/// with AT MOST ONE activation for a given store path.
/// This latter invariant is enforced by [Activations::get_or_create_activation_for_store_path]
/// being the only way to add an activation.
/// Activations are identifiable by their [Activation::id], for simpler lookups
/// and global uniqueness in case the that two environments have the same store path.
///
/// Notably, the [Activations] does not feature methods to remove activations.
/// Removing actiavtions falls onto the watchdog, which is responsible for cleaning up activations.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Activations {
    version: Version<1>,
    activations: Vec<Activation>,
}

impl Activations {
    /// Get a mutable reference to the activation with the given ID.
    ///
    /// Used internally to manipulate the state of an activation.
    pub fn activation_for_id_mut(
        &mut self,
        activation_id: impl AsRef<str>,
    ) -> Option<&mut Activation> {
        self.activations
            .iter_mut()
            .find(|activation| activation.id == activation_id.as_ref())
    }

    /// Get an immutable reference to the activation with the given ID.
    ///
    /// Used internally to manipulate the state of an activation.
    #[allow(unused)]
    pub fn activation_for_id_ref(&self, activation_id: impl AsRef<str>) -> Option<&Activation> {
        self.activations
            .iter()
            .find(|activation| activation.id == activation_id.as_ref())
    }

    /// Get a mutable reference to the activation with the given store path.
    pub fn activation_for_store_path(&self, store_path: &str) -> Option<&Activation> {
        self.activations
            .iter()
            .find(|activation| activation.store_path == store_path)
    }

    /// Get a mutable reference to the activation with the given store path.
    pub fn activation_for_store_path_mut(&mut self, store_path: &str) -> Option<&mut Activation> {
        self.activations
            .iter_mut()
            .find(|activation| activation.store_path == store_path)
    }

    /// Create a new activation for the given store path and attach a PID to it.
    ///
    /// If an activation for the store path already exists, return an error.
    pub fn create_activation(
        &mut self,
        store_path: &str,
        pid: u32,
    ) -> Result<&mut Activation, Error> {
        if self.activation_for_store_path(store_path).is_some() {
            anyhow::bail!("activation for store path '{store_path}' already exists");
        }

        let mut chars = blake3::hash(store_path.as_bytes()).to_hex();
        // We need something short to put in socket paths
        chars.truncate(8);
        let id = chars.to_string();
        let activation = Activation {
            id,
            store_path: store_path.to_string(),
            ready: false,
            attached_pids: vec![AttachedPid {
                pid,
                expiration: None,
            }],
        };

        self.activations.push(activation);

        Ok(self.activations.last_mut().unwrap())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Activation {
    /// Unique identifier for this activation
    ///
    /// There should only be a single activation for an environment + store_path
    /// combination.
    /// But there may be multiple activations since the environment's store_path
    /// may change.
    /// We generate a UUID so that we have something convenient to pass around
    /// and use as a directory name.
    id: String,
    /// The store path of the built environment
    store_path: String,
    /// Whether the activation of the environment is ready to be attached to.
    ///
    /// Since hooks may take an arbitrary amount of time, it takes an arbitrary
    /// amount of time before an environment is ready.
    ready: bool,
    /// PIDs that have registered interest in the activation.
    ///
    /// The activation should not be cleaned up until all PIDs have exited or
    /// expired.
    attached_pids: Vec<AttachedPid>,
}

impl Activation {
    pub fn id(&self) -> String {
        self.id.clone()
    }

    /// Whether the activation is ready to be attached to.
    ///
    /// "Readyness" is a one way state change, set via [Self::set_ready].
    pub fn ready(&self) -> bool {
        self.ready
    }

    /// Set the activation as ready to be attached to.
    pub fn set_ready(&mut self) {
        self.ready = true;
    }

    /// Whether the process that started the activation is still running.
    ///
    /// Used to determine if the attaching processes need to continue to wait,
    /// for the activation to become ready.
    pub fn startup_process_running(&self) -> bool {
        self.attached_pids
            .first()
            .map(|attached_pid| attached_pid.is_running())
            .unwrap_or_default()
    }

    /// Attach a PID to an activation.
    ///
    /// Register another PID that runs the same activation of an environment.
    /// Registered PIDs are used by the watchdog,
    /// to determine when an activation can be cleaned up.
    pub fn attach_pid(&mut self, pid: u32, timeout: Option<Duration>) {
        let expiration = timeout.map(|timeout| OffsetDateTime::now_utc() + timeout);
        let attached_pid = AttachedPid { pid, expiration };

        self.attached_pids.push(attached_pid);
    }

    /// Remove a PID from an activation.
    ///
    /// Unregister a PID that has previously been attached to an activation.
    ///
    /// Primarily, used as part of the `attach` subcommand to update,
    /// which PID is attached to an activation.
    /// I.e. in in-place activations, the process that started the activation will be flox,
    /// while the process that attaches to the activation will be the `eval`ing shell.
    pub fn remove_pid(&mut self, pid: u32) {
        self.attached_pids
            .retain(|attached_pid| attached_pid.pid != pid);
    }
}

#[cfg(test)]
impl Activation {
    pub(crate) fn attached_pids(&self) -> &[AttachedPid] {
        &self.attached_pids
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub(crate) struct AttachedPid {
    pub(crate) pid: u32,
    /// If Some, the time after which the activation can be cleaned up
    ///
    /// Even if the PID has exited, the activation should not be cleaned up
    /// until an expiration is reached.
    /// Expiration is used to support in-place activations.
    /// For an in-place activation, the `flox activate` command generating the
    /// script that can be evaluated by the shell will exit before the shell has
    /// time to evaluate the script.
    /// In that case, `flox activate` sets an expiration so that the shell has
    /// some time before the activation is cleaned up.
    pub(crate) expiration: Option<OffsetDateTime>,
}

impl AttachedPid {
    fn is_running(&self) -> bool {
        let pid = Pid::from_raw(self.pid as i32);
        match kill(pid, None) {
            // These semantics come from kill(2).
            Ok(_) => true,              // Process received the signal and is running.
            Err(Errno::EPERM) => true,  // No permission to send a signal but we know it's running.
            Err(Errno::ESRCH) => false, // No process running to receive the signal.
            Err(_) => false,            // Unknown error, assume no running process.
        }
    }
}

/// Acquires the filesystem-based lock on activations.json
#[allow(unused)]
fn acquire_activations_json_lock(
    activations_json_path: impl AsRef<Path>,
) -> Result<LockFile, Error> {
    let lock_path = activations_json_lock_path(activations_json_path);
    let lock_path_parent = lock_path.parent().expect("lock path has parent");
    if !(lock_path.exists()) {
        std::fs::create_dir_all(lock_path.parent().unwrap())?;
    }
    let mut lock = LockFile::open(&lock_path).context("failed to open lockfile")?;
    lock.lock().context("failed to lock lockfile")?;
    Ok(lock)
}

/// Returns the path to the lock file for activations.json.
/// The presence of the lock file does not indicate an active lock because the
/// file isn't removed after use.
/// This is a separate file because we replace activations.json on write.
#[allow(unused)]
fn activations_json_lock_path(activations_json_path: impl AsRef<Path>) -> PathBuf {
    activations_json_path.as_ref().with_extension("lock")
}

/// {flox_runtime_dir}/{path_hash(flox_env)}/activations.json
pub fn activations_json_path(
    runtime_dir: impl AsRef<Path>,
    flox_env: impl AsRef<Path>,
) -> Result<PathBuf, Error> {
    Ok(runtime_dir
        .as_ref()
        .join(path_hash(flox_env))
        .join("activations.json"))
}

/// {flox_runtime_dir}/{path_hash(flox_env)}/{activation_id}
pub fn activation_state_dir_path(
    runtime_dir: impl AsRef<Path>,
    flox_env: impl AsRef<Path>,
    activation_id: impl AsRef<str>,
) -> Result<PathBuf, Error> {
    Ok(runtime_dir
        .as_ref()
        .join(path_hash(flox_env))
        .join(activation_id.as_ref()))
}

/// Returns the parsed environment registry file or `None` if it doesn't yet exist.
///
/// The file can be written with [write_activations_json].
/// This function acquires a lock on the file,
/// which should be reused for writing, to avoid TOCTOU issues.
pub fn read_activations_json(
    path: impl AsRef<Path>,
) -> Result<(Option<Activations>, LockFile), Error> {
    let path = path.as_ref();
    let lock_file = acquire_activations_json_lock(path).context("failed to acquire lockfile")?;

    if !path.exists() {
        debug!(
            path = traceable_path(&path),
            "environment registry not found"
        );
        return Ok((None, lock_file));
    }

    let contents = std::fs::read_to_string(path)?;
    let parsed: Activations = serde_json::from_str(&contents)?;
    Ok((Some(parsed), lock_file))
}

/// Writes the environment registry file.
/// The file is written atomically.
/// The lock is released after the write.
///
/// This uses [flox_core::serialize_atomically] to write the file, and inherits its requirements.
/// * `path` must have a parent directory.
/// * The lock must correspond to the file being written.
pub fn write_activations_json(
    activations: &Activations,
    path: impl AsRef<Path>,
    lock: LockFile,
) -> Result<(), Error> {
    flox_core::serialize_atomically(&json!(activations), &path, lock)?;
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn create_activation() {
        let mut activations = Activations::default();
        let store_path = "/store/path";
        let activation = activations.create_activation(store_path, 123);

        assert!(activation.is_ok(), "{}", activation.unwrap_err());
        assert_eq!(activations.activations.len(), 1);

        let activation = activations.create_activation(store_path, 123);
        assert!(
            activation.is_err(),
            "adding the same activation twice should fail"
        );
        assert_eq!(activations.activations.len(), 1);
    }

    #[test]
    fn get_activation_by_id() {
        let mut activations = Activations::default();
        let store_path = "/store/path";
        let activation = activations.create_activation(store_path, 123).unwrap();
        let id = activation.id();

        let activation = activations.activation_for_id_ref(&id).unwrap();
        assert_eq!(activation.id(), id);
        assert_eq!(activation.store_path, store_path);
    }

    #[test]
    fn get_activation_by_id_mut() {
        let mut activations = Activations::default();
        let store_path = "/store/path";
        let activation = activations.create_activation(store_path, 123).unwrap();
        let id = activation.id();

        let activation = activations.activation_for_id_mut(&id).unwrap();
        assert_eq!(activation.id(), id);
        assert_eq!(activation.store_path, store_path);
    }

    #[test]
    fn activation_attach_pid() {
        let mut activation = Activation {
            id: "1".to_string(),
            store_path: "/store/path".to_string(),
            ready: false,
            attached_pids: vec![],
        };

        activation.attach_pid(123, None);
        assert_eq!(activation.attached_pids.len(), 1);
        assert_eq!(activation.attached_pids[0].pid, 123);
    }

    #[test]
    fn activation_remove_pid() {
        let mut activation = Activation {
            id: "1".to_string(),
            store_path: "/store/path".to_string(),
            ready: false,
            attached_pids: vec![AttachedPid {
                pid: 123,
                expiration: None,
            }],
        };

        activation.remove_pid(123);
        assert_eq!(activation.attached_pids.len(), 0);
    }
}
