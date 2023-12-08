use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use log::debug;

use super::{
    copy_dir_recursive,
    EditResult,
    EnvironmentError2,
    InstallationAttempt,
    LockedManifest,
    LOCKFILE_FILENAME,
    MANIFEST_FILENAME,
};
use crate::flox::Flox;
use crate::models::environment::{global_manifest_path, ENV_BUILDER_BIN};
use crate::models::manifest::{insert_packages, remove_packages};
use crate::models::search::PKGDB_BIN;

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
    fn manifest_path(&self) -> PathBuf {
        self.env_dir.join(MANIFEST_FILENAME)
    }

    /// Get the path to the lockfile
    ///
    /// Note: may not exist
    fn lockfile_path(&self) -> PathBuf {
        self.env_dir.join(LOCKFILE_FILENAME)
    }

    /// Read the manifest file
    fn manifest_content(&self) -> Result<String, EnvironmentError2> {
        fs::read_to_string(self.manifest_path()).map_err(EnvironmentError2::ReadManifest)
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
    pub fn lock(&mut self, flox: &Flox) -> Result<LockedManifest, EnvironmentError2> {
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
            .map_err(EnvironmentError2::WriteLockfile)?;

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
    pub fn build(&mut self, flox: &Flox) -> Result<PathBuf, EnvironmentError2> {
        let lockfile = self.lock(flox)?;

        debug!(
            "building environment: system={}, lockfilePath={}",
            &flox.system,
            self.lockfile_path().display()
        );

        let store_path = lockfile.build(Path::new(&*ENV_BUILDER_BIN), None)?;

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
    ) -> Result<(), EnvironmentError2> {
        let lockfile = self.lock(flox)?;
        debug!(
            "linking environment: system={}, lockfilePath={}, outLinkPath={}",
            &flox.system,
            self.lockfile_path().display(),
            out_link_path.as_ref().display()
        );
        lockfile.build(Path::new(&*ENV_BUILDER_BIN), Some(out_link_path.as_ref()))?;

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
    ) -> Result<InstallationAttempt, EnvironmentError2> {
        let current_manifest_contents = self.manifest_content()?;
        let installation =
            insert_packages(&current_manifest_contents, &packages).map(|insertion| {
                InstallationAttempt {
                    new_manifest: insertion.new_toml.map(|toml| toml.to_string()),
                    already_installed: insertion.already_installed,
                }
            })?;
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
    ) -> Result<String, EnvironmentError2> {
        let current_manifest_contents = self.manifest_content()?;
        let toml = remove_packages(&current_manifest_contents, &packages)?;
        self.transact_with_manifest_contents(toml.to_string(), flox)?;
        Ok(toml.to_string())
    }

    /// Atomically edit this environment, ensuring that it still builds
    pub fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError2> {
        let old_contents = self.manifest_content()?;
        // TODO we should probably skip this if the manifest hasn't changed
        self.transact_with_manifest_contents(&contents, flox)?;

        EditResult::new(&old_contents, &contents)
    }

    /// Makes a temporary copy of the environment so modifications to the manifest
    /// can be applied without modifying the original environment.
    fn writable(
        &mut self,
        tempdir: impl AsRef<Path>,
    ) -> Result<CoreEnvironment<ReadWrite>, EnvironmentError2> {
        copy_dir_recursive(&self.env_dir, &tempdir.as_ref(), true)
            .map_err(EnvironmentError2::MakeTemporaryEnv)?;

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
    ) -> Result<(), EnvironmentError2> {
        let transaction_backup = self.env_dir.with_extension("tmp");

        if transaction_backup.exists() {
            debug!(
                "transaction backup exists: {}",
                transaction_backup.display()
            );
            return Err(EnvironmentError2::PriorTransaction(transaction_backup));
        }
        debug!(
            "backing up env: from={}, to={}",
            self.env_dir.display(),
            transaction_backup.display()
        );
        fs::rename(&self.env_dir, &transaction_backup)
            .map_err(EnvironmentError2::BackupTransaction)?;
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
            fs::remove_dir_all(&self.env_dir).map_err(EnvironmentError2::AbortTransaction)?;
            fs::rename(transaction_backup, &self.env_dir)
                .map_err(EnvironmentError2::AbortTransaction)?;
            return Err(EnvironmentError2::Move(err));
        }
        debug!("removing backup: path={}", transaction_backup.display());
        fs::remove_dir_all(transaction_backup).map_err(EnvironmentError2::RemoveBackup)?;
        Ok(())
    }

    /// Attempt to transactionally replace the manifest contents
    fn transact_with_manifest_contents(
        &mut self,
        manifest_contents: impl AsRef<str>,
        flox: &Flox,
    ) -> Result<(), EnvironmentError2> {
        let tempdir = tempfile::tempdir_in(&flox.temp_dir)
            .map_err(EnvironmentError2::MakeSandbox)?
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
}

/// A writable view of an environment directory
///
/// Typically within a temporary directory created by [CoreEnvironment::writable].
/// This is not public to enforce that environments are only edited atomically.
impl CoreEnvironment<ReadWrite> {
    /// Updates the environment manifest with the provided contents
    fn update_manifest(&mut self, contents: &impl AsRef<str>) -> Result<(), EnvironmentError2> {
        debug!("writing new manifest to {}", self.manifest_path().display());
        let mut manifest_file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(self.manifest_path())
            .map_err(EnvironmentError2::OpenManifest)?;
        manifest_file
            .write_all(contents.as_ref().as_bytes())
            .map_err(EnvironmentError2::UpdateManifest)?;
        Ok(())
    }
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

        assert!(matches!(err, EnvironmentError2::PriorTransaction(_)));
    }

    /// creating backup should fail if env is readonly
    #[test]
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
            matches!(err, EnvironmentError2::BackupTransaction(err) if err.kind() == std::io::ErrorKind::PermissionDenied)
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
