use std::path::{Path, PathBuf};

use fslock::LockFile;
use serde::{Deserialize, Serialize};
#[cfg(target_os = "linux")]
use shared::canonical_path::CanonicalPath;
use shared::{path_hash, traceable_path, Version};
use time::OffsetDateTime;
use tracing::debug;
use uuid::Uuid;

type Error = anyhow::Error;

/// Deserialized contents of activations.json
#[derive(Clone, Debug, Deserialize, Serialize)]
struct Activations {
    version: Version<1>,
    activations: Vec<Activation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Activation {
    /// Unique identifier for this activation
    ///
    /// There should only be a single activation for an environment + store_path
    /// combination.
    /// But there may be multiple activations since the environment's store_path
    /// may change.
    /// We generate a UUID so that we have something convenient to pass around
    /// and use as a directory name.
    id: Uuid,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
struct AttachedPid {
    pid: u32,
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
    expiration: Option<OffsetDateTime>,
}

/// Acquires the filesystem-based lock on activations.json
#[allow(unused)]
fn acquire_activations_json_lock(
    activations_json_path: impl AsRef<Path>,
) -> Result<LockFile, Error> {
    let lock_path = activations_json_lock_path(activations_json_path);
    Ok(LockFile::open(lock_path.as_os_str())?)
}

/// Returns the path to the lock file for activations.json.
/// The presence of the lock file does not indicate an active lock because the
/// file isn't removed after use.
/// This is a separate file because we replace activations.json on write.
#[allow(unused)]
fn activations_json_lock_path(activations_json_path: impl AsRef<Path>) -> PathBuf {
    activations_json_path.as_ref().with_extension("lock")
}

/// Directory for flox to store runtime data in.
///
/// Typically
/// $XDG_RUNTIME_DIR/flox
/// or
/// ~/.cache/flox/run
///
/// For sockets and activation data, we want the guarantees provided by XDG_RUNTIME_DIR.
/// Per https://specifications.freedesktop.org/basedir-spec/latest/
/// XDG_RUNTIME_DIR
/// - MUST be owned by the user, and they MUST be the only one having read and write access to it. Its Unix access mode MUST be 0700
/// - MUST be on a local file system and not shared with any other system
/// - MUST be created when the user first logs in
///
/// On macOS we use cache directory.
// TODO: some of this logic should be deduplicated with services_socket_path
#[allow(unused)]
fn flox_runtime_dir(cache_dir: impl AsRef<Path>) -> Result<PathBuf, Error> {
    #[cfg(target_os = "macos")]
    let runtime_dir: Option<PathBuf> = None;
    #[cfg(target_os = "linux")]
    let runtime_dir = {
        let base_directories = xdg::BaseDirectories::new()?;
        let runtime_dir = base_directories.get_runtime_directory().ok();
        // Canonicalize so we error early if the path doesn't exist
        runtime_dir
            .map(|runtime_dir| CanonicalPath::new(runtime_dir).map(CanonicalPath::into_inner))
            .transpose()?
    };

    let flox_runtime_dir = match runtime_dir {
        Some(dir) => dir.join("flox"),
        None => cache_dir.as_ref().join("run"),
    };
    // We don't want to error if the directory already exists,
    // so use create_dir_all.
    std::fs::create_dir_all(&flox_runtime_dir)?;

    Ok(flox_runtime_dir)
}

/// {flox_runtime_dir}/{path_hash(flox_env)}/activations.json
#[allow(unused)]
fn activations_json_path(
    cache_dir: impl AsRef<Path>,
    flox_env: impl AsRef<Path>,
) -> Result<PathBuf, Error> {
    Ok(flox_runtime_dir(cache_dir)?
        .join(path_hash(flox_env))
        .join("activations.json"))
}

/// Returns the parsed environment registry file or `None` if it doesn't yet exist.
///
/// The file can be written with [shared::serialize_atomically]
#[allow(unused)]
fn read_activations_json(path: impl AsRef<Path>) -> Result<Option<Activations>, Error> {
    let path = path.as_ref();
    if !path.exists() {
        debug!(
            path = traceable_path(&path),
            "environment registry not found"
        );
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path)?;
    let parsed: Activations = serde_json::from_str(&contents)?;
    Ok(Some(parsed))
}
