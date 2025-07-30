use std::path::{Path, PathBuf};
use std::{fs, io};

use flox_core::Version;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, instrument};

use super::core_environment::{CoreEnvironment, UpgradeResult};
use super::fetcher::IncludeFetcher;
use super::generations::{AllGenerationsMetadata, Generations, GenerationsError, GenerationsExt};
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
use crate::flox::{EnvironmentRef, Flox};
use crate::models::env_registry::{EnvRegistryError, deregister, ensure_registered};
use crate::models::environment::{LOCKFILE_FILENAME, copy_dir_recursive};
use crate::models::environment_ref::{EnvironmentName, EnvironmentOwner};
use crate::models::floxmeta::{
    BRANCH_NAME_PATH_SEPARATOR,
    FloxMeta,
    FloxMetaError,
    floxmeta_git_options,
};
use crate::models::lockfile::{LockResult, Lockfile};
use crate::models::manifest::raw::PackageToInstall;
use crate::models::manifest::typed::IncludeDescriptor;
use crate::providers::buildenv::BuildEnvOutputs;
use crate::providers::git::{
    GitCommandBranchHashError,
    GitCommandError,
    GitProvider,
    GitRemoteCommandError,
};

pub const GENERATION_LOCK_FILENAME: &str = "env.lock";

#[derive(Debug)]
pub struct ManagedEnvironment {
    /// Absolute path to the directory containing `env.json`
    path: CanonicalPath,
    rendered_env_links: RenderedEnvironmentLinks,
    pointer: ManagedPointer,
    floxmeta: FloxMeta,
    include_fetcher: IncludeFetcher,
}

