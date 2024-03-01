use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use log::debug;
use thiserror::Error;

use super::{
    copy_dir_recursive,
    CanonicalizeError,
    InstallationAttempt,
    UninstallationAttempt,
    UpdateResult,
    LOCKFILE_FILENAME,
    MANIFEST_FILENAME,
};
use crate::flox::Flox;
use crate::models::container_builder::ContainerBuilder;
use crate::models::environment::{call_pkgdb, global_manifest_path, CanonicalPath};
use crate::models::lockfile::{LockedManifest, LockedManifestError};
use crate::models::manifest::{
    insert_packages,
    remove_packages,
    Manifest,
    PackageToInstall,
    TomlEditError,
};
use crate::models::pkgdb::{CallPkgDbError, UpgradeResult, UpgradeResultJSON, PKGDB_BIN};

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
        fs::read_to_string(self.manifest_path()).map_err(CoreEnvironmentError::OpenManifest)
    }

    /// Lock the environment.
    ///
    /// This re-writes the lock if it exists.
    /// If the lock doesn't exist, it uses the global lock, and then it writes
    /// a new lock.
    ///
    /// Technically this does write to disk as a side effect for now.
    /// It's included in the [ReadOnly] struct for ergonomic reasons
    /// and because it doesn't modify the manifest.
    ///
    /// todo: should we always write the lockfile to disk?
    pub fn lock(&mut self, flox: &Flox) -> Result<LockedManifest, CoreEnvironmentError> {
        let manifest_path = self.manifest_path();
        let environment_lockfile_path = self.lockfile_path();
        let existing_lockfile_path = if environment_lockfile_path.exists() {
            debug!(
                "found existing lockfile: {}",
                environment_lockfile_path.display()
            );
            environment_lockfile_path.clone()
        } else {
            debug!("no existing lockfile found, using the global lockfile as a base");
            // Use the global lock so we're less likely to kick off a pkgdb
            // scrape in e.g. an install.
            LockedManifest::ensure_global_lockfile(flox)
                .map_err(CoreEnvironmentError::LockedManifest)?
        };
        let lockfile_path = CanonicalPath::new(existing_lockfile_path)
            .map_err(CoreEnvironmentError::BadLockfilePath)?;

        let lockfile = LockedManifest::lock_manifest(
            Path::new(&*PKGDB_BIN),
            &manifest_path,
            &lockfile_path,
            &global_manifest_path(flox),
        )
        .map_err(CoreEnvironmentError::LockedManifest)?;

        // Write the lockfile to disk
        // todo: do we always want to do this?
        debug!(
            "generated lockfile, writing to {}",
            environment_lockfile_path.display()
        );
        std::fs::write(
            &environment_lockfile_path,
            serde_json::to_string_pretty(&lockfile).unwrap(),
        )
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
    #[must_use = "don't discard the store path of built environments"]
    pub fn build(&mut self, flox: &Flox) -> Result<PathBuf, CoreEnvironmentError> {
        let lockfile = self.lock(flox)?;

        debug!(
            "building environment: system={}, lockfilePath={}",
            &flox.system,
            self.lockfile_path().display()
        );

        let store_path = lockfile
            .build(Path::new(&*PKGDB_BIN), None, &None)
            .map_err(CoreEnvironmentError::LockedManifest)?;

        debug!(
            "built locked environment, store path={}",
            store_path.display()
        );

        Ok(store_path)
    }

    /// Creates a [ContainerBuilder] from the environment.
    ///
    /// The sink is typically a [File](std::fs::File), [Stdout](std::io::Stdout)
    /// but can be any type that implements [Write](std::io::Write).
    ///
    /// While container _images_ can be created on any platform,
    /// only linux _containers_ can be run with `docker` or `podman`.
    /// Building an environment for linux on a non-linux platform (macos),
    /// will likely fail unless all packages in the environment can be substituted.
    ///
    /// There are mitigations for this, such as building within a VM or container.
    /// Such solutions are out of scope at this point.
    /// Until then, this function will error with [CoreEnvironmentError::ContainerizeUnsupportedSystem]
    /// if the environment is not linux.
    ///
    /// [Self::lock]s if necessary.
    ///
    /// Technically this does write to disk as a side effect (i.e. by locking).
    /// It's included in the [ReadOnly] struct for ergonomic reasons
    /// and because it doesn't modify the manifest.
    ///
    /// todo: should we always write the lockfile to disk?
    pub fn build_container(
        &mut self,
        flox: &Flox,
    ) -> Result<ContainerBuilder, CoreEnvironmentError> {
        if std::env::consts::OS != "linux" {
            return Err(CoreEnvironmentError::ContainerizeUnsupportedSystem(
                std::env::consts::OS.to_string(),
            ));
        }

        let lockfile = self.lock(flox)?;

        debug!(
            "building container: system={}, lockfilePath={}",
            &flox.system,
            self.lockfile_path().display()
        );

        let builder = lockfile
            .build_container(Path::new(&*PKGDB_BIN))
            .map_err(CoreEnvironmentError::LockedManifest)?;
        Ok(builder)
    }

    /// Create a new out-link for the environment at the given path.
    ///
    /// Builds the environment if necessary.
    /// TODO: should we always build implicitly?
    pub fn link(
        &mut self,
        flox: &Flox,
        out_link_path: impl AsRef<Path>,
        store_path: &Option<PathBuf>,
    ) -> Result<(), CoreEnvironmentError> {
        let lockfile = self.lock(flox)?;
        debug!(
            "linking environment: system={}, lockfilePath={}, outLinkPath={}",
            &flox.system,
            self.lockfile_path().display(),
            out_link_path.as_ref().display()
        );
        lockfile
            .build(
                Path::new(&*PKGDB_BIN),
                Some(out_link_path.as_ref()),
                store_path,
            )
            .map_err(CoreEnvironmentError::LockedManifest)?;
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
        packages: &[PackageToInstall],
        flox: &Flox,
    ) -> Result<InstallationAttempt, CoreEnvironmentError> {
        let current_manifest_contents = self.manifest_content()?;
        let mut installation = insert_packages(&current_manifest_contents, packages)
            .map(|insertion| InstallationAttempt {
                new_manifest: insertion.new_toml.map(|toml| toml.to_string()),
                already_installed: insertion.already_installed,
                store_path: None,
            })
            .map_err(CoreEnvironmentError::ModifyToml)?;
        if let Some(ref new_manifest) = installation.new_manifest {
            let store_path = self.transact_with_manifest_contents(new_manifest, flox)?;
            installation.store_path = Some(store_path);
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
    ) -> Result<UninstallationAttempt, CoreEnvironmentError> {
        let current_manifest_contents = self.manifest_content()?;
        let toml = remove_packages(&current_manifest_contents, &packages)
            .map_err(CoreEnvironmentError::ModifyToml)?;
        let store_path = self.transact_with_manifest_contents(toml.to_string(), flox)?;
        Ok(UninstallationAttempt {
            new_manifest: Some(toml.to_string()),
            store_path: Some(store_path),
        })
    }

    /// Atomically edit this environment, ensuring that it still builds
    pub fn edit(
        &mut self,
        flox: &Flox,
        contents: String,
    ) -> Result<EditResult, CoreEnvironmentError> {
        let old_contents = self.manifest_content()?;

        // skip the edit if the contents are unchanged
        // note: consumers of this function may call [Self::link] separately,
        //       causing an evaluation/build of the environment.
        if contents == old_contents {
            return Ok(EditResult::Unchanged);
        }

        let store_path = self.transact_with_manifest_contents(&contents, flox)?;

        EditResult::new(&old_contents, &contents, Some(store_path))
    }

    /// Atomically edit this environment, without checking that it still builds
    ///
    /// This is unsafe as it can create broken environments!
    /// Used by the implementation of <https://github.com/flox/flox/issues/823>
    /// and may be removed in the future in favor of something like <https://github.com/flox/flox/pull/681>
    pub(crate) fn edit_unsafe(
        &mut self,
        flox: &Flox,
        contents: String,
    ) -> Result<Result<EditResult, CoreEnvironmentError>, CoreEnvironmentError> {
        let old_contents = self.manifest_content()?;

        // skip the edit if the contents are unchanged
        // note: consumers of this function may call [Self::link] separately,
        //       causing an evaluation/build of the environment.
        if contents == old_contents {
            return Ok(Ok(EditResult::Unchanged));
        }

        let tempdir = tempfile::tempdir_in(&flox.temp_dir)
            .map_err(CoreEnvironmentError::MakeSandbox)?
            .into_path();

        debug!(
            "transaction: making temporary environment in {}",
            tempdir.display()
        );
        let mut temp_env = self.writable(&tempdir)?;

        debug!("transaction: updating manifest");
        temp_env.update_manifest(&contents)?;

        debug!("transaction: building environment, ignoring errors (unsafe)");

        let build_attempt = temp_env.build(flox);

        debug!("transaction: replacing environment");
        self.replace_with(temp_env)?;

        match build_attempt {
            Ok(store_path) => Ok(EditResult::new(&old_contents, &contents, Some(store_path))),
            Err(err) => Ok(Err(err)),
        }
    }

    /// Update the inputs of an environment atomically.
    pub fn update(
        &mut self,
        flox: &Flox,
        inputs: Vec<String>,
    ) -> Result<UpdateResult, CoreEnvironmentError> {
        // TODO: double check canonicalization
        let UpdateResult {
            new_lockfile,
            old_lockfile,
            ..
        } = LockedManifest::update_manifest(
            flox,
            Some(self.manifest_path()),
            self.lockfile_path(),
            inputs,
        )
        .map_err(CoreEnvironmentError::LockedManifest)?;

        let store_path = self.transact_with_lockfile_contents(
            serde_json::to_string_pretty(&new_lockfile).unwrap(),
            flox,
        )?;

        Ok(UpdateResult {
            new_lockfile,
            old_lockfile,
            store_path: Some(store_path),
        })
    }

    /// Atomically upgrade packages in this environment
    pub fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[String],
    ) -> Result<UpgradeResult, CoreEnvironmentError> {
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
            .args(["manifest", "upgrade"])
            .arg("--ga-registry")
            .arg("--global-manifest")
            .arg(global_manifest_path(flox))
            .arg("--manifest")
            .arg(manifest_path);
        if let Some(lf_path) = maybe_lockfile {
            let canonical_lockfile_path =
                CanonicalPath::new(lf_path).map_err(CoreEnvironmentError::BadLockfilePath)?;
            pkgdb_cmd.arg("--lockfile").arg(canonical_lockfile_path);
        }
        pkgdb_cmd.args(groups_or_iids);

        debug!("upgrading environment with command: {pkgdb_cmd:?}");
        let json: UpgradeResultJSON = serde_json::from_value(
            call_pkgdb(pkgdb_cmd).map_err(CoreEnvironmentError::UpgradeFailed)?,
        )
        .map_err(CoreEnvironmentError::ParseUpgradeOutput)?;

        let store_path = self.transact_with_lockfile_contents(json.lockfile.to_string(), flox)?;

        Ok(UpgradeResult {
            packages: json.result.0,
            store_path: Some(store_path),
        })
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
    #[must_use = "don't discard the store path of built environments"]
    fn transact_with_manifest_contents(
        &mut self,
        manifest_contents: impl AsRef<str>,
        flox: &Flox,
    ) -> Result<PathBuf, CoreEnvironmentError> {
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
        let store_path = temp_env.build(flox)?;

        debug!("transaction: replacing environment");
        self.replace_with(temp_env)?;
        Ok(store_path)
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
    #[must_use = "don't discard the store path of built environments"]
    fn transact_with_lockfile_contents(
        &mut self,
        lockfile_contents: impl AsRef<str>,
        flox: &Flox,
    ) -> Result<PathBuf, CoreEnvironmentError> {
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
        let store_path = temp_env.build(flox)?;

        debug!("transaction: replacing environment");
        self.replace_with(temp_env)?;
        Ok(store_path)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditResult {
    /// The manifest was not modified.
    Unchanged,
    /// The manifest was modified, and the user needs to re-activate it.
    ReActivateRequired { store_path: Option<PathBuf> },
    /// The manifest was modified, but the user does not need to re-activate it.
    Success { store_path: Option<PathBuf> },
}

impl EditResult {
    pub fn new(
        old_manifest: &str,
        new_manifest: &str,
        store_path: Option<PathBuf>,
    ) -> Result<Self, CoreEnvironmentError> {
        if old_manifest == new_manifest {
            Ok(Self::Unchanged)
        } else {
            // todo: use a single toml crate (toml_edit already implements serde traits)
            // TODO: use different error variants, users _can_ fix errors in the _new_ manifest
            //       but they _can't_ fix errors in the _old_ manifest
            let old_manifest: Manifest =
                toml::from_str(old_manifest).map_err(CoreEnvironmentError::DeserializeManifest)?;
            let new_manifest: Manifest =
                toml::from_str(new_manifest).map_err(CoreEnvironmentError::DeserializeManifest)?;
            // TODO: some modifications to `install` currently require re-activation
            if old_manifest.hook != new_manifest.hook || old_manifest.vars != new_manifest.vars {
                Ok(Self::ReActivateRequired { store_path })
            } else {
                Ok(Self::Success { store_path })
            }
        }
    }

    pub fn store_path(&self) -> Option<PathBuf> {
        match self {
            EditResult::Unchanged => None,
            EditResult::ReActivateRequired { store_path } => store_path.clone(),
            EditResult::Success { store_path } => store_path.clone(),
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
    #[error(transparent)]
    LockedManifest(LockedManifestError),

    #[error(transparent)]
    BadLockfilePath(CanonicalizeError),

    // todo: refactor upgrade to use `LockedManifest`
    #[error("unexpected output from pkgdb upgrade")]
    ParseUpgradeOutput(#[source] serde_json::Error),
    #[error("failed to upgrade environment")]
    UpgradeFailed(#[source] CallPkgDbError),
    // endregion

    // endregion
    #[error("unsupported system to build container: {0}")]
    ContainerizeUnsupportedSystem(String),
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
            .link(&flox, env_path.path().with_extension("out-link"), &None)
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
