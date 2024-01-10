//! The directory structure for a path environment looks like this:
//!
//! ```ignore
//! .flox/
//!     ENVIRONMENT_POINTER_FILENAME
//!     ENVIRONMENT_DIR_NAME/
//!         MANIFEST_FILENAME
//!         LOCKFILE_FILENAME
//!     PATH_ENV_GCROOTS_DIR_NAME/
//!         $system.$name (out link)
//! ```
//!
//! `ENVIRONMENT_DIR_NAME` contains the environment definition
//! and is modified using [CoreEnvironment].

use std::ffi::OsStr;
use std::fs::{self};
use std::path::{Path, PathBuf};

use flox_types::catalog::System;
use log::debug;

use super::core_environment::CoreEnvironment;
use super::{
    copy_dir_recursive,
    CanonicalPath,
    EditResult,
    Environment,
    EnvironmentError2,
    EnvironmentPointer,
    InstallationAttempt,
    PathPointer,
    UpdateResult,
    DOT_FLOX,
    ENVIRONMENT_POINTER_FILENAME,
    GCROOTS_DIR_NAME,
    LOCKFILE_FILENAME,
};
use crate::flox::Flox;
use crate::models::environment::{ENV_DIR_NAME, FLOX_SYSTEM_PLACEHOLDER, MANIFEST_FILENAME};
use crate::models::environment_ref::EnvironmentName;
use crate::models::lockfile::LockedManifest;
use crate::models::manifest::PackageToInstall;
use crate::models::pkgdb::UpgradeResult;
use crate::utils::mtime_of;

/// Struct representing a local environment
///
/// This environment performs transactional edits by first copying the environment
/// to a temporary directory, making changes there, and attempting to build the
/// environment. If the build succeeds, the edit is considered a success and the
/// original environment contents are overwritten with the contents of the temporary
/// directory.
///
/// The transaction status is captured via the `state` field.
#[derive(Debug)]
pub struct PathEnvironment {
    /// Absolute path to the environment, typically `<...>/.flox`
    pub path: CanonicalPath,

    /// The temporary directory that this environment will use during transactions
    pub temp_dir: PathBuf,

    /// The associated [PathPointer] of this environment.
    ///
    /// Used to identify the environment.
    pub pointer: PathPointer,
}

impl PartialEq for PathEnvironment {
    fn eq(&self, other: &Self) -> bool {
        *self.path == *other.path
    }
}

impl PathEnvironment {
    pub fn new(
        dot_flox_path: impl AsRef<Path>,
        pointer: PathPointer,
        temp_dir: impl AsRef<Path>,
    ) -> Result<Self, EnvironmentError2> {
        let dot_flox_path = CanonicalPath::new(dot_flox_path)?;

        if &*dot_flox_path == Path::new("/") {
            return Err(EnvironmentError2::InvalidPath(
                dot_flox_path.into_path_buf(),
            ));
        }

        let env_path = dot_flox_path.join(ENV_DIR_NAME);
        if !env_path.exists() {
            Err(EnvironmentError2::EnvNotFound)?;
        }

        if !env_path.join(MANIFEST_FILENAME).exists() {
            Err(EnvironmentError2::ManifestNotFound)?
        }

        Ok(Self {
            // path must be absolute as it is used to set FLOX_ENV
            path: dot_flox_path,
            pointer,
            temp_dir: temp_dir.as_ref().to_path_buf(),
        })
    }

    /// Where to link a built environment to. The path may not exist if the environment has
    /// never been built.
    ///
    /// The existence of this path guarantees exactly two things:
    /// - The environment was built at some point in the past.
    /// - The environment can be activated.
    ///
    /// The existence of this path explicitly _does not_ guarantee that the current
    /// state of the environment is "buildable". The environment may have been modified
    /// since it was last built and therefore may no longer build. Thus, the presence of
    /// this path doesn't guarantee that the current environment can be built,
    /// just that it built at some point in the past.
    fn out_link(&self, system: &System) -> Result<PathBuf, EnvironmentError2> {
        let run_dir = self.path.join(GCROOTS_DIR_NAME);
        if !run_dir.exists() {
            std::fs::create_dir_all(&run_dir).map_err(EnvironmentError2::CreateGcRootDir)?;
        }
        Ok(run_dir.join([system.clone(), self.name().to_string()].join(".")))
    }

