use std::fs;
use std::path::{Path, PathBuf};

use flox_core::activate::mode::ActivateMode;
use flox_core::data::environment_ref::{EnvironmentName, EnvironmentOwner, RemoteEnvironmentRef};
use flox_manifest::lockfile::Lockfile;
use flox_manifest::raw::PackageToInstall;
use flox_manifest::{Manifest, Migrated, Validated};
use thiserror::Error;
use tracing::debug;

use super::core_environment::UpgradeResult;
use super::fetcher::IncludeFetcher;
use super::generations::{
    AllGenerationsMetadata,
    GenerationId,
    GenerationsError,
    GenerationsExt,
    WithOtherFields,
};
use super::managed_environment::{ManagedEnvironment, ManagedEnvironmentError};
use super::{
    CanonicalPath,
    CanonicalizeError,
    DOT_FLOX,
    ENVIRONMENT_POINTER_FILENAME,
    EditResult,
    Environment,
    EnvironmentError,
    GCROOTS_DIR_NAME,
    InstallationAttempt,
    ManagedPointer,
    RenderedEnvironmentLinks,
    UninstallationAttempt,
};
use crate::flox::Flox;
use crate::models::environment::PathPointer;
use crate::models::environment::floxmeta_branch::{
    BranchOrd,
    FloxmetaBranch,
    FloxmetaBranchError,
    GenerationLock,
    write_generation_lock,
};
use crate::models::environment::generations::SyncToGenerationResult;
use crate::models::environment::managed_environment::GENERATION_LOCK_FILENAME;
use crate::models::environment::path_environment::{InitCustomization, PathEnvironment};
use crate::providers::buildenv::BuildEnvOutputs;
use crate::providers::lock_manifest::LockResult;

const REMOTE_ENVIRONMENT_BASE_DIR: &str = "remote";

