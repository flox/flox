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

use itertools::Itertools;
use tracing::debug;

use super::core_environment::{CoreEnvironment, UpgradeResult};
use super::fetcher::IncludeFetcher;
use super::{
    CACHE_DIR_NAME,
    DOT_FLOX,
    DotFlox,
    ENVIRONMENT_POINTER_FILENAME,
    EditResult,
    Environment,
    EnvironmentError,
    EnvironmentPointer,
    GCROOTS_DIR_NAME,
    InstallationAttempt,
    LOCKFILE_FILENAME,
    LOG_DIR_NAME,
    PathPointer,
    RenderedEnvironmentLinks,
    UninstallationAttempt,
    path_hash,
    services_socket_path,
};
use crate::data::{CanonicalPath, System};
use crate::flox::Flox;
use crate::models::env_registry::{deregister, ensure_registered};
use crate::models::environment::{ENV_DIR_NAME, MANIFEST_FILENAME, create_dot_flox_gitignore};
use crate::models::environment_ref::EnvironmentName;
use crate::models::lockfile::{DEFAULT_SYSTEMS_STR, LockResult, Lockfile};
use crate::models::manifest::raw::{CatalogPackage, PackageToInstall, RawManifest};
use crate::models::manifest::typed::ActivateMode;
use crate::providers::buildenv::BuildEnvOutputs;

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

    /// The associated [PathPointer] of this environment.
    ///
    /// Used to identify the environment.
    pub pointer: PathPointer,

    /// The rendered environment links for this environment.
    /// These may not yet exist if the environment has not been built.
    rendered_env_links: RenderedEnvironmentLinks,
}

/// A profile script or list of packages to install when initializing an environment
#[derive(Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct InitCustomization {
    pub hook_on_activate: Option<String>,
    pub profile_common: Option<String>,
    pub profile_bash: Option<String>,
    pub profile_fish: Option<String>,
    pub profile_tcsh: Option<String>,
    pub profile_zsh: Option<String>,
    pub packages: Option<Vec<CatalogPackage>>,
    pub activate_mode: Option<ActivateMode>,
}

impl PartialEq for PathEnvironment {
    fn eq(&self, other: &Self) -> bool {
        *self.path == *other.path
    }
}

impl PathEnvironment {
    pub fn new(
        pointer: PathPointer,
        dot_flox_path: CanonicalPath,
        system: &System,
    ) -> Result<Self, EnvironmentError> {
        if &*dot_flox_path == Path::new("/") {
            return Err(EnvironmentError::InvalidPath(dot_flox_path.into_inner()));
        }

        let env_path = dot_flox_path.join(ENV_DIR_NAME);
        if !env_path.exists() {
            Err(EnvironmentError::EnvDirNotFound)?;
        }

        if !env_path.join(MANIFEST_FILENAME).exists() {
            Err(EnvironmentError::ManifestNotFound)?
        }

        let rendered_env_links = {
            let run_dir = dot_flox_path.join(GCROOTS_DIR_NAME);
            if !run_dir.exists() {
                std::fs::create_dir_all(&run_dir).map_err(EnvironmentError::CreateGcRootDir)?;
            }

            let base_dir = CanonicalPath::new(run_dir).expect("run dir is checked to exist");

            RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system(
                &base_dir,
                pointer.name.as_ref(),
                system,
            )
        };

        Ok(Self {
            // path must be absolute as it is used to set FLOX_ENV
            path: dot_flox_path,
            pointer,
            rendered_env_links,
        })
    }

    fn include_fetcher(&self) -> Result<IncludeFetcher, EnvironmentError> {
        Ok(IncludeFetcher {
            base_directory: Some(self.parent_path()?),
        })
    }

    /// Get a view of the environment that can be used to perform operations
    /// on the environment without side effects.
    ///
    /// This method should only be used to create [CoreEnvironment]s for a [PathEnvironment].
    /// To modify the environment, use the [PathEnvironment] methods instead.
    pub(super) fn into_core_environment(self) -> Result<CoreEnvironment, EnvironmentError> {
        self.as_core_environment()
    }