    /// Get a view of the environment that can be used to perform operations
    /// on the environment without side effects.
    ///
    /// This method should only be used to create [CoreEnvironment]s for a [PathEnvironment].
    /// To modify the environment, use the [PathEnvironment] methods instead.
    pub(super) fn into_core_environment(self) -> CoreEnvironment {
        CoreEnvironment::new(self.path.join(ENV_DIR_NAME))
    }
}

impl Environment for PathEnvironment {
    /// Build the environment with side effects:
    ///
    /// - Create a result link as gc-root.
    /// - Create a lockfile if one doesn't already exist, updating it with
    ///   any new packages.
    fn build(&mut self, flox: &Flox) -> Result<(), EnvironmentError2> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        env_view.build(flox)?;
        env_view.link(flox, self.out_link(&flox.system)?)?;

        Ok(())
    }

    fn lock(&mut self, flox: &Flox) -> Result<LockedManifest, EnvironmentError2> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        Ok(env_view.lock(flox)?)
    }

    fn build_container(
        &mut self,
        flox: &Flox,
        sink: &mut dyn Write,
    ) -> Result<(), EnvironmentError2> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        env_view.build_container(flox, sink)?;
        Ok(())
    }

    /// Install packages to the environment atomically
    ///
    /// Returns the new manifest content if the environment was modified. Also
    /// returns a map of the packages that were already installed. The installation
    /// will proceed if at least one of the requested packages were added to the
    /// manifest.
    ///
    /// Todo: remove async
    fn install(
        &mut self,
        packages: &[PackageToInstall],
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError2> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let result = env_view.install(packages, flox)?;
        env_view.link(flox, self.out_link(&flox.system)?)?;

        Ok(result)
    }

    /// Uninstall packages from the environment atomically
    ///
    /// Returns true if the environment was modified and false otherwise.
    /// TODO: this should return a list of packages that were actually
    /// uninstalled rather than a bool.
    fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<String, EnvironmentError2> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let result = env_view.uninstall(packages, flox)?;
        env_view.link(flox, self.out_link(&flox.system)?)?;

        Ok(result)
    }

    /// Atomically edit this environment, ensuring that it still builds
    fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError2> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let result = env_view.edit(flox, contents)?;
        env_view.link(flox, self.out_link(&flox.system)?)?;

        Ok(result)
    }

    /// Atomically update this environment's inputs
    fn update(
        &mut self,
        flox: &Flox,
        inputs: Vec<String>,
    ) -> Result<UpdateResult, EnvironmentError2> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let result = env_view.update(flox, inputs)?;
        env_view.link(flox, self.out_link(&flox.system)?)?;

        Ok(result)
    }

    /// Atomically upgrade packages in this environment
    fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[String],
    ) -> Result<UpgradeResult, EnvironmentError2> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let result = env_view.upgrade(flox, groups_or_iids)?;
        env_view.link(flox, self.out_link(&flox.system)?)?;

        Ok(result)
    }

    /// Read the environment definition file as a string
    fn manifest_content(&self, flox: &Flox) -> Result<String, EnvironmentError2> {
        fs::read_to_string(self.manifest_path(flox)?).map_err(EnvironmentError2::ReadManifest)
    }

    /// Returns the environment name
    fn name(&self) -> EnvironmentName {
        self.pointer.name.clone()
    }

    /// Delete the Environment
    fn delete(self, _flox: &Flox) -> Result<(), EnvironmentError2> {
        let dot_flox = &self.path;
        if Some(OsStr::new(".flox")) == dot_flox.file_name() {
            std::fs::remove_dir_all(dot_flox).map_err(EnvironmentError2::DeleteEnvironment)?;
        } else {
            return Err(EnvironmentError2::DotFloxNotFound);
        }
        Ok(())
    }

    fn activation_path(&mut self, flox: &Flox) -> Result<PathBuf, EnvironmentError2> {
        let out_link = self.out_link(&flox.system)?;

        if self.needs_rebuild(flox)? {
            self.build(flox)?;
        }

        Ok(out_link)
    }

    /// Path to the environment's parent directory
    fn parent_path(&self) -> Result<PathBuf, EnvironmentError2> {
        let mut path = self.path.to_path_buf();
        if path.pop() {
            Ok(path)
        } else {
            Err(EnvironmentError2::InvalidPath(path))
        }
    }

    /// Path to the environment definition file
    fn manifest_path(&self, _flox: &Flox) -> Result<PathBuf, EnvironmentError2> {
        Ok(self.path.join(ENV_DIR_NAME).join(MANIFEST_FILENAME))
    }

    /// Path to the lockfile. The path may not exist.
    fn lockfile_path(&self, _flox: &Flox) -> Result<PathBuf, EnvironmentError2> {
        Ok(self.path.join(ENV_DIR_NAME).join(LOCKFILE_FILENAME))
    }
}

