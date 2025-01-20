use std::fs;
use std::path::{Path, PathBuf};

use fslock::LockFile;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;
use tracing::debug;

use crate::models::environment::UpgradeResult;

#[derive(Debug, Error)]
pub enum UpgradeChecksError {
    #[error("Failed to acquire lock")]
    Lock(#[source] fslock::Error),
    #[error("Failed to read upgrade information")]
    Read(#[source] std::io::Error),
    #[error("Failed to write upgrade information")]
    Write(#[source] std::io::Error),
    #[error("Failed to parse upgrade information")]
    Deserialize(#[source] serde_json::Error),
    #[error("Failed to serialize upgrade information")]
    Serialize(#[source] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct UpgradeInformation {
    /// The last time the upgrade check was performed
    #[serde(with = "time::serde::iso8601")]
    pub last_checked: OffsetDateTime,
    /// The result of the last upgrade check
    pub result: UpgradeResult,
}

/// A guard for a file containing upgrade information,
/// located at [Self::upgrade_information_path].
/// A guard is created in an [Unlocked] state,
/// in which the upgrade information can be read but not written.
/// The guard can be locked to gain exclusive access to the file
/// via a [LockFile] file lock,
/// allowing the upgrade information to be mutated
/// and [Self::commit]ed back to the file.
#[derive(Debug)]
pub struct UpgradeInformationGuard<LockState> {
    /// The upgrade information found at [Self::upgrade_information_path]
    /// or [None] if no information was found.
    ///
    /// This field can be mutated iff the guard is [Locked].
    info: Option<UpgradeInformation>,
    upgrade_information_path: PathBuf,
    _lock: LockState,
}

/// The state marker for an unlocked guard.
#[derive(Debug)]
pub struct Unlocked;

/// The state marker for a locked guard.
/// This holds a file lock which prevents concurrent writes
/// to the underlying locked upgrade information file.
#[derive(Debug)]
pub struct Locked {
    _lock: LockFile,
}

/// Operations valid for a guard of any state.
impl<L> UpgradeInformationGuard<L> {
    /// Returns the upgrade information found at [Self::upgrade_information_path]
    /// or [None] if no information was found.
    pub fn info(&self) -> &Option<UpgradeInformation> {
        &self.info
    }
}

/// Operations valid for an [Unlocked] guard.
impl UpgradeInformationGuard<Unlocked> {
    /// Reads the upgrade information for an environment located at `dot_flox_path`
    /// and returns an unlocked guard.
    ///
    /// The upgrade information is stored in a file located at the path
    /// provided by [upgrade_information_path].
    /// If no information is found, this methods succeeds
    /// and returns a guard with [None] as the upgrade information.
    /// This guard can then be locked and mutated to store new information.
    pub fn read_in(
        cache_dir: impl AsRef<Path>,
    ) -> Result<UpgradeInformationGuard<Unlocked>, UpgradeChecksError> {
        let upgrade_information_path = upgrade_information_path(cache_dir);
        let info = read_upgrade_information(&upgrade_information_path)?;

        debug!(
            ?upgrade_information_path,
            ?info,
            "created unlocked upgrade information guard"
        );

        Ok(UpgradeInformationGuard {
            info,
            upgrade_information_path,
            _lock: Unlocked,
        })
    }

    /// Attempts to lock the guard and return a locked guard.
    ///
    /// If the guard is already locked, this method returns `Ok(Err(self))`
    /// without waiting.
    pub fn lock_if_unlocked(
        self,
    ) -> Result<Result<UpgradeInformationGuard<Locked>, Self>, UpgradeChecksError> {
        let Some(lock) = try_acquire_lock(&self.upgrade_information_path)? else {
            debug!(upgrade_information_path=?self.upgrade_information_path, "lock already taken");
            return Ok(Err(self));
        };

        debug!(upgrade_information_path=?self.upgrade_information_path, "lock acquired");
        Ok(Ok(UpgradeInformationGuard {
            info: self.info,
            upgrade_information_path: self.upgrade_information_path,
            _lock: Locked { _lock: lock },
        }))
    }
}

/// Operations valid for a [Locked] guard.
/// While locked, this guard has exclusive access to the underlying upgrade information file.
/// When the guard is dropped, the lock is released.
impl UpgradeInformationGuard<Locked> {
    /// Returns a mutable reference to the upgrade information.
    ///
    /// This method is used to set or update the stored upgrade information.
    pub fn info_mut(&mut self) -> &mut Option<UpgradeInformation> {
        &mut self.info
    }

    /// Commits the upgrade information to the file system.
    ///
    /// Updates the file at [Self::upgrade_information_path] to the current state of the guard.
    /// If the guard has no upgrade information, and a file exists at [Self::upgrade_information_path],
    /// the file is deleted.
    /// Otherwise, [Self::info] is serialized to json and written to the file.
    pub fn commit(&self) -> Result<(), UpgradeChecksError> {
        if self.info.is_none() && self.upgrade_information_path.exists() {
            debug!(upgrade_information_path=?self.upgrade_information_path, "deleting upgrade information");
            fs::remove_file(&self.upgrade_information_path).map_err(UpgradeChecksError::Write)?;
            return Ok(());
        }

        if self.info.is_none() {
            debug!("no upgrade information to write");
            return Ok(());
        }

        let info_str = serde_json::to_string(&self.info).map_err(UpgradeChecksError::Serialize)?;
        fs::write(&self.upgrade_information_path, info_str).map_err(UpgradeChecksError::Write)?;

        debug!(upgrade_information_path=?self.upgrade_information_path, "wrote upgrade information");
        Ok(())
    }
}

/// Returns the path to the upgrade information file
/// for an environment located at `dot_flox_path`.
///
/// The upgrade information is stored in a file located at the path
/// ```text
/// cache_dir/upgrade-checks-{path_hash(dot_flox_path)}.json
/// ```
fn upgrade_information_path(cache_dir: impl AsRef<Path>) -> PathBuf {
    cache_dir.as_ref().join("upgrade-checks.json")
}

/// Tries to acquire a lock on the upgrade information file.
/// Returns [None] if the lock is already taken, without waiting.
fn try_acquire_lock(
    upgrade_information_path: impl AsRef<Path>,
) -> Result<Option<LockFile>, UpgradeChecksError> {
    let lock_path = upgrade_information_path.as_ref().with_extension("lock");

    let mut lock = LockFile::open(&lock_path).map_err(UpgradeChecksError::Lock)?;

    if !lock.try_lock().map_err(UpgradeChecksError::Lock)? {
        return Ok(None);
    };

    Ok(Some(lock))
}

/// Reads an [UpgradeInformation] from a file at `upgrade_information_path`.
fn read_upgrade_information(
    upgrade_information_path: impl AsRef<Path>,
) -> Result<Option<UpgradeInformation>, UpgradeChecksError> {
    if !upgrade_information_path.as_ref().exists() {
        return Ok(None);
    }

    let info_str =
        fs::read_to_string(upgrade_information_path).map_err(UpgradeChecksError::Read)?;

    // todo: should we ignore deserialize errors and just return none here,
    // so the file may just get overriden with a good one later?
    let info = serde_json::from_str(&info_str).map_err(UpgradeChecksError::Deserialize)?;
    Ok(Some(info))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;
    use crate::models::lockfile::Lockfile;

    #[test]
    fn try_acquire_lock_does_not_wait_for_lock() {
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let path = temp_file.path();

        std::thread::scope(|scope| {
            let (wait_for_first_lock_sender, wait_for_first_lock_receiver) = mpsc::sync_channel(0);
            let (wait_for_second_lock_sender, wait_for_second_lock_receiver) =
                mpsc::sync_channel(0);

            // first lock
            scope.spawn(move || {
                // acquire the lock and hold it while another process attempts to acquire it
                let lock = try_acquire_lock(path).unwrap();
                assert!(lock.is_some());

                // syncronize second attempt to acquire the lock,
                // to run after the first lock.
                wait_for_first_lock_sender.send(()).unwrap();

                // fail the test if the second attempt to acquire the lock blocks
                wait_for_second_lock_receiver
                    .recv_timeout(Duration::from_secs(1))
                    .unwrap()
            });

            // second lock
            scope.spawn(move || {
                let _ = wait_for_first_lock_receiver.recv();
                let lock = try_acquire_lock(path).unwrap();
                assert!(lock.is_none());
                wait_for_second_lock_sender.send(()).unwrap();
            });
        });
    }

    #[test]
    fn upgrade_information_is_none_if_absent() {
        let temp_dir = tempfile::tempdir().unwrap();

        let guard = UpgradeInformationGuard::read_in(temp_dir.path()).unwrap();
        assert_eq!(guard.info(), &None);
    }

    #[test]
    fn upgrade_information_is_discarded_if_not_committed() {
        let temp_dir = tempfile::tempdir().unwrap();

        let guard = UpgradeInformationGuard::read_in(temp_dir.path()).unwrap();
        let mut locked = guard.lock_if_unlocked().unwrap().unwrap();

        *locked.info_mut() = Some(UpgradeInformation {
            last_checked: OffsetDateTime::now_utc().replace_millisecond(0).unwrap(),
            result: UpgradeResult {
                old_lockfile: None,
                new_lockfile: Lockfile::default(),
                store_path: None,
            },
        });

        drop(locked);

        let guard = UpgradeInformationGuard::read_in(temp_dir.path()).unwrap();
        assert_eq!(guard.info(), &None);
    }

    #[test]
    fn upgrade_information_is_written_if_committed() {
        let temp_dir = tempfile::tempdir().unwrap();

        let guard = UpgradeInformationGuard::read_in(temp_dir.path()).unwrap();
        let mut locked = guard.lock_if_unlocked().unwrap().unwrap();
        let info = UpgradeInformation {
            last_checked: OffsetDateTime::now_utc().replace_millisecond(0).unwrap(),
            result: UpgradeResult {
                old_lockfile: None,
                new_lockfile: Lockfile::default(),
                store_path: None,
            },
        };
        let _ = locked.info_mut().insert(info.clone());
        locked.commit().unwrap();

        drop(locked);

        let guard = UpgradeInformationGuard::read_in(temp_dir.path()).unwrap();
        assert_eq!(guard.info(), &Some(info));
    }
}