    fn as_core_environment(&self) -> Result<CoreEnvironment, EnvironmentError> {
        Ok(CoreEnvironment::new(
            self.path.join(ENV_DIR_NAME),
            self.include_fetcher()?,
        ))
    }

    fn as_core_environment_mut(&mut self) -> Result<CoreEnvironment, EnvironmentError> {
        self.as_core_environment()
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

    /// Returns a unique identifier for the location of the environment.
    fn path_hash(&self) -> String {
        path_hash(&self.path)
    }
}

impl Environment for PathEnvironment {
    /// This will lock the environment if it is not already locked.
    fn lockfile(&mut self, flox: &Flox) -> Result<LockResult, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        env_view.ensure_locked(flox)
    }

    /// Returns the lockfile if it already exists.
    fn existing_lockfile(&self, _flox: &Flox) -> Result<Option<Lockfile>, EnvironmentError> {
        self.as_core_environment()?
            .existing_lockfile()
            .map_err(EnvironmentError::Core)
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
        let mut env_view = self.as_core_environment_mut()?;
        let result = env_view.install(packages, flox)?;
        if let Some(ref store_paths) = result.built_environments {
            self.link(flox, store_paths)?;
        }

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
        let mut env_view = self.as_core_environment_mut()?;
        let result = env_view.uninstall(packages, flox)?;
        if let Some(ref store_paths) = result.built_environment_store_paths {
            self.link(flox, store_paths)?;
        }

        Ok(result)
    }

