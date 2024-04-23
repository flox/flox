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
use std::io::Write;
use std::path::{Path, PathBuf};

use indoc::{formatdoc, indoc};
use log::debug;

use super::core_environment::CoreEnvironment;
use super::{
    CanonicalizeError,
    DotFlox,
    EditResult,
    Environment,
    EnvironmentError,
    EnvironmentPointer,
    InstallationAttempt,
    PathPointer,
    UninstallationAttempt,
    UpdateResult,
    CACHE_DIR_NAME,
    DOT_FLOX,
    ENVIRONMENT_POINTER_FILENAME,
    GCROOTS_DIR_NAME,
    LOCKFILE_FILENAME,
};
use crate::data::{CanonicalPath, System};
use crate::flox::Flox;
use crate::models::container_builder::ContainerBuilder;
use crate::models::env_registry::{deregister, ensure_registered};
use crate::models::environment::{
    ENV_DIR_NAME,
    FLOX_HOOK_PLACEHOLDER,
    FLOX_INSTALL_PLACEHOLDER,
    FLOX_PROFILE_PLACEHOLDER,
    FLOX_SYSTEM_PLACEHOLDER,
    MANIFEST_FILENAME,
};
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

/// A profile script or list of packages to install when initializing an environment
#[derive(Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct InitCustomization {
    pub profile: Option<String>,
    pub packages: Option<Vec<PackageToInstall>>,
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
    ) -> Result<Self, EnvironmentError> {
        let dot_flox_path =
            CanonicalPath::new(dot_flox_path).map_err(|CanonicalizeError { path, err }| {
                EnvironmentError::InvalidDotFlox {
                    path,
                    source: Box::new(err),
                }
            })?;

        if &*dot_flox_path == Path::new("/") {
            return Err(EnvironmentError::InvalidPath(dot_flox_path.into_path_buf()));
        }

        let env_path = dot_flox_path.join(ENV_DIR_NAME);
        if !env_path.exists() {
            Err(EnvironmentError::EnvDirNotFound)?;
        }

        if !env_path.join(MANIFEST_FILENAME).exists() {
            Err(EnvironmentError::ManifestNotFound)?
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
    fn out_link(&self, system: &System) -> Result<PathBuf, EnvironmentError> {
        let run_dir = self.path.join(GCROOTS_DIR_NAME);
        if !run_dir.exists() {
            std::fs::create_dir_all(&run_dir).map_err(EnvironmentError::CreateGcRootDir)?;
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

    pub fn rename(&mut self, new_name: EnvironmentName) -> Result<(), EnvironmentError> {
        self.pointer.name = new_name;
        let pointer_content = serde_json::to_string_pretty(&self.pointer)
            .map_err(EnvironmentError::SerializeEnvJson)?;

        let mut tempfile =
            tempfile::NamedTempFile::new_in(&self.path).map_err(EnvironmentError::WriteEnvJson)?;

        tempfile
            .write_all(pointer_content.as_bytes())
            .map_err(EnvironmentError::WriteEnvJson)?;

        tempfile
            .persist(self.path.join(ENVIRONMENT_POINTER_FILENAME))
            .map_err(|e| e.error)
            .map_err(EnvironmentError::WriteEnvJson)?;

        Ok(())
    }
}

impl Environment for PathEnvironment {
    /// Build the environment with side effects:
    ///
    /// - Create a result link as gc-root.
    /// - Create a lockfile if one doesn't already exist, updating it with
    ///   any new packages.
    fn build(&mut self, flox: &Flox) -> Result<(), EnvironmentError> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let store_path = env_view.build(flox)?;
        env_view.link(flox, self.out_link(&flox.system)?, &Some(store_path))?;

        Ok(())
    }

    fn lock(&mut self, flox: &Flox) -> Result<LockedManifest, EnvironmentError> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        Ok(env_view.lock(flox)?)
    }

    fn build_container(&mut self, flox: &Flox) -> Result<ContainerBuilder, EnvironmentError> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let builder = env_view.build_container(flox)?;
        Ok(builder)
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
    ) -> Result<InstallationAttempt, EnvironmentError> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let result = env_view.install(packages, flox)?;
        env_view.link(flox, self.out_link(&flox.system)?, &result.store_path)?;

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
    ) -> Result<UninstallationAttempt, EnvironmentError> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let result = env_view.uninstall(packages, flox)?;
        env_view.link(flox, self.out_link(&flox.system)?, &result.store_path)?;

        Ok(result)
    }

    /// Atomically edit this environment, ensuring that it still builds
    fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let result = env_view.edit(flox, contents)?;
        if result != EditResult::Unchanged {
            env_view.link(flox, self.out_link(&flox.system)?, &result.store_path())?;
        }
        Ok(result)
    }

    /// Atomically update this environment's inputs
    fn update(
        &mut self,
        flox: &Flox,
        inputs: Vec<String>,
    ) -> Result<UpdateResult, EnvironmentError> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let result = env_view.update(flox, inputs)?;
        env_view.link(flox, self.out_link(&flox.system)?, &result.store_path)?;

        Ok(result)
    }

    /// Atomically upgrade packages in this environment
    fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[String],
    ) -> Result<UpgradeResult, EnvironmentError> {
        let mut env_view = CoreEnvironment::new(self.path.join(ENV_DIR_NAME));
        let result = env_view.upgrade(flox, groups_or_iids)?;
        env_view.link(flox, self.out_link(&flox.system)?, &result.store_path)?;

        Ok(result)
    }

    /// Read the environment definition file as a string
    fn manifest_content(&self, flox: &Flox) -> Result<String, EnvironmentError> {
        fs::read_to_string(self.manifest_path(flox)?).map_err(EnvironmentError::ReadManifest)
    }

    /// Returns the environment name
    fn name(&self) -> EnvironmentName {
        self.pointer.name.clone()
    }

    /// Delete the Environment
    fn delete(self, flox: &Flox) -> Result<(), EnvironmentError> {
        let dot_flox = &self.path;
        if Some(OsStr::new(".flox")) == dot_flox.file_name() {
            std::fs::remove_dir_all(dot_flox).map_err(EnvironmentError::DeleteEnvironment)?;
        } else {
            return Err(EnvironmentError::DotFloxNotFound(
                self.path.parent().unwrap_or(&self.path).to_path_buf(),
            ));
        }
        deregister(flox, &self.path, &EnvironmentPointer::Path(self.pointer))?;
        Ok(())
    }

    fn activation_path(&mut self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        let out_link = self.out_link(&flox.system)?;

        if self.needs_rebuild(flox)? {
            self.build(flox)?;
        }

        Ok(out_link)
    }

    /// Returns .flox/cache
    fn cache_path(&self) -> Result<PathBuf, EnvironmentError> {
        let cache_dir = self.path.join(CACHE_DIR_NAME);
        if !cache_dir.exists() {
            std::fs::create_dir_all(&cache_dir).map_err(EnvironmentError::CreateCacheDir)?;
        }
        Ok(cache_dir)
    }

    /// Returns parent path of .flox
    fn project_path(&self) -> Result<PathBuf, EnvironmentError> {
        self.parent_path()
    }

    /// Path to the environment's parent directory
    fn parent_path(&self) -> Result<PathBuf, EnvironmentError> {
        let mut path = self.path.to_path_buf();
        if path.pop() {
            Ok(path)
        } else {
            Err(EnvironmentError::InvalidPath(path))
        }
    }

    /// Path to the environment definition file
    fn manifest_path(&self, _flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        Ok(self.path.join(ENV_DIR_NAME).join(MANIFEST_FILENAME))
    }

    /// Path to the lockfile. The path may not exist.
    fn lockfile_path(&self, _flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        Ok(self.path.join(ENV_DIR_NAME).join(LOCKFILE_FILENAME))
    }
}

