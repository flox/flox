use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use log::debug;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use super::{copy_dir_recursive, InstallationAttempt, LOCKFILE_FILENAME, MANIFEST_FILENAME};
use crate::flox::Flox;
use crate::models::environment::{call_pkgdb, global_manifest_path};
use crate::models::manifest::{insert_packages, remove_packages, Manifest, TomlEditError};
use crate::models::pkgdb::{CallPkgDbError, UpdateResult, PKGDB_BIN};

pub struct ReadOnly {}
struct ReadWrite {}

/// A view of an environment directory
/// that can be used to build, link, and edit the environment.
///
/// This is a generic file based implementation that should be
/// used by implementations of [super::Environment].
pub struct CoreEnvironment<State = ReadOnly> {
    /// A generic environment directory containing
    /// `manifest.toml` and optionally `manifest.lock`,
    /// as well as any assets consumed by the environment.
    ///
    /// Commonly /.../.flox/env/
    env_dir: PathBuf,
    _state: State,
}

impl<State> CoreEnvironment<State> {
    /// Get the underlying path to the environment directory
    pub fn path(&self) -> &Path {
        &self.env_dir
    }

    /// Get the manifest file
    pub fn manifest_path(&self) -> PathBuf {
        self.env_dir.join(MANIFEST_FILENAME)
    }

    /// Get the path to the lockfile
    ///
    /// Note: may not exist
    pub fn lockfile_path(&self) -> PathBuf {
        self.env_dir.join(LOCKFILE_FILENAME)
    }

    /// Read the manifest file
    fn manifest_content(&self) -> Result<String, CoreEnvironmentError> {
        fs::read_to_string(self.manifest_path()).map_err(CoreEnvironmentError::ReadManifest)
    }

    /// Lock the environment.
    ///
    /// This updates the lock if it exists, or generates a new one if it doesn't.
    ///
    /// Technically this does write to disk as a side effect for now.
    /// It's included in the [ReadOnly] struct for ergonomic reasons
    /// and because it doesn't modify the manifest.
    ///
    /// todo: should we always write the lockfile to disk?
    pub fn lock(&mut self, flox: &Flox) -> Result<LockedManifest, CoreEnvironmentError> {
        let manifest_path = self.manifest_path();
        let lockfile_path = self.lockfile_path();
        let maybe_lockfile = if lockfile_path.exists() {
            debug!("found existing lockfile: {}", lockfile_path.display());
            Some(lockfile_path.as_ref())
        } else {
            debug!("no existing lockfile found");
            None
        };
        let lockfile = LockedManifest::lock_manifest(
            Path::new(&*PKGDB_BIN),
            &manifest_path,
            maybe_lockfile,
            &global_manifest_path(flox),
        )?;

        // Write the lockfile to disk
        // todo: do we always want to do this?
        debug!("generated lockfile, writing to {}", lockfile_path.display());
        std::fs::write(&lockfile_path, lockfile.to_string())
            .map_err(CoreEnvironmentError::WriteLockfile)?;

        Ok(lockfile)
    }

    /// Build the environment, [Self::lock] if necessary.
    ///
    /// Technically this does write to disk as a side effect for now.
    /// It's included in the [ReadOnly] struct for ergonomic reasons
    /// and because it doesn't modify the manifest.
    ///
    /// Does not link the environment to an out path.
    /// Linking should be done explicitly by the caller using [Self::link].
    ///
    /// todo: should we always write the lockfile to disk?
    pub fn build(&mut self, flox: &Flox) -> Result<PathBuf, CoreEnvironmentError> {
        let lockfile = self.lock(flox)?;

        debug!(
            "building environment: system={}, lockfilePath={}",
            &flox.system,
            self.lockfile_path().display()
        );

        let store_path = lockfile.build(Path::new(&*PKGDB_BIN), None)?;

        debug!(
            "built locked environment, store path={}",
            store_path.display()
        );

        Ok(store_path)
    }