    /// Atomically edit this environment, ensuring that it still builds
    fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let result = env_view.edit(flox, contents)?;
        match &result {
            EditResult::Changed {
                built_environment_store_paths,
                ..
            } => {
                self.link(flox, built_environment_store_paths)?;
            },
            EditResult::Unchanged => {},
        }
        Ok(result)
    }

    /// Upgrade packages in this environment and return the result, but do not
    fn dry_upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[&str],
    ) -> Result<UpgradeResult, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let result = env_view.upgrade(flox, groups_or_iids, false)?;
        Ok(result)
    }

    /// Atomically upgrade packages in this environment
    fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[&str],
    ) -> Result<UpgradeResult, EnvironmentError> {
        tracing::debug!(to_upgrade = groups_or_iids.join(","), "upgrading");
        let mut env_view = self.as_core_environment_mut()?;
        let result = env_view.upgrade(flox, groups_or_iids, true)?;
        if let Some(ref store_paths) = result.store_path {
            self.link(flox, store_paths)?;
        }

        Ok(result)
    }

    /// Upgrade environment with latest changes to included environments.
    fn include_upgrade(
        &mut self,
        flox: &Flox,
        to_upgrade: Vec<String>,
    ) -> Result<UpgradeResult, EnvironmentError> {
        tracing::debug!(
            includes = to_upgrade.iter().join(","),
            "upgrading included environments"
        );
        let mut env_view = self.as_core_environment_mut()?;
        let result = env_view.include_upgrade(flox, to_upgrade)?;
        if let Some(ref store_paths) = result.store_path {
            self.link(flox, store_paths)?;
        }

        Ok(result)
    }

    /// Extract the current content of the manifest
    ///
    /// This may differ from the locked manifest, which should typically be used unless you need to:
    /// - provide the latest editable contents to the user
    /// - avoid double-locking
    fn manifest_contents(&self, flox: &Flox) -> Result<String, EnvironmentError> {
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
            return Err(EnvironmentError::DotFloxNotFound(self.path.to_path_buf()));
        }
        deregister(flox, &self.path, &EnvironmentPointer::Path(self.pointer))?;
        Ok(())
    }

    /// This will lock the environment if it is not already locked.
    fn rendered_env_links(
        &mut self,
        flox: &Flox,
    ) -> Result<RenderedEnvironmentLinks, EnvironmentError> {
        let out_paths = self.rendered_env_links.clone();

        if self.needs_rebuild()? {
            let store_paths = self.build(flox)?;
            self.link(flox, &store_paths)?;
        }

        Ok(out_paths)
    }

    /// Build the environment
    /// This will lock the environment if it is not already locked.
    fn build(&mut self, flox: &Flox) -> Result<BuildEnvOutputs, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        env_view.lock(flox)?;
        let store_paths = env_view.build(flox)?;
        Ok(store_paths)
    }

    /// Returns .flox/cache
    fn cache_path(&self) -> Result<CanonicalPath, EnvironmentError> {
        let cache_dir = self.path.join(CACHE_DIR_NAME);
        if !cache_dir.exists() {
            std::fs::create_dir_all(&cache_dir).map_err(EnvironmentError::CreateCacheDir)?;
        }
        CanonicalPath::new(cache_dir).map_err(EnvironmentError::Canonicalize)
    }

    /// Returns .flox/log
    fn log_path(&self) -> Result<CanonicalPath, EnvironmentError> {
        let log_dir = self.path.join(LOG_DIR_NAME);
        if !log_dir.exists() {
            std::fs::create_dir_all(&log_dir).map_err(EnvironmentError::CreateLogDir)?;
        }
        CanonicalPath::new(log_dir).map_err(EnvironmentError::Canonicalize)
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

    /// Path to the environment's .flox directory
    fn dot_flox_path(&self) -> CanonicalPath {
        self.path.clone()
    }

    /// Path to the environment definition file
    fn manifest_path(&self, _flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        Ok(self.path.join(ENV_DIR_NAME).join(MANIFEST_FILENAME))
    }

    /// Path to the lockfile. The path may not exist.
    fn lockfile_path(&self, _flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        Ok(self.path.join(ENV_DIR_NAME).join(LOCKFILE_FILENAME))
    }

    /// Return the path where the process compose socket for an environment
    /// should be created
    fn services_socket_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        services_socket_path(&self.path_hash(), flox)
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
        dot_flox_path: CanonicalPath,
    ) -> Result<Self, EnvironmentError> {
        ensure_registered(
            flox,
            &dot_flox_path,
            &EnvironmentPointer::Path(pointer.clone()),
        )?;

        PathEnvironment::new(pointer, dot_flox_path, &flox.system)
    }

    /// Create a new env in a `.flox` directory within a specific path or open it if it exists.
    ///
    /// The method creates or opens a `.flox` directory _contained_ within `path`!
    pub fn init(
        pointer: PathPointer,
        dot_flox_parent_path: impl AsRef<Path>,
        customization: &InitCustomization,
        flox: &Flox,
    ) -> Result<Self, EnvironmentError> {
        // Ensure that the .flox directory does not already exist
        match DotFlox::open_in(dot_flox_parent_path.as_ref()) {
            // continue if the .flox directory does not exist, as it's being created by this method
            Err(EnvironmentError::DotFloxNotFound(_)) => {},
            // propagate any other error signaling a faulty .flox directory
            Err(e) => Err(e)?,
            // .flox directory exists, so we can't create a new environment here
            Ok(_) => Err(EnvironmentError::EnvironmentExists(
                dot_flox_parent_path.as_ref().to_path_buf(),
            ))?,
        }

        // Create manifest
        let manifest = {
            tracing::debug!("creating raw catalog manifest");
            RawManifest::new_documented(
                flox.features,
                &DEFAULT_SYSTEMS_STR.iter().collect::<Vec<_>>(),
                customization,
            )
        };

        let mut environment =
            Self::write_new_unchecked(flox, pointer, dot_flox_parent_path, manifest.to_string())?;

        // Build environment if customization installs at least one package
        if matches!(customization.packages.as_deref(), Some([_, ..])) {
            let mut env_view = CoreEnvironment::new(
                environment.path.join(ENV_DIR_NAME),
                environment.include_fetcher()?,
            );
            env_view.lock(flox)?;
            let store_paths = env_view.build(flox)?;
            environment.link(flox, &store_paths)?;
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
    fn write_new_unchecked(
        flox: &Flox,
        pointer: PathPointer,
        dot_flox_parent_path: impl AsRef<Path>,
        manifest: impl AsRef<str>,
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
            fs::write(manifest_path, manifest.as_ref()).map_err(EnvironmentError::WriteManifest);
        if let Err(e) = write_res {
            debug!("writing manifest did not complete successfully");
            fs::remove_dir_all(&env_dir).map_err(EnvironmentError::InitEnv)?;
            return Err(e);
        }

        // Write stateful directories to .flox/.gitignore
        create_dot_flox_gitignore(&dot_flox_path)?;

        let dot_flox_path = CanonicalPath::new(dot_flox_path).expect("the directory just created");

        Self::open(flox, pointer, dot_flox_path)
    }

    /// Determine if the environment needs to be rebuilt,
    /// based on the lockfile contents in the environment
    /// and the rendered environment link.
    ///
    /// If no lockfile exists in the rendered environment,
    /// or differs from the definition in the environment,
    /// the environment will be rebuilt.
    fn needs_rebuild(&self) -> Result<bool, EnvironmentError> {
        let env_view = self.as_core_environment()?;
        let Some(lockfile_contents) = env_view.existing_lockfile_contents()? else {
            return Ok(true);
        };

        let rendered_env_lockfile_path =
            self.rendered_env_links.development.join(LOCKFILE_FILENAME);

        if !rendered_env_lockfile_path.exists() {
            return Ok(true);
        }

        let Ok(rendered_env_lockfile_contents) = fs::read_to_string(&rendered_env_lockfile_path)
        else {
            return Ok(true);
        };

        if lockfile_contents != rendered_env_lockfile_contents {
            return Ok(true);
        }

        Ok(false)
    }

    /// The environment is locked,
    /// and the manifest in the lockfile matches that in the manifest.
    /// Note that the manifest could have whitespace or comment differences from
    /// the lockfile.
    pub fn lockfile_up_to_date(&self) -> Result<bool, EnvironmentError> {
        let env_view = self.as_core_environment()?;
        Ok(env_view.lockfile_if_up_to_date()?.is_some())
    }

    fn link(
        &mut self,
        _flox: &Flox,
        store_paths: &BuildEnvOutputs,
    ) -> Result<(), EnvironmentError> {
        CoreEnvironment::link(&self.rendered_env_links.development, &store_paths.develop)?;
        CoreEnvironment::link(&self.rendered_env_links.runtime, &store_paths.runtime)?;

        Ok(())
    }
}

pub mod test_helpers {
    use tempfile::tempdir_in;

    use super::*;

    pub fn new_path_environment_in(
        flox: &Flox,
        contents: &str,
        path: impl AsRef<Path>,
    ) -> PathEnvironment {
        let pointer = PathPointer::new(
            path.as_ref()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .parse()
                .unwrap(),
        );
        PathEnvironment::write_new_unchecked(flox, pointer, path, contents).unwrap()
    }

    pub fn new_named_path_environment_in(
        flox: &Flox,
        contents: &str,
        path: impl AsRef<Path>,
        name: &str,
    ) -> PathEnvironment {
        let pointer = PathPointer::new(name.parse().unwrap());
        PathEnvironment::write_new_unchecked(flox, pointer, path, contents).unwrap()
    }

    pub fn new_path_environment(flox: &Flox, contents: &str) -> PathEnvironment {
        new_path_environment_in(
            flox,
            contents,
            tempdir_in(&flox.temp_dir).unwrap().into_path(),
        )
    }

    pub fn new_named_path_environment(flox: &Flox, contents: &str, name: &str) -> PathEnvironment {
        new_named_path_environment_in(
            flox,
            contents,
            tempdir_in(&flox.temp_dir).unwrap().into_path(),
            name,
        )
    }

    pub fn new_path_environment_from_env_files(
        flox: &Flox,
        env_files_dir: impl AsRef<Path>,
    ) -> PathEnvironment {
        let dot_flox_parent_path = tempdir_in(&flox.temp_dir).unwrap().into_path();
        new_path_environment_from_env_files_in(flox, env_files_dir, dot_flox_parent_path, None)
    }

    pub fn new_named_path_environment_from_env_files(
        flox: &Flox,
        env_files_dir: impl AsRef<Path>,
        name: &str,
    ) -> PathEnvironment {
        let dot_flox_parent_path = tempdir_in(&flox.temp_dir).unwrap().into_path();
        new_path_environment_from_env_files_in(
            flox,
            env_files_dir,
            dot_flox_parent_path,
            Some(name),
        )
    }

    pub fn new_path_environment_from_env_files_in(
        flox: &Flox,
        env_files_dir: impl AsRef<Path>,
        dot_flox_parent_path: impl AsRef<Path>,
        name: Option<&str>,
    ) -> PathEnvironment {
        let env_files_dir = env_files_dir.as_ref();
        let manifest_contents = fs::read_to_string(env_files_dir.join(MANIFEST_FILENAME)).unwrap();
        let lockfile_contents = fs::read_to_string(env_files_dir.join(LOCKFILE_FILENAME)).unwrap();
        let pointer = PathPointer::new(
            name.unwrap_or_else(|| {
                dot_flox_parent_path
                    .as_ref()
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
            })
            .parse()
            .unwrap(),
        );
        PathEnvironment::write_new_unchecked(
            flox,
            pointer.clone(),
            &dot_flox_parent_path,
            &manifest_contents,
        )
        .unwrap();
        let dot_flox_path =
            CanonicalPath::new(dot_flox_parent_path.as_ref().join(DOT_FLOX)).unwrap();
        let env_dir = dot_flox_path.join(ENV_DIR_NAME);
        let lockfile_path = env_dir.join(LOCKFILE_FILENAME);
        fs::write(lockfile_path, lockfile_contents).unwrap();
        new_path_environment(flox, &manifest_contents);
        PathEnvironment::open(flox, pointer, dot_flox_path).unwrap()
    }
}

#[cfg(test)]
pub mod tests {

    use flox_test_utils::proptest::{alphanum_string, lowercase_alphanum_string};
    use indoc::indoc;
    use itertools::izip;
    use proptest::collection::{hash_set as prop_hash_set, vec as prop_vec};
    use proptest::prelude::*;
    use tempfile::TempDir;

    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::env_registry::{env_registry_path, read_environment_registry};
    use crate::models::environment::path_environment::test_helpers::{
        new_path_environment,
        new_path_environment_in,
    };
    use crate::models::lockfile::{Lockfile, RecoverableMergeError};
    use crate::models::manifest::typed::test::manifest_without_install_or_include;

    /// Returns (flox, tempdir, Vec<(dir relative to tempdir, PathEnvironment)>)
    /// This is a list of relative paths to environments that can be included in
    /// another environment.
    /// The environment names and directories are unique.
    pub fn generate_path_environments_without_install_or_include(
        max_size: usize,
    ) -> impl Strategy<Value = (Flox, TempDir, Vec<(PathBuf, PathEnvironment)>)> {
        (1..=max_size).prop_flat_map(|size| {
            (
                prop_vec(manifest_without_install_or_include(), size..=size),
                prop_hash_set(alphanum_string(2), size..=size),
                // macOS is case-insensitive,
                // so only use lowercase directories so there isn't a collision
                // between e.g. dir and DIR
                prop_hash_set(lowercase_alphanum_string(2), size..=size),
            )
                .prop_map(|(manifests, names, dirs)| {
                    let (flox, tempdir) = flox_instance();

                    let mut environments = vec![];
                    for (manifest, name, dir) in izip!(&manifests, &names, &dirs) {
                        let relative_path = PathBuf::from(dir);
                        let absolute_path = tempdir.path().join(&relative_path);
                        fs::create_dir(&absolute_path).unwrap();
                        let mut environment = test_helpers::new_named_path_environment_in(
                            &flox,
                            &toml_edit::ser::to_string_pretty(&manifest).unwrap(),
                            absolute_path,
                            name,
                        );
                        environment.lockfile(&flox).unwrap();
                        environments.push((relative_path, environment));
                    }
                    (flox, tempdir, environments)
                })
        })
    }

    #[test]
    fn create_env() {
        let (flox, temp_dir) = flox_instance();
        let environment_temp_dir = tempfile::tempdir_in(&temp_dir).unwrap();
        let pointer = PathPointer::new("test".parse().unwrap());

        let actual = PathEnvironment::init(
            pointer,
            environment_temp_dir.path(),
            &InitCustomization::default(),
            &flox,
        )
        .unwrap();

        let expected = PathEnvironment::new(
            PathPointer::new("test".parse().unwrap()),
            CanonicalPath::new(environment_temp_dir.path().join(DOT_FLOX)).unwrap(),
            &flox.system,
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
            &InitCustomization::default(),
            &flox,
        )
        .unwrap();

        assert!(env.needs_rebuild().unwrap());

        // build the environment -> out link is created -> no rebuild necessary
        let mut env_view =
            CoreEnvironment::new(env.path.join(ENV_DIR_NAME), env.include_fetcher().unwrap());
        env_view.lock(&flox).unwrap();
        let store_paths = env_view.build(&flox).unwrap();
        env.link(&flox, &store_paths).unwrap();

        assert!(!env.needs_rebuild().unwrap());

        // "modify" the lockfile by changing its formatting -> rebuild necessary
        let lockfile: Lockfile = env.lockfile(&flox).unwrap().into();
        let file = fs::write(env.lockfile_path(&flox).unwrap(), lockfile.to_string());
        drop(file);
        assert!(env.needs_rebuild().unwrap());
    }

    #[test]
    fn registers_on_init() {
        let (flox, tmp_dir) = flox_instance();
        let environment_temp_dir = tempfile::tempdir_in(&tmp_dir).unwrap();
        let ptr = PathPointer::new("test".parse().unwrap());
        let _env = PathEnvironment::init(
            ptr,
            environment_temp_dir.path(),
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
            &InitCustomization::default(),
            &flox,
        )
        .unwrap();
        let reg_path = env_registry_path(&flox);
        assert!(reg_path.exists());
        // Delete the registry so we can confirm that opening the environment creates it
        std::fs::remove_file(&reg_path).unwrap();
        let _env = PathEnvironment::open(&flox, ptr, env.path).unwrap();
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

    /// If an environment doesn't have any included environments, calling include_upgrade()
    /// should error
    #[test]
    fn include_upgrade_errors_without_includes() {
        let (flox, _tempdir) = flox_instance();

        // Create environment
        let manifest_contents = indoc! {r#"
        version = 1
        "#};
        let mut composer = new_path_environment(&flox, manifest_contents);
        composer.lockfile(&flox).unwrap();

        // Try to upgrade
        let err = composer.include_upgrade(&flox, vec![]).unwrap_err();

        let EnvironmentError::Recoverable(RecoverableMergeError::Catchall(message)) = err else {
            panic!("expected Catchall error, got: {:?}", err)
        };

        assert_eq!(message, "environment has no included environments",);
    }

    /// include_upgrade()errors when specified included environment doesn't exist
    #[test]
    fn include_upgrade_errors_when_included_environment_does_not_exist() {
        let (flox, tempdir) = flox_instance();

        // Create dep
        let dep_path = tempdir.path().join("dep");
        let dep_manifest_contents = indoc! {r#"
            version = 1
            [vars]
            foo = "v1"
            "#};
        fs::create_dir(&dep_path).unwrap();
        let mut dep = new_path_environment_in(&flox, dep_manifest_contents, &dep_path);
        dep.lockfile(&flox).unwrap();

        // Create composer
        let composer_manifest_contents = indoc! {r#"
            version = 1
            [include]
            environments = [
              { dir = "dep" },
            ]
            "#};
        let composer_path = tempdir.path();
        let mut composer =
            new_path_environment_in(&flox, composer_manifest_contents, composer_path);
        let lockfile: Lockfile = composer.lockfile(&flox).unwrap().into();

        assert_eq!(lockfile.manifest.vars.0["foo"], "v1");

        // Call include_upgrade() with a name of an included environment that does not exist
        let err = composer
            .include_upgrade(&flox, vec!["does_not_exist".to_string()])
            .unwrap_err();

        let EnvironmentError::Recoverable(RecoverableMergeError::Catchall(message)) = err else {
            panic!("expected Catchall error, got: {:?}", err)
        };

        assert_eq!(
            message,
            "unknown included environment to check for changes 'does_not_exist'"
        );
    }
}