/// Constructors of PathEnvironments
impl PathEnvironment {
    /// Open an environment at a given path
    ///
    /// Ensure that the path exists and contains files that "look" like an environment
    pub fn open(
        flox: &Flox,
        pointer: PathPointer,
        dot_flox_path: impl AsRef<Path>,
        temp_dir: impl AsRef<Path>,
    ) -> Result<Self, EnvironmentError> {
        let dot_flox = dot_flox_path.as_ref();
        log::debug!("attempting to open .flox directory: {}", dot_flox.display());
        if !dot_flox.exists() {
            Err(EnvironmentError::DotFloxNotFound(
                dot_flox.parent().unwrap_or(dot_flox).to_path_buf(),
            ))?;
        }

        let canonical_dot_flox =
            CanonicalPath::new(&dot_flox_path).map_err(EnvironmentError::CanonicalDotFlox)?;

        ensure_registered(
            flox,
            &canonical_dot_flox,
            &EnvironmentPointer::Path(pointer.clone()),
        )?;

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
        customization: &InitCustomization,
        flox: &Flox,
    ) -> Result<Self, EnvironmentError> {
        let system: &str = system.as_ref();

        // Ensure that the .flox directory does not already exist
        match DotFlox::open_default_in(dot_flox_parent_path.as_ref()) {
            // continue if the .flox directory does not exist, as it's being created by this method
            Err(EnvironmentError::DotFloxNotFound(_)) => {},
            // propagate any other error signalling a faulty .flox directory
            Err(e) => Err(e)?,
            // .flox directory exists, so we can't create a new environment here
            Ok(_) => Err(EnvironmentError::EnvironmentExists(
                dot_flox_parent_path.as_ref().to_path_buf(),
            ))?,
        }

        let template_contents = fs::read_to_string(env!("MANIFEST_TEMPLATE"))
            .map_err(EnvironmentError::ReadManifest)?;

        let manifest_contents =
            Self::replace_placeholders(&template_contents, system, customization);

        let mut environment = Self::write_new(
            flox,
            pointer,
            dot_flox_parent_path,
            temp_dir,
            &manifest_contents,
        )?;

        if let Some(ref packages) = customization.packages {
            // Ignore the result, because we know there can't be packages already installed
            // TODO: once we use toml_edit for replace_placeholders,
            // we should add packages using insert_packages() in replace_placeholders
            // and then do a build instead of calling `install`.
            environment.install(packages, flox)?;
        }

        Ok(environment)
    }

