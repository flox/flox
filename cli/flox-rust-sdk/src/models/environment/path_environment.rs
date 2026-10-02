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

use flox_core::activate::mode::ActivateMode;
use flox_core::data::environment_ref::EnvironmentName;
use flox_manifest::interfaces::{AsWritableManifest, WriteManifest};
use flox_manifest::lockfile::{LOCKFILE_FILENAME, Lockfile};
use flox_manifest::parsed::common::KnownSchemaVersion;
use flox_manifest::raw::{CatalogPackage, DEFAULT_SYSTEMS_STR, PackageToInstall};
use flox_manifest::{MANIFEST_FILENAME, Manifest, Migrated, Validated, Writable};
use indoc::formatdoc;
use itertools::Itertools;
use tracing::debug;

use super::core_environment::{CoreEnvironment, FollowMode, FollowedIncludes, UpgradeResult};
use super::fetcher::{FetchedIncludes, IncludeFetcher};
use super::followed_includes::{self, FollowedLockfiles, Following};
use super::uninstall::UninstallSpec;
use super::{
    CACHE_DIR_NAME,
    DOT_FLOX,
    DotFlox,
    ENVIRONMENT_POINTER_FILENAME,
    EditResult,
    EnvJson,
    Environment,
    EnvironmentError,
    EnvironmentPointer,
    GCROOTS_DIR_NAME,
    InstallOrUninstallError,
    InstallationAttempt,
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
use crate::models::environment::{ENV_DIR_NAME, create_dot_flox_gitignore};
use crate::providers::buildenv::BuildEnvOutputs;
use crate::providers::lock_manifest::LockResult;
use crate::providers::manifest_init::ManifestInitializer;

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

    /// Included environments fetched by this instance, so that a command
    /// that uses the environment more than once, or includes an environment
    /// through more than one other, fetches each of them once
    fetched_includes: FetchedIncludes,
}

/// A profile script or list of packages to install when initializing an environment
#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct InitCustomization {
    pub hook_on_activate: Option<String>,
    pub profile_common: Option<String>,
    pub profile_bash: Option<String>,
    pub profile_fish: Option<String>,
    pub profile_tcsh: Option<String>,
    pub profile_zsh: Option<String>,
    pub packages: Option<Vec<CatalogPackage>>,
    pub activate_mode: Option<ActivateMode>,
    pub description: Option<String>,
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
            fetched_includes: FetchedIncludes::default(),
        })
    }

    fn include_fetcher(&self) -> Result<IncludeFetcher, EnvironmentError> {
        Ok(
            IncludeFetcher::for_composer(self.parent_path()?, self.path.clone())
                .with_fetched_includes(self.fetched_includes.clone()),
        )
    }

    /// Get a view of the environment that can be used to perform operations
    /// on the environment without side effects.
    ///
    /// This method should only be used to create [CoreEnvironment]s for a [PathEnvironment].
    /// To modify the environment, use the [PathEnvironment] methods instead.
    pub(super) fn into_core_environment(self) -> Result<CoreEnvironment, EnvironmentError> {
        self.as_core_environment()
    }

    /// Get a view of an environment that is included by another environment,
    /// whose includes are fetched with `include_fetcher`.
    pub(super) fn into_core_environment_with_include_fetcher(
        self,
        include_fetcher: IncludeFetcher,
    ) -> CoreEnvironment {
        CoreEnvironment::new(self.path.join(ENV_DIR_NAME), include_fetcher)
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

    /// The copies of the lockfile with the latest changes to followed
    /// included environments
    fn followed_lockfiles(&self) -> FollowedLockfiles {
        FollowedLockfiles::in_dot_flox(&self.path)
    }

    /// Lock the environment, and use a copy of its lockfile with the latest
    /// changes to the included environments it follows if any changed,
    /// see [followed_includes::follow_includes].
    fn follow_includes(
        &mut self,
        flox: &Flox,
        mode: FollowMode,
    ) -> Result<Following, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let lock_result = env_view.ensure_locked(flox)?;
        followed_includes::follow_includes(
            &mut env_view,
            lock_result,
            &self.followed_lockfiles(),
            &self.rendered_env_links,
            flox,
            mode,
        )
    }

    /// Names of the included environments that the copy of the lockfile in
    /// use has changes to, which the lockfile doesn't have yet.
    ///
    /// Unlike [Self::follow_includes], this doesn't lock anything,
    /// so it reports the copy from the last command that used it.
    pub fn unsaved_followed_includes(&self, flox: &Flox) -> Result<Vec<String>, EnvironmentError> {
        let Some(committed) = self.existing_lockfile(flox)? else {
            return Ok(Vec::new());
        };
        Ok(followed_includes::unsaved_followed_includes(
            &committed,
            &self.followed_lockfiles(),
            &flox.system,
        ))
    }

    /// Build the latest changes to followed included environments into the
    /// rendered environment links again, after a command that changed the
    /// lockfile built the lockfile into them, so that activations keep using
    /// the changes.
    ///
    /// Failing only leaves the lockfile in the links until the next command
    /// that uses the environment.
    fn link_followed_changes(&mut self, flox: &Flox) {
        if let Err(err) = self.follow_includes(flox, FollowMode::LockAndBuild) {
            debug!(%err, "could not use the latest changes to included environments");
        }
    }

    /// Build the environment's lockfile, without the latest changes to
    /// followed included environments that it doesn't have yet.
    ///
    /// Publishing builds this, so that it builds what's committed.
    pub fn build_locked(&mut self, flox: &Flox) -> Result<BuildEnvOutputs, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let out_link_prefix = self.rendered_env_links.out_link_prefix();
        env_view.ensure_locked(flox)?;
        let store_paths = env_view.build(flox, Some(out_link_prefix))?;
        self.rendered_env_links.replace_legacy_links();
        Ok(store_paths)
    }

    pub fn rename(&mut self, new_name: EnvironmentName) -> Result<(), EnvironmentError> {
        self.pointer.name = new_name;

        // Rewrite from the full [EnvJson] rather than the bare pointer so
        // the sibling `env_id` survives the rename.
        let previous = EnvJson::read_from(&self.path).ok();
        if previous.is_none() {
            debug!("could not read or parse env.json before rename, dropping any env_id");
        }
        let env_id = previous.and_then(|env_json| env_json.env_id);
        let env_json = EnvJson {
            pointer: EnvironmentPointer::Path(self.pointer.clone()),
            env_id,
        };
        env_json.write_to(&self.path)
    }

    /// Returns a unique identifier for the location of the environment.
    fn path_hash(&self) -> String {
        path_hash(&self.path)
    }
}

impl Environment for PathEnvironment {
    /// This will lock the environment if it is not already locked,
    /// and use the latest changes to the included environments it follows.
    fn lockfile(&mut self, flox: &Flox) -> Result<LockResult, EnvironmentError> {
        let Following {
            lock_result,
            followed,
            ..
        } = self.follow_includes(flox, FollowMode::Lock)?;
        debug!(?followed, "followed included environments");
        Ok(lock_result)
    }

    fn lockfile_following_includes(
        &mut self,
        flox: &Flox,
        mode: FollowMode,
    ) -> Result<(LockResult, FollowedIncludes), EnvironmentError> {
        let Following {
            lock_result,
            followed,
            ..
        } = self.follow_includes(flox, mode)?;
        Ok((lock_result, followed))
    }

    fn fetch_included_remote_environments(&mut self, flox: &Flox) -> Result<(), EnvironmentError> {
        if let Some(lockfile) = self.existing_lockfile(flox)? {
            followed_includes::fetch_included_remotes(flox, &self.include_fetcher()?, &lockfile);
        }
        Ok(())
    }