/// Constructors of PathEnvironments
impl PathEnvironment {
    /// Open an environment at a given path
    ///
    /// Ensure that the path exists and contains files that "look" like an environment
    pub fn open(
        pointer: PathPointer,
        dot_flox_path: impl AsRef<Path>,
        temp_dir: impl AsRef<Path>,
    ) -> Result<Self, EnvironmentError2> {
        let dot_flox = dot_flox_path.as_ref();
        log::debug!("attempting to open .flox directory: {}", dot_flox.display());
        if !dot_flox.exists() {
            Err(EnvironmentError2::DotFloxNotFound)?;
        }

        PathEnvironment::new(dot_flox, pointer, temp_dir)
    }

    /// Create a new env in a `.flox` directory within a specific path or open it if it exists.
    ///
    /// The method creates or opens a `.flox` directory _contained_ within `path`!
    pub fn init(
        pointer: PathPointer,
        dot_flox_parent_path: impl AsRef<Path>,
        temp_dir: impl AsRef<Path>,
        system: impl AsRef<str>,
    ) -> Result<Self, EnvironmentError2> {
        let system: &str = system.as_ref();
        match EnvironmentPointer::open(dot_flox_parent_path.as_ref()) {
            Err(EnvironmentError2::EnvNotFound) => {},
            Err(e) => Err(e)?,
            Ok(_) => Err(EnvironmentError2::EnvironmentExists(
                dot_flox_parent_path.as_ref().to_path_buf(),
            ))?,
        }
        let dot_flox_path = dot_flox_parent_path.as_ref().join(DOT_FLOX);
        let env_dir = dot_flox_path.join(ENV_DIR_NAME);
        debug!("creating env dir: {}", env_dir.display());
        std::fs::create_dir_all(&env_dir).map_err(EnvironmentError2::InitEnv)?;
        let pointer_content =
            serde_json::to_string_pretty(&pointer).map_err(EnvironmentError2::SerializeEnvJson)?;
        let template_path = env!("FLOX_ENV_TEMPLATE");
        debug!(
            "copying environment template from {} to {}",
            template_path,
            env_dir.display()
        );
        copy_dir_recursive(&template_path, &env_dir, false).map_err(EnvironmentError2::InitEnv)?;
        let manifest_path = env_dir.join(MANIFEST_FILENAME);
        debug!(
            "replacing placeholder system in manifest: path={}, system={}",
            manifest_path.display(),
            system
        );
        let contents = fs::read_to_string(&manifest_path).map_err(EnvironmentError2::ManifestEdit);
        if let Err(e) = contents {
            debug!("couldn't open manifest to replace placeholder system");
            fs::remove_dir_all(&env_dir).map_err(EnvironmentError2::ManifestEdit)?;
            return Err(e);
        }
        let contents = contents.unwrap();
        let replaced = contents.replace(FLOX_SYSTEM_PLACEHOLDER, system);
        debug!(
            "manifest was updated successfully: {}",
            contents != replaced
        );
        let write_res =
            fs::write(&manifest_path, replaced).map_err(EnvironmentError2::ManifestEdit);
        if let Err(e) = write_res {
            debug!("overwriting manifest did not complete successfully");
            fs::remove_dir_all(&env_dir).map_err(EnvironmentError2::InitEnv)?;
            return Err(e);
        }

        // Write the `env.json` file
        if let Err(e) = fs::write(
            dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME),
            pointer_content,
        ) {
            fs::remove_dir_all(env_dir).map_err(EnvironmentError2::InitEnv)?;
            Err(EnvironmentError2::WriteEnvJson(e))?;
        }

