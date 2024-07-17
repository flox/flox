use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::process::Command;
use std::sync::{Arc, Mutex};

use enum_dispatch::enum_dispatch;
use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use thiserror::Error;
use tracing::debug;

use crate::models::pkgdb::{
    call_pkgdb,
    error_codes,
    CallPkgDbError,
    ContextMsgError,
    PkgDbError,
    PKGDB_BIN,
};

#[derive(Debug, Error)]
pub enum FlakeInstallableError {
    #[error(transparent)]
    Pkgdb(#[from] CallPkgDbError),
    // todo: do we need to break this into more specific errors?
    #[error("Failed to lock flake installable: {0}")]
    LockInstallable(String),
    #[error("Failed to deserialize locked installable")]
    DeserializeLockedInstallable(#[from] serde_json::Error),
}

/// Rust representation of the output of `pkgdb lock-flake-installable`
/// This is a direct translation of the definition in
/// `<flox>/pkgdb/include/flox/lock-flake-installable.hh`
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct LockedInstallable {
    /// locked url of the flakeref component of the installable
    pub locked_url: String,
    pub flake_description: Option<String>,
    pub locked_flake_attr_path: String,
    pub derivation: String,
    /// Map of output names to their paths
    /// The values are expected to be nix store paths
    pub outputs: BTreeMap<String, String>,
    /// List of output names in the original order
    pub output_names: Vec<String>,
    /// List of output names to install as defined by the package
    pub outputs_to_install: Option<Vec<String>>,
    /// List of output names to install as requested by the user
    pub requested_outputs_to_install: Option<Vec<String>>,
    /// System as defined by the package
    pub package_system: String,
    /// System as specified by the manifest and used to set default attribute
    /// paths when locking the installable
    pub system: String,
    pub name: String,
    pub pname: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub licenses: Option<Vec<String>>,
    pub broken: Option<bool>,
    pub unfree: Option<bool>,
}

/// Required functionality to lock a flake installable
///
/// Implemented as a trait to allow mocking as evaluation is
/// time-consuming unless cached.
///
/// The trait is implemented by the [`Pkgdb`] struct which is the canonical implementation
/// using the `pkgdb lock-flake-installable` command.
///
/// The trait is also implemented by the [`InstallableLockerMock`] struct which is used for testing.
#[enum_dispatch]
pub trait InstallableLocker {
    fn lock_flake_installable(
        &self,
        system: impl AsRef<str>,
        installable: impl AsRef<str>,
    ) -> Result<LockedInstallable, FlakeInstallableError>;
}

#[derive(Debug)]
#[enum_dispatch(InstallableLocker)]
pub enum InstallableLockerImpl {
    Pkgdb(Pkgdb),
    Mock(InstallableLockerMock),
}

impl Default for InstallableLockerImpl {
    fn default() -> Self {
        InstallableLockerImpl::Pkgdb(Pkgdb)
    }
}
/// A wrapper for (eventually) various `pkgdb` commands
/// Currently only implements [InstallableLocker] through
/// `pkgdb lock-flake-installable`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pkgdb;

impl InstallableLocker for Pkgdb {
    fn lock_flake_installable(
        &self,
        system: impl AsRef<str>,
        installable: impl AsRef<str>,
    ) -> Result<LockedInstallable, FlakeInstallableError> {
        let installable = installable.as_ref();
        let mut pkgdb_cmd = Command::new(&*PKGDB_BIN);

        pkgdb_cmd
            .arg("lock-flake-installable")
            .args(["--system", system.as_ref()])
            .arg(installable);

        debug!("locking installable: {pkgdb_cmd:?}");

        let lock = call_pkgdb(pkgdb_cmd).map_err(|err| match err {
            CallPkgDbError::PkgDbError(PkgDbError {
                exit_code: error_codes::NIX_LOCK_FLAKE,
                context_message:
                    Some(ContextMsgError {
                        caught: Some(nix_error),
                        ..
                    }),
                ..
            }) => FlakeInstallableError::LockInstallable(nix_error.message),
            CallPkgDbError::PkgDbError(PkgDbError {
                exit_code: error_codes::LOCK_LOCAL_FLAKE,
                category_message: message,
                ..
            }) => FlakeInstallableError::LockInstallable(message),
            _ => FlakeInstallableError::Pkgdb(err),
        })?;

        let lock = serde_json::from_value(lock)
            .map_err(FlakeInstallableError::DeserializeLockedInstallable)?;

        Ok(lock)
    }
}

/// Mock implementation of [`InstallableLocker`] for testing.
#[derive(Debug, Default)]
pub struct InstallableLockerMock {
    lock_flake_installable: Arc<Mutex<VecDeque<Result<LockedInstallable, FlakeInstallableError>>>>,
}

impl InstallableLockerMock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_lock_result(&self, result: Result<LockedInstallable, FlakeInstallableError>) {
        self.lock_flake_installable
            .lock()
            .unwrap()
            .push_back(result);
    }

    #[allow(unused)]
    fn set_lock_results(
        &self,
        results: impl IntoIterator<Item = Result<LockedInstallable, FlakeInstallableError>>,
    ) {
        let mut queue = self.lock_flake_installable.lock().unwrap();
        queue.clear();
        queue.extend(results);
    }
}

impl InstallableLocker for InstallableLockerMock {
    fn lock_flake_installable(
        &self,
        system: impl AsRef<str>,
        installable: impl AsRef<str>,
    ) -> Result<LockedInstallable, FlakeInstallableError> {
        let mocked_result = self
            .lock_flake_installable
            .lock()
            .unwrap()
            .pop_front()
            .expect("no more mock results");

        debug!(
            system=system.as_ref(),
            installable=installable.as_ref(),
            mocked_result=?mocked_result,
            "responding with mocked result"
        );

        mocked_result
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    const ALLOW_LOCAL_FLAKE_VAR: &str = "_PKGDB_ALLOW_LOCAL_FLAKE";

    /// Returns the path to a bundled flake that contains a number of test packages
    /// for sped up evaluation
    fn local_test_flake() -> String {
        let manifest_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let local_test_flake_path = manifest_root
            .join("../../pkgdb/tests/data/lock-flake-installable")
            .canonicalize()
            .unwrap();
        local_test_flake_path.to_str().unwrap().to_string()
    }

    /// Test that the output of `pkgdb lock-flake-installable` can be deserialized
    /// into a [LockedFlakeInstallble] struct.
    #[test]
    fn test_output_format() {
        // `$system` is set by the nix devshell
        let system = env!("system");
        let installable = format!("{flake}#hello", flake = local_test_flake());

        // make sure the deserialization is not accidentally optimized away
        temp_env::with_var(ALLOW_LOCAL_FLAKE_VAR, Some("1"), || {
            Pkgdb
                .lock_flake_installable(system, installable)
                .expect("locking local test flake should succeed")
        });
    }

    // Tests against locking errors thown by pkgdb.
    //
    // There is currently no coverage of error cases in the pkgdb unit tests,
    // because it's not yet clear how detailed we want tests to be.
    // Currently, flake lock errors are caught and thrown as `LockFlakeInstallableException`,
    // while most evaluation errors are thrown as plain nix errors.
    // While we should have coverage of error cases in pkgdb as well,
    // we need tests on the rust side that ensure
    // that the errors are mapped to the right [FlakeInstallableError] variant.
    // These also tests the error handling in the pkgdb implementation, indirectly.
    //
    // region: pkgdb errors

    #[test]
    fn test_catches_absent_flake() {
        let system = env!("system");
        let installable = "github:flox/trust-this-wont-be-added#hello";

        let result = Pkgdb.lock_flake_installable(system, installable);
        assert!(
            matches!(result, Err(FlakeInstallableError::LockInstallable(_))),
            "{result:#?}"
        );
    }

    #[test]
    fn test_catches_absent_flake_output() {
        let system = env!("system");
        let installable = format!("{flake}#nonexistent", flake = local_test_flake());

        let result = temp_env::with_var(ALLOW_LOCAL_FLAKE_VAR, Some("1"), || {
            Pkgdb.lock_flake_installable(system, installable)
        });

        assert!(
            matches!(result, Err(FlakeInstallableError::LockInstallable(_))),
            "{result:#?}"
        );
    }

    #[test]
    fn test_catches_local_flake_locking() {
        let system = env!("system");
        let installable = format!("{flake}#hello", flake = local_test_flake());

        let result = Pkgdb.lock_flake_installable(system, installable);

        assert!(
            matches!(
                result,
                Err(FlakeInstallableError::LockInstallable(ref message))
                if message.contains("flake must be hosted in a remote code repository")),
            "{result:#?}"
        );
    }

    // endregion: pkgdb errors
}