#[derive(Debug, Error)]
pub enum ManagedEnvironmentError {
    #[error("failed to open floxmeta git repo: {0}")]
    OpenFloxmeta(FloxMetaError),
    #[error("failed to update floxmeta git repo: {0}")]
    UpdateFloxmeta(FloxMetaError),
    #[error("failed to fetch environment: {0}")]
    Fetch(GitRemoteCommandError),
    #[error("failed to check for git revision: {0}")]
    CheckGitRevision(GitCommandError),
    #[error("failed to check for branch existence")]
    CheckBranchExists(#[source] GitCommandBranchHashError),
    #[error(
        "can't find local_rev specified in lockfile; local_rev could have been mistakenly committed on another machine"
    )]
    LocalRevDoesNotExist,
    #[error(
        "can't find environment at revision specified in lockfile; this could have been caused by force pushing"
    )]
    RevDoesNotExist,
    #[error("invalid {0} file: {filename}", filename = GENERATION_LOCK_FILENAME)]
    InvalidLock(serde_json::Error),
    #[error("failed to read pointer lockfile")]
    ReadPointerLock(#[source] io::Error),
    #[error("internal error: {0}")]
    Git(GitCommandError),
    #[error("internal error: {0}")]
    GitBranchHash(GitCommandBranchHashError),
    #[error("couldn't write environment lockfile: {0}")]
    WriteLock(io::Error),
    #[error("couldn't serialize environment lockfile: {0}")]
    SerializeLock(serde_json::Error),
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

    /// Error reading the generation manifest
    #[error("failed to read from generation")]
    Generations(#[source] GenerationsError),

    #[error("floxmeta branch name was malformed: {0}")]
    BadBranchName(String),
    #[error("project wasn't found at path {path}: {err}")]
    ProjectNotFound { path: PathBuf, err: std::io::Error },
    #[error("upstream floxmeta branch diverged from local branch")]
    Diverged,
    #[error("access to floxmeta repository was denied")]
    AccessDenied,
    #[error("environment '{env_ref}' does not exist at upstream '{upstream}'")]
    UpstreamNotFound {
        env_ref: EnvironmentRef,
        upstream: String,
        user: Option<String>,
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
    Core(#[from] CoreEnvironmentError),
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct GenerationLock {
    /// Revision of the environment on FloxHub.
    /// This could be stale if the environment has since been changed.
    rev: String,
    /// Revision of the environment in local floxmeta repository.
    /// Since an environment can be pulled into multiple different directories
    /// locally, each could have its own local_rev if the environments are
    /// modified.
    /// This is changed when the environment is modified locally,
    /// so it can diverge from both the remote environment and other copies of
    /// the environment pulled into other directories.
    local_rev: Option<String>,
    version: Version<1>,
}

impl GenerationLock {
    fn read_maybe(path: impl AsRef<Path>) -> Result<Option<Self>, ManagedEnvironmentError> {
        let lock_contents = match fs::read(path) {
            Ok(contents) => contents,
            Err(err) => match err.kind() {
                io::ErrorKind::NotFound => return Ok(None),
                _ => Err(ManagedEnvironmentError::ReadPointerLock(err))?,
            },
        };
        serde_json::from_slice(&lock_contents)
            .map(Some)
            .map_err(ManagedEnvironmentError::InvalidLock)
    }
}

impl Environment for ManagedEnvironment {
    /// This will lock if there is an out of sync local checkout
    fn lockfile(&mut self, flox: &Flox) -> Result<LockResult, EnvironmentError> {
        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;
        self.ensure_locked(flox, &mut local_checkout)
    }

    /// Returns the lockfile if it already exists.
    fn existing_lockfile(&self, flox: &Flox) -> Result<Option<Lockfile>, EnvironmentError> {
        self.local_env_or_copy_current_generation(flox)?
            .existing_lockfile()
            .map_err(EnvironmentError::Core)
    }

    /// Install packages to the environment atomically
    fn install(
        &mut self,
        packages: &[PackageToInstall],
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError> {
        let mut generations = self.generations();
        let mut generations = generations
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;

        if !Self::validate_checkout(&local_checkout, &generations)? {
            Err(EnvironmentError::ManagedEnvironment(
                ManagedEnvironmentError::CheckoutOutOfSync,
            ))?
        }

        let metadata = format!("installed packages: {:?}", &packages);
        let result = local_checkout.install(packages, flox)?;

        generations
            .add_generation(&mut local_checkout, metadata)
            .map_err(ManagedEnvironmentError::CommitGeneration)?;
        self.lock_pointer()?;
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
        let mut generations = self.generations();
        let mut generations = generations
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;

        if !Self::validate_checkout(&local_checkout, &generations)? {
            Err(EnvironmentError::ManagedEnvironment(
                ManagedEnvironmentError::CheckoutOutOfSync,
            ))?
        }

        let metadata = format!("uninstalled packages: {:?}", &packages);
        let result = local_checkout.uninstall(packages, flox)?;

        generations
            .add_generation(&mut local_checkout, metadata)
            .map_err(ManagedEnvironmentError::CommitGeneration)?;
        self.lock_pointer()?;
        if let Some(store_paths) = &result.built_environment_store_paths {
            self.link(store_paths)?;
        }

        Ok(result)
    }

    /// Atomically edit this environment, ensuring that it still builds
    fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError> {
        let mut generations = self.generations();
        let mut generations = generations
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;

        let result = local_checkout.edit(flox, contents)?;

        match &result {
            EditResult::Changed {
                built_environment_store_paths,
                ..
            } => {
                generations
                    .add_generation(&mut local_checkout, "manually edited".to_string())
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
        let mut generations = self.generations();
        let mut generations = generations
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;

        if !Self::validate_checkout(&local_checkout, &generations)? {
            Err(EnvironmentError::ManagedEnvironment(
                ManagedEnvironmentError::CheckoutOutOfSync,
            ))?
        }

        let result = local_checkout.upgrade(flox, groups_or_iids, true)?;

        let metadata = format!("upgraded packages: {}", result.packages().join(", "));

        generations
            .add_generation(&mut local_checkout, metadata)
            .map_err(ManagedEnvironmentError::CommitGeneration)?;

        write_pointer_lockfile(
            self.path.join(GENERATION_LOCK_FILENAME),
            &self.floxmeta,
            remote_branch_name(&self.pointer),
            branch_name(&self.pointer, &self.path).into(),
        )?;
        Ok(result)
    }

    /// Upgrade environment with latest changes to included environments.
    fn include_upgrade(
        &mut self,
        flox: &Flox,
        to_upgrade: Vec<String>,
    ) -> Result<UpgradeResult, EnvironmentError> {
        let mut generations = self.generations();
        let mut generations = generations
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        let mut local_checkout = self.local_env_or_copy_current_generation(flox)?;

        if !Self::validate_checkout(&local_checkout, &generations)? {
            Err(EnvironmentError::ManagedEnvironment(
                ManagedEnvironmentError::CheckoutOutOfSync,
            ))?
        }

        let metadata = if to_upgrade.is_empty() {
            "upgraded environment with latest changes to all included environments".to_string()
        } else {
            format!(
                "upgraded environment with latest change to included environments: {}",
                to_upgrade.iter().join(", ")
            )
        };

        let result = local_checkout.include_upgrade(flox, to_upgrade)?;

        generations
            .add_generation(&mut local_checkout, metadata)
            .map_err(ManagedEnvironmentError::CommitGeneration)?;

        write_pointer_lockfile(
            self.path.join(GENERATION_LOCK_FILENAME),
            &self.floxmeta,
            remote_branch_name(&self.pointer),
            branch_name(&self.pointer, &self.path).into(),
        )?;
        Ok(result)
    }

    /// Extract the current content of the manifest from disk.
    ///
    /// This may differ from the locked manifest, which should typically be used unless you need to:
    /// - provide the latest editable contents to the user
    /// - avoid double-locking
    fn manifest_contents(&self, flox: &Flox) -> Result<String, EnvironmentError> {
        let local_checkout = self.local_env_or_copy_current_generation(flox)?;
        let manifest = local_checkout.manifest_contents()?;
        Ok(manifest)
    }

    /// This will lock if there is an out of sync local checkout
    fn rendered_env_links(
        &mut self,
        flox: &Flox,
    ) -> Result<RenderedEnvironmentLinks, EnvironmentError> {
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

        self.floxmeta
            .prune_branches(&self.pointer, &self.path)
            .map_err(ManagedEnvironmentError::UpdateFloxmeta)?;

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
    fn generations_metadata(&self) -> Result<AllGenerationsMetadata, GenerationsError> {
        self.generations().metadata()
    }
}

/// Constructors and related functions
impl ManagedEnvironment {
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
    ) -> Result<Self, EnvironmentError> {
        let floxmeta = match FloxMeta::open(flox, &pointer) {
            Ok(floxmeta) => floxmeta,
            Err(FloxMetaError::NotFound(_)) => {
                debug!("cloning floxmeta for {}", pointer.owner);
                FloxMeta::clone(flox, &pointer).map_err(ManagedEnvironmentError::OpenFloxmeta)?
            },
            Err(FloxMetaError::CloneBranch(GitRemoteCommandError::AccessDenied))
            | Err(FloxMetaError::FetchBranch(GitRemoteCommandError::AccessDenied)) => {
                return Err(EnvironmentError::ManagedEnvironment(
                    ManagedEnvironmentError::AccessDenied,
                ));
            },
            Err(FloxMetaError::CloneBranch(GitRemoteCommandError::RefNotFound(_)))
            | Err(FloxMetaError::FetchBranch(GitRemoteCommandError::RefNotFound(_))) => {
                return Err(EnvironmentError::ManagedEnvironment(
                    ManagedEnvironmentError::UpstreamNotFound {
                        env_ref: pointer.into(),
                        upstream: flox.floxhub.base_url().to_string(),
                        user: flox.floxhub_token.as_ref().map(|t| t.handle().to_string()),
                    },
                ));
            },
            Err(e) => Err(ManagedEnvironmentError::OpenFloxmeta(e))?,
        };

        let dot_flox_path =
            CanonicalPath::new(dot_flox_path).map_err(ManagedEnvironmentError::CanonicalizePath)?;

        let rendered_env_links = {
            let run_dir = dot_flox_path.join(GCROOTS_DIR_NAME);
            if !run_dir.exists() {
                std::fs::create_dir_all(&run_dir)
                    .map_err(ManagedEnvironmentError::CreateLinksDir)?;
            }

            let base_dir = CanonicalPath::new(run_dir).expect("run dir is checked to exist");

            RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system(
                &base_dir,
                pointer.name.as_ref(),
                &flox.system,
            )
        };

        let parent_directory = dot_flox_path
            .parent()
            .ok_or(EnvironmentError::InvalidPath(dot_flox_path.to_path_buf()))?;
        let include_fetcher = IncludeFetcher {
            base_directory: Some(parent_directory.to_path_buf()),
        };
        Self::open_with(
            floxmeta,
            flox,
            pointer,
            dot_flox_path,
            rendered_env_links,
            include_fetcher,
        )
        .map_err(EnvironmentError::ManagedEnvironment)
    }

    /// Open a managed environment backed by a provided floxmeta clone.
    /// Ensure a branch for the environment exists in floxmeta and that there is
    /// a _unique_ branch to track its state.
    ///
    /// This method is primarily useful for testing.
    /// In most cases, you want to use [`ManagedEnvironment::open`] instead which provides the flox defaults.
    pub fn open_with(
        floxmeta: FloxMeta,
        flox: &Flox,
        pointer: ManagedPointer,
        dot_flox_path: CanonicalPath,
        rendered_env_links: RenderedEnvironmentLinks,
        include_fetcher: IncludeFetcher,
    ) -> Result<Self, ManagedEnvironmentError> {
        let lock = Self::ensure_generation_locked(&pointer, &dot_flox_path, &floxmeta)?;

        Self::ensure_branch(&branch_name(&pointer, &dot_flox_path), &lock, &floxmeta)?;

        ensure_registered(
            flox,
            &dot_flox_path,
            &EnvironmentPointer::Managed(pointer.clone()),
        )?;

        let env = ManagedEnvironment {
            path: dot_flox_path,
            rendered_env_links,
            pointer,
            floxmeta,
            include_fetcher,
        };

        Ok(env)
    }

    /// Ensure:
    /// - a lockfile exists, creating one if necessary
    /// - the commit in the lockfile (`local_rev` or `rev`) exists in floxmeta
    ///
    /// This may perform a fetch to update the sync branch if
    /// * the lockfile contains a `rev` that is not in the floxmeta clone on the local machine
    /// * no lockfile exists
    ///
    /// Committing a lockfile with a local revision
    /// will evoke an error on any but the original machine.
    ///
    /// Currently we can only recommend to not commit lockfiles with a local revision.
    /// This behavior may change in the future.
    fn ensure_generation_locked(
        pointer: &ManagedPointer,
        dot_flox_path: &CanonicalPath,
        floxmeta: &FloxMeta,
    ) -> Result<GenerationLock, ManagedEnvironmentError> {
        let lock_path = dot_flox_path.join(GENERATION_LOCK_FILENAME);
        let maybe_lock = GenerationLock::read_maybe(&lock_path)?;

        Ok(match maybe_lock {
            // Use local_rev if we have it
            Some(lock) if lock.local_rev.is_some() => {
                // Because a single floxmeta clone contains multiple
                // environments, `local_rev` might refer to a commit on a
                // branch for another environment. We protect against this
                // for `rev` below, but it doesn't seem worth doing so for
                // `local_rev`, because the environment directory may have
                // been moved. This means we can't require the commit is on
                // the {system}.{name}.{encode(project dir)} branch. We
                // could require the commit to be on some {system}.{name}.*
                // branch, but that doesn't seem worth the effort.
                if !floxmeta
                    .git
                    // we know local_rev is Some because of the above match
                    .contains_commit(lock.local_rev.as_ref().unwrap())
                    .map_err(ManagedEnvironmentError::CheckGitRevision)?
                {
                    Err(ManagedEnvironmentError::LocalRevDoesNotExist)?;
                }
                lock
            },
            // We have rev but not local_rev
            Some(lock) => {
                let span = tracing::info_span!(
                    "ensure_generation_locked::restore_locked",
                    rev = %lock.rev,
                    progress = "Fetching locked generation"
                );
                let _guard = span.enter();

                let remote_branch = remote_branch_name(pointer);
                // Check that the commit not only exists but is on the
                // correct branch - we don't want to allow grabbing commits
                // from other environments.
                if !floxmeta
                    .git
                    .branch_contains_commit(&lock.rev, &remote_branch)
                    .map_err(ManagedEnvironmentError::Git)?
                {
                    // Maybe the lock refers to a new generation that has
                    // been pushed, so fetch. We fetch the branch rather
                    // than the rev because we don't want to grab a commit
                    // from another environment.
                    floxmeta
                        .git
                        .fetch_ref("dynamicorigin", &format!("+{0}:{0}", remote_branch))
                        .map_err(|err| match err {
                            GitRemoteCommandError::Command(e @ GitCommandError::Command(_)) => {
                                ManagedEnvironmentError::Git(e)
                            },
                            _ => ManagedEnvironmentError::Fetch(err),
                        })?;
                }
                // If it still doesn't exist after fetching,
                // the upstream branch has diverged from the local branch.
                let in_remote = floxmeta
                    .git
                    .branch_contains_commit(&lock.rev, &remote_branch)
                    .map_err(ManagedEnvironmentError::Git)?;

                if in_remote {
                    return Ok(lock);
                }

                // locked reference not found in remote/sync branch
                // check if it's in the project's branch.
                // If the project's branch doesn't exist, or the project was moved,
                // this will still fail to resolve.

                let local_branch = branch_name(pointer, dot_flox_path);

                let has_branch = floxmeta
                    .git
                    .has_branch(&local_branch)
                    .map_err(ManagedEnvironmentError::CheckBranchExists)?;

                let in_local = has_branch
                    && floxmeta
                        .git
                        .branch_contains_commit(&lock.rev, &local_branch)
                        .map_err(ManagedEnvironmentError::Git)?;

                if !in_remote && !in_local {
                    Err(ManagedEnvironmentError::RevDoesNotExist)?;
                };

                lock
            },
            // There's no lockfile, so write a new one with whatever remote
            // branch is after fetching.
            None => {
                let span = tracing::info_span!(
                    "ensure_generation_locked::lock_latest",
                    progress = "Fetching latest generation"
                );
                let _guard = span.enter();
                let remote_branch = remote_branch_name(pointer);

                floxmeta
                    .git
                    .fetch_ref("dynamicorigin", &format!("+{0}:{0}", remote_branch))
                    .map_err(ManagedEnvironmentError::Fetch)?;

                // Fresh lockfile, so we don't want to set local_rev
                write_pointer_lockfile(lock_path, floxmeta, remote_branch, None)?
            },
        })
    }

    /// Ensure the branch exists and points at rev or local_rev
    fn ensure_branch(
        branch: &str,
        lock: &GenerationLock,
        floxmeta: &FloxMeta,
    ) -> Result<(), ManagedEnvironmentError> {
        let current_rev = lock.local_rev.as_ref().unwrap_or(&lock.rev);
        match floxmeta.git.branch_hash(branch) {
            Ok(ref branch_rev) => {
                if branch_rev != current_rev {
                    // Maybe the user pulled a new lockfile or there was a race with
                    // another `flox` process and the ManagedLock has now been
                    // updated.
                    // TODO need to clarify the meaning of the branch name and what
                    // guarantees it represents
                    // For now just point the branch at current_rev.
                    // We're not discarding work, just allowing it to possibly be
                    // garbage collected.
                    floxmeta
                        .git
                        .reset_branch(branch, current_rev)
                        .map_err(ManagedEnvironmentError::Git)?;
                }
            },
            // create branch if it doesn't exist
            Err(GitCommandBranchHashError::DoesNotExist) => {
                floxmeta
                    .git
                    .create_branch(branch, current_rev)
                    .map_err(ManagedEnvironmentError::Git)?;
            },
            Err(err) => Err(ManagedEnvironmentError::GitBranchHash(err))?,
        }
        Ok(())
    }

    pub fn env_ref(&self) -> EnvironmentRef {
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
            .writable(flox.temp_dir.clone())
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
            .add_generation(&mut temporary, "manually edited".to_string())
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
        let store_paths = local_checkout
            .build(flox)
            .map_err(ManagedEnvironmentError::Build)?;

        // TODO: should use self.link but that returns an EnvironmentError
        CoreEnvironment::link(&self.rendered_env_links.development, &store_paths.develop)
            .map_err(ManagedEnvironmentError::Link)?;
        CoreEnvironment::link(&self.rendered_env_links.runtime, &store_paths.runtime)
            .map_err(ManagedEnvironmentError::Link)?;

        let mut generations = self.generations();
        let mut generations = generations
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        generations
            .add_generation(
                &mut local_checkout,
                "Synchronized manual changes to generation".to_string(),
            )
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

        let local_checkout = CoreEnvironment::new(env_dir, self.include_fetcher.clone());

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
            CoreEnvironment::new(self.path.join(ENV_DIR_NAME), self.include_fetcher.clone());
        Ok(local)
    }

    /// Validate that the local manifest checkout matches the one in the current generation.
    ///
    /// Returns true if they match, false otherwise.
    /// Manifests are compared byte-for-byte, such semantically equivalent modifications
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

        let local_manifest_bytes = local
            .manifest_contents()
            .map_err(ManagedEnvironmentError::ReadLocalManifest)?;

        let remote_manifest_bytes = generations
            .current_gen_manifest()
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
        let mut generations = self.generations();
        let generations = generations
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        let local_checkout = self.local_env_or_copy_current_generation(flox)?;

        Ok(!Self::validate_checkout(&local_checkout, &generations)?)
    }

    /// Lock the environment to the current revision
    fn lock_pointer(&self) -> Result<(), ManagedEnvironmentError> {
        let lock_path = self.path.join(GENERATION_LOCK_FILENAME);

        write_pointer_lockfile(
            lock_path,
            &self.floxmeta,
            remote_branch_name(&self.pointer),
            branch_name(&self.pointer, &self.path).into(),
        )?;
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
        Generations::new(
            self.floxmeta.git.clone(),
            branch_name(&self.pointer, &self.path),
        )
    }

    fn get_current_generation(
        &self,
        flox: &Flox,
    ) -> Result<CoreEnvironment, ManagedEnvironmentError> {
        let tempdir = tempfile::tempdir_in(&flox.temp_dir).unwrap();

        self.generations()
            .writable(tempdir)
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?
            .get_current_generation(self.include_fetcher.clone())
            .map_err(ManagedEnvironmentError::CreateGenerationFiles)
    }
}

/// Write a pointer lockfile to the specified `lock_path`.
///
/// The lockfile stores the current git revision of the tracked upstream repository.
/// When a local revision is specified,
/// and the local revision is different from the remote revision,
/// the local revision is also stored in the lockfile.
///
/// When committed to a project,
/// guarantees that the same version of the linked environment
/// is used by all instances across different machines.
/// When a local revision is specified,
/// flox will **try to** use the local revision
/// rather than the remote revision **failing if it can't**.
///
/// Committing a lockfile with a local revision will thus cause flox to fail
/// if the local revision is not available on the machine,
/// i.e. any machine other than the one that committed the lockfile.
/// See [`ManagedEnvironment::ensure_locked`] for more details.
///
/// todo: allow updating only the local revision
/// avoid race conditions where the remote revision is unintentionally updated.
/// If I pull an environment at rev A,
/// -> somebody pushes rev B,
/// -> I do an operation with -r that fetches the environment,
/// -> and then I make a change that takes me from rev A to rev C,
/// my lock will set rev to B.
/// That's undesirable, and rev should always be in local_rev's history.
fn write_pointer_lockfile(
    lock_path: PathBuf,
    floxmeta: &FloxMeta,
    remote_ref: String,
    local_ref: Option<String>,
) -> Result<GenerationLock, ManagedEnvironmentError> {
    let rev = floxmeta
        .git
        .branch_hash(&remote_ref)
        .map_err(ManagedEnvironmentError::GitBranchHash)?;

    let local_rev = if let Some(ref local_ref) = local_ref {
        match floxmeta.git.branch_hash(local_ref) {
            Ok(local_rev) if local_rev == rev => None,
            Ok(local_rev) => Some(local_rev),
            Err(err) => Err(ManagedEnvironmentError::GitBranchHash(err))?,
        }
    } else {
        None
    };

    if let Some(ref local_rev) = local_rev {
        debug!(
            "writing pointer lockfile: remote_rev='{rev}', local_rev='{local_rev}', lockfile={lock_path:?}"
        );
    } else {
        debug!(
            "writing pointer lockfile: remote_rev='{rev}', local_rev=<unset>, ,lockfile={lock_path:?}"
        );
    }

    let lock = GenerationLock {
        rev,
        local_rev,
        version: Version::<1> {},
    };

    {
        let existing_lock = GenerationLock::read_maybe(&lock_path);

        if matches!(existing_lock, Ok(Some(ref existing_lock)) if existing_lock == &lock) {
            debug!("skip writing unchanged generation lock");
            return Ok(lock);
        }
    }

    let lock_contents =
        serde_json::to_string_pretty(&lock).map_err(ManagedEnvironmentError::SerializeLock)?;

    fs::write(lock_path, lock_contents).map_err(ManagedEnvironmentError::WriteLock)?;
    Ok(lock)
}

/// Unique branch name for a specific link.
///
/// Use this function over [`remote_branch_name`] within the context of an instance of [ManagedEnvironment]
///
/// When pulling the same remote environment in multiple directories,
/// unique copies of the environment are created.
/// I.e. `install`ing a package in one directory does not affect the other
/// until synchronized through FloxHub.
///
/// `dot_flox_path` is expected to point to the `.flox/` directory
/// that link to an environment identified by `pointer`.
pub fn branch_name(pointer: &ManagedPointer, dot_flox_path: &CanonicalPath) -> String {
    format!(
        "{}{}{}",
        pointer.name,
        BRANCH_NAME_PATH_SEPARATOR,
        path_hash(dot_flox_path)
    )
}

/// The original branch name of an environment that is used to sync an environment with the hub
///
/// In most cases [`branch_name`] should be used over this,
/// within the context of an instance of [ManagedEnvironment].
///
/// [`remote_branch_name`] is primarily used when talking to upstream on FloxHub,
/// during opening to reconciliate with the upstream repo
/// as well as during [`ManagedEnvironment::pull`].
pub fn remote_branch_name(pointer: &ManagedPointer) -> String {
    format!("{}", pointer.name)
}

#[derive(Clone, Debug, PartialEq)]
pub enum PullResult {
    /// The environment was already up to date
    UpToDate,
    /// The environment was reset to the latest upstream version
    Updated,
}

impl ManagedEnvironment {
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
    #[instrument(skip(flox), fields(progress = "Pushing new environment to FloxHub"))]
    pub fn push_new(
        flox: &Flox,
        path_environment: PathEnvironment,
        owner: EnvironmentOwner,
        force: bool,
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

        Self::push_new_without_building(flox, owner, name, force, dot_flox_path, core_environment)
    }

    /// Push an environment and open the resulting [ManagedEnvironment],
    /// but don't build the environment first.
    ///
    /// This is split out for the purposes of testing -
    /// some tests need an environment that has build errors.
    fn push_new_without_building(
        flox: &Flox,
        owner: EnvironmentOwner,
        name: EnvironmentName,
        force: bool,
        dot_flox_path: CanonicalPath,
        mut core_environment: CoreEnvironment,
    ) -> Result<Self, EnvironmentError> {
        let pointer = ManagedPointer::new(owner, name.clone(), &flox.floxhub);

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
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        // Add this environment as a new generation, which involves pushing to
        // the fake remote.
        generations
            .add_generation(&mut core_environment, "Add first generation".to_string())
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
            Err(GitRemoteCommandError::Diverged) => Err(ManagedEnvironmentError::Diverged)?,
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

        write_pointer_lockfile(
            dot_flox_path.join(GENERATION_LOCK_FILENAME),
            &FloxMeta {
                git: temp_floxmeta_git,
            },
            remote_branch_name(&pointer),
            None,
        )?;

        let env = ManagedEnvironment::open(flox, pointer, dot_flox_path)?;

        Ok(env)
    }

    #[instrument(skip(self, flox), fields(progress = "Pushing updates to FloxHub"))]
    pub fn push(&mut self, flox: &Flox, force: bool) -> Result<(), ManagedEnvironmentError> {
        let project_branch = branch_name(&self.pointer, &self.path);
        let sync_branch = remote_branch_name(&self.pointer);

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
        {
            let generations = self.generations();

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

        // Fetch the remote branch into sync branch
        match self
            .floxmeta
            .git
            .fetch_ref("dynamicorigin", &format!("+{sync_branch}:{sync_branch}",))
        {
            Ok(_) => {},
            Err(GitRemoteCommandError::RefNotFound(_)) => {
                debug!("Upstream environment was deleted.")
            },
            Err(e) => Err(ManagedEnvironmentError::FetchUpdates(e))?,
        };

        // Check whether we can fast-forward merge the remote branch into the local branch
        // If "not" the environment has diverged.
        // if `--force` flag is set we skip this check
        if !force {
            let consistent_history = self
                .floxmeta
                .git
                .branch_contains_commit(&sync_branch, &project_branch)
                .map_err(ManagedEnvironmentError::Git)?;

            if !consistent_history {
                Err(ManagedEnvironmentError::Diverged)?;
            }
        }
        self.floxmeta
            .git
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

        Ok(())
    }

    /// Pull new generation data from floxhub
    ///
    /// Requires the local checkout to be synched with the current generation.
    /// If the environment has diverged, the pull will fail.
    ///
    /// If `force == true`, the pull will proceed even if the environment has diverged.
    #[instrument(skip(self, flox), fields(progress = "Pulling updates from FloxHub"))]
    pub fn pull(
        &mut self,
        flox: &Flox,
        force: bool,
    ) -> Result<PullResult, ManagedEnvironmentError> {
        // Check whether the local checkout is in sync with the current generation
        // before potentially updating generations and resetting the local checkout.
        let generations = self.generations();
        let local_checkout = self.local_env_or_copy_current_generation(flox)?;
        let checkout_valid = Self::validate_checkout(&local_checkout, &generations)?;

        // With `force` we pull even if the local checkout is out of sync.
        if !force && !checkout_valid {
            Err(ManagedEnvironmentError::CheckoutOutOfSync)?
        }

        let sync_branch = remote_branch_name(&self.pointer);
        let project_branch = branch_name(&self.pointer, &self.path);

        // Fetch the remote branch into the local sync branch.
        // The sync branch is always a reset to the remote branch
        // and it's state should not be depended on.
        match self
            .floxmeta
            .git
            .fetch_ref("dynamicorigin", &format!("+{sync_branch}:{sync_branch}"))
        {
            Ok(_) => {},
            Err(GitRemoteCommandError::RefNotFound(_)) => {
                Err(ManagedEnvironmentError::UpstreamNotFound {
                    env_ref: self.env_ref(),
                    upstream: self.pointer.floxhub_base_url.to_string(),
                    user: flox.floxhub_token.as_ref().map(|t| t.handle().to_string()),
                })?
            },
            Err(e) => Err(ManagedEnvironmentError::FetchUpdates(e))?,
        };

        // Check whether we can fast-forward the remote branch to the local branch,
        // if not the environment has diverged.
        let consistent_history = self
            .floxmeta
            .git
            .branch_contains_commit(&project_branch, &sync_branch)
            .map_err(ManagedEnvironmentError::Git)?;
        if !consistent_history && !force {
            Err(ManagedEnvironmentError::Diverged)?;
        }

        let sync_branch_commit = self.floxmeta.git.branch_hash(&sync_branch).ok();
        let project_branch_commit = self.floxmeta.git.branch_hash(&project_branch).ok();

        // Regardless of whether `--force` is set, we want to accurately return UpToDate
        // If the checkout is not the same as the current generation, we should
        // instead reset_local_env_to_current_generation below
        if checkout_valid && sync_branch_commit == project_branch_commit {
            return Ok(PullResult::UpToDate);
        }

        // update the project branch to the remote branch, using `force` if specified
        self.floxmeta
            .git
            .push_ref(
                ".",
                format!("refs/heads/{sync_branch}:refs/heads/{project_branch}",),
                force, // Set the force parameter to false or true based on your requirement
            )
            .map_err(ManagedEnvironmentError::ApplyUpdates)?;

        // update the pointer lockfile
        self.lock_pointer()?;
        self.reset_local_env_to_current_generation(flox)?;

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
        self.floxmeta
            .prune_branches(&self.pointer, &self.path)
            .unwrap();

        fs::remove_file(self.path.join(GENERATION_LOCK_FILENAME))
            .map_err(ManagedEnvironmentError::WriteLock)?;

        // forget that this environment exists
        deregister(
            flox,
            &self.path,
            &EnvironmentPointer::Managed(self.pointer.clone()),
        )?;

        // create the metadata for a path environment
        let path_pointer = PathPointer::new(self.name());
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

/// Ensure that the environment does not include local includes before pushing it to FloxHub
fn check_for_local_includes(lockfile: &Lockfile) -> Result<(), ManagedEnvironmentError> {
    let manifest = lockfile.user_manifest();
    let has_local_include = manifest
        .include
        .environments
        .iter()
        .any(|include| matches!(include, IncludeDescriptor::Local { .. }));

    if has_local_include {
        Err(ManagedEnvironmentError::PushWithLocalIncludes)?;
    }

    Ok(())
}

pub mod test_helpers {

    use tempfile::tempdir_in;

    use super::*;
    use crate::flox::{DEFAULT_FLOXHUB_URL, Floxhub};
    use crate::models::environment::fetcher::test_helpers::mock_include_fetcher;
    use crate::models::environment::path_environment::test_helpers::{
        new_named_path_environment_from_env_files,
        new_named_path_environment_in,
    };
    use crate::models::environment::test_helpers::new_core_environment;
    use crate::models::floxmeta::test_helpers::unusable_mock_floxmeta;

    /// Get a [ManagedEnvironment] that is invalid but can be used in tests
    /// where methods on [ManagedEnvironment] will never be called.
    ///
    /// For a [ManagedEnvironment] with methods that can be called use
    /// [mock_managed_environment].
    pub fn unusable_mock_managed_environment() -> ManagedEnvironment {
        let floxhub = Floxhub::new(DEFAULT_FLOXHUB_URL.clone(), None).unwrap();
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
            floxmeta: unusable_mock_floxmeta(),
            include_fetcher: mock_include_fetcher(),
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

        ManagedEnvironment::push_new(flox, path_environment, owner, false).unwrap()
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

        ManagedEnvironment::push_new(flox, path_environment, owner, false).unwrap()
    }
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;
    use std::str::FromStr;

    use indoc::indoc;
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
    use crate::models::environment::test_helpers::{
        new_core_environment,
        new_core_environment_with_lockfile,
    };
    use crate::models::environment::{DOT_FLOX, MANIFEST_FILENAME};
    use crate::models::floxmeta::floxmeta_dir;
    use crate::models::lockfile::Lockfile;
    use crate::models::lockfile::test_helpers::fake_catalog_package_lock;
    use crate::models::manifest::typed::{Inner, Manifest, PackageDescriptorCatalog, Vars};
    use crate::providers::catalog::test_helpers::catalog_replay_client;
    use crate::providers::catalog::{GENERATED_DATA, MockClient};
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

    /// Create a .flox directory at dot_flox_path with a pointer
    /// and optional generation lock.
    ///
    /// Mimics the state of a managed environment
    /// without an existing view of the current generation.
    fn create_dot_flox(
        dot_flox_path: &Path,
        pointer: &ManagedPointer,
        lock: Option<&GenerationLock>,
    ) -> CanonicalPath {
        if !dot_flox_path.exists() {
            fs::create_dir(dot_flox_path).unwrap();
        }

        let pointer_path = dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME);
        fs::write(
            pointer_path,
            serde_json::to_string_pretty(&pointer).unwrap(),
        )
        .unwrap();

        if let Some(lock) = lock {
            let lock_path = dot_flox_path.join(GENERATION_LOCK_FILENAME);
            fs::write(lock_path, serde_json::to_string_pretty(lock).unwrap()).unwrap();
        }

        CanonicalPath::new(dot_flox_path).unwrap()
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

    /// Clone a git repo specified by remote_path into the floxmeta dir
    /// corresponding to test_pointer,
    /// and open that as a FloxmetaV2
    ///
    /// TODO: creating the remote repo should probably be pulled into this
    /// function
    fn create_floxmeta(
        flox: &Flox,
        remote_path: &Path,
        test_pointer: &ManagedPointer,
        branch: &str,
    ) -> FloxMeta {
        let user_floxmeta_dir = floxmeta_dir(flox, &test_pointer.owner);
        fs::create_dir_all(&user_floxmeta_dir).unwrap();
        GitCommandProvider::clone_branch(
            format!("file://{}", remote_path.to_string_lossy()),
            user_floxmeta_dir,
            branch,
            true,
        )
        .unwrap();

        FloxMeta::open(flox, test_pointer).unwrap()
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

        let mut writable = generations.writable(base_tempdir.as_ref()).unwrap();
        writable
            .add_generation(env, "initial generation".to_string())
            .unwrap();

        generations
    }

    /// Test that when ensure_locked has input state of:
    /// - no lock
    /// - floxmeta at commit 1
    /// - remote at commit 2
    ///
    /// It results in output state of:
    /// - lock at commit 2
    /// - floxmeta at commit 2
    #[test]
    fn test_ensure_locked_case_1() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &branch);

        // add a second commit to the remote
        commit_file(&remote, "file 2");
        let hash_2 = remote.branch_hash(&branch).unwrap();

        // create a .flox directory
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        let dot_flox_path = create_dot_flox(&dot_flox_path, &test_pointer, None);

        ManagedEnvironment::ensure_generation_locked(&test_pointer, &dot_flox_path, &floxmeta)
            .unwrap();

        let lock_path = dot_flox_path.join(GENERATION_LOCK_FILENAME);
        let lock: GenerationLock = serde_json::from_slice(&fs::read(lock_path).unwrap()).unwrap();
        assert_eq!(lock, GenerationLock {
            rev: hash_2.clone(),
            local_rev: None,
            version: Version::<1> {},
        });

        assert_eq!(floxmeta.git.branch_hash(&branch).unwrap(), hash_2);
    }

    /// Test that when ensure_locked has input state of:
    /// - lock at commit 1
    /// - floxmeta at commit 1
    /// - remote at commit 1
    ///
    /// It results in output state of:
    /// - lock at commit 1
    /// - floxmeta at commit 1
    #[test]
    fn test_ensure_locked_case_2() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&branch).unwrap();

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &branch);

        // create a .flox directory
        let lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: None,
            version: Version::<1> {},
        };
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        let dot_flox_path = create_dot_flox(&dot_flox_path, &test_pointer, Some(&lock));

        ManagedEnvironment::ensure_generation_locked(&test_pointer, &dot_flox_path, &floxmeta)
            .unwrap();

        let lock_path = dot_flox_path.join(GENERATION_LOCK_FILENAME);
        let lock: GenerationLock = serde_json::from_slice(&fs::read(lock_path).unwrap()).unwrap();
        assert_eq!(lock, GenerationLock {
            rev: hash_1.clone(),
            local_rev: None,
            version: Version::<1> {},
        });

        assert_eq!(floxmeta.git.branch_hash(&branch).unwrap(), hash_1);
    }

    /// Test that when ensure_locked has input state of:
    /// - lock at commit 2
    /// - floxmeta at commit 1
    /// - remote at commit 3
    ///
    /// It results in output state of:
    /// - lock at commit 2
    /// - floxmeta at commit 3
    #[test]
    fn test_ensure_locked_case_3() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &branch);

        // add a second commit to the remote
        commit_file(&remote, "file 2");
        let hash_2 = remote.branch_hash(&branch).unwrap();

        // add a third commit to the remote
        commit_file(&remote, "file 3");
        let hash_3 = remote.branch_hash(&branch).unwrap();

        // create a .flox directory
        let lock = GenerationLock {
            rev: hash_2.clone(),
            local_rev: None,
            version: Version::<1> {},
        };
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        let dot_flox_path = create_dot_flox(
            &dot_flox_path,
            &make_test_pointer(&remote_path),
            Some(&lock),
        );

        ManagedEnvironment::ensure_generation_locked(
            &make_test_pointer(&remote_path),
            &dot_flox_path,
            &floxmeta,
        )
        .unwrap();

        let lock_path = dot_flox_path.join(GENERATION_LOCK_FILENAME);
        let lock: GenerationLock = serde_json::from_slice(&fs::read(lock_path).unwrap()).unwrap();
        assert_eq!(lock, GenerationLock {
            rev: hash_2,
            local_rev: None,
            version: Version::<1> {},
        });

        assert_eq!(floxmeta.git.branch_hash(&branch).unwrap(), hash_3);
    }

    /// Test that when ensure_locked has input state of:
    /// - lock at branch_2
    /// - floxmeta at branch_1
    /// - remote at branch_1
    /// - branch_2 present on remote
    ///
    /// It results in output state of:
    /// - error
    #[test]
    fn test_ensure_locked_case_4() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &branch);

        // add a second branch to the remote
        remote.checkout("branch_2", true).unwrap();
        commit_file(&remote, "file 2");
        let hash_2 = remote.branch_hash("branch_2").unwrap();

        // create a .flox directory
        let lock = GenerationLock {
            rev: hash_2,
            local_rev: None,
            version: Version::<1> {},
        };
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        let dot_flox_path = create_dot_flox(&dot_flox_path, &test_pointer, Some(&lock));

        assert!(matches!(
            ManagedEnvironment::ensure_generation_locked(&test_pointer, &dot_flox_path, &floxmeta),
            Err(ManagedEnvironmentError::RevDoesNotExist)
        ));
    }

    /// Test that when ensure_locked has input state of:
    /// - lock at nonexistent commit
    /// - floxmeta at commit 1
    /// - remote at commit 2
    ///
    /// It results in output state of:
    /// - error
    #[test]
    fn test_ensure_locked_case_5() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &branch);

        // add a second commit to the remote
        commit_file(&remote, "file 2");
        let hash_2 = remote.branch_hash(&branch).unwrap();

        // create a .flox directory
        let lock = GenerationLock {
            rev: "does not exist".to_string(),
            local_rev: None,
            version: Version::<1> {},
        };
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        let dot_flox_path = create_dot_flox(&dot_flox_path, &test_pointer, Some(&lock));

        assert!(matches!(
            ManagedEnvironment::ensure_generation_locked(&test_pointer, &dot_flox_path, &floxmeta),
            Err(ManagedEnvironmentError::RevDoesNotExist)
        ));

        assert_eq!(floxmeta.git.branch_hash(&branch).unwrap(), hash_2);
    }

    /// Test that when ensure_locked has input state of:
    /// - lock at {rev: commit 1, local_rev: commit 1}
    /// - floxmeta at commit 1
    /// - remote at commit 1
    ///
    /// It results in output state of:
    /// - no change
    #[test]
    fn test_ensure_locked_case_6() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&branch).unwrap();

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &branch);

        // create a .flox directory
        let lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: Some(hash_1.clone()),
            version: Version::<1> {},
        };
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        let dot_flox_path = create_dot_flox(&dot_flox_path, &test_pointer, Some(&lock));

        ManagedEnvironment::ensure_generation_locked(&test_pointer, &dot_flox_path, &floxmeta)
            .unwrap();

        let lock_path = dot_flox_path.join(GENERATION_LOCK_FILENAME);

        assert_eq!(
            lock,
            serde_json::from_slice(&fs::read(lock_path).unwrap()).unwrap()
        );

        assert_eq!(floxmeta.git.branch_hash(&branch).unwrap(), hash_1);
    }

    /// Test that when ensure_locked has input state of:
    /// - lock at {rev: commit 1, local_rev: does not exist}
    /// - floxmeta at commit 1
    /// - remote at commit 1
    ///
    /// It results in output state of:
    /// - error
    #[test]
    fn test_ensure_locked_case_7() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&branch).unwrap();

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &branch);

        // create a .flox directory
        let lock = GenerationLock {
            rev: hash_1,
            local_rev: Some("does not exist".to_string()),
            version: Version::<1> {},
        };
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        let dot_flox_path = create_dot_flox(&dot_flox_path, &test_pointer, Some(&lock));

        assert!(matches!(
            ManagedEnvironment::ensure_generation_locked(&test_pointer, &dot_flox_path, &floxmeta),
            Err(ManagedEnvironmentError::LocalRevDoesNotExist)
        ));
    }

    /// Test that when ensure_locked has input state of:
    /// - lock at { rev: commit A1, local_rev: commit A1 }
    /// - floxmeta
    ///   (project) at commit A1
    ///   (sync) at commit B1
    /// - remote at commit B1
    ///
    /// It results in output state of:
    /// - lock at { rev: commit A1, local_rev: commit A1 }
    #[test]
    fn test_ensure_locked_case_9() {
        let (flox, _temp_dir_handle) = flox_instance();

        fs::create_dir(flox.temp_dir.join(DOT_FLOX)).unwrap();
        let dot_flox_path = CanonicalPath::new(flox.temp_dir.join(DOT_FLOX)).unwrap();

        // create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let diverged_remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&diverged_remote_branch, true).unwrap();
        commit_file(&remote, "file 1");

        let locked_branch = branch_name(&test_pointer, &dot_flox_path);
        remote.checkout(&locked_branch, true).unwrap();
        commit_file(&remote, "file 2");
        let hash_1 = remote.branch_hash(&locked_branch).unwrap();

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &diverged_remote_branch);
        floxmeta.git.fetch_branch("origin", &locked_branch).unwrap();

        // create a .flox directory
        let lock = GenerationLock {
            rev: hash_1,
            local_rev: None,
            version: Version::<1> {},
        };
        let dot_flox_path = create_dot_flox(&dot_flox_path, &test_pointer, Some(&lock));

        assert_eq!(
            ManagedEnvironment::ensure_generation_locked(&test_pointer, &dot_flox_path, &floxmeta)
                .unwrap(),
            lock
        );
    }

    /// Test that ensure_branch is a no-op with input state:
    /// - branch at commit 1
    /// - rev at commit 1
    /// - local_rev at commit 1
    #[test]
    fn test_ensure_branch_noop() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&branch).unwrap();

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &branch);

        let lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: Some(hash_1.clone()),
            version: Version::<1> {},
        };
        ManagedEnvironment::ensure_branch(&branch, &lock, &floxmeta).unwrap();
        assert_eq!(floxmeta.git.branch_hash(&branch).unwrap(), hash_1);
    }

    /// Test that with input state:
    /// - branch at commit 1
    /// - rev at commit 1
    /// - local_rev at commit 2
    ///
    /// ensure_branch resets the branch to commit 2
    #[test]
    fn test_ensure_branch_resets_branch() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&branch).unwrap();

        // add a second branch to the remote
        remote.checkout("branch_2", true).unwrap();
        commit_file(&remote, "file 2");
        let hash_2 = remote.branch_hash("branch_2").unwrap();

        // Create a mock floxmeta (note clone is used instead of clone_branch,
        // which is used in create_floxmeta, because we need both branches)
        let user_floxmeta_dir = floxmeta_dir(&flox, &test_pointer.owner);
        fs::create_dir_all(&user_floxmeta_dir).unwrap();
        <GitCommandProvider as GitProvider>::clone(
            format!("file://{}", remote_path.to_string_lossy()),
            user_floxmeta_dir,
            true,
        )
        .unwrap();

        let floxmeta = FloxMeta::open(&flox, &test_pointer).unwrap();

        let lock = GenerationLock {
            rev: hash_1,
            local_rev: Some(hash_2.clone()),
            version: Version::<1> {},
        };
        ManagedEnvironment::ensure_branch(&branch, &lock, &floxmeta).unwrap();
        assert_eq!(floxmeta.git.branch_hash(&branch).unwrap(), hash_2);
    }

    /// Test that with input state:
    /// - branch_2 does not exist
    /// - rev at commit 1
    /// - local_rev at commit 1
    ///
    /// ensure_branch creates branch_2 at commit 1
    #[test]
    fn test_ensure_branch_creates_branch() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&branch).unwrap();

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &branch);

        let lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: Some(hash_1.clone()),
            version: Version::<1> {},
        };
        ManagedEnvironment::ensure_branch("branch_2", &lock, &floxmeta).unwrap();
        assert_eq!(floxmeta.git.branch_hash("branch_2").unwrap(), hash_1);
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
            .manifest_contents()
            .unwrap();
        assert_eq!(local_manifest, locally_edited_content);
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
        let local_checkout = managed_env
            .local_env_or_copy_current_generation(&flox)
            .unwrap();
        let generation_manifest = managed_env
            .get_current_generation(&flox)
            .unwrap()
            .manifest_contents()
            .unwrap();

        assert_eq!(
            local_checkout.manifest_contents().unwrap(),
            generation_manifest
        );

        fs::write(local_checkout.manifest_path(), indoc! {"
            version = 1

            # nothing else but certinainly different from before
        "})
        .unwrap();

        // sanity check that before syncing, the manifest is now different
        assert_ne!(
            local_checkout.manifest_contents().unwrap(),
            generation_manifest
        );

        // check that after syncing, the manifest is the same
        managed_env.create_generation_from_local_env(&flox).unwrap();

        let generation_manifest = managed_env
            .get_current_generation(&flox)
            .unwrap()
            .manifest_contents()
            .unwrap();

        assert_eq!(
            local_checkout.manifest_contents().unwrap(),
            generation_manifest
        );
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

        let mut new_manifest = Manifest::default();
        new_manifest.install.inner_mut().insert(
            "hello".to_string(),
            PackageDescriptorCatalog {
                pkg_path: "hello".to_string(),
                pkg_group: None,
                priority: None,
                version: None,
                systems: None,
            }
            .into(),
        );

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

        let mut manifest_a = Manifest::default();
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

        // dummy paths since we are not rendering the environment
        let rendered_env_links =
            RenderedEnvironmentLinks::new_unchecked(PathBuf::new(), PathBuf::new());

        // create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &branch);

        let _env = ManagedEnvironment::open_with(
            floxmeta,
            &flox,
            test_pointer,
            CanonicalPath::new(&dot_flox_path).unwrap(),
            rendered_env_links,
            IncludeFetcher {
                base_directory: Some(dot_flox_path.parent().unwrap().to_path_buf()),
            },
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

        // dummy paths since we are not rendering the environment
        let rendered_env_links =
            RenderedEnvironmentLinks::new_unchecked(PathBuf::new(), PathBuf::new());

        // create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &branch);

        // Create the registry
        let env = ManagedEnvironment::open_with(
            floxmeta,
            &flox,
            test_pointer,
            CanonicalPath::new(&dot_flox_path).unwrap(),
            rendered_env_links,
            IncludeFetcher {
                base_directory: Some(dot_flox_path.parent().unwrap().to_path_buf()),
            },
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
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));

        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");

        let env1_branch = branch_name(&test_pointer, &env1_dir);
        let env2_branch = branch_name(&test_pointer, &env2_dir);

        // Create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);

        // Create two environments that use the same pointer.
        let env1 = ManagedEnvironment::open_with(
            floxmeta.clone(),
            &flox,
            test_pointer.clone(),
            env1_dir.clone(),
            rendered_env_links.clone(),
            IncludeFetcher {
                base_directory: Some(env1_dir.parent().unwrap().to_path_buf()),
            },
        )
        .unwrap();
        let env2 = ManagedEnvironment::open_with(
            floxmeta.clone(),
            &flox,
            test_pointer.clone(),
            env2_dir.clone(),
            rendered_env_links.clone(),
            IncludeFetcher {
                base_directory: Some(env2_dir.parent().unwrap().to_path_buf()),
            },
        )
        .unwrap();

        // All branches should exist.
        assert!(floxmeta.git.has_branch(&remote_branch).unwrap());
        assert!(floxmeta.git.has_branch(&env1_branch).unwrap());
        assert!(floxmeta.git.has_branch(&env2_branch).unwrap());

        // env2 is pruned when no longer on disk.
        fs::remove_dir_all(&env2.path).unwrap();
        garbage_collect(&flox).unwrap();
        assert!(floxmeta.git.has_branch(&remote_branch).unwrap());
        assert!(floxmeta.git.has_branch(&env1_branch).unwrap());
        assert!(!floxmeta.git.has_branch(&env2_branch).unwrap());

        // env1 is pruned when no longer on disk, remote is pruned when there
        // are no local branches, and is resilient to the branch not existing,
        // e.g. if the floxmeta repo has been manually deleted or the hashing
        // algorithm has changed in the past.
        fs::remove_dir_all(&env1.path).unwrap();
        floxmeta.git.delete_branch(&env1_branch, true).unwrap();
        garbage_collect(&flox).unwrap();
        assert!(!floxmeta.git.has_branch(&remote_branch).unwrap());
        assert!(!floxmeta.git.has_branch(&env1_branch).unwrap());
        assert!(!floxmeta.git.has_branch(&env2_branch).unwrap());
    }

    #[test]
    fn convert_to_path_environment() {
        let owner = "owner".parse().unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        let environment = mock_managed_environment_from_env_files(
            &flox,
            GENERATED_DATA.join("custom/hello"),
            owner,
        );

        // Assert that the environment looks like a managed environment
        // - it has a ManagedPointer
        // - it has a generation lock
        // - it has a branch in the git repo
        let pointer: ManagedPointer = serde_json::from_str(
            &fs::read_to_string(environment.path.join(ENVIRONMENT_POINTER_FILENAME)).unwrap(),
        )
        .expect("env pointer should be a managed pointer");
        assert!(
            environment.path.join(GENERATION_LOCK_FILENAME).exists(),
            "generation lock should exist"
        );
        assert!(
            environment
                .floxmeta
                .git
                .has_branch(&branch_name(&pointer, &environment.path))
                .unwrap()
        );

        // Unsafe to create a copy of the git provider
        // due to risk of corrupting the state of the git repo.
        // Since the original will be dropped however,
        // its safe to do so in this instance.
        let git = environment.floxmeta.git.clone();
        let path_before = environment.path.clone();
        let out_links_before = environment.rendered_env_links.clone();

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
            !git.has_branch(&branch_name(&pointer, &path_env.path))
                .unwrap(),
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

        assert_eq!(lockfile.manifest, Manifest {
            version: Version,
            vars: Vars(BTreeMap::from([("foo".to_string(), "dep".to_string()),])),
            ..Default::default()
        });

        assert_eq!(
            lockfile.compose.unwrap().include[0].manifest,
            toml_edit::de::from_str(dep_manifest_contents).unwrap()
        );
    }
}