    /// Create a new out-link for the environment at the given path.
    ///
    /// Builds the environment if necessary.
    /// todo: should we always build implicitly?
    pub fn link(
        &mut self,
        flox: &Flox,
        out_link_path: impl AsRef<Path>,
    ) -> Result<(), CoreEnvironmentError> {
        let lockfile = self.lock(flox)?;
        debug!(
            "linking environment: system={}, lockfilePath={}, outLinkPath={}",
            &flox.system,
            self.lockfile_path().display(),
            out_link_path.as_ref().display()
        );
        lockfile.build(Path::new(&*PKGDB_BIN), Some(out_link_path.as_ref()))?;

        Ok(())
    }
}

/// Environment modifying methods do not link the new environment to an out path.
/// Linking should be done by the caller.
/// Since files referenced by the environment are ingested into the nix store,
/// the same [CoreEnvironment] instance can be used
/// even if the concrete [super::Environment] tracks the files in a different way
/// such as a git repository or a database.
impl CoreEnvironment<ReadOnly> {
    /// Create a new environment view for the given directory
    ///
    /// This assumes that the directory contains a valid manifest.
    pub fn new(env_dir: impl AsRef<Path>) -> Self {
        CoreEnvironment {
            env_dir: env_dir.as_ref().to_path_buf(),
            _state: ReadOnly {},
        }
    }

