use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{fs, io};

use flox_core::data::environment_ref::{EnvironmentName, EnvironmentOwner, RemoteEnvironmentRef};
use flox_manifest::interfaces::{AsWritableManifest, CommonFields, WriteManifest};
use flox_manifest::lockfile::{LOCKFILE_FILENAME, Lockfile};
use flox_manifest::parsed::common::IncludeDescriptor;
use flox_manifest::raw::{CatalogPackage, FlakePackage, PackageToInstall, StorePath};
use flox_manifest::{Manifest, ManifestError, Migrated, Validated};
use thiserror::Error;
use tracing::{debug, instrument};

use super::core_environment::{CoreEnvironment, UpgradeResult};
use super::fetcher::IncludeFetcher;
use super::generations::{
    AllGenerationsMetadata,
    GenerationId,
    Generations,
    GenerationsError,
    GenerationsExt,
    HistoryKind,
    WithOtherFields,
};
use super::path_environment::PathEnvironment;
use super::{
    CACHE_DIR_NAME,
    CanonicalizeError,
    CoreEnvironmentError,
    ENV_DIR_NAME,
    ENVIRONMENT_POINTER_FILENAME,
    EditResult,
    Environment,
    EnvironmentError,
    EnvironmentPointer,
    GCROOTS_DIR_NAME,
    InstallationAttempt,
    LOG_DIR_NAME,
    ManagedPointer,
    PathPointer,
    RenderedEnvironmentLinks,
    UninstallationAttempt,
    path_hash,
    services_socket_path,
};
use crate::data::CanonicalPath;
use crate::flox::Flox;
use crate::models::env_registry::{EnvRegistryError, deregister, ensure_registered};
use crate::models::environment::copy_dir_recursive;
use crate::models::environment::floxmeta_branch::{
    BranchOrd,
    FloxmetaBranch,
    FloxmetaBranchError,
    GenerationLock,
    remote_branch_name,
    write_generation_lock,
};
use crate::models::floxmeta::{FloxMetaError, floxmeta_git_options};
use crate::providers::buildenv::BuildEnvOutputs;
use crate::providers::git::{GitCommandError, GitProvider, GitRemoteCommandError, PushFlag};
use crate::providers::lock_manifest::LockResult;

pub const GENERATION_LOCK_FILENAME: &str = "env.lock";

#[derive(Debug)]
pub struct ManagedEnvironment {
    /// Absolute path to the directory containing `env.json`
    path: CanonicalPath,
    rendered_env_links: RenderedEnvironmentLinks,
    pointer: ManagedPointer,
    floxmeta_branch: FloxmetaBranch,
    include_fetcher: IncludeFetcher,
    /// Specific generation to use, i.e. from `flox activate`
    /// This doesn't represent the live generation.
    generation: Option<GenerationId>,
}