    /// Write files for a [PathEnvironment] to `dot_flox_parent_path` unchecked.
    ///
    /// * write the .flox directory
    /// * write the environment pointer to `.flox/env.json`
    /// * write the manifest to `.flox/env/manifest.toml`
    ///
    /// Note: The directory and the written environment are **not verified**.
    ///       This function may override any existing env,
    ///       or write nonsense content to the manifest.
    ///       [PathEnvironment::init] implements the relevant checks
    ///       to make this safe in practice.
    ///
    /// This functionality is shared between [PathEnvironment::init] and tests.
    fn write_new(
        flox: &Flox,
        pointer: PathPointer,
        dot_flox_parent_path: impl AsRef<Path>,
        temp_dir: impl AsRef<Path>,
        manifest_contents: &str,
    ) -> Result<Self, EnvironmentError> {
        let dot_flox_path = dot_flox_parent_path.as_ref().join(DOT_FLOX);
        let env_dir = dot_flox_path.join(ENV_DIR_NAME);
        let manifest_path = env_dir.join(MANIFEST_FILENAME);
        debug!("creating env dir: {}", env_dir.display());
        std::fs::create_dir_all(&env_dir).map_err(EnvironmentError::InitEnv)?;

        // Write the `env.json` file
        let pointer_content =
            serde_json::to_string_pretty(&pointer).map_err(EnvironmentError::SerializeEnvJson)?;
        if let Err(e) = fs::write(
            dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME),
            pointer_content,
        ) {
            fs::remove_dir_all(&env_dir).map_err(EnvironmentError::InitEnv)?;
            Err(EnvironmentError::WriteEnvJson(e))?;
        }

        // Write `manifest.toml`
        let write_res =
            fs::write(manifest_path, manifest_contents).map_err(EnvironmentError::WriteManifest);
        if let Err(e) = write_res {
            debug!("writing manifest did not complete successfully");
            fs::remove_dir_all(&env_dir).map_err(EnvironmentError::InitEnv)?;
            return Err(e);
        }

