use std::path::{Path, PathBuf};

use flox_core::{WriteError, serialize_atomically, traceable_path};
use fslock::LockFile;
use serde::{Deserialize, Serialize};
use tracing::debug;
use url::Url;

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
    WriteFile(#[source] WriteError),
    #[error("couldn't find parent for path: {0}")]
    BadFilePath(PathBuf),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserState {
    pub confirmed_create_default_env: Option<bool>,
    /// The FloxHub identity from the most recent login or logout. The
    /// credential is the only live source of the user's handle, so this is
    /// what lets `flox activate --default` name `<owner>/default` after the
    /// credential is gone (DEV-269).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_floxhub_auth: Option<LastFloxhubAuth>,
}

/// The last known FloxHub identity, keyed by the hub it belongs to so that a
/// handle recorded against one FloxHub (e.g. a local dev stack) is never used
/// to resolve environments on another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LastFloxhubAuth {
    pub handle: String,
    pub floxhub_base_url: Url,
}

/// Returns the last recorded FloxHub handle if it belongs to the FloxHub
/// instance this [Flox] is configured for.
pub fn last_floxhub_handle(flox: &Flox) -> Option<String> {
    let state = read_user_state_file(user_state_path(flox)).ok()??;
    let auth = state.last_floxhub_auth?;
    (auth.floxhub_base_url == *flox.floxhub.base_url()).then_some(auth.handle)
}

/// Record the FloxHub identity so it can be recovered after logout.
pub fn remember_floxhub_auth(flox: &Flox, handle: &str) -> Result<(), UserStateError> {
    let path = user_state_path(flox);
    let (lock, mut state) = lock_and_read_user_state_file(&path)?;
    state.last_floxhub_auth = Some(LastFloxhubAuth {
        handle: handle.to_string(),
        floxhub_base_url: flox.floxhub.base_url().clone(),
    });
    write_user_state_file(&state, &path, lock)
}

// TODO: These functions are very close to their counterparts in
// `env_registry.rs` and `activations.rs`.
//       The main differences are error types. We could share a common set of functionality
//       by creating a trait that uses associated types/constants to identify the error types
//       that are used at the different steps, then provide default implementations for the
//       operations since they're essentially identical.

/// Returns the path to the user's state file.
pub fn user_state_path(flox: &Flox) -> PathBuf {
    flox.state_dir.join(USER_STATE_FILENAME)
}

/// Returns the path to the user state lock file. The presence
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
    debug!(
        path = traceable_path(&lock_path),
        "acquiring user state lock"
    );
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
    let path = path.as_ref();
    debug!(path = traceable_path(&path), "reading user state file");
    if !path.exists() {
        std::fs::create_dir_all(
            path.parent()
                .ok_or(UserStateError::BadFilePath(path.to_owned()))?,
        )
        .map_err(UserStateError::ReadFile)?;
    }
    let lock = acquire_user_state_lock(path)?;
    let state = read_user_state_file(path)?.unwrap_or_default();
    Ok((lock, state))
}

#[cfg(test)]
mod tests {
    use flox_core::floxhub::Floxhub;

    use super::*;
    use crate::flox::test_helpers::flox_instance;

    #[test]
    fn last_floxhub_handle_only_matches_same_floxhub() {
        let (mut flox, _tempdir) = flox_instance();
        assert_eq!(last_floxhub_handle(&flox), None);

        remember_floxhub_auth(&flox, "somebody").unwrap();
        assert_eq!(last_floxhub_handle(&flox), Some("somebody".to_string()));

        // A handle recorded against one FloxHub must not resolve
        // environments on another.
        flox.floxhub = Floxhub::new(
            Url::parse("https://hub.other.example.com").unwrap(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(last_floxhub_handle(&flox), None);
    }

    #[test]
    fn remember_floxhub_auth_preserves_other_state() {
        let (flox, _tempdir) = flox_instance();
        let path = user_state_path(&flox);

        let (lock, mut state) = lock_and_read_user_state_file(&path).unwrap();
        state.confirmed_create_default_env = Some(true);
        write_user_state_file(&state, &path, lock).unwrap();

        remember_floxhub_auth(&flox, "somebody").unwrap();

        let state = read_user_state_file(&path).unwrap().unwrap();
        assert_eq!(state.confirmed_create_default_env, Some(true));
        assert_eq!(
            state.last_floxhub_auth,
            Some(LastFloxhubAuth {
                handle: "somebody".to_string(),
                floxhub_base_url: flox.floxhub.base_url().clone(),
            })
        );
    }
}
