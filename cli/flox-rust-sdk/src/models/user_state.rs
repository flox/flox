use std::path::{Path, PathBuf};

use flox_core::{serialize_atomically, traceable_path, SerializeError};
use fslock::LockFile;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::flox::Flox;

pub const USER_STATE_FILENAME: &str = "user_state.json";

#[derive(Debug, thiserror::Error)]
pub enum UserStateError {
    #[error("couldn't acquire user state file lock")]
    AcquireLock(#[source] fslock::Error),
    #[error("couldn't read user state file")]
    ReadFile(#[source] std::io::Error),
    #[error("couldn't parse user state file")]
    Parse(#[source] serde_json::Error),
    #[error("failed to write user state file")]
    WriteFile(#[source] SerializeError),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserState {
    pub confirmed_create_default_env: Option<bool>,
}

// TODO: These functions are very close to their counterparts in
// `env_registry.rs` and `activations.rs`.
//       The main differences are error types. We could share a common set of functionality
//       by creating a trait that uses associated types/constants to identify the error types
//       that are used at the different steps, then provide default implementations for the
//       operations since they're essentially identical.

/// Returns the path to the user's state file.
pub fn user_state_path(flox: &Flox) -> PathBuf {
    flox.cache_dir.join(USER_STATE_FILENAME)
}

/// Returns the path to the user state lock file. The presensce
/// of the lock file does not indicate an active lock because the file isn't
/// removed after use. This is a separate file because we replace the state file
/// on write.
pub(crate) fn user_state_lock_path(state_file_path: impl AsRef<Path>) -> PathBuf {
    state_file_path.as_ref().with_extension("lock")
}

/// Returns the parsed state file or `None` if it doesn't yet exist.
pub fn read_user_state_file(path: impl AsRef<Path>) -> Result<Option<UserState>, UserStateError> {
    let path = path.as_ref();
    if !path.exists() {
        debug!(path = traceable_path(&path), "user state file not found");
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path).map_err(UserStateError::ReadFile)?;
    let parsed: UserState = serde_json::from_str(&contents).map_err(UserStateError::Parse)?;
    Ok(Some(parsed))
}

/// Acquires the filesystem-based lock on the user state file
pub fn acquire_user_state_lock(
    state_file_path: impl AsRef<Path>,
) -> Result<LockFile, UserStateError> {
    let lock_path = user_state_lock_path(state_file_path);
    let mut lock = LockFile::open(lock_path.as_os_str()).map_err(UserStateError::AcquireLock)?;
    lock.lock().map_err(UserStateError::AcquireLock)?;
    Ok(lock)
}

/// Writes the user state file to disk.
///
/// First the registry is written to a temporary file and then it is renamed so the write appears
/// atomic. This also takes a [LockFile] argument to ensure that the write can only be performed
/// when the lock is acquired. It is a bug if you pass a [LockFile] that doesn't correspond to the
/// user state file, as that is essentially bypassing the lock.
pub fn write_user_state_file(
    state: &UserState,
    path: impl AsRef<Path>,
    lock: LockFile,
) -> Result<(), UserStateError> {
    serialize_atomically(state, &path, lock).map_err(UserStateError::WriteFile)
}

/// Acquires the lock on the user state file before reading it, returning
/// both the lock and the parsed file contents.
pub fn lock_and_read_user_state_file(
    path: impl AsRef<Path>,
) -> Result<(LockFile, UserState), UserStateError> {
    debug!(path = traceable_path(&path), "reading user state file");
    let lock = acquire_user_state_lock(&path)?;
    let state = read_user_state_file(&path)?.unwrap_or_default();
    Ok((lock, state))
}