    /// Install packages to the environment atomically
    ///
    /// Returns the new manifest content if the environment was modified. Also
    /// returns a map of the packages that were already installed. The installation
    /// will proceed if at least one of the requested packages were added to the
    /// manifest.
    pub fn install(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<InstallationAttempt, CoreEnvironmentError> {
        let current_manifest_contents = self.manifest_content()?;
        let installation = insert_packages(&current_manifest_contents, &packages)
            .map(|insertion| InstallationAttempt {
                new_manifest: insertion.new_toml.map(|toml| toml.to_string()),
                already_installed: insertion.already_installed,
            })
            .map_err(CoreEnvironmentError::ModifyToml)?;
        if let Some(ref new_manifest) = installation.new_manifest {
            self.transact_with_manifest_contents(new_manifest, flox)?;
        }
        Ok(installation)
    }

    /// Uninstall packages from the environment atomically
    ///
    /// Returns true if the environment was modified and false otherwise.
    /// TODO: this should return a list of packages that were actually
    /// uninstalled rather than a bool.
    pub fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<String, CoreEnvironmentError> {
        let current_manifest_contents = self.manifest_content()?;
        let toml = remove_packages(&current_manifest_contents, &packages)
            .map_err(CoreEnvironmentError::ModifyToml)?;
        self.transact_with_manifest_contents(toml.to_string(), flox)?;
        Ok(toml.to_string())
    }

    /// Atomically edit this environment, ensuring that it still builds
    pub fn edit(
        &mut self,
        flox: &Flox,
        contents: String,
    ) -> Result<EditResult, CoreEnvironmentError> {
        let old_contents = self.manifest_content()?;
        // TODO we should probably skip this if the manifest hasn't changed
        self.transact_with_manifest_contents(&contents, flox)?;

        EditResult::new(&old_contents, &contents)
    }

    /// Update the inputs of an environment atomically.
    pub fn update(
        &mut self,
        flox: &Flox,
        inputs: Vec<String>,
    ) -> Result<String, CoreEnvironmentError> {
        // TODO double check canonicalization
        let manifest_path = self.manifest_path();
        let lockfile_path = self.lockfile_path();
        let maybe_lockfile = if lockfile_path.exists() {
            debug!("found existing lockfile: {}", lockfile_path.display());
            Some(lockfile_path)
        } else {
            debug!("no existing lockfile found");
            None
        };
        let mut pkgdb_cmd = Command::new(Path::new(&*PKGDB_BIN));
        pkgdb_cmd
            .args(["manifest", "update"])
            .arg("--ga-registry")
            .arg("--global-manifest")
            .arg(global_manifest_path(flox))
            .arg("--manifest")
            .arg(manifest_path);
        if let Some(lf_path) = maybe_lockfile {
            let canonical_lockfile_path = lf_path
                .canonicalize()
                .map_err(|e| CoreEnvironmentError::BadLockfilePath(e, lf_path.to_path_buf()))?;
            pkgdb_cmd.arg("--lockfile").arg(canonical_lockfile_path);
        }
        pkgdb_cmd.args(inputs);

        debug!("updating lockfile with command: {pkgdb_cmd:?}");
        let result: UpdateResult = serde_json::from_value(
            call_pkgdb(pkgdb_cmd).map_err(CoreEnvironmentError::UpdateFailed)?,
        )
        .map_err(CoreEnvironmentError::ParseUpdateOutput)?;

        self.transact_with_lockfile_contents(result.lockfile.to_string(), flox)?;

        Ok(result.message)
    }

    /// Makes a temporary copy of the environment so modifications to the manifest
    /// can be applied without modifying the original environment.
    fn writable(
        &mut self,
        tempdir: impl AsRef<Path>,
    ) -> Result<CoreEnvironment<ReadWrite>, CoreEnvironmentError> {
        copy_dir_recursive(&self.env_dir, &tempdir.as_ref(), true)
            .map_err(CoreEnvironmentError::MakeTemporaryEnv)?;

        Ok(CoreEnvironment {
            env_dir: tempdir.as_ref().to_path_buf(),
            _state: ReadWrite {},
        })
    }

    /// Replace the contents of this environment (e.g. `.flox/env`)
    /// with that of another environment.
    ///
    /// This will **not** set any out-links to updated versions of the environment.
    fn replace_with(
        &mut self,
        replacement: CoreEnvironment<ReadWrite>,
    ) -> Result<(), CoreEnvironmentError> {
        let transaction_backup = self.env_dir.with_extension("tmp");

        if transaction_backup.exists() {
            debug!(
                "transaction backup exists: {}",
                transaction_backup.display()
            );
            return Err(CoreEnvironmentError::PriorTransaction(transaction_backup));
        }
        debug!(
            "backing up env: from={}, to={}",
            self.env_dir.display(),
            transaction_backup.display()
        );
        fs::rename(&self.env_dir, &transaction_backup)
            .map_err(CoreEnvironmentError::BackupTransaction)?;
        // try to restore the backup if the move fails
        debug!(
            "replacing original env: from={}, to={}",
            replacement.env_dir.display(),
            self.env_dir.display()
        );
        if let Err(err) = copy_dir_recursive(&replacement.env_dir, &self.env_dir, true) {
            debug!(
                "failed to replace env ({}), restoring backup: from={}, to={}",
                err,
                transaction_backup.display(),
                self.env_dir.display(),
            );
            fs::remove_dir_all(&self.env_dir).map_err(CoreEnvironmentError::AbortTransaction)?;
            fs::rename(transaction_backup, &self.env_dir)
                .map_err(CoreEnvironmentError::AbortTransaction)?;
            return Err(CoreEnvironmentError::Move(err));
        }
        debug!("removing backup: path={}", transaction_backup.display());
        fs::remove_dir_all(transaction_backup).map_err(CoreEnvironmentError::RemoveBackup)?;
        Ok(())
    }

    /// Attempt to transactionally replace the manifest contents
    fn transact_with_manifest_contents(
        &mut self,
        manifest_contents: impl AsRef<str>,
        flox: &Flox,
    ) -> Result<(), CoreEnvironmentError> {
        let tempdir = tempfile::tempdir_in(&flox.temp_dir)
            .map_err(CoreEnvironmentError::MakeSandbox)?
            .into_path();

        debug!(
            "transaction: making temporary environment in {}",
            tempdir.display()
        );
        let mut temp_env = self.writable(&tempdir)?;

        debug!("transaction: updating manifest");
        temp_env.update_manifest(&manifest_contents)?;

        debug!("transaction: building environment");
        temp_env.build(flox)?;

        debug!("transaction: replacing environment");
        self.replace_with(temp_env)?;
        Ok(())
    }

    /// Attempt to transactionally replace the lockfile contents
    ///
    /// The lockfile_contents passed to this function must be generated by pkgdb
    /// so that calling `pkgdb manifest lock` with the new lockfile_contents is
    /// idempotent.
    ///
    /// TODO: this is separate from transact_with_manifest_contents because it
    /// shouldn't have to call lock. Currently build calls lock, but we
    /// shouldn't have to lock a second time.
    fn transact_with_lockfile_contents(
        &mut self,
        lockfile_contents: impl AsRef<str>,
        flox: &Flox,
    ) -> Result<(), CoreEnvironmentError> {
        let tempdir = tempfile::tempdir_in(&flox.temp_dir)
            .map_err(CoreEnvironmentError::MakeSandbox)?
            .into_path();

        debug!(
            "transaction: making temporary environment in {}",
            tempdir.display()
        );
        let mut temp_env = self.writable(&tempdir)?;

        debug!("transaction: updating lockfile");
        temp_env.update_lockfile(&lockfile_contents)?;

        debug!("transaction: building environment");
        temp_env.build(flox)?;

        debug!("transaction: replacing environment");
        self.replace_with(temp_env)?;
        Ok(())
    }
}

/// A writable view of an environment directory
///
/// Typically within a temporary directory created by [CoreEnvironment::writable].
/// This is not public to enforce that environments are only edited atomically.
impl CoreEnvironment<ReadWrite> {
    /// Updates the environment manifest with the provided contents
    fn update_manifest(&mut self, contents: &impl AsRef<str>) -> Result<(), CoreEnvironmentError> {
        debug!("writing new manifest to {}", self.manifest_path().display());
        let mut manifest_file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(self.manifest_path())
            .map_err(CoreEnvironmentError::OpenManifest)?;
        manifest_file
            .write_all(contents.as_ref().as_bytes())
            .map_err(CoreEnvironmentError::UpdateManifest)?;
        Ok(())
    }