#[derive(Debug, Error)]
pub enum ManagedEnvironmentError {
    #[error(transparent)]
    FloxmetaBranch(#[from] FloxmetaBranchError),

    #[error("failed to update floxmeta git repo: {0}")]
    UpdateFloxmeta(FloxMetaError),
    #[error("internal error: {0}")]
    Git(GitCommandError),
    #[error("couldn't write environment lockfile: {0}")]
    WriteLock(io::Error),
    #[error("couldn't create symlink to project: {0}")]
    ReverseLink(std::io::Error),
    #[error("couldn't create links directory: {0}")]
    CreateLinksDir(std::io::Error),

    /// Error while creating or populating `.flox/env` from the current generation
    #[error("failed copying environment directory to .flox")]
    CreateLocalEnvironmentView(#[source] std::io::Error),

    #[error("local checkout and remote checkout are out of sync")]
    CheckoutOutOfSync,

    /// Error reading the local manifest
    #[error("failed to read local manifest")]
    ReadLocalManifest(#[source] CoreEnvironmentError),

    #[error("generations error")]
    Generations(#[source] GenerationsError),

    #[error("floxmeta branch name was malformed: {0}")]
    BadBranchName(String),
    #[error("project wasn't found at path {path}: {err}")]
    ProjectNotFound { path: PathBuf, err: std::io::Error },
    #[error("upstream floxmeta branch diverged from local branch")]
    Diverged(DivergedMetadata),
    #[error("access to floxmeta repository was denied")]
    AccessDenied,
    #[error("environment '{env_ref}' does not exist at upstream '{upstream}'")]
    UpstreamNotFound {
        env_ref: RemoteEnvironmentRef,
        upstream: String,
        user: Option<String>,
    },
    /// [ManagedEnvironment::push_new] may return this
    /// if the pushed environmentname already exists
    #[error("environment '{env_ref}' already exists at upstream '{upstream}'")]
    UpstreamAlreadyExists {
        env_ref: RemoteEnvironmentRef,
        upstream: String,
    },

    #[error("failed to push environment")]
    Push(#[source] GitRemoteCommandError),
    #[error("cannot push environment that includes local environments")]
    PushWithLocalIncludes,
    #[error("failed to delete local environment branch")]
    DeleteBranch(#[source] GitCommandError),
    #[error("failed to delete environment directory {0:?}")]
    DeleteEnvironment(PathBuf, #[source] std::io::Error),
    #[error("failed to delete environment link {0:?}")]
    DeleteEnvironmentLink(PathBuf, #[source] std::io::Error),
    #[error("failed to delete environment reverse link {0:?}")]
    DeleteEnvironmentReverseLink(PathBuf, #[source] std::io::Error),

    #[error("could not sync environment from upstream")]
    FetchUpdates(#[source] GitRemoteCommandError),
    #[error("could not apply updates from upstream")]
    ApplyUpdates(#[source] GitRemoteCommandError),

    #[error("couldn't initialize floxmeta")]
    InitializeFloxmeta(#[source] GenerationsError),
    #[error("couldn't serialize environment pointer")]
    SerializePointer(#[source] serde_json::Error),
    #[error("couldn't write environment pointer")]
    WritePointer(#[source] std::io::Error),

    // todo: improve description
    #[error("could not create floxmeta directory")]
    CreateFloxmetaDir(#[source] GenerationsError),

    // todo: improve description
    #[error("could not create files for current generation")]
    CreateGenerationFiles(#[source] GenerationsError),

    #[error("could not commit generation")]
    CommitGeneration(#[source] GenerationsError),

    #[error("could not build environment")]
    Build(#[source] CoreEnvironmentError),

    #[error("could not link environment")]
    Link(#[source] CoreEnvironmentError),

    #[error("could not read manifest")]
    ReadManifest(#[source] GenerationsError),

    #[error("could not canonicalize environment path")]
    CanonicalizePath(#[source] CanonicalizeError),

    #[error("invalid FloxHub base url")]
    InvalidFloxhubBaseUrl(#[source] url::ParseError),

    #[error("failed to locate project in environment registry")]
    Registry(#[from] EnvRegistryError),

    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Core(#[from] CoreEnvironmentError),
    #[error(transparent)]
    Environment(#[from] Box<EnvironmentError>),
}

#[derive(Debug)]
pub struct DivergedMetadata {
    pub local: AllGenerationsMetadata,
    pub remote: AllGenerationsMetadata,
}

impl Environment for ManagedEnvironment {
    /// This will lock if there is an out of sync local checkout
    fn lockfile(&mut self, flox: &Flox) -> Result<LockResult, EnvironmentError> {
        if let Some(generation) = self.generation {
            let lockfile_contents = self
                .generations()
                .lockfile_contents(*generation)
                .map_err(ManagedEnvironmentError::Generations)?;

            let lockfile: Lockfile =
                Lockfile::from_str(&lockfile_contents).map_err(EnvironmentError::Lockfile)?;

            return Ok(LockResult::Unchanged(lockfile));
        }

        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;
        self.ensure_locked(flox, &mut local_checkout)
    }

    /// Returns the lockfile if it already exists.
    fn existing_lockfile(&self, flox: &Flox) -> Result<Option<Lockfile>, EnvironmentError> {
        if let Some(generation) = self.generation {
            let lockfile_contents = self
                .generations()
                .lockfile_contents(*generation)
                .map_err(ManagedEnvironmentError::Generations)?;

            return Lockfile::from_str(&lockfile_contents)
                .map_err(EnvironmentError::Lockfile)
                .map(Some);
        }

        self.local_env_or_copy_current_generation(flox)?
            .existing_lockfile()
            .map_err(EnvironmentError::Core)
    }

    fn pre_migration_manifest(&self, flox: &Flox) -> Result<Manifest<Validated>, EnvironmentError> {
        if let Some(generation) = self.generation {
            let pre_migration_manifest_contents = self
                .generations()
                .manifest_contents(*generation)
                .map_err(ManagedEnvironmentError::Generations)?;
            let manifest = Manifest::parse_toml_typed(pre_migration_manifest_contents)?;
            return Ok(manifest);
        }

        // Read straight from disk
        let env_view = self.local_env_or_copy_current_generation(flox)?;
        env_view
            .pre_migration_manifest()
            .map_err(EnvironmentError::ManifestError)
    }

    fn manifest(&mut self, flox: &Flox) -> Result<Manifest<Migrated>, EnvironmentError> {
        if let Some(generation) = self.generation {
            let lockfile_contents = self
                .generations()
                .lockfile_contents(*generation)
                .map_err(ManagedEnvironmentError::Generations)?;
            let lockfile = Lockfile::from_str(lockfile_contents.as_str())?;
            let manifest = self
                .pre_migration_manifest(flox)?
                .migrate(Some(&lockfile))?;
            return Ok(manifest);
        }

        // read and migrate
        let mut env_view = self.local_env_or_copy_current_generation(flox)?;
        env_view.manifest(flox)
    }

    /// Install packages to the environment atomically
    fn install(
        &mut self,
        packages: &[PackageToInstall],
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError> {
        self.guard_generation_immutable()?;

        let mut generations = self.generations();
        let mut generations = generations
            .writable(
                &flox.temp_dir,
                &flox.system_user_name,
                &flox.system_hostname,
                &flox.argv,
            )
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;

        if !Self::validate_checkout(&local_checkout, &generations)? {
            Err(EnvironmentError::ManagedEnvironment(
                ManagedEnvironmentError::CheckoutOutOfSync,
            ))?
        }

        let targets = packages
            .iter()
            .map(|p| match p {
                PackageToInstall::Catalog(CatalogPackage {
                    id,
                    pkg_path,
                    version: Some(version),
                    ..
                }) => format!("{id} ({pkg_path}@{version})"),
                PackageToInstall::Catalog(CatalogPackage { id, pkg_path, .. }) => {
                    format!("{id} ({pkg_path})")
                },
                PackageToInstall::Flake(FlakePackage { id, url }) => format!("{id} ({url})"),
                PackageToInstall::StorePath(StorePath { id, store_path, .. }) => {
                    format!("{id} ({})", store_path.display())
                },
            })
            .collect();

        let result = local_checkout.install(packages, flox)?;
        if result.new_manifest.is_some() {
            let change = HistoryKind::Install { targets };
            generations
                .add_generation(&mut local_checkout, change)
                .map_err(ManagedEnvironmentError::CommitGeneration)?;
            self.lock_pointer()?;
        }
        if let Some(store_paths) = &result.built_environments {
            self.link(store_paths)?;
        }

        Ok(result)
    }

    /// Uninstall packages from the environment atomically
    fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<UninstallationAttempt, EnvironmentError> {
        self.guard_generation_immutable()?;

        let mut generations = self.generations();
        let mut generations = generations
            .writable(
                &flox.temp_dir,
                &flox.system_user_name,
                &flox.system_hostname,
                &flox.argv,
            )
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;

        if !Self::validate_checkout(&local_checkout, &generations)? {
            Err(EnvironmentError::ManagedEnvironment(
                ManagedEnvironmentError::CheckoutOutOfSync,
            ))?
        }

        let result = local_checkout.uninstall(packages.clone(), flox)?;
        let change = HistoryKind::Uninstall { targets: packages };

        // It's an error to uninstall a package that isn't installed so if we
        // got this far then we need a new generation.
        generations
            .add_generation(&mut local_checkout, change)
            .map_err(ManagedEnvironmentError::CommitGeneration)?;
        self.lock_pointer()?;
        if let Some(store_paths) = &result.built_environment_store_paths {
            self.link(store_paths)?;
        }

        Ok(result)
    }

    /// Atomically edit this environment, ensuring that it still builds
    fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError> {
        self.guard_generation_immutable()?;

        let mut generations = self.generations();
        let mut generations = generations
            .writable(
                &flox.temp_dir,
                &flox.system_user_name,
                &flox.system_hostname,
                &flox.argv,
            )
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;

        let result = local_checkout.edit(flox, contents)?;

        match &result {
            EditResult::Changed {
                built_environment_store_paths,
                ..
            } => {
                generations
                    .add_generation(&mut local_checkout, HistoryKind::Edit)
                    .map_err(ManagedEnvironmentError::CommitGeneration)?;
                self.lock_pointer()?;
                self.link(built_environment_store_paths)?;
            },
            EditResult::Unchanged => {},
        }

        Ok(result)
    }

    /// Try to upgrade packages in the current local checkout
    /// without committing a new generation
    fn dry_upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[&str],
    ) -> Result<UpgradeResult, EnvironmentError> {
        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;
        let result = local_checkout.upgrade(flox, groups_or_iids, false)?;
        Ok(result)
    }

    /// Atomically upgrade packages in this environment
    fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[&str],
    ) -> Result<UpgradeResult, EnvironmentError> {
        self.guard_generation_immutable()?;

        let mut generations = self.generations();
        let mut generations = generations
            .writable(
                &flox.temp_dir,
                &flox.system_user_name,
                &flox.system_hostname,
                &flox.argv,
            )
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;

        if !Self::validate_checkout(&local_checkout, &generations)? {
            Err(EnvironmentError::ManagedEnvironment(
                ManagedEnvironmentError::CheckoutOutOfSync,
            ))?
        }

        let result = local_checkout.upgrade(flox, groups_or_iids, true)?;
        if !result.diff().is_empty() {
            let change = HistoryKind::Upgrade {
                targets: result.packages().collect(),
            };
            generations
                .add_generation(&mut local_checkout, change)
                .map_err(ManagedEnvironmentError::CommitGeneration)?;

            self.lock_pointer()?;
        }
        Ok(result)
    }

    /// Upgrade environment with latest changes to included environments.
    fn include_upgrade(
        &mut self,
        flox: &Flox,
        to_upgrade: Vec<String>,
    ) -> Result<UpgradeResult, EnvironmentError> {
        self.guard_generation_immutable()?;

        let mut generations = self.generations();
        let mut generations = generations
            .writable(
                &flox.temp_dir,
                &flox.system_user_name,
                &flox.system_hostname,
                &flox.argv,
            )
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;

        if !Self::validate_checkout(&local_checkout, &generations)? {
            Err(EnvironmentError::ManagedEnvironment(
                ManagedEnvironmentError::CheckoutOutOfSync,
            ))?
        }

        let result = local_checkout.include_upgrade(flox, to_upgrade.clone())?;
        if !result.include_diff().is_empty() {
            let change = HistoryKind::IncludeUpgrade {
                targets: to_upgrade,
            };
            generations
                .add_generation(&mut local_checkout, change)
                .map_err(ManagedEnvironmentError::CommitGeneration)?;

            self.lock_pointer()?;
        }

        Ok(result)
    }

    /// This will lock if there is an out of sync local checkout
    fn rendered_env_links(
        &mut self,
        flox: &Flox,
    ) -> Result<RenderedEnvironmentLinks, EnvironmentError> {
        if let Some(generation) = self.generation {
            return self.rendered_env_links_for_generation(flox, generation);
        }

        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;
        self.ensure_locked(flox, &mut local_checkout)?;

        let lockfile_contents = local_checkout
            .existing_lockfile_contents()
            .map_err(ManagedEnvironmentError::Core)?
            .expect("lockfile presence checked");

        let rendered_env_lockfile_path =
            self.rendered_env_links.development.join(LOCKFILE_FILENAME);

        let mut build_and_link = || -> Result<(), EnvironmentError> {
            let store_paths = self.build(flox)?;
            self.link(&store_paths)?;
            Ok(())
        };

        if !rendered_env_lockfile_path.exists() {
            build_and_link()?;
            return Ok(self.rendered_env_links.clone());
        }

        let Ok(rendered_env_lockfile_contents) = fs::read_to_string(&rendered_env_lockfile_path)
        else {
            build_and_link()?;
            return Ok(self.rendered_env_links.clone());
        };

        if lockfile_contents != rendered_env_lockfile_contents {
            build_and_link()?;
            return Ok(self.rendered_env_links.clone());
        }

        Ok(self.rendered_env_links.clone())
    }

    fn build(&mut self, flox: &Flox) -> Result<BuildEnvOutputs, EnvironmentError> {
        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;
        // todo: ensure lockfile exists?

        Ok(local_checkout.build(flox)?)
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

    /// Returns parent of .flox
    fn project_path(&self) -> Result<PathBuf, EnvironmentError> {
        self.parent_path()
    }

    fn parent_path(&self) -> Result<PathBuf, EnvironmentError> {
        self.path
            .parent()
            .ok_or(EnvironmentError::InvalidPath(self.path.to_path_buf()))
            .map(|p| p.to_path_buf())
    }

    /// Path to the environment's .flox directory
    fn dot_flox_path(&self) -> CanonicalPath {
        self.path.clone()
    }

    /// Path to the environment definition file
    ///
    /// Path will not share a common prefix with the path returned by [`ManagedEnvironment::lockfile_path`]
    fn manifest_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        let path = self
            .local_env_or_copy_current_generation(flox)?
            .manifest_path();
        Ok(path)
    }

    /// Path to the lockfile. The path may not exist.
    ///
    /// Path will not share a common prefix with the path returned by [`ManagedEnvironment::manifest_path`]
    fn lockfile_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        let path = self
            .local_env_or_copy_current_generation(flox)?
            .lockfile_path();
        Ok(path)
    }

    /// Returns the environment name
    fn name(&self) -> EnvironmentName {
        self.pointer.name.clone()
    }

    /// Delete the Environment
    fn delete(self, flox: &Flox) -> Result<(), EnvironmentError> {
        fs::remove_dir_all(&self.path)
            .map_err(|e| ManagedEnvironmentError::DeleteEnvironment(self.path.to_path_buf(), e))?;

        self.floxmeta_branch
            .delete()
            .map_err(ManagedEnvironmentError::FloxmetaBranch)?;

        deregister(flox, &self.path, &EnvironmentPointer::Managed(self.pointer))?;

        Ok(())
    }

    /// Return the path where the process compose socket for an environment
    /// should be created
    fn services_socket_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        services_socket_path(&self.path_hash(), flox)
    }
}

impl GenerationsExt for ManagedEnvironment {
    fn generations_metadata(
        &self,
    ) -> Result<WithOtherFields<AllGenerationsMetadata>, GenerationsError> {
        self.generations().metadata()
    }

    fn switch_generation(
        &mut self,
        flox: &Flox,
        generation: GenerationId,
    ) -> Result<(), EnvironmentError> {
        let mut generations = self.generations();

        let local_checkout = self.local_env_or_copy_current_generation(flox)?;
        if !Self::validate_checkout(&local_checkout, &generations)? {
            Err(EnvironmentError::ManagedEnvironment(
                ManagedEnvironmentError::CheckoutOutOfSync,
            ))?
        }

        let mut generations = generations
            .writable(
                &flox.temp_dir,
                &flox.system_user_name,
                &flox.system_hostname,
                &flox.argv,
            )
            .map_err(ManagedEnvironmentError::Generations)?;

        generations
            .set_current_generation(generation)
            .map_err(ManagedEnvironmentError::CommitGeneration)?;

        // update the rendered environment
        self.lock_pointer()?;
        self.reset_local_env_to_current_generation(flox)?;
        let buildenv_paths = self.build(flox)?;
        self.link(&buildenv_paths)?;

        Ok(())
    }

    fn remote_lockfile_contents_for_current_generation(&self) -> Result<String, GenerationsError> {
        self.floxmeta_branch
            .remote_generations()
            .current_gen_lockfile()
    }

    fn remote_manifest_contents_for_current_generation(&self) -> Result<String, GenerationsError> {
        self.floxmeta_branch
            .remote_generations()
            .current_gen_manifest_contents()
    }

    fn lockfile_contents_for_generation(
        &self,
        generation: usize,
    ) -> Result<String, GenerationsError> {
        self.generations().lockfile_contents(generation)
    }

    fn rendered_env_links_for_generation(
        &self,
        flox: &Flox,
        generation: GenerationId,
    ) -> Result<RenderedEnvironmentLinks, EnvironmentError> {
        let mut generations = self.generations();
        let generation_rw = generations
            .writable(
                &flox.temp_dir,
                &flox.system_user_name,
                &flox.system_hostname,
                &flox.argv,
            )
            .map_err(ManagedEnvironmentError::Generations)?;

        let mut core_environment = generation_rw
            .get_generation(*generation, self.include_fetcher.clone())
            .map_err(ManagedEnvironmentError::Generations)?;

        let store_paths = core_environment.build(flox)?;

        let run_dir = &self.path.join(GCROOTS_DIR_NAME);
        if !run_dir.exists() {
            std::fs::create_dir_all(run_dir).map_err(ManagedEnvironmentError::CreateLinksDir)?;
        }

        let base_dir = CanonicalPath::new(run_dir).expect("run dir is checked to exist");

        let rendered_env_links =
            RenderedEnvironmentLinks::new_in_base_dir_with_name_system_and_generation(
                &base_dir,
                self.name().as_ref(),
                &flox.system,
                generation,
            );

        CoreEnvironment::link(&rendered_env_links.development, &store_paths.develop)?;
        CoreEnvironment::link(&rendered_env_links.runtime, &store_paths.runtime)?;

        Ok(rendered_env_links)
    }

    fn remote_generations_metadata(
        &self,
    ) -> Result<WithOtherFields<AllGenerationsMetadata>, GenerationsError> {
        self.floxmeta_branch.remote_generations().metadata()
    }

    fn compare_remote(&self) -> Result<BranchOrd, EnvironmentError> {
        Ok(self
            .floxmeta_branch
            .compare_remote()
            .map_err(ManagedEnvironmentError::FloxmetaBranch)?)
    }
}

/// Constructors and related functions
impl ManagedEnvironment {
    /// Guard against modifying an environment that is activated at a specific
    /// generation so that we don't create unecessary branches in the generation
    /// history.
    fn guard_generation_immutable(&self) -> Result<(), EnvironmentError> {
        if let Some(generation) = self.generation {
            return Err(EnvironmentError::ManagedEnvironment(
                ManagedEnvironmentError::Generations(
                    GenerationsError::ActivatedGenerationImmutable(generation),
                ),
            ));
        }

        Ok(())
    }

    /// If there's an out of sync local checkout, ensure it's locked.
    /// If the checkout is in sync, return it's lock contents.
    ///
    /// This errors if an in-sync checkout doesn't have a lockfile, since that's
    /// a bad state.
    fn ensure_locked(
        &mut self,
        flox: &Flox,
        local_checkout: &mut CoreEnvironment,
    ) -> Result<LockResult, EnvironmentError> {
        // Otherwise, there would be a generation without a lockfile, which is a bad state,
        // and we error.
        if !Self::validate_checkout(local_checkout, &self.generations())? {
            Ok(local_checkout.ensure_locked(flox)?)
        } else {
            match local_checkout.existing_lockfile()? {
                Some(lockfile) => Ok(LockResult::Unchanged(lockfile)),
                None => Err(EnvironmentError::MissingLockfile),
            }
        }
    }

    pub fn link(&mut self, store_paths: &BuildEnvOutputs) -> Result<(), EnvironmentError> {
        CoreEnvironment::link(&self.rendered_env_links.development, &store_paths.develop)?;
        CoreEnvironment::link(&self.rendered_env_links.runtime, &store_paths.runtime)?;

        Ok(())
    }

    /// Returns a unique identifier for the location of the environment.
    fn path_hash(&self) -> String {
        path_hash(&self.path)
    }

    /// Open a managed environment by reading its lockfile and ensuring there is
    /// a unique branch to track its state in floxmeta.
    ///
    /// The definition of a managed environment is stored on a branch in a
    /// central clone of the environment owner's floxmeta repository located in
    /// `$FLOX_DATA_DIR/meta/<owner>`. Every .flox directory will correspond to
    /// a unique branch.
    ///
    /// To open an environment at a given location:
    ///
    /// - We open a user's floxmeta clone, i.e. `$FLOX_DATA_DIR/meta/<owner>`.
    ///   If that repo doesn't exist, it will be cloned.
    ///
    /// - We open the lockfile and ensure that the specific commit referenced is
    ///   present in floxmeta. If it is not, we fetch the environment and lock
    ///   to `HEAD`.
    ///
    /// - We check whether a unique branch for the .flox directory exists in
    ///   floxmeta and points at the commit in the lockfile, creating or
    ///   resetting the correct branch if necessary.
    ///
    /// At some point, it may be useful to create a ManagedEnvironment without
    /// fetching or cloning. This would be more correct for commands like delete
    /// that don't need to fetch the environment.
    pub fn open(
        flox: &Flox,
        pointer: ManagedPointer,
        dot_flox_path: impl AsRef<Path>,
        generation: Option<GenerationId>,
    ) -> Result<Self, EnvironmentError> {
        let dot_flox_path =
            CanonicalPath::new(dot_flox_path).map_err(ManagedEnvironmentError::CanonicalizePath)?;

        // Read existing lockfile
        let lock_path = dot_flox_path.join(GENERATION_LOCK_FILENAME);
        let maybe_lock =
            GenerationLock::read_maybe(&lock_path).map_err(ManagedEnvironmentError::from)?;

        // ALL git validation in ONE call - errors bubble up through ManagedEnvironmentError
        let (floxmeta_branch, validated_lock) =
            FloxmetaBranch::new(flox, &pointer, &dot_flox_path, maybe_lock)
                .map_err(ManagedEnvironmentError::from)?;

        // Write validated lock
        write_generation_lock(&lock_path, &validated_lock)
            .map_err(ManagedEnvironmentError::from)?;

        // Setup rendered_env_links
        let rendered_env_links = {
            let run_dir = dot_flox_path.join(GCROOTS_DIR_NAME);
            if !run_dir.exists() {
                std::fs::create_dir_all(&run_dir)
                    .map_err(ManagedEnvironmentError::CreateLinksDir)?;
            }

            let base_dir = CanonicalPath::new(run_dir).expect("run dir is checked to exist");

            if let Some(generation) = generation {
                RenderedEnvironmentLinks::new_in_base_dir_with_name_system_and_generation(
                    &base_dir,
                    pointer.name.as_ref(),
                    &flox.system,
                    generation,
                )
            } else {
                RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system(
                    &base_dir,
                    pointer.name.as_ref(),
                    &flox.system,
                )
            }
        };

        let parent_directory = dot_flox_path
            .parent()
            .ok_or(EnvironmentError::InvalidPath(dot_flox_path.to_path_buf()))?;
        let include_fetcher = IncludeFetcher {
            base_directory: Some(parent_directory.to_path_buf()),
        };

        Self::open_with(
            flox,
            floxmeta_branch,
            pointer,
            dot_flox_path,
            rendered_env_links,
            include_fetcher,
            generation,
        )
        .map_err(EnvironmentError::ManagedEnvironment)
    }

    /// Open a managed environment backed by a provided floxmeta_branch.
    ///
    /// This method is primarily useful for testing.
    /// In most cases, you want to use [`ManagedEnvironment::open`] instead which provides the flox defaults.
    pub fn open_with(
        flox: &Flox,
        floxmeta_branch: FloxmetaBranch,
        pointer: ManagedPointer,
        dot_flox_path: CanonicalPath,
        rendered_env_links: RenderedEnvironmentLinks,
        include_fetcher: IncludeFetcher,
        generation: Option<GenerationId>,
    ) -> Result<Self, ManagedEnvironmentError> {
        ensure_registered(
            flox,
            &dot_flox_path,
            &EnvironmentPointer::Managed(pointer.clone()),
        )?;

        let env = ManagedEnvironment {
            path: dot_flox_path,
            rendered_env_links,
            pointer,
            floxmeta_branch,
            include_fetcher,
            generation,
        };

        Ok(env)
    }

    pub fn env_ref(&self) -> RemoteEnvironmentRef {
        self.pointer.clone().into()
    }
}

/// Result of creating a generation from local changes with
/// [ManagedEnvironment::create_generation_from_local_env]
pub enum SyncToGenerationResult {
    /// The environment was already up to date
    UpToDate,
    /// The environment was successfully synced to the generation
    Synced,
}

/// Utility instance methods
impl ManagedEnvironment {
    /// Edit the environment without checking that it builds
    ///
    /// This is used to allow `flox pull` to work with environments
    /// that don't specify the current system as supported.
    pub fn edit_unsafe(
        &mut self,
        flox: &Flox,
        contents: String,
    ) -> Result<Result<EditResult, EnvironmentError>, EnvironmentError> {
        let mut generations = self.generations();
        let mut generations = generations
            .writable(
                &flox.temp_dir,
                &flox.system_user_name,
                &flox.system_hostname,
                &flox.argv,
            )
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        let mut temporary = self.local_env_or_copy_current_generation(flox)?;

        if !Self::validate_checkout(&temporary, &generations)? {
            Err(EnvironmentError::ManagedEnvironment(
                ManagedEnvironmentError::CheckoutOutOfSync,
            ))?
        }

        let result = temporary.edit_unsafe(flox, contents)?;

        if matches!(result, Ok(EditResult::Unchanged)) {
            return Ok(result);
        }

        debug!("Environment changed, create and lock generation");

        generations
            .add_generation(&mut temporary, HistoryKind::Edit)
            .map_err(ManagedEnvironmentError::CommitGeneration)?;
        self.lock_pointer()?;

        // don't link, the environment may be broken

        Ok(result)
    }

    /// Create a new generation from local changes,
    /// and updates the generation lock.
    ///
    /// If the environment was already up to date,
    /// [ManagedEnvironment::create_generation_from_local_env] should return successfully.
    /// In that case the result is [SyncToGenerationResult::UpToDate] to signal
    /// that no changes were made.
    /// Before creating a new generation, the local environment is locked and built,
    /// to ensure the validity of the new generation.
    /// Unless an error occurs, [SyncToGenerationResult::Synced] is returned.
    pub fn create_generation_from_local_env(
        &mut self,
        flox: &Flox,
    ) -> Result<SyncToGenerationResult, EnvironmentError> {
        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;

        if Self::validate_checkout(&local_checkout, &self.generations())? {
            debug!("local checkout and remote checkout equal, nothing to apply");
            return Ok(SyncToGenerationResult::UpToDate);
        }

        // Ensure the environment is locked
        // PathEnvironment may not have a lockfile or an outdated lockfile
        // if the environment was modified primarily through editing the manifest manually.
        // Call lock rather than ensure_locked because the primary purpose of
        // ensure_locked is avoiding locking of v0 manifests,
        // but we don't need to support pushing old manifests.
        local_checkout.lock(flox)?;

        // Ensure the created generation is valid
        let store_paths = local_checkout.build(flox)?;

        // TODO: should use self.link but that returns an EnvironmentError
        self.link(&store_paths)?;

        let mut generations = self.generations();
        let mut generations = generations
            .writable(
                flox.temp_dir.clone(),
                &flox.system_user_name,
                &flox.system_hostname,
                &flox.argv,
            )
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        generations
            .add_generation(&mut local_checkout, HistoryKind::Edit)
            .map_err(ManagedEnvironmentError::CommitGeneration)?;

        self.lock_pointer()?;
        Ok(SyncToGenerationResult::Synced)
    }

    /// Discards local changes in `.flox/env` and recreates the directory from the current generation.
    ///
    /// Returns the new [CoreEnvironment] for the `.flox/env` directory.
    /// Unlike [ManagedEnvironment::create_generation_from_local_env],
    /// this method **does not** build the environment as previous generations
    /// may fail to build, unrelated to the success of resetting the environment.
    /// Pulling an environment for example may result in an invalid environment
    /// e.g. because the manifest does not specify the current system,
    /// resetting in that context should not fail either.
    /// Like [ManagedEnvironment::pull], downtream commands should check that the environment builds
    /// if applicable.
    ///
    /// TODO: Specific behavior for other files than the manifest should is undefined.
    /// Currently the entire environment directory is **deleted and recreated**.
    /// Any other files are lost.
    pub fn reset_local_env_to_current_generation(
        &self,
        flox: &Flox,
    ) -> Result<CoreEnvironment, ManagedEnvironmentError> {
        let current_generation = self.get_current_generation(flox)?;
        let env_dir = self.path.join(ENV_DIR_NAME);

        if let Err(e) = fs::remove_dir_all(&env_dir) {
            return Err(ManagedEnvironmentError::DeleteEnvironment(env_dir, e));
        }

        fs::create_dir_all(&env_dir)
            .map_err(ManagedEnvironmentError::CreateLocalEnvironmentView)?;

        copy_dir_recursive(
            current_generation.path(),
            self.path.join(ENV_DIR_NAME),
            true,
        )
        .map_err(ManagedEnvironmentError::CreateLocalEnvironmentView)?;

        let local_checkout = CoreEnvironment::new(env_dir, self.include_fetcher.clone())?;

        Ok(local_checkout)
    }

    /// Return a [CoreEnvironment] for an existing local checkout
    /// or create one from the current generation.
    ///
    /// Copies the `env/` directory from the current generation to the `.flox/` directory
    /// and returns a [CoreEnvironment] for the `.flox/env`.
    fn local_env_or_copy_current_generation(
        &self,
        flox: &Flox,
    ) -> Result<CoreEnvironment, ManagedEnvironmentError> {
        if !self.path.join(ENV_DIR_NAME).exists() {
            debug!("creating environment directory");
            let current_generation = self.get_current_generation(flox)?;
            fs::create_dir_all(self.path.join(ENV_DIR_NAME))
                .map_err(ManagedEnvironmentError::CreateLocalEnvironmentView)?;
            copy_dir_recursive(
                current_generation.path(),
                self.path.join(ENV_DIR_NAME),
                true,
            )
            .map_err(ManagedEnvironmentError::CreateLocalEnvironmentView)?;
        }

        let local =
            CoreEnvironment::new(self.path.join(ENV_DIR_NAME), self.include_fetcher.clone())?;
        Ok(local)
    }

    /// Validate that the local manifest checkout matches the one in the current generation.
    ///
    /// Returns true if they match, false otherwise.
    /// Manifests are compared byte-for-byte, so that semantically equivalent modifications
    /// such as whitespace changes are still detected.
    ///
    /// Note:
    /// This is not a method on CoreEnvironment because its currently only relevant
    /// in the context of a ManagedEnvironment.
    /// A potential future version could provide more detailed comparison/diff information
    /// that may be more generally useful and see this method changed or moved.
    fn validate_checkout<State>(
        local: &CoreEnvironment,
        generations: &Generations<State>,
    ) -> Result<bool, ManagedEnvironmentError> {
        let local_lockfile_bytes = local.existing_lockfile_contents()?;
        let remote_lockfile_bytes = generations.current_gen_lockfile().ok();
        if local_lockfile_bytes != remote_lockfile_bytes {
            return Ok(false);
        }

        let local_manifest_bytes = local.pre_migration_manifest()?.as_writable().to_string();

        let remote_manifest_bytes = generations
            .current_gen_manifest_contents()
            .map_err(ManagedEnvironmentError::Generations)?;

        Ok(local_manifest_bytes == remote_manifest_bytes)
    }

    /// Convenience method to check if the local environment has changes.
    /// To be used by consumers to check
    /// if the environment is in sync with its current generation
    /// outside of the context of a specific operation.
    /// E.g. `flox edit`.
    ///
    /// Not having local changes means the environment has a lockfile, since we
    /// only create generations with lockfiles
    pub fn has_local_changes(&self, flox: &Flox) -> Result<bool, ManagedEnvironmentError> {
        let generations = self.generations();

        let local_checkout = self.local_env_or_copy_current_generation(flox)?;

        Ok(!Self::validate_checkout(&local_checkout, &generations)?)
    }

    /// Lock the environment to the current revision
    fn lock_pointer(&self) -> Result<(), ManagedEnvironmentError> {
        let lock_path = self.path.join(GENERATION_LOCK_FILENAME);
        let lock = self.floxmeta_branch.generation_lock()?;

        write_generation_lock(lock_path, &lock)?;
        Ok(())
    }

    /// Returns the environment owner
    /// The path may not exist.
    pub fn owner(&self) -> &EnvironmentOwner {
        &self.pointer.owner
    }

    /// Return the managed pointer
    pub fn pointer(&self) -> &ManagedPointer {
        &self.pointer
    }

    pub(crate) fn generations(&self) -> Generations {
        self.floxmeta_branch.generations()
    }

    fn get_current_generation(
        &self,
        flox: &Flox,
    ) -> Result<CoreEnvironment, ManagedEnvironmentError> {
        self.generations()
            .writable(
                &flox.temp_dir,
                &flox.system_user_name,
                &flox.system_hostname,
                &flox.argv,
            )
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?
            .get_current_generation(self.include_fetcher.clone())
            .map_err(ManagedEnvironmentError::CreateGenerationFiles)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PullResult {
    /// The environment was already up to date
    UpToDate,
    /// The environment was reset to the latest upstream version
    Updated,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushResult {
    /// The environment was already up to date
    UpToDate,
    /// The environment was reset to the latest upstream version
    Updated,
}

/// Ensure that the environment does not include local includes before pushing it to FloxHub
fn check_for_local_includes(lockfile: &Lockfile) -> Result<(), ManagedEnvironmentError> {
    let manifest = lockfile.user_manifest();
    let has_local_include = manifest
        .include()
        .environments
        .iter()
        .any(|include| matches!(include, IncludeDescriptor::Local { .. }));

    if has_local_include {
        Err(ManagedEnvironmentError::PushWithLocalIncludes)?;
    }

    Ok(())
}

/// FloxHub synchronization implementation (pull/push)
impl ManagedEnvironment {
    /// Fetch the remote branch into the local sync branch.
    /// The sync branch is always a reset to the remote branch
    /// and it's state should not be depended on.
    #[instrument(skip(flox), fields(progress = "Looking up environment on FloxHub"))]
    pub fn fetch_remote_state(&self, flox: &Flox) -> Result<(), ManagedEnvironmentError> {
        self.floxmeta_branch
            .fetch_remote_state(flox, &self.pointer)
            .map_err(ManagedEnvironmentError::FloxmetaBranch)
    }

    /// Create a new [ManagedEnvironment] from a [PathEnvironment]
    /// by pushing the contents of the original environment as a generation to floxhub.
    ///
    /// The environment is pushed to the `owner` specified
    /// and will retain the name of the original path environment.
    ///
    /// By default, if an environment with the same name already exists in the owner's repository,
    /// the push will fail, unless `force` is set to `true`.
    ///
    /// If access to a remote repository requires authentication,
    /// the FloxHub token must be set in the flox instance.
    /// The caller is responsible for ensuring that the token is present and valid.
    ///
    /// `initializing` controls whether the initial history entry is
    /// [HistoryKind::Import] for pushing existing environments or
    /// [HistoryKind::Initialize] for environments that are (virtually) created on FloxHub.
    #[instrument(skip(flox), fields(progress = "Pushing new environment to FloxHub"))]
    pub fn push_new(
        flox: &Flox,
        path_environment: PathEnvironment,
        owner: EnvironmentOwner,
        force: bool,
        initializing: bool,
    ) -> Result<Self, EnvironmentError> {
        // path of the original .flox directory
        let dot_flox_path = path_environment.path.clone();
        let name = path_environment.name();

        let mut core_environment = path_environment.into_core_environment()?;

        // Ensure the environment is locked
        // PathEnvironment may not have a lockfile or an outdated lockfile
        // if the environment was modified primarily through editing the manifest manually.
        // Call `ensure_locked` to avoid locking of v0 manifests,
        // but permit pushing old manifests that are already locked.
        let lockfile: Lockfile = core_environment.ensure_locked(flox)?.into();

        // Ensure the environment builds before we push it
        core_environment
            .build(flox)
            .map_err(ManagedEnvironmentError::Build)?;

        // Ensure that the environment does not include other local ennvironments
        check_for_local_includes(&lockfile)?;

        Self::push_new_without_building(
            flox,
            owner,
            name,
            force,
            initializing,
            dot_flox_path,
            core_environment,
        )
    }

    /// Push an environment and open the resulting [ManagedEnvironment],
    /// but don't build the environment first.
    ///
    /// This is split out for the purposes of testing -
    /// some tests need an environment that has build errors.
    ///
    /// `initializing` controls whether the initial history entry is
    /// [HistoryKind::Import] for pushing existing environments or
    /// [HistoryKind::Initialize] for environments that are (virtually) created on FloxHub.
    fn push_new_without_building(
        flox: &Flox,
        owner: EnvironmentOwner,
        name: EnvironmentName,
        force: bool,
        initializing: bool,
        dot_flox_path: CanonicalPath,
        mut core_environment: CoreEnvironment,
    ) -> Result<Self, EnvironmentError> {
        let pointer = ManagedPointer::new(owner.clone(), name.clone(), &flox.floxhub);

        let checkedout_floxmeta_path = tempfile::tempdir_in(&flox.temp_dir).unwrap().keep();
        let temp_floxmeta_path = tempfile::tempdir_in(&flox.temp_dir).unwrap().keep();

        // Caller decides whether to set token
        let token = flox.floxhub_token.as_ref();

        let git_url = flox.floxhub.git_url();

        let options = floxmeta_git_options(git_url, &pointer.owner, token);

        // Initialize a new branch for this environment in a new, temporary,
        // bare repo. This acts like part of the bare repo that backs a user's
        // real floxmeta repo.
        let mut generations = Generations::init(
            options,
            checkedout_floxmeta_path,
            temp_floxmeta_path,
            remote_branch_name(&pointer),
            &name,
        )
        .map_err(ManagedEnvironmentError::InitializeFloxmeta)?;

        // Creates a temporary floxmeta repo that we'll add generations to
        // and push to FloxHub from. This also acts as the `origin` remote
        // for the bare repo we created above.
        let temp_floxmeta_git = generations.git().clone();

        let mut generations = generations
            .writable(
                &flox.temp_dir,
                &flox.system_user_name,
                &flox.system_hostname,
                &flox.argv,
            )
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        // Add this environment as a new generation, which involves pushing to
        // the fake remote.
        let kind = if initializing {
            HistoryKind::Initialize
        } else {
            HistoryKind::Import
        };
        generations
            .add_generation(&mut core_environment, kind)
            .map_err(ManagedEnvironmentError::CommitGeneration)?;

        temp_floxmeta_git
            .add_remote(
                "upstream",
                &format!("{}/{}/floxmeta", &git_url, &pointer.owner),
            )
            .unwrap();

        // Push the branch for this environment to FloxHub
        match temp_floxmeta_git.push_ref("upstream", "HEAD", force) {
            Err(GitRemoteCommandError::AccessDenied) => Err(ManagedEnvironmentError::AccessDenied)?,
            // If run in close succession, given equal data,
            // git may produce two identical commits despite different repos.
            // Therefore the push to "FloxHub" will succeed with [PushFlag::UpToDate].
            // Since we want to signal that the upstream repo already exists
            // we need to also catch this success.
            Err(GitRemoteCommandError::Diverged) | Ok(PushFlag::UptoDate) => {
                Err(ManagedEnvironmentError::UpstreamAlreadyExists {
                    env_ref: RemoteEnvironmentRef::new_from_parts(owner, name),
                    upstream: flox.floxhub.base_url().to_string(),
                })?
            },
            Err(e) => Err(ManagedEnvironmentError::Push(e))?,
            _ => {},
        }

        // Change the `env.json` file to indicate that the environment is no longer
        // a path environment, and instead is managed centrally on FloxHub.
        fs::write(
            dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME),
            serde_json::to_string(&pointer).map_err(ManagedEnvironmentError::SerializePointer)?,
        )
        .map_err(ManagedEnvironmentError::WritePointer)?;

        let env = ManagedEnvironment::open(flox, pointer, dot_flox_path, None)?;

        Ok(env)
    }

    #[instrument(skip(self, flox), fields(progress = "Pushing updates to FloxHub"))]
    pub fn push(&mut self, flox: &Flox, force: bool) -> Result<PushResult, EnvironmentError> {
        // TODO: move git pushing logic into floxmeta_branch module

        let project_branch = self.floxmeta_branch.branch();
        let sync_branch = self.floxmeta_branch.remote_branch();

        // Ensure the environment builds before we push it,
        // and that it does not include local environments.
        //
        // Usually we don't create generations unless they build,
        // but that is not always the case.
        // If a user pulls an environment that is broken on their system, we may
        // create a "broken" generation.
        // That generation could have a divergent manifest and lock,
        // or it could fail to build.
        // So we have to verify we don't have a "broken" generation before pushing.
        let generations = self.generations();
        {
            let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;

            if !Self::validate_checkout(&local_checkout, &generations)? {
                Err(ManagedEnvironmentError::CheckoutOutOfSync)?
            }

            // we should already be locked here,
            // as a valid lockfile is a precondition for creating a generation.
            let lockfile: Lockfile = local_checkout
                .ensure_locked(flox)
                .map_err(|_| ManagedEnvironmentError::CheckoutOutOfSync)?
                .into();
            local_checkout
                .build(flox)
                .map_err(ManagedEnvironmentError::Build)?;

            check_for_local_includes(&lockfile)?;
        }

        // Fetch the remote branch into sync branch,
        // we can ignore if the upstream was deleted since we are going to create it on push anyway.
        match self.fetch_remote_state(flox) {
            Ok(_) => {},
            Err(ManagedEnvironmentError::UpstreamNotFound { .. }) => {
                debug!("Upstream environment was deleted.")
            },
            e @ Err(_) => e?,
        };

        let branch_ord = self
            .floxmeta_branch
            .compare_remote()
            .map_err(ManagedEnvironmentError::FloxmetaBranch)?;

        if matches!(branch_ord, BranchOrd::Equal | BranchOrd::Behind) && !force {
            return Ok(PushResult::UpToDate);
        }

        // If the local branch is already ahead, or both branches have changes,
        // we diverged and need to abort (unless we blot over local state explicitly with `force`)
        if (matches!(branch_ord, BranchOrd::Diverged)) && !force {
            let local = generations
                .metadata()
                .map_err(ManagedEnvironmentError::Generations)?
                .into_inner();

            let remote = self
                .floxmeta_branch
                .remote_generations()
                .metadata()
                .map_err(ManagedEnvironmentError::Generations)?
                .into_inner();

            Err(ManagedEnvironmentError::Diverged(DivergedMetadata {
                local,
                remote,
            }))?;
        }

        self.floxmeta_branch
            .git()
            .push_ref(
                "dynamicorigin",
                format!("{}:{}", project_branch, sync_branch),
                force,
            )
            .map_err(|err| match err {
                GitRemoteCommandError::AccessDenied => ManagedEnvironmentError::AccessDenied,
                _ => ManagedEnvironmentError::Push(err),
            })?;

        // update local environment branch, should be fast-forward and a noop if the branches didn't diverge
        self.pull(flox, force)?;

        Ok(PushResult::Updated)
    }

    /// Pull new generation data from floxhub
    ///
    /// Requires the local checkout to be synched with the current generation.
    /// If the environment has diverged, the pull will fail.
    ///
    /// If `force == true`, the pull will proceed even if the environment has diverged.
    #[instrument(skip(self, flox), fields(progress = "Pulling updates from FloxHub"))]
    pub fn pull(&mut self, flox: &Flox, force: bool) -> Result<PullResult, EnvironmentError> {
        // TODO: move git pull logic into floxmeta_branch module

        // Check whether the local checkout is in sync with the current generation
        // before potentially updating generations and resetting the local checkout.
        let generations = self.generations();
        let local_checkout = self.local_env_or_copy_current_generation(flox)?;
        let checkout_valid = Self::validate_checkout(&local_checkout, &generations)?;

        // With `force` we pull even if the local checkout is out of sync.
        if !checkout_valid && !force {
            Err(ManagedEnvironmentError::CheckoutOutOfSync)?
        }

        let sync_branch = self.floxmeta_branch.remote_branch();
        let project_branch = self.floxmeta_branch.branch();

        self.fetch_remote_state(flox)?;

        let branch_ord = self
            .floxmeta_branch
            .compare_remote()
            .map_err(ManagedEnvironmentError::FloxmetaBranch)?;

        let is_uptodate = matches!(branch_ord, BranchOrd::Equal | BranchOrd::Ahead);

        if is_uptodate && !checkout_valid && force {
            self.reset_local_env_to_current_generation(flox)?;
            let store_paths = self.build(flox)?;
            self.link(&store_paths)?;

            return Ok(PullResult::Updated);
        } else if is_uptodate {
            return Ok(PullResult::UpToDate);
        }

        // If the local branch is already ahead, or both branches have changes,
        // we diverged and need to abort (unless we blot over local state explicitly with `force`)
        if (matches!(branch_ord, BranchOrd::Diverged)) && !force {
            let local = generations
                .metadata()
                .map_err(ManagedEnvironmentError::Generations)?
                .into_inner();

            let remote = self
                .floxmeta_branch
                .remote_generations()
                .metadata()
                .map_err(ManagedEnvironmentError::Generations)?
                .into_inner();

            Err(ManagedEnvironmentError::Diverged(DivergedMetadata {
                local,
                remote,
            }))?;
        }

        // update the project branch to the remote branch, using `force` if specified
        self.floxmeta_branch
            .git()
            .push_ref(
                ".",
                format!("refs/heads/{sync_branch}:refs/heads/{project_branch}",),
                force, // Set the force parameter to false or true based on your requirement
            )
            .map_err(ManagedEnvironmentError::ApplyUpdates)?;

        // update the pointer lockfile and build
        self.lock_pointer()?;
        self.reset_local_env_to_current_generation(flox)?;
        let store_paths = self.build(flox)?;
        self.link(&store_paths)?;

        Ok(PullResult::Updated)
    }

    /// Detach the environment from the remote repository.
    ///
    /// And return a [PathEnvironment] representing
    /// the current local state of the environment.
    ///
    /// This method converts the [ManagedEnvironment] into a [PathEnvironment]
    /// and deletes the corresponding branch in the [FloxMeta] repository.
    ///
    /// **The remote repository is not affected.**
    pub fn into_path_environment(self, flox: &Flox) -> Result<PathEnvironment, EnvironmentError> {
        let _ = self.local_env_or_copy_current_generation(flox)?;

        // Since conversion happens in place, i.e. not in a transaction,
        // we need to be careful not to break the environment as much as possible,
        // if any of the intermediate steps fails.

        // remove the environment branch
        // this can be recovered from the generation lock
        self.floxmeta_branch
            .delete()
            .map_err(ManagedEnvironmentError::FloxmetaBranch)?;

        fs::remove_file(self.path.join(GENERATION_LOCK_FILENAME))
            .map_err(ManagedEnvironmentError::WriteLock)?;

        // forget that this environment exists
        deregister(
            flox,
            &self.path,
            &EnvironmentPointer::Managed(self.pointer.clone()),
        )?;

        // create the metadata for a path environment
        let path_pointer = PathPointer::new(self.pointer.name);
        fs::write(
            self.path.join(ENVIRONMENT_POINTER_FILENAME),
            serde_json::to_string(&path_pointer)
                .map_err(ManagedEnvironmentError::SerializePointer)?,
        )
        .map_err(ManagedEnvironmentError::WritePointer)?;

        // open the environment to register it
        let mut path_env = PathEnvironment::open(flox, path_pointer, self.path)?;

        // trigger creation of an environment link
        // todo: should we rather expose build/link methods for `PathEnv`?
        let _ = path_env.rendered_env_links(flox)?;

        Ok(path_env)
    }
}

#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {

    use tempfile::tempdir_in;

    use super::*;
    use crate::flox::{DEFAULT_FLOXHUB_URL, Floxhub};
    use crate::models::environment::fetcher::test_helpers::mock_include_fetcher;
    use crate::models::environment::floxmeta_branch::test_helpers::unusable_mock_floxmeta_branch;
    use crate::models::environment::path_environment::test_helpers::{
        new_named_path_environment_from_env_files,
        new_named_path_environment_in,
    };
    use crate::models::environment::test_helpers::new_core_environment;

    /// Get a [ManagedEnvironment] that is invalid but can be used in tests
    /// where methods on [ManagedEnvironment] will never be called.
    ///
    /// For a [ManagedEnvironment] with methods that can be called use
    /// [mock_managed_environment].
    pub fn unusable_mock_managed_environment() -> ManagedEnvironment {
        let floxhub = Floxhub::new(DEFAULT_FLOXHUB_URL.clone(), None).unwrap();
        let floxmeta_branch = unusable_mock_floxmeta_branch();
        ManagedEnvironment {
            path: CanonicalPath::new(PathBuf::from("/")).unwrap(),
            rendered_env_links: RenderedEnvironmentLinks::new_unchecked(
                PathBuf::new(),
                PathBuf::new(),
            ),
            pointer: ManagedPointer::new(
                "owner".parse().unwrap(),
                "test".parse().unwrap(),
                &floxhub,
            ),
            floxmeta_branch,
            include_fetcher: mock_include_fetcher(),
            generation: None,
        }
    }

    /// Get a [ManagedEnvironment] that has been pushed to (a mock) FloxHub and
    /// can be built.
    ///
    /// This should be passed a [Flox] instance created with a mock FloxHub
    /// setup.
    ///
    /// If a [ManagedEnvironment] will be unused in tests, use
    /// [unusable_mock_managed_environment] instead.
    ///
    /// This doesn't lock the environment, which puts us in what should be an
    /// unreachable state compared to normal use.
    /// Not locking is depended on by some tests.
    pub fn mock_managed_environment_unlocked(
        flox: &Flox,
        contents: &str,
        owner: EnvironmentOwner,
    ) -> ManagedEnvironment {
        ManagedEnvironment::push_new_without_building(
            flox,
            owner,
            "name".parse().unwrap(),
            false,
            false,
            CanonicalPath::new(tempdir_in(&flox.temp_dir).unwrap().keep()).unwrap(),
            new_core_environment(flox, contents),
        )
        .unwrap()
    }

    /// Get a [ManagedEnvironment] that has been pushed to (a mock) FloxHub and
    /// can be built.
    ///
    /// This should be passed a [Flox] instance created with a mock FloxHub
    /// setup.
    ///
    /// If a [ManagedEnvironment] will be unused in tests, use
    /// [unusable_mock_managed_environment] instead.
    pub fn mock_managed_environment_in(
        flox: &Flox,
        contents: &str,
        owner: EnvironmentOwner,
        path: impl AsRef<Path>,
        name: Option<&str>,
    ) -> ManagedEnvironment {
        let path_environment =
            new_named_path_environment_in(flox, contents, path, name.unwrap_or("name"));

        ManagedEnvironment::push_new(flox, path_environment, owner, false, false).unwrap()
    }

    /// Get a [ManagedEnvironment] that has been pushed to (a mock) FloxHub and
    /// can be built.
    ///
    /// This should be passed a [Flox] instance created with a mock FloxHub
    /// setup.
    ///
    /// If a [ManagedEnvironment] will be unused in tests, use
    /// [unusable_mock_managed_environment] instead.
    pub fn mock_managed_environment_from_env_files(
        flox: &Flox,
        env_files_dir: impl AsRef<Path>,
        owner: EnvironmentOwner,
    ) -> ManagedEnvironment {
        let path_environment =
            new_named_path_environment_from_env_files(flox, env_files_dir, "name");

        ManagedEnvironment::push_new(flox, path_environment, owner, false, false).unwrap()
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use flox_core::Version;
    use flox_manifest::interfaces::{AsLatestSchema, AsTypedOnlyManifest};
    use flox_manifest::lockfile::test_helpers::fake_catalog_package_lock;
    use flox_manifest::parsed::Inner;
    use flox_manifest::parsed::latest::{self, ManifestLatest};
    use flox_manifest::{MANIFEST_FILENAME, Manifest};
    use flox_test_utils::GENERATED_DATA;
    use indoc::{formatdoc, indoc};
    use test_helpers::{
        mock_managed_environment_from_env_files,
        mock_managed_environment_unlocked,
    };
    use url::Url;

    use super::test_helpers::mock_managed_environment_in;
    use super::*;
    use crate::flox::test_helpers::{flox_instance, flox_instance_with_optional_floxhub};
    use crate::models::env_registry::{
        env_registry_path,
        garbage_collect,
        read_environment_registry,
    };
    use crate::models::environment::DOT_FLOX;
    use crate::models::environment::test_helpers::{
        new_core_environment,
        new_core_environment_with_lockfile,
    };
    use crate::models::floxmeta::floxmeta_dir;
    use crate::providers::catalog::MockClient;
    use crate::providers::catalog::test_helpers::catalog_replay_client;
    use crate::providers::git::tests::commit_file;
    use crate::providers::git::{GitCommandOptions, GitCommandProvider};

    /// Create a [ManagedPointer] for testing with mock owner and name data
    /// as well as an override for the floxhub git url to fetch from local
    /// git repositories.
    fn make_test_pointer(mock_floxhub_git_path: &Path) -> ManagedPointer {
        ManagedPointer {
            owner: EnvironmentOwner::from_str("owner").unwrap(),
            name: EnvironmentName::from_str("name").unwrap(),
            floxhub_base_url: Url::from_str("https://hub.flox.dev").unwrap(),
            floxhub_git_url_override: Some(
                Url::from_directory_path(mock_floxhub_git_path).unwrap(),
            ),
            version: Version::<1> {},
        }
    }

    /// Create an empty mock remote repository
    fn create_mock_remote(path: impl AsRef<Path>) -> (ManagedPointer, PathBuf, GitCommandProvider) {
        let test_pointer = make_test_pointer(path.as_ref());
        let remote_path = path
            .as_ref()
            .join(test_pointer.owner.as_str())
            .join("floxmeta");
        fs::create_dir_all(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).unwrap();
        (test_pointer, remote_path, remote)
    }

    fn init_generations_from_core_env(
        base_tempdir: impl AsRef<Path>,
        name: &str,
        env: &mut CoreEnvironment,
    ) -> Generations {
        let checked_out_tempdir = tempfile::tempdir_in(&base_tempdir).unwrap();
        let bare_tempdir = base_tempdir.as_ref().join(name);

        let mut generations = Generations::init(
            GitCommandOptions::default(),
            &checked_out_tempdir,
            bare_tempdir,
            name.to_string(),
            &EnvironmentName::from_str(name).unwrap(),
        )
        .unwrap();

        let mut writable = generations
            .writable(&base_tempdir, "username", "hostname", &[
                "flox".to_string(),
                "subcommand".to_string(),
            ])
            .unwrap();
        writable.add_generation(env, HistoryKind::Import).unwrap();

        generations
    }

    /// Test that the manifest content is reset to the current generation
    ///
    /// TODO: Specific behavior for other files than the manifest should is undefined
    #[test]
    fn reset_local_checkout_discards_local_changes() {
        let owner = EnvironmentOwner::from_str("owner").unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        let original_manifest = toml_edit::ser::to_string_pretty(&Manifest::default()).unwrap();

        let managed_env =
            test_helpers::mock_managed_environment_unlocked(&flox, &original_manifest, owner);

        let _ = managed_env
            .local_env_or_copy_current_generation(&flox)
            .unwrap();

        fs::write(
            managed_env.path.join(ENV_DIR_NAME).join(MANIFEST_FILENAME),
            "changed",
        )
        .unwrap();

        {
            // before reset
            let contents =
                fs::read_to_string(managed_env.path.join(ENV_DIR_NAME).join(MANIFEST_FILENAME))
                    .unwrap();

            assert_eq!(contents, "changed");
        }

        let _ = managed_env
            .reset_local_env_to_current_generation(&flox)
            .unwrap();

        {
            // after reset, the manifest should be the same as before
            let contents =
                fs::read_to_string(managed_env.path.join(ENV_DIR_NAME).join(MANIFEST_FILENAME))
                    .unwrap();

            assert_eq!(contents, original_manifest);
        }
    }

    /// `local_checkout` should create a `.flox/env` directory with the manifest.{toml,lock}
    /// from the generation.
    #[test]
    fn test_local_checkout_recreates_env_dir() {
        let owner = EnvironmentOwner::from_str("owner").unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        let managed_env = test_helpers::mock_managed_environment_unlocked(
            &flox,
            &toml_edit::ser::to_string_pretty(&Manifest::default()).unwrap(),
            owner,
        );

        // TODO: `local_checkout` may be called implicitly earlier in the process
        //       making this call redundant.
        //       revisit this when working on #1650
        let _ = managed_env
            .local_env_or_copy_current_generation(&flox)
            .unwrap();

        // check that local_checkout created files
        assert!(managed_env.path.join(ENV_DIR_NAME).exists());
        assert!(
            managed_env
                .path
                .join(ENV_DIR_NAME)
                .join(MANIFEST_FILENAME)
                .exists()
        );

        // dlete env dir to see whether it is recreated
        fs::remove_dir_all(managed_env.path.join(ENV_DIR_NAME)).unwrap();

        let _ = managed_env
            .local_env_or_copy_current_generation(&flox)
            .unwrap();

        // check that local_checkout created files
        assert!(managed_env.path.join(ENV_DIR_NAME).exists());
        assert!(
            managed_env
                .path
                .join(ENV_DIR_NAME)
                .join(MANIFEST_FILENAME)
                .exists()
        );
    }

    /// Local checkout should not overwrite existing files
    #[test]
    fn test_local_checkout_keeps_local_modifications() {
        let owner = EnvironmentOwner::from_str("owner").unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        let managed_env = test_helpers::mock_managed_environment_unlocked(
            &flox,
            &toml_edit::ser::to_string_pretty(&Manifest::default()).unwrap(),
            owner,
        );

        // TODO: `local_checkout` may be called implicitly earlier in the process
        //       making this call redundant.
        //       revisit this when working on #1650
        let _ = managed_env
            .local_env_or_copy_current_generation(&flox)
            .unwrap();

        // check that modifications in an existing `.flox/env` are _not_ discarded
        let locally_edited_content = "edited manifest";
        fs::write(
            managed_env.path.join(ENV_DIR_NAME).join(MANIFEST_FILENAME),
            locally_edited_content,
        )
        .unwrap();

        let local_manifest = managed_env
            .local_env_or_copy_current_generation(&flox)
            .unwrap()
            .manifest(&flox)
            .unwrap();
        assert_eq!(
            local_manifest.as_writable().to_string(),
            locally_edited_content
        );
    }

    #[test]
    fn test_sync_local() {
        let owner = EnvironmentOwner::from_str("owner").unwrap();
        let (mut flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        let client = MockClient::new();
        flox.catalog_client = client.into();

        let mut managed_env = test_helpers::mock_managed_environment_unlocked(
            &flox,
            &toml_edit::ser::to_string_pretty(&Manifest::default()).unwrap(),
            owner,
        );

        // TODO: `local_checkout` may be called implicitly earlier in the process
        //       making this call redundant.
        //       revisit this when working on #1650
        let mut local_checkout = managed_env
            .local_env_or_copy_current_generation(&flox)
            .unwrap();
        let generation_manifest = managed_env
            .get_current_generation(&flox)
            .unwrap()
            .manifest(&flox)
            .unwrap();

        let local_checkout_manifest = local_checkout.manifest(&flox).unwrap();
        assert_eq!(
            local_checkout_manifest.as_latest_schema(),
            generation_manifest.as_latest_schema()
        );
        assert_eq!(
            local_checkout_manifest.as_writable().to_string(),
            generation_manifest.as_writable().to_string()
        );

        fs::write(local_checkout.manifest_path(), indoc! {"
            version = 1

            # nothing else but certainly different from before
        "})
        .unwrap();

        // sanity check that before syncing, the manifest is now different
        assert_ne!(local_checkout.manifest(&flox).unwrap(), generation_manifest);

        // check that after syncing, the manifest is the same
        managed_env.create_generation_from_local_env(&flox).unwrap();

        let generation_manifest = managed_env
            .get_current_generation(&flox)
            .unwrap()
            .manifest(&flox)
            .unwrap();

        assert_eq!(local_checkout.manifest(&flox).unwrap(), generation_manifest);
    }

    /// Test that a lockfile is created when a generation is created from a local environment
    #[tokio::test(flavor = "multi_thread")]
    async fn create_generation_from_local_env_builds_and_locks() {
        let owner = EnvironmentOwner::from_str("owner").unwrap();
        let (mut flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        let mut managed_env = test_helpers::mock_managed_environment_unlocked(
            &flox,
            &toml_edit::ser::to_string_pretty(&Manifest::default()).unwrap(),
            owner,
        );

        let _ = managed_env
            .local_env_or_copy_current_generation(&flox)
            .unwrap();

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;

        let mut new_manifest = ManifestLatest::default();
        new_manifest.install.inner_mut().insert(
            "hello".to_string(),
            latest::PackageDescriptorCatalog {
                pkg_path: "hello".to_string(),
                pkg_group: None,
                priority: None,
                version: None,
                systems: None,
                outputs: None,
            }
            .into(),
        );
        let new_manifest = new_manifest.as_typed_only();

        fs::write(
            managed_env.manifest_path(&flox).unwrap(),
            toml::ser::to_string_pretty(&new_manifest).unwrap(),
        )
        .unwrap();

        managed_env.create_generation_from_local_env(&flox).unwrap();

        assert!(managed_env.lockfile_path(&flox).unwrap().exists());

        let lockfile_content =
            fs::read_to_string(managed_env.lockfile_path(&flox).unwrap()).unwrap();
        let lockfile: Lockfile = serde_json::from_str(&lockfile_content).unwrap();

        assert_eq!(lockfile.manifest, new_manifest);
        assert_eq!(lockfile.packages.len(), 4); // 1 x 4 systems

        let lockfile_in_generation_content =
            fs::read_to_string(managed_env.lockfile_path(&flox).unwrap()).unwrap();
        let lockfile_in_generation: Lockfile =
            serde_json::from_str(&lockfile_in_generation_content).unwrap();

        assert_eq!(lockfile_in_generation, lockfile);
    }

    /// Validate should return true if the manifest in two environments is the same
    #[test]
    fn test_validate_local_same_manifest() {
        let (flox, _temp_dir_handle) = flox_instance();

        let manifest_a = Manifest::default();
        let manifest_b = Manifest::default();

        let env_a = new_core_environment(
            &flox,
            &toml_edit::ser::to_string_pretty(&manifest_a).unwrap(),
        );
        let mut env_b = new_core_environment(
            &flox,
            &toml_edit::ser::to_string_pretty(&manifest_b).unwrap(),
        );
        let env_b_generations = init_generations_from_core_env(&flox.temp_dir, "env_b", &mut env_b);

        assert!(ManagedEnvironment::validate_checkout(&env_a, &env_b_generations).unwrap());
    }

    /// Validate should return false if the manifest in two environments is different.
    #[test]
    fn test_validate_local_different_manifest() {
        let (flox, _temp_dir_handle) = flox_instance();

        let mut manifest_a = ManifestLatest::default();
        let manifest_b = ManifestLatest::default();

        let env_a = new_core_environment(
            &flox,
            &toml_edit::ser::to_string_pretty(&manifest_a).unwrap(),
        );
        let mut env_b = new_core_environment(
            &flox,
            &toml_edit::ser::to_string_pretty(&manifest_b).unwrap(),
        );
        let env_b_generations = init_generations_from_core_env(&flox.temp_dir, "env_b", &mut env_b);

        let (iid, descriptor, _) = fake_catalog_package_lock("package", None);
        manifest_a.install.inner_mut().insert(iid, descriptor);

        fs::write(
            env_a.path().join(MANIFEST_FILENAME),
            toml_edit::ser::to_string_pretty(&manifest_a).unwrap(),
        )
        .unwrap();

        assert!(!ManagedEnvironment::validate_checkout(&env_a, &env_b_generations).unwrap());
    }

    /// Validate that two environments with equivalent manifests fail validation
    /// if the binary representationnof the manifest differs.
    #[test]
    fn test_validate_local_different_binary_content() {
        let (flox, _temp_dir_handle) = flox_instance();

        let manifest_a = Manifest::default();
        let manifest_b = Manifest::default();

        // Serialize the same manifest to two different environments
        // once with pretty formatting and once without.
        // Today the default manifest will serialize with different newlines
        // which this test depends on.
        let env_a = new_core_environment(
            &flox,
            &toml_edit::ser::to_string_pretty(&manifest_a).unwrap(),
        );
        let mut env_b =
            new_core_environment(&flox, &toml_edit::ser::to_string(&manifest_b).unwrap());
        let env_b_generations = init_generations_from_core_env(&flox.temp_dir, "env_b", &mut env_b);

        assert!(!ManagedEnvironment::validate_checkout(&env_a, &env_b_generations).unwrap());
    }

    // An out of sync lockfile should fail validation
    #[test]
    fn validate_different_lockfiles() {
        let (flox, _temp_dir_handle) = flox_instance();

        let env_a = new_core_environment_with_lockfile(&flox, "manifest", "{}");
        let mut env_b = new_core_environment_with_lockfile(&flox, "manifest", "{ }");
        let env_b_generations = init_generations_from_core_env(&flox.temp_dir, "env_b", &mut env_b);

        assert!(!ManagedEnvironment::validate_checkout(&env_a, &env_b_generations).unwrap());
    }

    #[test]
    fn registers_on_open() {
        let (flox, _temp_dir_handle) = flox_instance();
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        std::fs::create_dir_all(&dot_flox_path).unwrap();
        let dot_flox_path = CanonicalPath::new(&dot_flox_path).unwrap();

        // dummy paths since we are not rendering the environment
        let rendered_env_links =
            RenderedEnvironmentLinks::new_unchecked(PathBuf::new(), PathBuf::new());

        // create a mock remote
        let (test_pointer, _remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");

        // create a mock floxmeta
        let (floxmeta_branch, _lock) =
            FloxmetaBranch::new(&flox, &test_pointer, &dot_flox_path, None).unwrap();

        let _env = ManagedEnvironment::open_with(
            &flox,
            floxmeta_branch,
            test_pointer,
            dot_flox_path.clone(),
            rendered_env_links,
            IncludeFetcher {
                base_directory: Some(dot_flox_path.parent().unwrap().to_path_buf()),
            },
            None,
        )
        .unwrap();
        let reg_path = env_registry_path(&flox);
        assert!(reg_path.exists());
        let reg = read_environment_registry(&reg_path).unwrap().unwrap();
        assert!(matches!(
            reg.entries[0].envs[0].pointer,
            EnvironmentPointer::Managed(_)
        ));
    }

    #[test]
    fn deregisters_on_delete() {
        let (flox, _temp_dir_handle) = flox_instance();
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        std::fs::create_dir_all(&dot_flox_path).unwrap();
        let dot_flox_path = CanonicalPath::new(&dot_flox_path).unwrap();

        // dummy paths since we are not rendering the environment
        let rendered_env_links =
            RenderedEnvironmentLinks::new_unchecked(PathBuf::new(), PathBuf::new());

        // create a mock remote
        let (test_pointer, _remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");

        // create a mock floxmeta
        let (floxmeta_branch, _lock) =
            FloxmetaBranch::new(&flox, &test_pointer, &dot_flox_path, None).unwrap();
        // Create the registry
        let env = ManagedEnvironment::open_with(
            &flox,
            floxmeta_branch,
            test_pointer,
            CanonicalPath::new(&dot_flox_path).unwrap(),
            rendered_env_links,
            IncludeFetcher {
                base_directory: Some(dot_flox_path.parent().unwrap().to_path_buf()),
            },
            None,
        )
        .unwrap();
        let reg_path = env_registry_path(&flox);
        assert!(reg_path.exists());

        // Delete the environment from the registry
        env.delete(&flox).unwrap();
        let reg = read_environment_registry(&reg_path).unwrap().unwrap();
        assert!(reg.entries.is_empty());
    }

    #[test]
    fn gc_prunes_floxmeta_branches() {
        let (flox, _temp_dir_handle) = flox_instance();

        let env1_dir = flox.temp_dir.join("env1");
        std::fs::create_dir_all(&env1_dir).unwrap();
        let env1_dir = CanonicalPath::new(env1_dir).unwrap();

        let env2_dir = flox.temp_dir.join("env2");
        std::fs::create_dir_all(&env2_dir).unwrap();
        let env2_dir = CanonicalPath::new(env2_dir).unwrap();

        // dummy paths since we are not rendering the environment
        let rendered_env_links =
            RenderedEnvironmentLinks::new_unchecked(PathBuf::new(), PathBuf::new());

        // create a mock remote
        let (test_pointer, _remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");

        // create a mock floxmeta
        let (floxmeta_branch_1, _lock) =
            FloxmetaBranch::new(&flox, &test_pointer, &env1_dir, None).unwrap();
        let env1_branch = floxmeta_branch_1.branch().to_owned();

        let (floxmeta_branch_2, _lock) =
            FloxmetaBranch::new(&flox, &test_pointer, &env2_dir, None).unwrap();
        let env2_branch = floxmeta_branch_2.branch().to_owned();

        // both environments refer to the same git repo,
        // so lets extract a reference to perform assertions against the git state
        let common_git_repo = floxmeta_branch_1.git().clone();

        // Create two environments that use the same pointer.
        let env1 = ManagedEnvironment::open_with(
            &flox,
            floxmeta_branch_1,
            test_pointer.clone(),
            env1_dir.clone(),
            rendered_env_links.clone(),
            IncludeFetcher {
                base_directory: Some(env1_dir.parent().unwrap().to_path_buf()),
            },
            None,
        )
        .unwrap();
        let env2 = ManagedEnvironment::open_with(
            &flox,
            floxmeta_branch_2,
            test_pointer.clone(),
            env2_dir.clone(),
            rendered_env_links.clone(),
            IncludeFetcher {
                base_directory: Some(env2_dir.parent().unwrap().to_path_buf()),
            },
            None,
        )
        .unwrap();

        // All branches should exist.
        assert!(common_git_repo.has_branch(&remote_branch).unwrap());
        assert!(common_git_repo.has_branch(&env1_branch).unwrap());
        assert!(common_git_repo.has_branch(&env2_branch).unwrap());

        // env2 is pruned when no longer on disk.
        fs::remove_dir_all(&env2.path).unwrap();
        garbage_collect(&flox).unwrap();
        assert!(common_git_repo.has_branch(&remote_branch).unwrap());
        assert!(common_git_repo.has_branch(&env1_branch).unwrap());
        assert!(!common_git_repo.has_branch(&env2_branch).unwrap());

        // env1 is pruned when no longer on disk, remote is pruned when there
        // are no local branches, and is resilient to the branch not existing,
        // e.g. if the floxmeta repo has been manually deleted or the hashing
        // algorithm has changed in the past.
        fs::remove_dir_all(&env1.path).unwrap();
        common_git_repo.delete_branch(&env1_branch, true).unwrap();
        garbage_collect(&flox).unwrap();
        assert!(!common_git_repo.has_branch(&remote_branch).unwrap());
        assert!(!common_git_repo.has_branch(&env1_branch).unwrap());
        assert!(!common_git_repo.has_branch(&env2_branch).unwrap());
    }

    #[test]
    fn convert_to_path_environment() {
        let owner = "owner".parse().unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        let environment = mock_managed_environment_from_env_files(
            &flox,
            GENERATED_DATA.join("envs").join("hello"),
            owner,
        );

        // Assert that the environment looks like a managed environment
        // - it has a ManagedPointer
        // - it has a generation lock
        // - it has a branch in the git repo
        let _pointer: ManagedPointer = serde_json::from_str(
            &fs::read_to_string(environment.path.join(ENVIRONMENT_POINTER_FILENAME)).unwrap(),
        )
        .expect("env pointer should be a managed pointer");
        assert!(
            environment.path.join(GENERATION_LOCK_FILENAME).exists(),
            "generation lock should exist"
        );
        assert!(
            environment
                .floxmeta_branch
                .git()
                .has_branch(environment.floxmeta_branch.branch())
                .unwrap()
        );

        // Unsafe to create a copy of the git provider
        // due to risk of corrupting the state of the git repo.
        // Since the original will be dropped however,
        // its safe to do so in this instance.
        let git = environment.floxmeta_branch.git().clone();
        let path_before = environment.path.clone();
        let out_links_before = environment.rendered_env_links.clone();

        let branch_name = environment.floxmeta_branch.branch().to_owned();

        // Convert the environment to a path environment
        let mut path_env = environment.into_path_environment(&flox).unwrap();

        // Assert that the environment looks like a path environment after conversion
        // - its path is the same as before
        // - it has a PathPointer
        // - it does not have a generation lock
        // - it does not have a branch in the git repo
        assert_eq!(
            path_env.path, path_before,
            "the path of the environment should not change"
        );
        let _: PathPointer = serde_json::from_str(
            &fs::read_to_string(path_env.path.join(ENVIRONMENT_POINTER_FILENAME)).unwrap(),
        )
        .expect("env pointer should be a path pointer");
        assert!(
            !path_env.path.join(GENERATION_LOCK_FILENAME).exists(),
            "generation lock should be deleted"
        );
        assert!(
            !git.has_branch(&branch_name).unwrap(),
            "branch should be deleted"
        );

        // Assert that the rendered environment links are the same as before
        assert_eq!(
            path_env.rendered_env_links(&flox).unwrap(),
            out_links_before
        )
    }

    #[test]
    fn force_pull_returns_up_to_date() {
        let owner = "owner".parse().unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        let mut environment = mock_managed_environment_unlocked(&flox, "version = 1", owner);

        assert_eq!(environment.pull(&flox, true).unwrap(), PullResult::UpToDate);
    }

    /// Managed environment can include a managed environment
    #[test]
    fn managed_can_include_managed() {
        let owner = "owner".parse().unwrap();
        let (flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));

        // Create dep
        let dep_path = tempdir.path().join("dep");
        let dep_manifest_contents = indoc! {r#"
        version = 1

        [vars]
        foo = "dep"
        "#};
        fs::create_dir(&dep_path).unwrap();
        mock_managed_environment_in(
            &flox,
            dep_manifest_contents,
            owner.clone(),
            dep_path,
            Some("dep"),
        );

        // Create composer, which locks implicitly
        // Create environment _without_ including `dep`,
        // because _pushing_ an environment with local imports is not allowed.
        let composer_manifest_contents = indoc! {r#"
        version = 1
        "#};

        let mut composer = mock_managed_environment_in(
            &flox,
            composer_manifest_contents,
            owner,
            tempdir.path(),
            Some("composer"),
        );

        // Add an include of an environment by local path
        let composer_manifest_contents_with_include = indoc! {r#"
        version = 1

        [include]
        environments = [
          { dir = "dep" },
        ]
        "#};

        composer
            .edit(&flox, composer_manifest_contents_with_include.to_string())
            .unwrap();

        // Check lockfile
        let lockfile: Lockfile = composer.lockfile(&flox).unwrap().into();
        let expected_manifest_json = serde_json::json!({
            "version": 1,
            "vars": {
                "foo": "bar"
            }
        });
        let expected_manifest = serde_json::from_value(expected_manifest_json).unwrap();

        assert_eq!(lockfile.manifest, expected_manifest);

        assert_eq!(
            lockfile.compose.unwrap().include[0].manifest,
            toml_edit::de::from_str(dep_manifest_contents).unwrap()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn install_generations() {
        let owner = "owner".parse().unwrap();
        let (mut flox, temp_dir) = flox_instance_with_optional_floxhub(Some(&owner));

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;

        let mut env = mock_managed_environment_in(&flox, "version = 1", owner, &temp_dir, None);
        assert_eq!(
            env.generations_metadata().unwrap().current_gen().as_deref(),
            Some(&1),
            "initialised environment should have generation 1"
        );

        let packages = [PackageToInstall::Catalog(
            CatalogPackage::from_str("hello").unwrap(),
        )];

        env.install(&packages, &flox).unwrap();
        assert_eq!(
            env.generations_metadata().unwrap().current_gen().as_deref(),
            Some(&2),
            "installing a package should create a new generation"
        );

        env.install(&packages, &flox).unwrap();
        assert_eq!(
            env.generations_metadata().unwrap().current_gen().as_deref(),
            Some(&2),
            "installing the same package should not change the generation"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn uninstall_generations() {
        let owner = "owner".parse().unwrap();
        let (mut flox, temp_dir) = flox_instance_with_optional_floxhub(Some(&owner));

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;

        let package = "hello".to_string();
        let manifest = formatdoc! {r#"
            version = 1

            [install]
            {package}.pkg-path = "{package}"
        "#};

        let mut env = mock_managed_environment_in(&flox, &manifest, owner, &temp_dir, None);
        assert_eq!(
            env.generations_metadata().unwrap().current_gen().as_deref(),
            Some(&1),
            "initialised environment should have generation 1"
        );

        env.uninstall(vec![package.clone()], &flox).unwrap();
        assert_eq!(
            env.generations_metadata().unwrap().current_gen().as_deref(),
            Some(&2),
            "uninstalling a package should create a new generation"
        );

        env.uninstall(vec![package.clone()], &flox)
            .expect_err("uninstalling a package should fail if it is not installed");
        assert_eq!(
            env.generations_metadata().unwrap().current_gen().as_deref(),
            Some(&2),
            "uninstalling the same package should not change the generation"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn edit_generations() {
        let owner = "owner".parse().unwrap();
        let (flox, temp_dir) = flox_instance_with_optional_floxhub(Some(&owner));

        let mut env = mock_managed_environment_in(&flox, "version = 1", owner, &temp_dir, None);
        assert_eq!(
            env.generations_metadata().unwrap().current_gen().as_deref(),
            Some(&1),
            "initialised environment should have generation 1"
        );

        let manifest_updated = indoc! {r#"
            # updated
            version = 1
        "#};

        env.edit(&flox, manifest_updated.to_string()).unwrap();
        assert_eq!(
            env.generations_metadata().unwrap().current_gen().as_deref(),
            Some(&2),
            "edit with manifest changes should create a new generation"
        );

        env.edit(&flox, manifest_updated.to_string()).unwrap();
        assert_eq!(
            env.generations_metadata().unwrap().current_gen().as_deref(),
            Some(&2),
            "edit with the same manifest should not change the generation"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn upgrade_generations() {
        let owner = "owner".parse().unwrap();
        let (mut flox, temp_dir) = flox_instance_with_optional_floxhub(Some(&owner));

        let manifest = indoc! {r#"
            version = 1

            [install]
            hello.pkg-path = "hello"
        "#};

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/old_hello.yaml")).await;

        let mut env = mock_managed_environment_in(&flox, manifest, owner, &temp_dir, None);
        assert_eq!(
            env.generations_metadata().unwrap().current_gen().as_deref(),
            Some(&1),
            "initialised environment should have generation 1"
        );

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;

        env.upgrade(&flox, &[]).unwrap();
        assert_eq!(
            env.generations_metadata().unwrap().current_gen().as_deref(),
            Some(&2),
            "upgrade with changes should create a new generation"
        );

        env.upgrade(&flox, &[]).unwrap();
        assert_eq!(
            env.generations_metadata().unwrap().current_gen().as_deref(),
            Some(&2),
            "upgrade with no changes should not change the generation"
        );
    }

    /// Test that multiple parallel attempts to initialize [FloxMeta]
    /// for the same owner are possible.
    #[test]
    fn allow_parallel_open() {
        let owner = "owner".parse().unwrap();
        let (flox, temp_dir) = flox_instance_with_optional_floxhub(Some(&owner));

        // populate our local floxhub mock with an environment
        let pointer = {
            let mock = mock_managed_environment_unlocked(&flox, "version = 1", owner);
            let pointer = mock.pointer.clone();
            mock.delete(&flox).unwrap();
            pointer
        };

        // remove the local floxmeta
        let floxmeta_dir = {
            let floxmeta_dir = floxmeta_dir(&flox, &pointer.owner);
            assert!(floxmeta_dir.exists());
            std::fs::remove_dir_all(&floxmeta_dir).unwrap();
            assert!(!floxmeta_dir.exists());
            floxmeta_dir
        };

        // create directories to pull into

        let dot_flox_dir_1 = {
            let dir = temp_dir.path().join("1");
            std::fs::create_dir_all(&dir).unwrap();
            dir
        };
        let dot_flox_dir_2 = {
            let dir = temp_dir.path().join("2");
            std::fs::create_dir_all(&dir).unwrap();
            dir
        };

        // pull into the respective dot_flox_dir from two threads,
        // that will concurrently try to open or create a FloxMeta for the owner.
        std::thread::scope(|scope| {
            let child_1 = scope
                .spawn(|| ManagedEnvironment::open(&flox, pointer.clone(), &dot_flox_dir_1, None));
            let child_2 = scope
                .spawn(|| ManagedEnvironment::open(&flox, pointer.clone(), &dot_flox_dir_2, None));

            child_1
                .join()
                .expect("parallel opening does not panic")
                .expect("parallel opening returns successfully");
            child_2
                .join()
                .expect("parallel opening does not panic")
                .expect("parallel opening returns successfully");
        });

        assert!(floxmeta_dir.exists());
    }

    /// Test that remote_lockfile_contents_for_current_generation returns remote data, not local
    #[tokio::test(flavor = "multi_thread")]
    async fn remote_lockfile_contents_returns_remote_not_local() {
        let owner = "owner".parse().unwrap();
        let (mut flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));

        flox.catalog_client = catalog_replay_client(GENERATED_DATA.join("empty.yaml")).await;
        let initial_manifest = indoc! {r#"
            version = 1
            [install]
        "#};

        let mut environment = mock_managed_environment_in(
            &flox,
            initial_manifest,
            owner.clone(),
            &tempdir,
            Some("test-env"),
        );

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;
        environment
            .install(
                &[PackageToInstall::parse(&flox.system, "hello").unwrap()],
                &flox,
            )
            .unwrap();

        // Get remote lockfile
        environment.fetch_remote_state(&flox).unwrap();
        let remote_lockfile_contents = environment
            .remote_lockfile_contents_for_current_generation()
            .unwrap();
        let remote_lockfile: Lockfile = serde_json::from_str(&remote_lockfile_contents).unwrap();

        // Verify local lockfile has a package
        let local_lockfile: Lockfile = environment.lockfile(&flox).unwrap().into();
        let local_packages = local_lockfile.list_packages(&flox.system).unwrap();
        assert_eq!(local_packages.len(), 1, "Local should have hello");

        // Verify remote lockfile has no packages yet
        let packages = remote_lockfile.list_packages(&flox.system).unwrap();
        assert_eq!(
            packages.len(),
            0,
            "Remote should not yet have hello package"
        );
    }
}

#[cfg(test)]
mod compare_remote_tests {
    use tempfile::TempDir;

    use super::*;
    use crate::flox::test_helpers::flox_instance_with_optional_floxhub;
    use crate::models::environment::managed_environment::test_helpers::mock_managed_environment_in;
    use crate::providers::catalog::MockClient;

    /// Helper to create a pair of environment instances at different paths
    /// sharing the same remote environment
    fn setup_env_pair(env_name: &str) -> (Flox, TempDir, ManagedEnvironment, ManagedEnvironment) {
        let owner = "owner".parse().unwrap();
        let (mut flox, temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        let client = MockClient::new();
        flox.catalog_client = client.into();

        // Create first instance (env_a)
        let project_a_path = flox.temp_dir.join("project_a");
        std::fs::create_dir_all(&project_a_path).unwrap();

        let env_a = mock_managed_environment_in(
            &flox,
            "version = 1",
            owner.clone(),
            &project_a_path,
            Some(env_name),
        );

        // Create second instance (env_b) at different path, same remote
        let project_b_path = flox.temp_dir.join("project_b");
        std::fs::create_dir_all(&project_b_path).unwrap();
        let project_b_path = CanonicalPath::new(project_b_path).unwrap();

        let env_b =
            ManagedEnvironment::open(&flox, env_a.pointer.clone(), project_b_path, None).unwrap();

        (flox, temp_dir_handle, env_a, env_b)
    }

    /// Test that compare_remote shows Ahead before push, Equal after push
    #[test]
    fn compare_remote_transitions_ahead_to_equal_after_push() {
        let (flox, _temp_dir_handle, mut env_a, _env_b) = setup_env_pair("test-env");

        assert_eq!(env_a.compare_remote().unwrap(), BranchOrd::Equal);

        env_a
            .edit(&flox, "version = 1\n\n# local change".to_string())
            .unwrap();
        assert_eq!(env_a.compare_remote().unwrap(), BranchOrd::Ahead);

        assert_eq!(env_a.push(&flox, false).unwrap(), PushResult::Updated);
        assert_eq!(env_a.compare_remote().unwrap(), BranchOrd::Equal);
    }

    /// Test that push returns UpToDate when already synced
    #[test]
    fn push_returns_up_to_date_when_synced() {
        let (flox, _temp_dir_handle, mut env_a, _env_b) = setup_env_pair("test-env");

        assert_eq!(env_a.push(&flox, false).unwrap(), PushResult::UpToDate);
        assert_eq!(env_a.compare_remote().unwrap(), BranchOrd::Equal);
    }

    /// Test that pull returns UpToDate when synced or ahead
    #[test]
    fn pull_returns_up_to_date_when_synced_or_ahead() {
        let (flox, _temp_dir_handle, mut env_a, _env_b) = setup_env_pair("test-env");

        // Synced
        assert_eq!(env_a.pull(&flox, false).unwrap(), PullResult::UpToDate);

        // Ahead also returns UpToDate
        env_a
            .edit(&flox, "version = 1\n\n# local".to_string())
            .unwrap();
        assert_eq!(env_a.compare_remote().unwrap(), BranchOrd::Ahead);
        assert_eq!(env_a.pull(&flox, false).unwrap(), PullResult::UpToDate);
    }

    /// Test that after pushing from one instance, another instance sees Behind
    #[test]
    fn compare_remote_shows_behind_after_other_instance_pushes() {
        let (flox, _temp_dir_handle, mut env_a, mut env_b) = setup_env_pair("shared-env");

        // Both start synced
        assert_eq!(env_a.compare_remote().unwrap(), BranchOrd::Equal);
        assert_eq!(env_b.compare_remote().unwrap(), BranchOrd::Equal);

        // A pushes a change
        env_a
            .edit(&flox, "version = 1\n\n# modified by A".to_string())
            .unwrap();
        assert_eq!(env_a.push(&flox, false).unwrap(), PushResult::Updated);
        assert_eq!(env_a.compare_remote().unwrap(), BranchOrd::Equal);

        // B is now behind
        // Note: B should see its behind without fetching
        assert_eq!(env_b.compare_remote().unwrap(), BranchOrd::Behind);
        // Note: Since B is strictly behind, pushing can return uptodate
        assert_eq!(env_b.push(&flox, false).unwrap(), PushResult::UpToDate);

        // B pulls and syncs
        assert_eq!(env_b.pull(&flox, false).unwrap(), PullResult::Updated);
        assert_eq!(env_b.compare_remote().unwrap(), BranchOrd::Equal);
    }

    /// Test that compare_remote shows Diverged and operations require force
    #[test]
    fn compare_remote_diverged_requires_force() {
        let (flox, _temp_dir_handle, mut env_a, mut env_b) = setup_env_pair("shared-env");

        // Both make conflicting changes
        env_a
            .edit(&flox, "version = 1\n\n# change by A".to_string())
            .unwrap();
        env_b
            .edit(&flox, "version = 1\n\n# change by B".to_string())
            .unwrap();

        env_a.push(&flox, false).unwrap();

        // Note: B should see its behind without fetching
        assert_eq!(env_b.compare_remote().unwrap(), BranchOrd::Diverged);

        // Operations fail without force
        assert!(env_b.pull(&flox, false).is_err());
        assert!(env_b.push(&flox, false).is_err());

        // Force pull succeeds
        assert_eq!(env_b.pull(&flox, true).unwrap(), PullResult::Updated);
        assert_eq!(env_b.compare_remote().unwrap(), BranchOrd::Equal);
    }
}