        // write "run" and "cache" to .flox/.gitignore
        fs::write(dot_flox_path.join(".gitignore"), formatdoc! {"
            {GCROOTS_DIR_NAME}/
            {CACHE_DIR_NAME}/
            "})
        .map_err(EnvironmentError::WriteGitignore)?;

        Self::open(flox, pointer, dot_flox_path, temp_dir)
    }

    /// Replace all placeholders in the manifest file contents
    ///
    /// TODO: we should probably be using toml_edit
    fn replace_placeholders(
        contents: &String,
        system: &str,
        customization: &InitCustomization,
    ) -> String {
        // Replace system
        let mut replaced = contents.replace(FLOX_SYSTEM_PLACEHOLDER, system);

        // Don't add example packages if packages are being installed
        let packages = if customization.packages.is_some() {
            ""
        } else {
            // The install method adds a newline, so add one here as well
            indoc! {r#"
            # hello.pkg-path = "hello"
            # nodejs = { version = "^18.4.2", pkg-path = "nodejs_18" }
        "#}
        };
        replaced = replaced.replace(FLOX_INSTALL_PLACEHOLDER, packages);

        // Replace profile
        let profile = if let Some(ref custom_profile) = customization.profile {
            formatdoc! {r#"
                common = """
                {}
                """"#, indent::indent_all_by(2, custom_profile)}
        } else {
            formatdoc! {r#"
                # common = """
                #   echo "it's gettin flox in here";
                # """"#}
        };
        replaced = replaced.replace(FLOX_PROFILE_PLACEHOLDER, &profile);

        // Replace the hook
        let default_hook = formatdoc! {r#"
            # on-activate = """
            #     mkdir my_data_dir
            # """"#};

        let replaced = replaced.replace(FLOX_HOOK_PLACEHOLDER, &default_hook);

        debug!(
            "manifest was updated successfully: {}",
            contents != &replaced
        );

        replaced
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
    fn needs_rebuild(&self, flox: &Flox) -> Result<bool, EnvironmentError> {
        let manifest_modified_at = mtime_of(self.manifest_path(flox)?);
        let out_link_modified_at = mtime_of(self.out_link(&flox.system)?);

        debug!(
            "manifest_modified_at: {manifest_modified_at:?},
             out_link_modified_at: {out_link_modified_at:?}"
        );

        Ok(manifest_modified_at >= out_link_modified_at)
    }
}

pub mod test_helpers {
    use tempfile::tempdir_in;

    use super::*;

    pub fn new_path_environment(flox: &Flox, contents: &str) -> PathEnvironment {
        let pointer = PathPointer::new("name".parse().unwrap());
        PathEnvironment::write_new(
            flox,
            pointer,
            tempdir_in(&flox.temp_dir).unwrap().into_path(),
            &flox.temp_dir,
            contents,
        )
        .unwrap()
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::env_registry::{env_registry_path, read_environment_registry};

    #[test]
    fn create_env() {
        let (flox, temp_dir) = flox_instance();
        let environment_temp_dir = tempfile::tempdir_in(&temp_dir).unwrap();
        let pointer = PathPointer::new("test".parse().unwrap());

        let before = PathEnvironment::open(
            &flox,
            pointer.clone(),
            environment_temp_dir.path(),
            temp_dir.path(),
        );

        assert!(
            matches!(before, Err(EnvironmentError::EnvDirNotFound)),
            "{before:?}"
        );

        let actual = PathEnvironment::init(
            pointer,
            environment_temp_dir.path(),
            temp_dir.path(),
            &flox.system,
            &InitCustomization::default(),
            &flox,
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
            &InitCustomization::default(),
            &flox,
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

    #[test]
    fn registers_on_init() {
        let (flox, tmp_dir) = flox_instance();
        let environment_temp_dir = tempfile::tempdir_in(&tmp_dir).unwrap();
        let ptr = PathPointer::new("test".parse().unwrap());
        let _env = PathEnvironment::init(
            ptr,
            environment_temp_dir.path(),
            tmp_dir.path(),
            &flox.system,
            &InitCustomization::default(),
            &flox,
        )
        .unwrap();
        let reg_path = env_registry_path(&flox);
        assert!(reg_path.exists());
        let reg = read_environment_registry(&reg_path).unwrap().unwrap();
        assert!(matches!(
            reg.entries[0].envs[0].pointer,
            EnvironmentPointer::Path(_)
        ));
    }

    #[test]
    fn registers_on_open() {
        let (flox, tmp_dir) = flox_instance();
        let environment_temp_dir = tempfile::tempdir_in(&tmp_dir).unwrap();
        // Create an environment so that the .flox directory is populated and we can open it later
        let ptr = PathPointer::new("test".parse().unwrap());
        let env = PathEnvironment::init(
            ptr.clone(),
            environment_temp_dir.path(),
            tmp_dir.path(),
            &flox.system,
            &InitCustomization::default(),
            &flox,
        )
        .unwrap();
        let reg_path = env_registry_path(&flox);
        assert!(reg_path.exists());
        // Delete the registry so we can confirm that opening the environment creates it
        std::fs::remove_file(&reg_path).unwrap();
        let _env = PathEnvironment::open(&flox, ptr, env.path, tmp_dir.path()).unwrap();
        let reg = read_environment_registry(&reg_path).unwrap().unwrap();
        assert!(matches!(
            reg.entries[0].envs[0].pointer,
            EnvironmentPointer::Path(_)
        ));
    }

    #[test]
    fn deregisters_on_delete() {
        let (flox, tmp_dir) = flox_instance();
        let environment_temp_dir = tempfile::tempdir_in(&tmp_dir).unwrap();
        // Create an environment so that the .flox directory is populated and we can open it later
        let ptr = PathPointer::new("test".parse().unwrap());
        let env = PathEnvironment::init(
            ptr.clone(),
            environment_temp_dir.path(),
            tmp_dir.path(),
            &flox.system,
            &InitCustomization::default(),
            &flox,
        )
        .unwrap();
        let reg_path = env_registry_path(&flox);
        assert!(reg_path.exists());
        env.delete(&flox).unwrap();
        assert!(reg_path.exists());
        let reg = read_environment_registry(&reg_path).unwrap().unwrap();
        assert!(reg.entries.is_empty());
    }
}