    /// Updates the environment lockfile with the provided contents
    fn update_lockfile(&mut self, contents: &impl AsRef<str>) -> Result<(), CoreEnvironmentError> {
        debug!("writing lockfile to {}", self.lockfile_path().display());
        std::fs::write(self.lockfile_path(), contents.as_ref())
            .map_err(CoreEnvironmentError::WriteLockfile)?;
        Ok(())
    }
}

#[derive(Debug, Serialize, Clone, Deserialize, PartialEq)]
pub struct LockedManifest(Value);
impl LockedManifest {
    /// Use pkgdb to lock a manifest
    pub fn lock_manifest(
        pkgdb: &Path,
        manifest_path: &Path,
        existing_lockfile_path: Option<&Path>,
        global_manifest_path: &Path,
    ) -> Result<Self, CoreEnvironmentError> {
        let canonical_manifest_path = manifest_path
            .canonicalize()
            .map_err(|e| CoreEnvironmentError::BadManifestPath(e, manifest_path.to_path_buf()))?;

        let mut pkgdb_cmd = Command::new(pkgdb);
        pkgdb_cmd
            .args(["manifest", "lock"])
            .arg("--ga-registry")
            .arg("--global-manifest")
            .arg(global_manifest_path)
            .arg("--manifest")
            .arg(canonical_manifest_path);
        if let Some(lf_path) = existing_lockfile_path {
            let canonical_lockfile_path = lf_path
                .canonicalize()
                .map_err(|e| CoreEnvironmentError::BadLockfilePath(e, lf_path.to_path_buf()))?;
            pkgdb_cmd.arg("--lockfile").arg(canonical_lockfile_path);
        }

        debug!("locking manifest with command: {pkgdb_cmd:?}");
        call_pkgdb(pkgdb_cmd)
            .map_err(CoreEnvironmentError::LockManifest)
            .map(Self)
    }