    fn check_followed_includes(
        &mut self,
        flox: &Flox,
    ) -> Result<Option<FollowedIncludes>, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let committed = env_view.lockfile_if_up_to_date()?;
        followed_includes::check_followed_includes(&mut env_view, committed, flox)
    }

    /// Returns the lockfile if it already exists.
    fn existing_lockfile(&self, _flox: &Flox) -> Result<Option<Lockfile>, EnvironmentError> {
        self.as_core_environment()?
            .existing_lockfile()
            .map_err(EnvironmentError::Core)
    }

    fn manifest_without_migrating(
        &self,
        _flox: &Flox,
    ) -> Result<Manifest<Validated>, EnvironmentError> {
        let manifest = self.as_core_environment()?.manifest_without_migrating()?;
        Ok(manifest)
    }

    fn manifest(&mut self, flox: &Flox) -> Result<Manifest<Migrated>, EnvironmentError> {
        let manifest = self.as_core_environment()?.manifest(flox)?;
        Ok(manifest)
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
        let out_link_prefix = self.rendered_env_links.out_link_prefix();
        let result = env_view.install(packages, flox, Some(out_link_prefix))?;
        if result.built_environments.is_some() {
            self.rendered_env_links.replace_legacy_links();
            self.link_followed_changes(flox);
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
        specs: Vec<UninstallSpec>,
        flox: &Flox,
    ) -> Result<UninstallationAttempt, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let out_link_prefix = self.rendered_env_links.out_link_prefix();
        let result = match env_view.uninstall(specs, flox, Some(out_link_prefix)) {
            Err(EnvironmentError::InstallOrUninstall(
                InstallOrUninstallError::PackageOnlyIncluded {
                    package, include, ..
                },
            )) => {
                let followed = followed_includes::follows_include_named(
                    self.existing_lockfile(flox)?.as_ref(),
                    &self.include_fetcher()?,
                    &include,
                );
                return Err(EnvironmentError::InstallOrUninstall(
                    InstallOrUninstallError::PackageOnlyIncluded {
                        package,
                        include,
                        followed,
                    },
                ));
            },
            result => result?,
        };
        if result.built_environment_store_paths.is_some() {
            self.rendered_env_links.replace_legacy_links();
            self.link_followed_changes(flox);
        }
        Ok(result)
    }

    /// Atomically edit this environment, ensuring that it still builds
    fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError> {
        let mut env_view = self.as_core_environment_mut()?;
        let out_link_prefix = self.rendered_env_links.out_link_prefix();
        let result = env_view.edit(flox, contents, Some(out_link_prefix))?;
        if matches!(&result, EditResult::Changed { .. }) {
            self.rendered_env_links.replace_legacy_links();
            self.link_followed_changes(flox);
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
        let result = env_view.upgrade(flox, groups_or_iids, false, None)?; // dry-run: no out-link
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
        let out_link_prefix = self.rendered_env_links.out_link_prefix();
        let result = env_view.upgrade(flox, groups_or_iids, true, Some(out_link_prefix))?;
        if result.store_path.is_some() {
            self.rendered_env_links.replace_legacy_links();
            self.link_followed_changes(flox);
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
        let committed: Lockfile = env_view.ensure_locked(flox)?.into();
        let copy = followed_includes::copy_to_save(
            &committed,
            &self.followed_lockfiles(),
            &flox.system,
            &to_upgrade,
        );
        let out_link_prefix = self.rendered_env_links.out_link_prefix();
        let result = env_view.include_upgrade(
            flox,
            to_upgrade,
            copy.as_ref().map(|copy| &copy.lockfile),
            Some(out_link_prefix),
        )?;
        if result.store_path.is_some() {
            self.rendered_env_links.replace_legacy_links();
        }
        // Copies are of the lockfile before the upgrade, so the latest changes
        // to included environments that weren't saved are copied again the
        // next time they're used.
        self.followed_lockfiles().remove_all();
        if result.store_path.is_some() {
            self.link_followed_changes(flox);
        }
        Ok(result)
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

        let lockfile = self
            .follow_includes(flox, FollowMode::Lock)?
            .lock_result
            .into();
        if self.needs_rebuild(&lockfile) {
            self.build(flox)?;
        }

        Ok(out_paths)
    }

    /// Build the environment
    /// This will lock the environment if it is not already locked,
    /// and build it with the latest changes to the included environments it
    /// follows if it builds with them, see [Self::follow_includes].
    fn build(&mut self, flox: &Flox) -> Result<BuildEnvOutputs, EnvironmentError> {
        if let Some(mut copy) = self.follow_includes(flox, FollowMode::Lock)?.copy {
            let mut env_view = self.as_core_environment_mut()?;
            match followed_includes::build_followed_lockfile(
                &mut env_view,
                &self.followed_lockfiles(),
                &self.rendered_env_links,
                flox,
                &mut copy,
            ) {
                Ok(store_paths) => return Ok(store_paths),
                Err(err) => debug!(
                    %err,
                    "building with the latest changes to included environments failed, building the lockfile instead"
                ),
            }
        }
        self.build_locked(flox)
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

    /// Create a new env in a `.flox` directory within a specific path
    /// or open it if it exists. Will create a very minimal manifest.
    ///
    /// The method creates or opens a `.flox` directory _contained_ within `path`!
    pub fn init_bare(
        pointer: PathPointer,
        dot_flox_parent_path: impl AsRef<Path>,
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

        // The most minimal manifest we can generate.
        let manifest = Manifest::parse_toml_typed(format!(
            "schema-version = \"{}\"\n",
            KnownSchemaVersion::latest()
        ))?;

        let environment = Self::write_new_unchecked(
            flox,
            pointer,
            dot_flox_parent_path,
            &manifest.as_writable(),
        )?;

        // Lock but don't build
        let mut env_view = CoreEnvironment::new(
            environment.path.join(ENV_DIR_NAME),
            environment.include_fetcher()?,
        );
        env_view.lock(flox)?;

        Ok(environment)
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
            ManifestInitializer::new_documented(
                flox.features,
                &DEFAULT_SYSTEMS_STR.iter().collect::<Vec<_>>(),
                customization,
            )?
        };

        let environment = Self::write_new_unchecked(
            flox,
            pointer,
            dot_flox_parent_path,
            &manifest.as_writable(),
        )?;

        // Always lock
        let mut env_view = CoreEnvironment::new(
            environment.path.join(ENV_DIR_NAME),
            environment.include_fetcher()?,
        );
        env_view.lock(flox)?;

        // Build only when packages are present; nix writes the activation
        // symlinks via --out-link, so no separate link step is needed.
        if matches!(customization.packages.as_deref(), Some([_, ..])) {
            let out_link_prefix = environment.rendered_env_links.out_link_prefix();
            env_view.build(flox, Some(out_link_prefix))?;
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
        manifest: &Manifest<Writable>,
    ) -> Result<Self, EnvironmentError> {
        let dot_flox_path = dot_flox_parent_path.as_ref().join(DOT_FLOX);
        let env_dir = dot_flox_path.join(ENV_DIR_NAME);
        let manifest_path = env_dir.join(MANIFEST_FILENAME);
        debug!("creating env dir: {}", env_dir.display());
        std::fs::create_dir_all(&env_dir).map_err(EnvironmentError::InitEnv)?;

        // Write the `env.json` file
        let mut pointer_content =
            serde_json::to_string_pretty(&pointer).map_err(EnvironmentError::SerializeEnvJson)?;
        pointer_content.push('\n');
        if let Err(e) = fs::write(
            dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME),
            pointer_content,
        ) {
            fs::remove_dir_all(&env_dir).map_err(EnvironmentError::InitEnv)?;
            Err(EnvironmentError::WriteEnvJson(Box::new(e)))?;
        }

        // Write `manifest.toml`
        let write_res = manifest.write_to_file(&manifest_path);
        if let Err(e) = write_res {
            debug!("writing manifest did not complete successfully");
            fs::remove_dir_all(&env_dir).map_err(EnvironmentError::InitEnv)?;
            return Err(EnvironmentError::ManifestError(e));
        }

        // Write stateful directories to .flox/.gitignore
        create_dot_flox_gitignore(&dot_flox_path)?;

        // Write (configure) Git attributes to ./flox/.gitattributes
        fs::write(dot_flox_path.join(".gitattributes"), formatdoc! {"
            {ENV_DIR_NAME}/{LOCKFILE_FILENAME} linguist-generated=true linguist-language=JSON
            "})
        .map_err(EnvironmentError::WriteGitattributes)?;

        let dot_flox_path = CanonicalPath::new(dot_flox_path).expect("the directory just created");

        Self::open(flox, pointer, dot_flox_path)
    }

    /// Determine if the environment needs to be rebuilt,
    /// based on the lockfile in use and the rendered environment link.
    ///
    /// If no lockfile exists in the rendered environment,
    /// or it differs from the lockfile in use,
    /// the environment will be rebuilt.
    fn needs_rebuild(&self, lockfile: &Lockfile) -> bool {
        let rendered_env_lockfile_path = self.rendered_env_links.dev.join(LOCKFILE_FILENAME);

        let Ok(rendered_env_lockfile_path) = CanonicalPath::new(rendered_env_lockfile_path) else {
            return true;
        };

        let Ok(rendered_lockfile) = Lockfile::read_from_file(&rendered_env_lockfile_path) else {
            return true;
        };

        *lockfile != rendered_lockfile
    }

    /// The environment is locked,
    /// and the manifest in the lockfile matches that in the manifest.
    /// Note that the manifest could have whitespace or comment differences from
    /// the lockfile.
    pub fn lockfile_up_to_date(&self) -> Result<bool, EnvironmentError> {
        let env_view = self.as_core_environment()?;
        Ok(env_view.lockfile_if_up_to_date()?.is_some())
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
        let manifest = Manifest::parse_toml_typed(contents).unwrap();
        PathEnvironment::write_new_unchecked(flox, pointer, path, &manifest.as_writable()).unwrap()
    }

    pub fn new_named_path_environment_in(
        flox: &Flox,
        contents: &str,
        path: impl AsRef<Path>,
        name: &str,
    ) -> PathEnvironment {
        let pointer = PathPointer::new(name.parse().unwrap());
        let manifest = Manifest::parse_toml_typed(contents).unwrap();
        PathEnvironment::write_new_unchecked(flox, pointer, path, &manifest.as_writable()).unwrap()
    }

    pub fn new_path_environment(flox: &Flox, contents: &str) -> PathEnvironment {
        new_path_environment_in(flox, contents, tempdir_in(&flox.temp_dir).unwrap().keep())
    }

    pub fn new_named_path_environment(flox: &Flox, contents: &str, name: &str) -> PathEnvironment {
        new_named_path_environment_in(
            flox,
            contents,
            tempdir_in(&flox.temp_dir).unwrap().keep(),
            name,
        )
    }

    pub fn new_path_environment_from_env_files(
        flox: &Flox,
        env_files_dir: impl AsRef<Path>,
    ) -> PathEnvironment {
        let dot_flox_parent_path = tempdir_in(&flox.temp_dir).unwrap().keep();
        new_path_environment_from_env_files_in(flox, env_files_dir, dot_flox_parent_path, None)
    }

    pub fn new_named_path_environment_from_env_files(
        flox: &Flox,
        env_files_dir: impl AsRef<Path>,
        name: &str,
    ) -> PathEnvironment {
        let dot_flox_parent_path = tempdir_in(&flox.temp_dir).unwrap().keep();
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
        let manifest = Manifest::read_typed(env_files_dir.join(MANIFEST_FILENAME)).unwrap();
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
            &manifest.as_writable(),
        )
        .unwrap();
        let dot_flox_path =
            CanonicalPath::new(dot_flox_parent_path.as_ref().join(DOT_FLOX)).unwrap();
        let env_dir = dot_flox_path.join(ENV_DIR_NAME);
        let lockfile_path = env_dir.join(LOCKFILE_FILENAME);
        fs::write(lockfile_path, lockfile_contents).unwrap();
        new_path_environment(flox, &manifest.as_writable().to_string());
        PathEnvironment::open(flox, pointer, dot_flox_path).unwrap()
    }
}

#[cfg(test)]
pub mod tests {

    use std::collections::BTreeMap;
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    use flox_core::data::environment_ref::RemoteEnvironmentRef;
    use flox_manifest::interfaces::AsLatestSchema;
    use flox_manifest::lockfile::LockedPackage;
    use flox_manifest::parsed::Inner;
    use flox_manifest::parsed::common::KnownSchemaVersion;
    use flox_manifest::parsed::v1::test_helpers::manifest_without_install_or_include;
    use flox_manifest::test_helpers::{with_latest_schema, with_schema};
    use flox_test_utils::GENERATED_DATA;
    use flox_test_utils::proptest::{alphanum_string, lowercase_alphanum_string};
    use indoc::indoc;
    use itertools::izip;
    use proptest::collection::{hash_set as prop_hash_set, vec as prop_vec};
    use proptest::prelude::*;
    use tempfile::TempDir;

    use super::*;
    use crate::flox::test_helpers::{flox_instance, flox_instance_with_optional_floxhub};
    use crate::models::env_registry::{env_registry_path, read_environment_registry};
    use crate::models::environment::core_environment::NotAppliedIncludes;
    use crate::models::environment::floxmeta_branch::remote_branch_name;
    use crate::models::environment::path_environment::test_helpers::{
        new_path_environment,
        new_path_environment_from_env_files,
        new_path_environment_from_env_files_in,
        new_path_environment_in,
    };
    use crate::models::environment::remote_environment::RemoteEnvironment;
    use crate::models::environment::remote_environment::test_helpers::mock_remote_environment;
    use crate::models::floxmeta::{FloxMeta, floxmeta_dir};
    use crate::providers::lock_manifest::RecoverableMergeError;
    use crate::utils::serialize_json_with_newline;

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
    fn rename_preserves_env_id() {
        let (flox, _temp_dir) = flox_instance();
        let mut environment = new_path_environment(&flox, &with_latest_schema(""));

        let env_id = uuid::Uuid::new_v4();
        let env_json_path = environment.path.join(ENVIRONMENT_POINTER_FILENAME);
        let stamped = EnvJson {
            pointer: EnvironmentPointer::Path(environment.pointer.clone()),
            env_id: Some(env_id),
        };
        fs::write(&env_json_path, stamped.to_pretty_string().unwrap()).unwrap();

        environment.rename("renamed".parse().unwrap()).unwrap();

        assert_eq!(EnvJson::read_from(&environment.path).unwrap(), EnvJson {
            pointer: EnvironmentPointer::Path(PathPointer::new("renamed".parse().unwrap())),
            env_id: Some(env_id),
        });
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
        assert!(
            actual.lockfile_path(&flox).unwrap().exists(),
            "lockfile exists"
        );
        assert!(actual.path.is_absolute());
    }

    /// Write a manifest file with invalid toml to ensure we can catch
    #[test]
    fn cache_activation_path() {
        let (flox, temp_dir) = flox_instance();

        let environment_temp_dir = tempfile::tempdir_in(&temp_dir).unwrap();
        let pointer = PathPointer::new("test".parse().unwrap());

        let env = PathEnvironment::init(
            pointer,
            environment_temp_dir.path(),
            &InitCustomization::default(),
            &flox,
        )
        .unwrap();

        let lockfile = env.existing_lockfile(&flox).unwrap().unwrap();
        assert!(env.needs_rebuild(&lockfile));

        // build the environment -> out link is created -> no rebuild necessary
        let mut env_view =
            CoreEnvironment::new(env.path.join(ENV_DIR_NAME), env.include_fetcher().unwrap());
        let out_link_prefix = env.rendered_env_links.out_link_prefix();
        env_view.build(&flox, Some(out_link_prefix)).unwrap();

        assert!(!env.needs_rebuild(&lockfile));

        // modify the lockfile  -> rebuild necessary
        let mut lockfile = env.existing_lockfile(&flox).unwrap().unwrap();
        let mut manifest = lockfile.migrated_manifest().unwrap();
        manifest.as_latest_schema_mut().options.activate.mode = Some(ActivateMode::Dev);
        lockfile.manifest = manifest.into();
        let lockfile_contents = serialize_json_with_newline(&lockfile).unwrap();
        fs::write(env.lockfile_path(&flox).unwrap(), lockfile_contents).unwrap();
        assert!(env.needs_rebuild(&lockfile));
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

        let manifest = lockfile.migrated_manifest().unwrap();
        assert_eq!(manifest.as_latest_schema().vars.inner()["foo"], "v1");

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

    /// An include upgrade that changes nothing doesn't write the lockfile,
    /// so it doesn't rebuild the environment,
    /// or create a generation for managed environments.
    #[test]
    fn include_upgrade_without_changes_does_not_write_lockfile() {
        let (flox, tempdir) = flox_instance();

        let dep_path = tempdir.path().join("dep");
        fs::create_dir(&dep_path).unwrap();
        let mut dep = new_path_environment_in(
            &flox,
            indoc! {r#"
                version = 1
                [vars]
                foo = "v1"
            "#},
            &dep_path,
        );
        dep.lockfile(&flox).unwrap();

        let mut composer = new_path_environment_in(
            &flox,
            indoc! {r#"
                version = 1
                [include]
                environments = [
                  { dir = "dep" },
                ]
            "#},
            tempdir.path(),
        );
        let lockfile: Lockfile = composer.lockfile(&flox).unwrap().into();

        let result = composer.include_upgrade(&flox, vec![]).unwrap();

        assert_eq!(result, UpgradeResult {
            old_lockfile: Some(lockfile.clone()),
            new_lockfile: lockfile,
            store_path: None,
        });
    }

    #[test]
    fn no_rebuild_on_lockfile_formatting_change() {
        let (flox, _temp_dir) = flox_instance();

        let mut environment = new_path_environment(&flox, "version = 1");
        let lockfile: Lockfile = environment.lockfile(&flox).unwrap().into();

        assert!(environment.needs_rebuild(&lockfile));

        environment.rendered_env_links(&flox).unwrap();

        assert!(!environment.needs_rebuild(&lockfile));

        let mut lockfile_file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(environment.lockfile_path(&flox).unwrap())
            .unwrap();

        writeln!(lockfile_file, "\n\n\n",).unwrap();

        let lockfile = environment.existing_lockfile(&flox).unwrap().unwrap();
        assert!(!environment.needs_rebuild(&lockfile));
    }

    // -------------------------------------------------------------------------
    // Regression tests: PathEnvironment::build must not rewrite a current lock
    //
    // Previously, `build` called `lock()` unconditionally.  Even when the
    // lockfile was already up-to-date, `lock()` could rewrite it (e.g. due to
    // formatting normalisation), updating the mtime and triggering a spurious
    // needs_rebuild() on the next activate.
    //
    // After the fix, `build` calls `ensure_locked()`, which skips the catalog
    // round-trip and the file write when lockfile_if_up_to_date() returns
    // Some(_).  These tests assert the "no rewrite" invariant on three
    // environment shapes.
    // -------------------------------------------------------------------------

    /// Plain environment (latest schema, no packages).
    /// After locking once, calling build() must not alter manifest.lock.
    #[test]
    fn build_does_not_rewrite_current_lockfile_plain() {
        let (flox, _temp_dir) = flox_instance();
        let manifest = with_latest_schema("");
        let mut env = new_path_environment(&flox, &manifest);

        // Lock the environment to produce an up-to-date manifest.lock.
        env.lockfile(&flox).unwrap();

        let lockfile_path = env.lockfile_path(&flox).unwrap();
        let bytes_before = fs::read(&lockfile_path).unwrap();
        let mtime_before = fs::metadata(&lockfile_path).unwrap().modified().unwrap();

        // Build must not rewrite the lockfile.
        env.build(&flox).unwrap();

        let bytes_after = fs::read(&lockfile_path).unwrap();
        let mtime_after = fs::metadata(&lockfile_path).unwrap().modified().unwrap();

        assert_eq!(bytes_before, bytes_after, "lockfile bytes changed");
        assert_eq!(mtime_before, mtime_after, "lockfile mtime changed");
    }

    /// Composed environment (v1 composer that includes a child environment).
    /// After locking once, calling build() on the composer must not alter its
    /// manifest.lock.
    #[test]
    fn build_does_not_rewrite_current_lockfile_composed() {
        let (flox, tempdir) = flox_instance();

        // Set up an included environment with only vars (backwards compatible
        // with v1, so the composer won't be migrated).
        let included_manifest = with_latest_schema(indoc! {r#"
            [vars]
            included_var = "value"
        "#});
        let included_path = tempdir.path().join("included");
        let mut included_env = new_path_environment_in(&flox, &included_manifest, &included_path);
        included_env.lockfile(&flox).unwrap();

        // Create a v1 composer that includes the child.
        let composer_manifest = with_schema(KnownSchemaVersion::V1, indoc! {r#"
            [include]
            environments = [
              { dir = "../included" },
            ]
        "#});
        let composer_path = tempdir.path().join("composer");
        let mut composer = new_path_environment_in(&flox, &composer_manifest, &composer_path);

        // Lock the composer to produce an up-to-date manifest.lock.
        composer.lockfile(&flox).unwrap();

        let lockfile_path = composer.lockfile_path(&flox).unwrap();
        let bytes_before = fs::read(&lockfile_path).unwrap();
        let mtime_before = fs::metadata(&lockfile_path).unwrap().modified().unwrap();

        // Build must not rewrite the lockfile.
        composer.build(&flox).unwrap();

        let bytes_after = fs::read(&lockfile_path).unwrap();
        let mtime_after = fs::metadata(&lockfile_path).unwrap().modified().unwrap();

        assert_eq!(bytes_before, bytes_after, "lockfile bytes changed");
        assert_eq!(mtime_before, mtime_after, "lockfile mtime changed");
    }

    /// Create `<tempdir>/<name>` with `contents` and lock it
    fn locked_path_environment(
        flox: &Flox,
        tempdir: &TempDir,
        name: &str,
        contents: &str,
    ) -> PathEnvironment {
        let mut environment = new_path_environment_in(flox, contents, tempdir.path().join(name));
        environment.lockfile(flox).unwrap();
        environment
    }

    /// Replace the manifest of an environment and lock it, like 'flox edit'
    /// without building
    fn edit_and_lock(environment: &mut PathEnvironment, flox: &Flox, contents: &str) {
        fs::write(environment.manifest_path(flox).unwrap(), contents).unwrap();
        environment
            .as_core_environment_mut()
            .unwrap()
            .ensure_locked(flox)
            .unwrap();
    }

    /// The vars of the merged manifest in a lockfile
    fn locked_vars(lockfile: &Lockfile) -> BTreeMap<String, String> {
        lockfile
            .migrated_manifest()
            .unwrap()
            .as_latest_schema()
            .vars
            .inner()
            .clone()
    }

    fn vars_map(vars: &[(&str, &str)]) -> BTreeMap<String, String> {
        vars.iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    /// The names in [FollowedIncludes], as
    /// (unsaved, unreadable, not locked, not built),
    /// since the reasons can't be compared
    #[allow(clippy::type_complexity)]
    fn followed_names(followed: &FollowedIncludes) -> (Vec<&str>, Vec<&str>, Vec<&str>, Vec<&str>) {
        fn names(not_applied: &Option<NotAppliedIncludes>) -> Vec<&str> {
            not_applied
                .iter()
                .flat_map(|not_applied| not_applied.names.iter().map(String::as_str))
                .collect()
        }
        (
            followed.unsaved.iter().map(String::as_str).collect(),
            followed
                .unreadable
                .iter()
                .map(|unreadable| unreadable.name.as_str())
                .collect(),
            names(&followed.not_locked),
            names(&followed.not_built),
        )
    }

    /// Forget the included environments fetched so far, which a command only
    /// fetches once
    fn new_command(environment: &mut PathEnvironment) {
        environment.fetched_includes = FetchedIncludes::default();
    }

    /// Follow included environments as a new command would, and return the
    /// lockfile in use, asserting that the environment's own lockfile isn't
    /// written
    fn follow(
        environment: &mut PathEnvironment,
        flox: &Flox,
        mode: FollowMode,
    ) -> (Lockfile, FollowedIncludes) {
        new_command(environment);
        let lockfile_path = environment.lockfile_path(flox).unwrap();
        let bytes_before = fs::read(&lockfile_path).unwrap();
        let (lock_result, followed) = environment.lockfile_following_includes(flox, mode).unwrap();
        assert_eq!(
            bytes_before,
            fs::read(&lockfile_path).unwrap(),
            "following included environments wrote the lockfile"
        );
        (lock_result.into(), followed)
    }

    /// Locked changes to an included path environment are used without
    /// 'flox include upgrade', and without writing the lockfile.
    #[test]
    fn lockfile_follows_locked_changes_to_path_include() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        // A v1 composer, like most composers in the wild
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_schema(
                KnownSchemaVersion::V1,
                "[include]\nenvironments = [{ dir = \"../included\" }]",
            ),
        );

        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema("[vars]\nincluded = \"v2\""),
        );

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["included"], vec![], vec![], vec![])
        );
        assert_eq!(locked_vars(&lockfile), vars_map(&[("included", "v2")]));
        // Locking alone doesn't build the copy
        let copy_build = |composer: &PathEnvironment| {
            composer
                .followed_lockfiles()
                .read(&flox.system)
                .map(|copy| copy.build)
        };
        assert_eq!(
            copy_build(&composer),
            Some(followed_includes::FollowedBuild::Pending)
        );
        follow(&mut composer, &flox, FollowMode::LockAndBuild);
        assert_eq!(
            copy_build(&composer),
            Some(followed_includes::FollowedBuild::Succeeded)
        );

        // The copy is reused, and the changes are still unsaved
        let (lock_result, followed) = composer
            .lockfile_following_includes(&flox, FollowMode::Lock)
            .unwrap();
        assert!(matches!(lock_result, LockResult::Unchanged(_)));
        assert_eq!(
            followed_names(&followed),
            (vec!["included"], vec![], vec![], vec![])
        );
    }

    /// Changes that an included environment hasn't locked aren't used.
    #[test]
    fn lockfile_does_not_follow_unlocked_changes_to_path_include() {
        let (flox, tempdir) = flox_instance();
        let included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../included\" }]"),
        );

        fs::write(
            included.manifest_path(&flox).unwrap(),
            with_latest_schema("[vars]\nincluded = \"v2\""),
        )
        .unwrap();

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec![], vec!["included"], vec![], vec![])
        );
        assert!(matches!(
            followed.unreadable[0].reason.as_ref(),
            EnvironmentError::Recoverable(RecoverableMergeError::PathOutOfSync(_))
        ));
        assert_eq!(locked_vars(&lockfile), vars_map(&[("included", "v1")]));
    }

    /// An included environment that can't be read keeps the version in use,
    /// rather than going back to the version in the lockfile.
    #[test]
    fn lockfile_keeps_version_in_use_of_path_include_that_cannot_be_read() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../included\" }]"),
        );
        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema("[vars]\nincluded = \"v2\""),
        );
        follow(&mut composer, &flox, FollowMode::Lock);

        fs::remove_dir_all(tempdir.path().join("included")).unwrap();

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["included"], vec!["included"], vec![], vec![])
        );
        assert!(matches!(
            followed.unreadable[0].reason.as_ref(),
            EnvironmentError::DotFloxNotFound(_)
        ));
        assert_eq!(locked_vars(&lockfile), vars_map(&[("included", "v2")]));
    }

    /// A -> B -> C: a change that C locked reaches A through B,
    /// without B locking it.
    #[test]
    fn lockfile_follows_locked_changes_to_nested_path_include() {
        let (flox, tempdir) = flox_instance();
        let mut c = locked_path_environment(
            &flox,
            &tempdir,
            "c",
            &with_latest_schema("[vars]\nc = \"v1\""),
        );
        let b = locked_path_environment(
            &flox,
            &tempdir,
            "b",
            &with_latest_schema(
                "[vars]\nb = \"v1\"\n[include]\nenvironments = [{ dir = \"../c\" }]",
            ),
        );
        let mut a = locked_path_environment(
            &flox,
            &tempdir,
            "a",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../b\" }]"),
        );
        let b_lockfile_before = fs::read(b.lockfile_path(&flox).unwrap()).unwrap();

        edit_and_lock(&mut c, &flox, &with_latest_schema("[vars]\nc = \"v2\""));

        let (lockfile, followed) = follow(&mut a, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["b"], vec![], vec![], vec![])
        );
        assert_eq!(
            locked_vars(&lockfile),
            vars_map(&[("b", "v1"), ("c", "v2")])
        );
        assert_eq!(
            b_lockfile_before,
            fs::read(b.lockfile_path(&flox).unwrap()).unwrap()
        );
    }

    /// Contents of a manifest that installs hello, as in the generated
    /// `envs/hello` environment, with `extra` appended
    fn hello_manifest(extra: &str) -> String {
        with_latest_schema(formatdoc! {r#"
            [install]
            hello.pkg-path = "hello"

            [options]
            systems = ["aarch64-darwin", "aarch64-linux", "x86_64-darwin", "x86_64-linux"]

            {extra}
        "#})
    }

    /// Change the derivations an environment locks, as 'flox upgrade' would,
    /// without resolving anything
    fn upgrade_locked_packages(environment: &PathEnvironment, flox: &Flox) {
        upgrade_locked_packages_by(environment, flox, "upgraded");
    }

    /// Like [upgrade_locked_packages], with derivations marked as upgraded by
    /// `by`, so that environments can upgrade to different ones
    fn upgrade_locked_packages_by(environment: &PathEnvironment, flox: &Flox, by: &str) {
        edit_lockfile(environment, flox, |lockfile| {
            for package in &mut lockfile.packages {
                if let LockedPackage::Catalog(package) = package {
                    package.derivation = format!("{}-{by}", package.derivation);
                    package.version = "2.12.4".to_string();
                }
            }
        });
    }

    fn edit_lockfile(environment: &PathEnvironment, flox: &Flox, edit: impl FnOnce(&mut Lockfile)) {
        let lockfile_path = environment.lockfile_path(flox).unwrap();
        let mut lockfile = environment.existing_lockfile(flox).unwrap().unwrap();
        edit(&mut lockfile);
        fs::write(
            lockfile_path,
            serialize_json_with_newline(&lockfile).unwrap(),
        )
        .unwrap();
    }

    /// A composing environment including the hello environment from a
    /// directory, and that environment
    fn composer_including_hello(
        flox: &Flox,
        tempdir: &TempDir,
    ) -> (PathEnvironment, PathEnvironment) {
        let included = new_path_environment_from_env_files_in(
            flox,
            GENERATED_DATA.join("envs/hello"),
            tempdir.path().join("included"),
            None,
        );
        let composer = locked_path_environment(
            flox,
            tempdir,
            "composer",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../included\" }]"),
        );
        (composer, included)
    }

    /// Upgrades that an included environment locked without changing its
    /// manifest are followed.
    #[test]
    fn lockfile_follows_upgrades_an_included_environment_locked_without_editing_it() {
        let (flox, tempdir) = flox_instance();
        let (mut composer, included) = composer_including_hello(&flox, &tempdir);

        upgrade_locked_packages(&included, &flox);

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["included"], vec![], vec![], vec![])
        );
        assert_eq!(
            sorted_packages(&lockfile),
            sorted_packages(&included.existing_lockfile(&flox).unwrap().unwrap())
        );
    }

    /// A -> B -> C: upgrades that C locked without changing its manifest reach
    /// A without B locking them.
    #[test]
    fn lockfile_follows_upgrades_a_nested_include_locked_without_editing_it() {
        let (flox, tempdir) = flox_instance();
        let c = new_path_environment_from_env_files_in(
            &flox,
            GENERATED_DATA.join("envs/hello"),
            tempdir.path().join("c"),
            None,
        );
        locked_path_environment(
            &flox,
            &tempdir,
            "b",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../c\" }]"),
        );
        let mut a = locked_path_environment(
            &flox,
            &tempdir,
            "a",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../b\" }]"),
        );

        upgrade_locked_packages(&c, &flox);

        let (lockfile, followed) = follow(&mut a, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["b"], vec![], vec![], vec![])
        );
        assert_eq!(
            sorted_packages(&lockfile),
            sorted_packages(&c.existing_lockfile(&flox).unwrap().unwrap())
        );
    }

    /// An included environment's upgrades of packages that the composing
    /// environment doesn't use, here for a system it isn't locked for, change
    /// only what its lockfile records, which isn't an unsaved change.
    #[test]
    fn lockfile_does_not_report_upgrades_of_packages_it_does_not_use() {
        let (flox, tempdir) = flox_instance();
        let included = new_path_environment_from_env_files_in(
            &flox,
            GENERATED_DATA.join("envs/hello"),
            tempdir.path().join("included"),
            None,
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema(indoc! {r#"
                [include]
                environments = [{ dir = "../included" }]

                [options]
                systems = ["aarch64-darwin"]
            "#}),
        );
        let committed = composer.existing_lockfile(&flox).unwrap().unwrap();

        edit_lockfile(&included, &flox, |lockfile| {
            for package in &mut lockfile.packages {
                if let LockedPackage::Catalog(package) = package
                    && package.system != "aarch64-darwin"
                {
                    package.derivation = format!("{}-upgraded", package.derivation);
                }
            }
        });

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(followed_names(&followed), (vec![], vec![], vec![], vec![]));
        assert_eq!(lockfile, committed);
    }

    /// The composing environment's own upgrades of packages that an included
    /// environment provides are kept, by following and by
    /// 'flox include upgrade', until the included environment locks
    /// something new.
    #[test]
    fn lockfile_keeps_upgrades_of_the_composer_until_its_include_upgrades() {
        let (flox, tempdir) = flox_instance();
        let (mut composer, included) = composer_including_hello(&flox, &tempdir);

        upgrade_locked_packages_by(&composer, &flox, "composer");
        let upgraded = composer.existing_lockfile(&flox).unwrap().unwrap();
        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(followed_names(&followed), (vec![], vec![], vec![], vec![]));
        assert_eq!(lockfile, upgraded);
        new_command(&mut composer);
        composer.include_upgrade(&flox, vec![]).unwrap();
        assert_eq!(
            composer.existing_lockfile(&flox).unwrap().unwrap(),
            upgraded
        );

        upgrade_locked_packages_by(&included, &flox, "included");
        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["included"], vec![], vec![], vec![])
        );
        assert_eq!(
            sorted_packages(&lockfile),
            sorted_packages(&included.existing_lockfile(&flox).unwrap().unwrap())
        );
    }

    /// A lockfile from a version of Flox that didn't record what an included
    /// environment locked follows it as before, and isn't rewritten just to
    /// record it.
    #[test]
    fn lockfile_without_recorded_include_locks_follows_as_before() {
        let (flox, tempdir) = flox_instance();
        let (mut composer, included) = composer_including_hello(&flox, &tempdir);
        edit_lockfile(&composer, &flox, |lockfile| {
            for include in &mut lockfile.compose.as_mut().unwrap().include {
                include.packages_hash = None;
            }
        });
        let lockfile_path = composer.lockfile_path(&flox).unwrap();
        let before = fs::read(&lockfile_path).unwrap();

        new_command(&mut composer);
        composer.include_upgrade(&flox, vec![]).unwrap();
        assert_eq!(fs::read(&lockfile_path).unwrap(), before);

        upgrade_locked_packages(&included, &flox);
        let (_, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(followed_names(&followed), (vec![], vec![], vec![], vec![]));
    }

    /// The packages in a lockfile, in a stable order
    fn sorted_packages(lockfile: &Lockfile) -> Vec<LockedPackage> {
        lockfile
            .packages
            .iter()
            .cloned()
            .sorted_by_key(|package| (package.install_id().to_string(), package.system().clone()))
            .collect()
    }

    /// Following copies the packages that an included environment locked,
    /// including ones it upgraded, instead of resolving them,
    /// which would fail with the mock catalog client.
    #[test]
    fn lockfile_follows_packages_locked_by_path_include() {
        let (flox, tempdir) = flox_instance();
        let mut included = new_path_environment_from_env_files_in(
            &flox,
            GENERATED_DATA.join("envs/hello"),
            tempdir.path().join("included"),
            None,
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../included\" }]"),
        );

        edit_and_lock(
            &mut included,
            &flox,
            &hello_manifest("[vars]\nincluded = \"v2\""),
        );
        upgrade_locked_packages(&included, &flox);

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["included"], vec![], vec![], vec![])
        );
        assert_eq!(
            sorted_packages(&lockfile),
            sorted_packages(&included.existing_lockfile(&flox).unwrap().unwrap())
        );
    }

    /// A -> B -> C: the packages that C locked reach A through B,
    /// without B locking them and without resolving them.
    #[test]
    fn lockfile_follows_packages_locked_by_nested_path_include() {
        let (flox, tempdir) = flox_instance();
        let mut c = new_path_environment_from_env_files_in(
            &flox,
            GENERATED_DATA.join("envs/hello"),
            tempdir.path().join("c"),
            None,
        );
        let b = locked_path_environment(
            &flox,
            &tempdir,
            "b",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../c\" }]"),
        );
        let mut a = locked_path_environment(
            &flox,
            &tempdir,
            "a",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../b\" }]"),
        );
        let b_lockfile_before = fs::read(b.lockfile_path(&flox).unwrap()).unwrap();

        edit_and_lock(&mut c, &flox, &hello_manifest("[vars]\nc = \"v2\""));
        upgrade_locked_packages(&c, &flox);

        let (lockfile, followed) = follow(&mut a, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["b"], vec![], vec![], vec![])
        );
        assert_eq!(locked_vars(&lockfile), vars_map(&[("c", "v2")]));
        assert_eq!(
            sorted_packages(&lockfile),
            sorted_packages(&c.existing_lockfile(&flox).unwrap().unwrap())
        );
        assert_eq!(
            b_lockfile_before,
            fs::read(b.lockfile_path(&flox).unwrap()).unwrap()
        );
    }

    /// The lockfile that the rendered environment links point to
    fn rendered_lockfile(environment: &PathEnvironment) -> Lockfile {
        let path = environment.rendered_env_links.dev.join(LOCKFILE_FILENAME);
        Lockfile::read_from_file(&CanonicalPath::new(path).unwrap()).unwrap()
    }

    /// A command that changes the lockfile keeps the latest changes to
    /// followed includes in the rendered environment, which it rebuilt.
    #[test]
    fn edit_keeps_followed_changes_in_rendered_environment() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        let include = "[include]\nenvironments = [{ dir = \"../included\" }]";
        let mut composer =
            locked_path_environment(&flox, &tempdir, "composer", &with_latest_schema(include));
        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema("[vars]\nincluded = \"v2\""),
        );
        follow(&mut composer, &flox, FollowMode::LockAndBuild);

        new_command(&mut composer);
        composer
            .edit(
                &flox,
                with_latest_schema(format!("[vars]\ncomposer = \"v1\"\n{include}")),
            )
            .unwrap();
        assert_eq!(
            locked_vars(&rendered_lockfile(&composer)),
            vars_map(&[("composer", "v1"), ("included", "v2")])
        );
    }

    /// A composer at `<tempdir>/composer` that includes `owner/remote` with
    /// `include_options`, and the included environment on FloxHub
    fn composer_including_remote(
        flox: &Flox,
        tempdir: &TempDir,
        include_options: &str,
    ) -> (PathEnvironment, RemoteEnvironment) {
        let remote = mock_remote_environment(
            flox,
            &with_latest_schema("[vars]\nremote = \"v1\""),
            "owner".parse().unwrap(),
            Some("remote"),
        );
        let composer = locked_path_environment(
            flox,
            tempdir,
            "composer",
            &with_latest_schema(format!(
                "[include]\nenvironments = [{{ remote = \"owner/remote\"{include_options} }}]"
            )),
        );
        (composer, remote)
    }

    /// Following reads an environment included from FloxHub as it was last
    /// fetched, without contacting FloxHub, and uses its new generation once
    /// the background upgrade check fetches it.
    #[test]
    fn lockfile_follows_remote_include_as_last_fetched() {
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(&"owner".parse().unwrap()));
        let (mut composer, mut remote) =
            composer_including_remote(&flox, &tempdir, ", auto-upgrade = true");
        remote
            .edit(&flox, with_latest_schema("[vars]\nremote = \"v2\""))
            .unwrap();
        remote.push(&flox, true).unwrap();
        // Like a machine that hasn't fetched the new generation
        fs::remove_dir_all(floxmeta_dir(&flox, &"owner".parse().unwrap())).unwrap();

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(followed_names(&followed), (vec![], vec![], vec![], vec![]));
        assert_eq!(locked_vars(&lockfile), vars_map(&[("remote", "v1")]));

        composer.fetch_included_remote_environments(&flox).unwrap();
        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["remote"], vec![], vec![], vec![])
        );
        assert_eq!(locked_vars(&lockfile), vars_map(&[("remote", "v2")]));
    }

    /// An environment included from FloxHub that isn't followed is reported
    /// when the generation last fetched is newer than the locked one.
    #[test]
    fn lockfile_reports_new_generation_of_remote_include_that_is_not_followed() {
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(&"owner".parse().unwrap()));
        let (mut composer, mut remote) = composer_including_remote(&flox, &tempdir, "");
        let (_, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(followed.upstream_changes, vec![]);

        remote
            .edit(&flox, with_latest_schema("[vars]\nremote = \"v2\""))
            .unwrap();
        remote.push(&flox, true).unwrap();
        composer.fetch_included_remote_environments(&flox).unwrap();

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(followed.upstream_changes, vec![
            "owner/remote".parse::<RemoteEnvironmentRef>().unwrap()
        ]);
        assert_eq!(locked_vars(&lockfile), vars_map(&[("remote", "v1")]));
    }

    /// A sync branch fetched before the generation that the lockfile records,
    /// e.g. on a machine that fetched before a teammate saved a newer one,
    /// doesn't take the composer back to an older generation.
    #[test]
    fn lockfile_keeps_remote_include_locked_at_generation_newer_than_last_fetched() {
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(&"owner".parse().unwrap()));
        let (mut composer, mut remote) =
            composer_including_remote(&flox, &tempdir, ", auto-upgrade = true");
        let pointer = remote.pointer().clone();
        let floxmeta = FloxMeta::open_local(&flox, &pointer).unwrap();
        let sync_branch = remote_branch_name(&pointer);
        let fetched_before = floxmeta.git.branch_hash(&sync_branch).unwrap();
        remote
            .edit(&flox, with_latest_schema("[vars]\nremote = \"v2\""))
            .unwrap();
        remote.push(&flox, true).unwrap();
        new_command(&mut composer);
        composer.include_upgrade(&flox, vec![]).unwrap();

        floxmeta
            .git
            .reset_branch(&sync_branch, &fetched_before)
            .unwrap();
        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(followed_names(&followed), (vec![], vec![], vec![], vec![]));
        assert_eq!(locked_vars(&lockfile), vars_map(&[("remote", "v2")]));
    }

    /// A -> B -> owner/remote: when this machine hasn't fetched the remote
    /// environment, A keeps the version of B in its lockfile rather than B
    /// with the older version of the remote environment in B's lockfile.
    #[test]
    fn lockfile_keeps_path_include_whose_remote_include_was_not_fetched() {
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(&"owner".parse().unwrap()));
        let mut remote = mock_remote_environment(
            &flox,
            &with_latest_schema("[vars]\nremote = \"v1\""),
            "owner".parse().unwrap(),
            Some("remote"),
        );
        locked_path_environment(
            &flox,
            &tempdir,
            "b",
            &with_latest_schema(
                "[include]\nenvironments = [{ remote = \"owner/remote\", auto-upgrade = true }]",
            ),
        );
        let mut a = locked_path_environment(
            &flox,
            &tempdir,
            "a",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../b\" }]"),
        );
        remote
            .edit(&flox, with_latest_schema("[vars]\nremote = \"v2\""))
            .unwrap();
        remote.push(&flox, true).unwrap();
        new_command(&mut a);
        a.include_upgrade(&flox, vec![]).unwrap();

        fs::remove_dir_all(floxmeta_dir(&flox, &"owner".parse().unwrap())).unwrap();
        let (lockfile, followed) = follow(&mut a, &flox, FollowMode::Lock);
        assert_eq!(followed_names(&followed), (vec![], vec![], vec![], vec![]));
        assert_eq!(locked_vars(&lockfile), vars_map(&[("remote", "v2")]));
    }

    /// The notice about new generations on FloxHub is an upgrade notification,
    /// which an environment can turn off
    #[test]
    fn lockfile_does_not_report_new_generations_without_upgrade_notifications() {
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(&"owner".parse().unwrap()));
        let (mut composer, mut remote) = composer_including_remote(&flox, &tempdir, "");
        edit_and_lock(
            &mut composer,
            &flox,
            &with_latest_schema(
                "[include]\nenvironments = [{ remote = \"owner/remote\" }]\n[options]\nactivate.upgrade-notifications = false",
            ),
        );
        remote
            .edit(&flox, with_latest_schema("[vars]\nremote = \"v2\""))
            .unwrap();
        remote.push(&flox, true).unwrap();
        composer.fetch_included_remote_environments(&flox).unwrap();

        let (_, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(followed.upstream_changes, vec![]);
    }

    /// An include cycle in an existing lockfile, which earlier versions could
    /// lock, keeps the version in use rather than failing or growing.
    #[test]
    fn lockfile_keeps_path_include_that_became_a_cycle() {
        let (flox, tempdir) = flox_instance();
        locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../included\" }]"),
        );

        // The include now resolves to the composer itself
        fs::remove_dir_all(tempdir.path().join("included")).unwrap();
        std::os::unix::fs::symlink(
            tempdir.path().join("composer"),
            tempdir.path().join("included"),
        )
        .unwrap();

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec![], vec!["included"], vec![], vec![])
        );
        assert!(matches!(
            followed.unreadable[0].reason.as_ref(),
            EnvironmentError::Recoverable(RecoverableMergeError::IncludeCycle(_))
        ));
        assert_eq!(locked_vars(&lockfile), vars_map(&[("included", "v1")]));
    }

    /// Adding an include that closes a cycle is an error, even though the
    /// environment it includes is locked with the other half of the cycle.
    #[test]
    fn lockfile_errors_on_new_include_cycle() {
        let (flox, tempdir) = flox_instance();
        let mut b = locked_path_environment(
            &flox,
            &tempdir,
            "b",
            &with_latest_schema("[vars]\nb = \"v1\""),
        );
        let a = locked_path_environment(
            &flox,
            &tempdir,
            "a",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../b\" }]"),
        );

        fs::write(
            b.manifest_path(&flox).unwrap(),
            with_latest_schema("[include]\nenvironments = [{ dir = \"../a\" }]"),
        )
        .unwrap();

        let err = b.lockfile(&flox).unwrap_err();
        let EnvironmentError::Recoverable(RecoverableMergeError::Fetch { err, .. }) = err else {
            panic!("expected fetching a to fail, got: {err:?}");
        };
        let EnvironmentError::Recoverable(RecoverableMergeError::IncludeCycle(cycle)) = *err else {
            panic!("expected an include cycle, got: {err:?}");
        };
        assert_eq!(cycle, vec![
            b.dot_flox_path().to_path_buf(),
            a.dot_flox_path().to_path_buf(),
            b.dot_flox_path().to_path_buf(),
        ]);
    }

    /// Changes that can't be locked together keep the versions in use,
    /// here because a renamed included environment takes another's name.
    #[test]
    fn lockfile_keeps_versions_in_use_when_changes_cannot_be_locked() {
        let (flox, tempdir) = flox_instance();
        let mut included1 = locked_path_environment(
            &flox,
            &tempdir,
            "included1",
            &with_latest_schema("[vars]\nincluded1 = \"v1\""),
        );
        locked_path_environment(
            &flox,
            &tempdir,
            "included2",
            &with_latest_schema("[vars]\nincluded2 = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema(indoc! {r#"
                [include]
                environments = [{ dir = "../included1" }, { dir = "../included2" }]
            "#}),
        );

        included1.rename("included2".parse().unwrap()).unwrap();

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec![], vec![], vec!["included1"], vec![])
        );
        assert_eq!(
            locked_vars(&lockfile),
            vars_map(&[("included1", "v1"), ("included2", "v1")])
        );
    }

    /// Changes that lock but don't build, here a cycle between the vars of the
    /// composer and an included environment, fall back to the lockfile,
    /// which is remembered until they change.
    #[test]
    fn lockfile_falls_back_to_lockfile_when_changes_do_not_build() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\ny = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema(indoc! {r#"
                [vars]
                x = "$y"
                [include]
                environments = [{ dir = "../included" }]
            "#}),
        );

        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema("[vars]\ny = \"$x\""),
        );

        for mode in [FollowMode::LockAndBuild, FollowMode::Lock] {
            let (lockfile, followed) = follow(&mut composer, &flox, mode);
            assert!(matches!(
                composer
                    .followed_lockfiles()
                    .read(&flox.system)
                    .map(|copy| copy.build),
                Some(followed_includes::FollowedBuild::Failed(_))
            ));
            assert_eq!(
                followed_names(&followed),
                (vec![], vec![], vec![], vec!["included"])
            );
            assert_eq!(
                locked_vars(&lockfile),
                vars_map(&[("x", "$y"), ("y", "v1")])
            );
        }
    }

    /// Changes that need a newer schema than the composer's are used without
    /// touching the composer's manifest, since only the copy has them.
    #[test]
    fn lockfile_follows_changes_that_require_newer_schema() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_schema(KnownSchemaVersion::V1_10_0, "[vars]\nincluded = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_schema(
                KnownSchemaVersion::V1_10_0,
                "[include]\nenvironments = [{ dir = \"../included\" }]",
            ),
        );
        let manifest_path = composer.manifest_path(&flox).unwrap();
        let manifest_before = fs::read(&manifest_path).unwrap();

        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema(indoc! {r#"
                [vars]
                included = "v2"
                [options.activate]
                upgrade-notifications = false
            "#}),
        );

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["included"], vec![], vec![], vec![])
        );
        assert_eq!(locked_vars(&lockfile), vars_map(&[("included", "v2")]));
        assert_eq!(manifest_before, fs::read(&manifest_path).unwrap());
    }

    /// 'flox include upgrade' fails rather than saving older versions of
    /// nested includes than the ones in use, and keeps the copy in use.
    #[test]
    fn include_upgrade_fails_when_nested_include_cannot_be_read() {
        let (flox, tempdir) = flox_instance();
        let mut c = locked_path_environment(
            &flox,
            &tempdir,
            "c",
            &with_latest_schema("[vars]\nc = \"v1\""),
        );
        locked_path_environment(
            &flox,
            &tempdir,
            "b",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../c\" }]"),
        );
        let mut a = locked_path_environment(
            &flox,
            &tempdir,
            "a",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../b\" }]"),
        );
        edit_and_lock(&mut c, &flox, &with_latest_schema("[vars]\nc = \"v2\""));
        follow(&mut a, &flox, FollowMode::Lock);

        fs::write(
            c.manifest_path(&flox).unwrap(),
            with_latest_schema("[vars]\nc = \"v3\""),
        )
        .unwrap();

        new_command(&mut a);
        a.include_upgrade(&flox, vec![]).unwrap_err();
        let (lockfile, followed) = follow(&mut a, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["b"], vec!["b"], vec![], vec![])
        );
        assert_eq!(locked_vars(&lockfile), vars_map(&[("c", "v2")]));
    }

    /// A -> B -> C: when C can't be read anymore, A keeps B with the version
    /// of C in use, rather than the older one in B's lockfile.
    #[test]
    fn lockfile_keeps_version_in_use_when_nested_include_cannot_be_read() {
        let (flox, tempdir) = flox_instance();
        let mut c = locked_path_environment(
            &flox,
            &tempdir,
            "c",
            &with_latest_schema("[vars]\nc = \"v1\""),
        );
        locked_path_environment(
            &flox,
            &tempdir,
            "b",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../c\" }]"),
        );
        let mut a = locked_path_environment(
            &flox,
            &tempdir,
            "a",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../b\" }]"),
        );
        edit_and_lock(&mut c, &flox, &with_latest_schema("[vars]\nc = \"v2\""));
        follow(&mut a, &flox, FollowMode::Lock);

        fs::remove_dir_all(tempdir.path().join("c")).unwrap();

        let (lockfile, followed) = follow(&mut a, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["b"], vec!["b"], vec![], vec![])
        );
        assert_eq!(locked_vars(&lockfile), vars_map(&[("c", "v2")]));
    }

    /// New changes are added to the changes already in use.
    #[test]
    fn lockfile_adds_new_changes_to_changes_in_use() {
        let (flox, tempdir) = flox_instance();
        let mut included1 = locked_path_environment(
            &flox,
            &tempdir,
            "included1",
            &with_latest_schema("[vars]\nincluded1 = \"v1\""),
        );
        let mut included2 = locked_path_environment(
            &flox,
            &tempdir,
            "included2",
            &with_latest_schema("[vars]\nincluded2 = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema(indoc! {r#"
                [include]
                environments = [{ dir = "../included1" }, { dir = "../included2" }]
            "#}),
        );

        edit_and_lock(
            &mut included1,
            &flox,
            &with_latest_schema("[vars]\nincluded1 = \"v2\""),
        );
        follow(&mut composer, &flox, FollowMode::Lock);
        edit_and_lock(
            &mut included2,
            &flox,
            &with_latest_schema("[vars]\nincluded2 = \"v2\""),
        );

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["included1", "included2"], vec![], vec![], vec![])
        );
        assert_eq!(
            locked_vars(&lockfile),
            vars_map(&[("included1", "v2"), ("included2", "v2")])
        );
    }

    /// When the lockfile changes, the latest changes to the included
    /// environments are copied again on top of it.
    #[test]
    fn lockfile_copies_changes_in_use_again_after_lockfile_changes() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../included\" }]"),
        );
        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema("[vars]\nincluded = \"v2\""),
        );
        follow(&mut composer, &flox, FollowMode::Lock);

        edit_and_lock(
            &mut composer,
            &flox,
            &with_latest_schema(indoc! {r#"
                [vars]
                composer = "v2"
                [include]
                environments = [{ dir = "../included" }]
            "#}),
        );

        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(
            followed_names(&followed),
            (vec!["included"], vec![], vec![], vec![])
        );
        assert_eq!(
            locked_vars(&lockfile),
            vars_map(&[("composer", "v2"), ("included", "v2")])
        );
    }

    /// Building uses the changes in use, while publishing builds the
    /// lockfile.
    #[test]
    fn build_uses_changes_in_use_and_build_locked_does_not() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../included\" }]"),
        );
        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema("[vars]\nincluded = \"v2\""),
        );
        let (in_use, _) = follow(&mut composer, &flox, FollowMode::Lock);
        let rendered_lockfile = |composer: &PathEnvironment| {
            Lockfile::read_from_file(
                &CanonicalPath::new(composer.rendered_env_links.dev.join(LOCKFILE_FILENAME))
                    .unwrap(),
            )
            .unwrap()
        };

        composer.build(&flox).unwrap();
        assert_eq!(rendered_lockfile(&composer), in_use);

        composer.build_locked(&flox).unwrap();
        assert_eq!(
            Some(rendered_lockfile(&composer)),
            composer.existing_lockfile(&flox).unwrap()
        );
    }

    /// 'flox include upgrade' saves the changes that were in use to the
    /// lockfile.
    #[test]
    fn include_upgrade_saves_followed_changes() {
        let (flox, tempdir) = flox_instance();
        let mut included = locked_path_environment(
            &flox,
            &tempdir,
            "included",
            &with_latest_schema("[vars]\nincluded = \"v1\""),
        );
        let mut composer = locked_path_environment(
            &flox,
            &tempdir,
            "composer",
            &with_latest_schema("[include]\nenvironments = [{ dir = \"../included\" }]"),
        );
        edit_and_lock(
            &mut included,
            &flox,
            &with_latest_schema("[vars]\nincluded = \"v2\""),
        );
        let (followed_lockfile, _) = follow(&mut composer, &flox, FollowMode::Lock);

        composer.include_upgrade(&flox, vec![]).unwrap();
        assert!(composer.followed_lockfiles().read(&flox.system).is_none());

        // The lockfile gets exactly what was in use, not a new lock of it
        let saved = composer.existing_lockfile(&flox).unwrap().unwrap();
        assert_eq!(saved, followed_lockfile);
        let (lockfile, followed) = follow(&mut composer, &flox, FollowMode::Lock);
        assert_eq!(followed_names(&followed), (vec![], vec![], vec![], vec![]));
        assert_eq!(lockfile, saved);
    }

    /// v1 schema environment (no packages).
    /// After locking once, calling build() must not alter manifest.lock.
    /// Exercises the v1 → ensure_locked path in lockfile_if_up_to_date.
    #[test]
    fn build_does_not_rewrite_current_lockfile_v1() {
        let (flox, _temp_dir) = flox_instance();
        let manifest = with_schema(KnownSchemaVersion::V1, "");
        let mut env = new_path_environment(&flox, &manifest);

        // Lock the environment to produce an up-to-date manifest.lock.
        env.lockfile(&flox).unwrap();

        let lockfile_path = env.lockfile_path(&flox).unwrap();
        let bytes_before = fs::read(&lockfile_path).unwrap();
        let mtime_before = fs::metadata(&lockfile_path).unwrap().modified().unwrap();

        // Build must not rewrite the lockfile.
        env.build(&flox).unwrap();

        let bytes_after = fs::read(&lockfile_path).unwrap();
        let mtime_after = fs::metadata(&lockfile_path).unwrap().modified().unwrap();

        assert_eq!(bytes_before, bytes_after, "lockfile bytes changed");
        assert_eq!(mtime_before, mtime_after, "lockfile mtime changed");
    }

    // -------------------------------------------------------------------------
    // Cross-release coverage: a current-release activate must accept the build
    // stamp left by an earlier release without triggering a needless rebuild.
    //
    // Fixtures live in test_data/manually_generated/prior_release_baselines/
    // and are captured by the `regen-prior-release-fixtures` Justfile recipe.
    // -------------------------------------------------------------------------

    /// Building a prior-release manifest + lockfile with the current release
    /// and then checking `needs_rebuild()` must return false: the current
    /// release accepts the prior-release build state without rebuilding.
    #[test]
    fn needs_rebuild_accepts_prior_release_stamp_plain() {
        use flox_test_utils::MANUALLY_GENERATED;

        let (flox, _temp_dir) = flox_instance();

        let base = MANUALLY_GENERATED
            .join("prior_release_baselines")
            .join("plain");

        // new_path_environment_from_env_files reads manifest.toml and
        // manifest.lock from the given directory and writes them into a
        // fresh .flox/env/ directory, giving us a PathEnvironment with the
        // prior-release lockfile already in place.
        let mut env = new_path_environment_from_env_files(&flox, &base);

        // Before the first build, the rendered-env link does not exist, so
        // needs_rebuild() returns true — the env needs to be built once.
        let lockfile = env.existing_lockfile(&flox).unwrap().unwrap();
        assert!(
            env.needs_rebuild(&lockfile),
            "needs_rebuild() should return true before first build"
        );

        // Lock (no-op if the prior-release lockfile is up-to-date) and
        // build, creating the rendered-env stamp.
        env.build(&flox)
            .expect("build should succeed with the prior-release lockfile");

        // After the build, needs_rebuild() must return false: the
        // rendered-env stamp's lockfile matches the env's lockfile.
        assert!(
            !env.needs_rebuild(&lockfile),
            "needs_rebuild() returned true after building from a prior-release \
             lockfile: the rendered-env stamp's lockfile diverged from the \
             env's lockfile across releases",
        );
    }
}