        // write "run" >> .flox/.gitignore
        fs::write(dot_flox_path.join(".gitignore"), "run/\n")
            .map_err(EnvironmentError2::WriteGitignore)?;

        Self::open(pointer, dot_flox_path, temp_dir)
    }

    /// Determine if the environment needs to be rebuilt
    /// based on the modification times of the manifest and the out link
    ///
    /// If the manifest was modified after the out link was set,
    /// the environment needs to be rebuilt.
    ///
    /// This is a heuristic to avoid rebuilding the environment when it is not necessary.
    /// However, it is not perfect.
    /// For example,
    /// if the manifest is modified through as a whole idempotent git operations
    ///   e.g. from branch `a`
    ///   `git switch b; git switch a;`
    /// or the manifest was reformatted,
    /// the modification time of the manifest will change triggering a rebuild although nothing changed.
    ///
    /// Similarly, if any adjacent files are modified, the environment will not be rebuilt.
    fn needs_rebuild(&self, flox: &Flox) -> Result<bool, EnvironmentError2> {
        let manifest_modified_at = mtime_of(self.manifest_path(flox)?);
        let out_link_modified_at = mtime_of(self.out_link(&flox.system)?);

        debug!(
            "manifest_modified_at: {manifest_modified_at:?},
             out_link_modified_at: {out_link_modified_at:?}"
        );

        Ok(manifest_modified_at >= out_link_modified_at)
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::flox::tests::flox_instance;

    #[test]
    fn create_env() {
        let (flox, temp_dir) = flox_instance();
        let environment_temp_dir = tempfile::tempdir_in(&temp_dir).unwrap();
        let pointer = PathPointer::new("test".parse().unwrap());

        let before = PathEnvironment::open(
            pointer.clone(),
            environment_temp_dir.path(),
            temp_dir.path(),
        );

        assert!(
            matches!(before, Err(EnvironmentError2::EnvNotFound)),
            "{before:?}"
        );

        let actual = PathEnvironment::init(
            pointer,
            environment_temp_dir.path(),
            temp_dir.path(),
            &flox.system,
        )
        .unwrap();

        let expected = PathEnvironment::new(
            environment_temp_dir.into_path().join(".flox"),
            PathPointer::new("test".parse().unwrap()),
            temp_dir.path(),
        )
        .unwrap();

        assert_eq!(actual, expected);

        assert!(
            actual.manifest_path(&flox).unwrap().exists(),
            "manifest exists"
        );
        assert!(actual.path.is_absolute());
    }

    /// Write a manifest file with invalid toml to ensure we can catch
    #[test]
    fn cache_activation_path() {
        let (flox, temp_dir) = flox_instance();

        let environment_temp_dir = tempfile::tempdir_in(&temp_dir).unwrap();
        let pointer = PathPointer::new("test".parse().unwrap());

        let mut env = PathEnvironment::init(
            pointer,
            environment_temp_dir.path(),
            temp_dir.path(),
            &flox.system,
        )
        .unwrap();

        assert!(env.needs_rebuild(&flox).unwrap());

        // build the environment -> out link is created -> no rebuild necessary
        env.build(&flox).unwrap();
        assert!(!env.needs_rebuild(&flox).unwrap());

        // "modify" the manifest -> rebuild necessary
        // TODO: there will be better methods to explicitly set mtime when we upgrade to rust >= 1.75.0
        let file = fs::write(env.manifest_path(&flox).unwrap(), "");
        drop(file);
        assert!(env.needs_rebuild(&flox).unwrap());
    }
}