#[derive(Debug, Error)]
pub enum RemoteEnvironmentError {
    #[error("open managed environment")]
    OpenManagedEnvironment(#[source] ManagedEnvironmentError),

    #[error("could not create gc-root directory")]
    CreateGcRootDir(#[source] std::io::Error),

    #[error("could not get latest version of environment")]
    GetLatestVersion(#[source] FloxmetaBranchError),

    #[error("could not reset managed environment")]
    ResetManagedEnvironment(#[source] ManagedEnvironmentError),

    #[error("could not update upstream environment")]
    UpdateUpstream(#[source] ManagedEnvironmentError),

    #[error("invalid temporary path for new environment")]
    InvalidTempPath(#[source] CanonicalizeError),

    #[error("could not create temporary environment")]
    CreateTempDotFlox(#[source] std::io::Error),

    /// the internal [ManagedEnvironment::activation_path] returned an invalid path
    #[error("could not determine location of new install prefix")]
    ReadInternalOutLink(#[source] std::io::Error),

    #[error("could not remove the existing install prefix")]
    DeleteOldOutLink(#[source] std::io::Error),

    #[error("could not set a new install prefix")]
    WriteNewOutlink(#[source] std::io::Error),

    #[error("generations error")]
    Generations(#[source] GenerationsError),
}

#[derive(Debug)]
pub struct RemoteEnvironment {
    inner: ManagedEnvironment,
    /// Specific generation to use, i.e. from `flox activate`
    /// This doesn't represent the live generation.
    generation: Option<GenerationId>,
}

impl RemoteEnvironment {
    /// Check if a remote environment is already cached locally.
    /// I.e. whether there is a backing managed environment in the cache.
    pub fn is_cached(flox: &Flox, pointer: &ManagedPointer) -> bool {
        let path = flox
            .cache_dir
            .join(REMOTE_ENVIRONMENT_BASE_DIR)
            .join(pointer.owner.as_ref())
            .join(pointer.name.as_ref())
            .join(DOT_FLOX);
        path.exists()
    }

    /// Pull a remote environment into a flox-provided managed environment
    /// in `<FLOX_CACHE_DIR>/remote/<owner>/<name>`
    ///
    /// This function provides the sensible default directory to [RemoteEnvironment::new_in].
    /// The directory will be created by [RemoteEnvironment::new_in].
    pub fn new(
        flox: &Flox,
        pointer: ManagedPointer,
        generation: Option<GenerationId>,
    ) -> Result<Self, RemoteEnvironmentError> {
        let path = flox
            .cache_dir
            .join(REMOTE_ENVIRONMENT_BASE_DIR)
            .join(pointer.owner.as_ref())
            .join(pointer.name.as_ref());

        Self::new_in(flox, path, pointer, generation)
    }

    /// Pull a remote environment into a provided (temporary) managed environment.
    /// Constructing a [RemoteEnvironment] _does not_ create a gc-root
    /// or guarantee that the environment is valid.
    pub fn new_in(
        flox: &Flox,
        path: impl AsRef<Path>,
        pointer: ManagedPointer,
        generation: Option<GenerationId>,
    ) -> Result<Self, RemoteEnvironmentError> {
        let path = path.as_ref().join(DOT_FLOX);
        fs::create_dir_all(&path).map_err(RemoteEnvironmentError::CreateTempDotFlox)?;

        let dot_flox_path =
            CanonicalPath::new(&path).map_err(RemoteEnvironmentError::InvalidTempPath)?;

        // Read existing lockfile
        let lock_path = dot_flox_path.join(GENERATION_LOCK_FILENAME);
        let maybe_lock = GenerationLock::read_maybe(&lock_path)
            .map_err(ManagedEnvironmentError::from)
            .map_err(RemoteEnvironmentError::OpenManagedEnvironment)?;

        let (floxmeta_branch, lock) =
            FloxmetaBranch::new(flox, &pointer, &dot_flox_path, maybe_lock)
                .map_err(RemoteEnvironmentError::GetLatestVersion)?;

        write_generation_lock(lock_path, &lock)
            .map_err(ManagedEnvironmentError::from)
            .map_err(RemoteEnvironmentError::OpenManagedEnvironment)?;

        let pointer_content = serde_json::to_string_pretty(&pointer).unwrap();

        fs::write(
            dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME),
            pointer_content,
        )
        .unwrap();

        let inner_rendered_env_links = {
            let gcroots_dir = dot_flox_path.join(GCROOTS_DIR_NAME);

            // `.flox/run` used to be a link until flox versions 1.3.3!
            // If we find a symlink, we need to delete it to create a directory
            // with symlinked files in the following step.
            if gcroots_dir.is_symlink() {
                // returns true if the file exists and is a symlink
                debug!(gcroot=?gcroots_dir, "removing symlink");
                fs::remove_file(&gcroots_dir).map_err(RemoteEnvironmentError::CreateGcRootDir)?;
            }

            if !gcroots_dir.exists() {
                debug!(path = %gcroots_dir.display(), "creating gc roots directory");
                std::fs::create_dir_all(&gcroots_dir)
                    .map_err(RemoteEnvironmentError::CreateGcRootDir)?;
            }

            let base_dir =
                CanonicalPath::new(gcroots_dir).expect("gcroots_dir is not a valid path");

            RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system(
                &base_dir,
                pointer.name.as_ref(),
                &flox.system,
            )
        };

        let inner = ManagedEnvironment::open_with(
            flox,
            floxmeta_branch,
            pointer.clone(),
            dot_flox_path,
            inner_rendered_env_links,
            // remote environments shouldn't be able to fetch dir includes,
            // so set base_directory to None
            IncludeFetcher {
                base_directory: None,
            },
            generation,
        )
        .map_err(RemoteEnvironmentError::OpenManagedEnvironment)?;

        // Note: We used to have links for RemoteEnvironments in two places
        //
        // 1. the links associated with the inner managed env.
        //    These may be updated but ultimately fail to push,
        //    rendering the remote environment inconsistent with the remote.
        // 2. a separate set of links in ~/.cache/flox/remote
        //    updated upon successful push to avoid the caveat above.
        //
        // Neither reason is relevant any longer, as we explicitly
        // _want_ to allow the local state of floxhub environments to move independently.
        // We therefore only track links for the inner managed environment going forward.
        // To avoid stale gcroots, we remove the additional dir in ~/.cache/flox/remote/
        {
            // Directory containing nix gc roots for (previous) builds of environments of a given owner
            let gcroots_dir = {
                let owner: &EnvironmentOwner = &pointer.owner;
                flox.cache_dir.join(GCROOTS_DIR_NAME).join(owner.as_str())
            };

            if gcroots_dir.exists() {
                debug!(
                    owner=%&pointer.owner,
                    gcroots_dir=?gcroots_dir,
                    "found existing legacy gcroot base dir for remote environments");

                let base_dir =
                    CanonicalPath::new(gcroots_dir).expect("gcroots_dir is not a valid path");

                let old_links = RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system(
                    &base_dir,
                    pointer.name.as_ref(),
                    &flox.system,
                );

                if old_links.development.is_symlink() {
                    debug!(
                        out_link=?old_links.development,
                        "deleting legacy outlink");
                    std::fs::remove_file(&old_links.development)
                        .map_err(RemoteEnvironmentError::DeleteOldOutLink)?;
                }
                if old_links.runtime.is_symlink() {
                    debug!(
                        out_link=?old_links.runtime,
                        "deleting legacy outlink");
                    std::fs::remove_file(&old_links.runtime)
                        .map_err(RemoteEnvironmentError::DeleteOldOutLink)?;
                }

                // if all links of environments of the same owner have been removed, remove owner dir as well
                let is_dir_empty = fs::read_dir(&base_dir)
                    .ok()
                    .map(|mut entries| entries.next().is_none())
                    .unwrap_or(false);

                if is_dir_empty {
                    debug!(
                        base_dir=?base_dir,
                        "deleting empty legacy outlink base_dir");
                    fs::remove_dir(&base_dir).map_err(RemoteEnvironmentError::DeleteOldOutLink)?;
                }
            }
        };

        // Note: Remote environments used to get reset to the latest upstream here.
        // Now they require explicit `pull`s to refresh upstream state.
        Ok(Self { inner, generation })
    }

    pub fn owner(&self) -> &EnvironmentOwner {
        self.inner.owner()
    }

    pub fn env_ref(&self) -> RemoteEnvironmentRef {
        RemoteEnvironmentRef::new_from_parts(self.owner().clone(), self.name())
    }

    pub fn pointer(&self) -> &ManagedPointer {
        self.inner.pointer()
    }

    /// Push local changes to FloxHub for this remote environment
    ///
    /// This pushes any local changes made to the cached remote environment back to FloxHub.
    pub fn push(
        &mut self,
        flox: &Flox,
        force: bool,
    ) -> Result<super::managed_environment::PushResult, EnvironmentError> {
        self.inner.push(flox, force)
    }

    /// Pull updates from FloxHub for this remote environment
    ///
    /// This updates the cached remote environment with the latest changes from FloxHub.
    pub fn pull(
        &mut self,
        flox: &Flox,
        force: bool,
    ) -> Result<super::managed_environment::PullResult, EnvironmentError> {
        let result = self.inner.pull(flox, force)?;
        Ok(result)
    }

    pub fn fetch_remote_state(&self, flox: &Flox) -> Result<(), EnvironmentError> {
        self.inner.fetch_remote_state(flox)?;
        Ok(())
    }

    /// Ensure that the environment `<env_ref>` is initialized on FloxHub.
    /// That is, attempt to create environment or use the existing one upstream.
    pub fn init_floxhub_environment(
        flox: &Flox,
        env_ref: RemoteEnvironmentRef,
        bare: bool,
    ) -> Result<RemoteEnvironment, EnvironmentError> {
        let temp_env_dir = tempfile::TempDir::new_in(&flox.temp_dir)
            .map_err(RemoteEnvironmentError::CreateTempDotFlox)?;

        let path_pointer = PathPointer::new(env_ref.name().clone());

        let path_environment = if bare {
            PathEnvironment::init_bare(path_pointer, temp_env_dir.path(), flox)?
        } else {
            let customization = InitCustomization {
                activate_mode: Some(ActivateMode::Run),
                ..Default::default()
            };

            PathEnvironment::init(path_pointer, temp_env_dir.path(), &customization, flox)?
        };

        let managed = ManagedEnvironment::push_new(
            flox,
            path_environment,
            env_ref.owner().clone(),
            false,
            true,
        )?;
        let pointer = managed.pointer();

        // validate that the environment exists
        let validated = RemoteEnvironment::new(flox, pointer.clone(), None)?;

        Ok(validated)
    }
}

impl Environment for RemoteEnvironment {
    /// Return the lockfile content,
    /// or error if the lockfile doesn't exist.
    fn lockfile(&mut self, flox: &Flox) -> Result<LockResult, EnvironmentError> {
        self.inner.lockfile(flox)
    }

    /// Returns the lockfile if it exists.
    fn existing_lockfile(&self, flox: &Flox) -> Result<Option<Lockfile>, EnvironmentError> {
        self.inner.existing_lockfile(flox)
    }

    fn pre_migration_manifest(&self, flox: &Flox) -> Result<Manifest<Validated>, EnvironmentError> {
        self.inner.pre_migration_manifest(flox)
    }

    fn manifest(&mut self, flox: &Flox) -> Result<Manifest<Migrated>, EnvironmentError> {
        self.inner.manifest(flox)
    }

    /// Install packages to the environment atomically
    fn install(
        &mut self,
        packages: &[PackageToInstall],
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError> {
        let result = self.inner.install(packages, flox)?;
        // TODO: clean up git branch for temporary environment
        Ok(result)
    }

    /// Uninstall packages from the environment atomically
    fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<UninstallationAttempt, EnvironmentError> {
        let result = self.inner.uninstall(packages, flox)?;

        Ok(result)
    }

    /// Atomically edit this environment, ensuring that it still builds
    fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError> {
        let result = self.inner.edit(flox, contents)?;
        if result == EditResult::Unchanged {
            return Ok(result);
        }

        Ok(result)
    }

    fn dry_upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[&str],
    ) -> Result<UpgradeResult, EnvironmentError> {
        self.inner.dry_upgrade(flox, groups_or_iids)
    }

    /// Atomically upgrade packages in this environment
    fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[&str],
    ) -> Result<UpgradeResult, EnvironmentError> {
        let result = self.inner.upgrade(flox, groups_or_iids)?;

        Ok(result)
    }

    /// Upgrade environment with latest changes to included environments.
    fn include_upgrade(
        &mut self,
        flox: &Flox,
        to_upgrade: Vec<String>,
    ) -> Result<UpgradeResult, EnvironmentError> {
        let result = self.inner.include_upgrade(flox, to_upgrade)?;

        Ok(result)
    }

    fn rendered_env_links(
        &mut self,
        flox: &Flox,
    ) -> Result<RenderedEnvironmentLinks, EnvironmentError> {
        if let Some(generation) = self.generation {
            return self.rendered_env_links_for_generation(flox, generation);
        }
        self.inner.rendered_env_links(flox)
    }

    fn build(
        &mut self,
        flox: &Flox,
    ) -> Result<crate::providers::buildenv::BuildEnvOutputs, EnvironmentError> {
        self.inner.build(flox)
    }

    fn link(&mut self, store_paths: &BuildEnvOutputs) -> Result<(), EnvironmentError> {
        self.inner.link(store_paths)
    }

    fn cache_path(&self) -> Result<CanonicalPath, EnvironmentError> {
        self.inner.cache_path()
    }

    fn log_path(&self) -> Result<CanonicalPath, EnvironmentError> {
        self.inner.log_path()
    }

    fn project_path(&self) -> Result<PathBuf, EnvironmentError> {
        std::env::current_dir().map_err(EnvironmentError::GetCurrentDir)
    }

    fn parent_path(&self) -> Result<PathBuf, EnvironmentError> {
        self.inner.parent_path()
    }

    /// Path to the environment's .flox directory
    fn dot_flox_path(&self) -> CanonicalPath {
        self.inner.dot_flox_path()
    }

    /// Path to the environment definition file
    fn manifest_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        self.inner.manifest_path(flox)
    }

    /// Path to the lockfile. The path may not exist.
    fn lockfile_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        self.inner.lockfile_path(flox)
    }

    /// Returns the environment name
    fn name(&self) -> EnvironmentName {
        self.inner.name()
    }

    /// Delete the Environment
    ///
    /// The local version of this is rather ... useless.
    /// It just deletes the temporary directory.
    /// When extended to delete upstream environments, this will be more useful.
    fn delete(self, flox: &Flox) -> Result<(), EnvironmentError> {
        self.inner.delete(flox)
    }

    fn services_socket_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        self.inner.services_socket_path(flox)
    }
}

impl GenerationsExt for RemoteEnvironment {
    fn generations_metadata(
        &self,
    ) -> Result<WithOtherFields<AllGenerationsMetadata>, GenerationsError> {
        self.inner.generations_metadata()
    }

    fn switch_generation(
        &mut self,
        flox: &Flox,
        generation: GenerationId,
    ) -> Result<(), EnvironmentError> {
        self.inner.switch_generation(flox, generation)?;
        Ok(())
    }

    fn remote_lockfile_contents_for_current_generation(&self) -> Result<String, GenerationsError> {
        self.inner.remote_lockfile_contents_for_current_generation()
    }

    fn remote_manifest_contents_for_current_generation(&self) -> Result<String, GenerationsError> {
        self.inner.remote_manifest_contents_for_current_generation()
    }

    fn lockfile_contents_for_generation(
        &self,
        generation: usize,
    ) -> Result<String, GenerationsError> {
        self.inner.generations().lockfile_contents(generation)
    }

    fn rendered_env_links_for_generation(
        &self,
        flox: &Flox,
        generation: GenerationId,
    ) -> Result<RenderedEnvironmentLinks, EnvironmentError> {
        // These are rendered in the managed environment's run dir rather than
        // `~/.flox/cache/run` because the environment is treated as immutable.
        self.inner
            .rendered_env_links_for_generation(flox, generation)
    }

    fn remote_generations_metadata(
        &self,
    ) -> Result<WithOtherFields<AllGenerationsMetadata>, GenerationsError> {
        self.inner.remote_generations_metadata()
    }

    fn compare_remote(&self) -> Result<BranchOrd, EnvironmentError> {
        self.inner.compare_remote()
    }

    fn create_generation_from_local_env(
        &mut self,
        flox: &Flox,
    ) -> Result<SyncToGenerationResult, EnvironmentError> {
        self.inner.create_generation_from_local_env(flox)
    }

    fn reset_local_env_to_current_generation(&self, flox: &Flox) -> Result<(), EnvironmentError> {
        self.inner.reset_local_env_to_current_generation(flox)
    }
}

#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    use tempfile::tempdir_in;

    use super::*;
    use crate::models::environment::managed_environment::test_helpers::mock_managed_environment_in;

    pub fn mock_remote_environment(
        flox: &Flox,
        contents: &str,
        owner: EnvironmentOwner,
        name: Option<&str>,
    ) -> RemoteEnvironment {
        let managed_environment = mock_managed_environment_in(
            flox,
            contents,
            owner,
            tempdir_in(&flox.temp_dir).unwrap().keep(),
            name,
        );
        RemoteEnvironment::new_in(
            flox,
            managed_environment.parent_path().unwrap(),
            managed_environment.pointer().clone(),
            None,
        )
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;
    use std::str::FromStr;

    use flox_manifest::interfaces::{AsWritableManifest, WriteManifest};
    use flox_manifest::test_helpers::with_latest_schema;
    use flox_test_utils::GENERATED_DATA;
    use indoc::indoc;

    use super::test_helpers::mock_remote_environment;
    use super::*;
    use crate::flox::test_helpers::flox_instance_with_optional_floxhub;
    use crate::models::environment::generations::HistoryKind;
    use crate::models::environment::managed_environment::test_helpers::mock_managed_environment_from_env_files;
    use crate::providers::lock_manifest::RecoverableMergeError;

    #[test]
    fn migrate_remote_gcroot_link_to_dir() {
        let owner = "owner".parse().unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        // Create a remote environment "owner/name"
        let environment = mock_managed_environment_from_env_files(
            &flox,
            GENERATED_DATA.join("envs").join("hello"),
            owner,
        );

        // Create a symlink, as it was done in older versions of flox prior to 1.3.4
        fs::remove_dir_all(environment.dot_flox_path().join(GCROOTS_DIR_NAME)).unwrap();
        symlink(
            "/dev/null",
            environment.dot_flox_path().join(GCROOTS_DIR_NAME),
        )
        .unwrap();

        assert!(
            environment
                .dot_flox_path()
                .join(GCROOTS_DIR_NAME)
                .is_symlink()
        );

        // Create a remote environment with the existing managed environment as its backend
        let _ = RemoteEnvironment::new_in(
            &flox,
            environment.parent_path().unwrap(),
            environment.pointer().clone(),
            None,
        )
        .unwrap();

        // Once created, the symlink should be replaced with a directory
        assert!(environment.dot_flox_path().join(GCROOTS_DIR_NAME).is_dir())
    }

    /// Remote environment cannot include local environment
    #[test]
    fn remote_cannot_include_local() {
        let owner = "owner".parse().unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        let manifest_contents = indoc! {r#"
        version = 1
        "#};

        let mut environment =
            mock_remote_environment(&flox, manifest_contents, owner, Some("name"));

        let manifest_edited_contents = indoc! {r#"
        version = 1

        [include]
        environments = [
          { dir = "dep" }
        ]
        "#};
        let err = environment
            .edit(&flox, manifest_edited_contents.to_string())
            .unwrap_err();

        let EnvironmentError::Recoverable(RecoverableMergeError::Fetch { err, .. }) = err else {
            panic!("expected Fetch error, got: {err:?}");
        };
        let EnvironmentError::Recoverable(RecoverableMergeError::RemoteCannotIncludeLocal) = *err
        else {
            panic!("expected CannotIncludeInRemote error, got: {err:?}");
        };
    }

    #[test]
    fn init_floxhub_environment_can_create_bare_env() {
        let owner = EnvironmentOwner::from_str("test").unwrap();
        let name = EnvironmentName::from_str("foo").unwrap();
        let env_ref = RemoteEnvironmentRef::new_from_parts(owner.clone(), name.clone());

        let (flox, _tempdir_handle) = flox_instance_with_optional_floxhub(Some(&owner));
        RemoteEnvironment::init_floxhub_environment(&flox, env_ref.clone(), true).unwrap();

        let env =
            RemoteEnvironment::new(&flox, ManagedPointer::new(owner, name, &flox.floxhub), None)
                .expect("find initialized remote environment");

        // TODO: should be changed to version 2 once released!
        assert_eq!(
            env.pre_migration_manifest(&flox)
                .unwrap()
                .as_writable()
                .to_string(),
            with_latest_schema("")
        );
    }

    #[test]
    fn init_existing_floxhub_environment_fails() {
        let owner = EnvironmentOwner::from_str("test").unwrap();
        let name = EnvironmentName::from_str("foo").unwrap();
        let env_ref = RemoteEnvironmentRef::new_from_parts(owner.clone(), name.clone());

        let (flox, _tempdir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        RemoteEnvironment::init_floxhub_environment(&flox, env_ref.clone(), false)
            .expect("first init succeeds");

        let err = RemoteEnvironment::init_floxhub_environment(&flox, env_ref.clone(), false)
            .expect_err("second init should fail");

        assert!(
            matches!(
                err,
                EnvironmentError::ManagedEnvironment(
                    ManagedEnvironmentError::UpstreamAlreadyExists { .. }
                ),
            ),
            "{err:?}"
        );
    }

    /// Prove that initialized environments have a history that starts with `initialized`.
    #[test]
    fn init_floxhub_environment_create_initialize_history() {
        let owner = EnvironmentOwner::from_str("test").unwrap();
        let name = EnvironmentName::from_str("foo").unwrap();
        let env_ref = RemoteEnvironmentRef::new_from_parts(owner.clone(), name.clone());

        let (flox, _tempdir_handle) = flox_instance_with_optional_floxhub(Some(&owner));
        RemoteEnvironment::init_floxhub_environment(&flox, env_ref.clone(), true).unwrap();

        let env =
            RemoteEnvironment::new(&flox, ManagedPointer::new(owner, name, &flox.floxhub), None)
                .expect("find initialized remote environment");

        let generation_metadata = env.generations_metadata().unwrap();
        let history = generation_metadata.history();
        let history_kind = &history.iter().next().unwrap().kind;

        assert_eq!(history_kind, &HistoryKind::Initialize);
    }
}