    /// Build a locked manifest
    ///
    /// if a gcroot_out_link_path is provided,
    /// the environment will be linked to that path and a gcroot will be created
    pub fn build(
        &self,
        pkgdb: &Path,
        gcroot_out_link_path: Option<&Path>,
    ) -> Result<PathBuf, CoreEnvironmentError> {
        let mut pkgdb_cmd = Command::new(pkgdb);
        pkgdb_cmd.arg("buildenv").arg(&self.0.to_string());

        if let Some(gcroot_out_link_path) = gcroot_out_link_path {
            pkgdb_cmd.args(["--out-link", &gcroot_out_link_path.to_string_lossy()]);
        }

        debug!("building environment with command: {pkgdb_cmd:?}");

        let pkgdb_output = pkgdb_cmd
            .output()
            .map_err(CoreEnvironmentError::BuildEnvCall)?;

        if !pkgdb_output.status.success() {
            let stderr = String::from_utf8_lossy(&pkgdb_output.stderr).into_owned();
            return Err(CoreEnvironmentError::BuildEnv(stderr));
        }

        let stdout = String::from_utf8_lossy(&pkgdb_output.stdout).into_owned();

        Ok(PathBuf::from(stdout.trim()))
    }
}
impl ToString for LockedManifest {
    fn to_string(&self) -> String {
        self.0.to_string()
    }
}

#[derive(Debug)]
pub enum EditResult {
    /// The manifest was not modified.
    Unchanged,
    /// The manifest was modified, and the user needs to re-activate it.
    ReActivateRequired,
    /// The manifest was modified, but the user does not need to re-activate it.
    Success,
}

impl EditResult {
    pub fn new(old_manifest: &str, new_manifest: &str) -> Result<Self, CoreEnvironmentError> {
        if old_manifest == new_manifest {
            Ok(Self::Unchanged)
        } else {
            // todo: use a single toml crate (toml_edit already implements serde traits)
            let old_manifest: Manifest =
                toml::from_str(old_manifest).map_err(CoreEnvironmentError::DeserializeManifest)?;
            let new_manifest: Manifest =
                toml::from_str(new_manifest).map_err(CoreEnvironmentError::DeserializeManifest)?;
            // TODO: some modifications to `install` currently require re-activation
            if old_manifest.hook != new_manifest.hook || old_manifest.vars != new_manifest.vars {
                Ok(Self::ReActivateRequired)
            } else {
                Ok(Self::Success)
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum CoreEnvironmentError {
    // region: immutable manifest errors
    #[error("could not modify manifest")]
    ModifyToml(#[source] TomlEditError),
    #[error("could not deserialize manifest")]
    DeserializeManifest(#[source] toml::de::Error),
    // endregion

    // region: transaction errors
    #[error("could not make temporary directory for transaction")]
    MakeSandbox(#[source] std::io::Error),

    #[error("couldn't write new lockfile contents")]
    WriteLockfile(#[source] std::io::Error),

    #[error("could not make temporary copy of environment")]
    MakeTemporaryEnv(#[source] std::io::Error),
    /// Thrown when a .flox/env.tmp directory already exists
    #[error("prior transaction in progress -- delete {0} to discard")]
    PriorTransaction(PathBuf),
    #[error("could not create backup for transaction")]
    BackupTransaction(#[source] std::io::Error),
    #[error("Failed to abort transaction; backup could not be moved back into place")]
    AbortTransaction(#[source] std::io::Error),
    #[error("Failed to move modified environment into place")]
    Move(#[source] std::io::Error),
    #[error("Failed to remove transaction backup")]
    RemoveBackup(#[source] std::io::Error),

    // endregion

    // region: mutable manifest errors
    #[error("could not open manifest")]
    OpenManifest(#[source] std::io::Error),
    #[error("could not write manifest")]
    UpdateManifest(#[source] std::io::Error),
    // endregion

    // region: pkgdb manifest errors
    #[error("provided manifest path does not exist ({1:?})")]
    BadManifestPath(#[source] std::io::Error, PathBuf),

    #[error("provided lockfile path does not exist ({1:?})")]
    BadLockfilePath(#[source] std::io::Error, PathBuf),

    #[error("call to pkgdb failed")]
    PkgDbCall(#[source] std::io::Error),

    #[error("could not lock manifest")]
    LockManifest(#[source] CallPkgDbError),

    #[error("unknown error locking manifest {0}")]
    ParsePkgDbError(String),

    #[error("couldn't parse lockfile as JSON")]
    ParseLockfileJSON(#[source] serde_json::Error),

    #[error("could not open manifest file")]
    ReadManifest(#[source] std::io::Error),

    #[error("unexpected output from pkgdb update")]
    ParseUpdateOutput(#[source] serde_json::Error),

    #[error("failed to update environment")]
    UpdateFailed(#[source] CallPkgDbError),
    // endregion
    #[error("call to `pkgdb buildenv' failed")]
    BuildEnvCall(#[source] std::io::Error),

    #[error("error building environment: {0}")]
    BuildEnv(String),
    // endregion
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    #[cfg(feature = "impure-unit-tests")]
    use serial_test::serial;

    use super::*;
    use crate::flox::tests::flox_instance;
    #[cfg(feature = "impure-unit-tests")]
    use crate::models::environment::init_global_manifest;

    /// Check that `edit` updates the manifest and creates a lockfile
    #[test]
    #[serial]
    #[cfg(feature = "impure-unit-tests")]
    fn edit_env_creates_manifest_and_lockfile() {
        let (flox, tempdir) = flox_instance();
        init_global_manifest(&global_manifest_path(&flox)).unwrap();

        let env_path = tempfile::tempdir_in(&tempdir).unwrap();
        fs::write(env_path.path().join(MANIFEST_FILENAME), "").unwrap();

        let mut env_view = CoreEnvironment::new(&env_path);

        let new_env_str = r#"
        [install]
        hello = {}
        "#;

        env_view.edit(&flox, new_env_str.to_string()).unwrap();

        assert_eq!(env_view.manifest_content().unwrap(), new_env_str);
        assert!(env_view.env_dir.join(LOCKFILE_FILENAME).exists());
    }

    /// replacing an environment should fail if a backup exists
    #[test]
    fn detects_existing_backup() {
        let (_flox, tempdir) = flox_instance();

        let env_path = tempfile::tempdir_in(&tempdir).unwrap();
        let sandbox_path = tempfile::tempdir_in(&tempdir).unwrap();
        fs::create_dir(env_path.path().with_extension("tmp")).unwrap();

        let mut env_view = CoreEnvironment::new(&env_path);
        let temp_env = env_view.writable(&sandbox_path).unwrap();

        let err = env_view
            .replace_with(temp_env)
            .expect_err("Should fail if backup exists");

        assert!(matches!(err, CoreEnvironmentError::PriorTransaction(_)));
    }

    /// creating backup should fail if env is readonly
    #[test]
    #[ignore = "On Ubuntu github runners this moving a read only directory succeeds.
        thread 'models::environment::core_environment::tests::fails_to_create_backup' panicked at 'Should fail to create backup: dir is readonly: 40555: ()'"]
    fn fails_to_create_backup() {
        let (_flox, tempdir) = flox_instance();

        let env_path = tempfile::tempdir_in(&tempdir).unwrap();
        let sandbox_path = tempfile::tempdir_in(&tempdir).unwrap();

        let mut env_path_permissions = fs::metadata(env_path.path()).unwrap().permissions();
        env_path_permissions.set_readonly(true);

        // force fail by setting dir readonly
        fs::set_permissions(&env_path, env_path_permissions.clone()).unwrap();

        let mut env_view = CoreEnvironment::new(&env_path);
        let temp_env = env_view.writable(&sandbox_path).unwrap();

        let err = env_view.replace_with(temp_env).expect_err(&format!(
            "Should fail to create backup: dir is readonly: {:o}",
            env_path_permissions.mode()
        ));

        assert!(
            matches!(err, CoreEnvironmentError::BackupTransaction(err) if err.kind() == std::io::ErrorKind::PermissionDenied)
        );
    }

    /// linking an environment should set a gc-root
    #[test]
    #[serial]
    #[cfg(feature = "impure-unit-tests")]
    fn build_flox_environment_and_links() {
        let (flox, tempdir) = flox_instance();
        init_global_manifest(&global_manifest_path(&flox)).unwrap();

        let env_path = tempfile::tempdir_in(&tempdir).unwrap();
        fs::write(
            env_path.path().join(MANIFEST_FILENAME),
            "
        [install]
        hello = {}
        ",
        )
        .unwrap();

        let mut env_view = CoreEnvironment::new(&env_path);

        env_view.build(&flox).expect("build should succeed");
        env_view
            .link(&flox, env_path.path().with_extension("out-link"))
            .expect("link should succeed");

        // very rudimentary check that the environment manifest built correctly
        // and linked to the out-link.
        assert!(env_path
            .path()
            .with_extension("out-link")
            .join("bin/hello")
            .exists());
    }
}
