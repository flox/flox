use std::path::{Path, PathBuf};
use std::{fs, io};

use log::debug;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::core_environment::CoreEnvironment;
use super::generations::{Generations, GenerationsError};
use super::path_environment::PathEnvironment;
use super::{
    gcroots_dir,
    path_hash,
    CanonicalizeError,
    CoreEnvironmentError,
    EditResult,
    Environment,
    EnvironmentError,
    EnvironmentPointer,
    InstallationAttempt,
    ManagedPointer,
    UninstallationAttempt,
    UpdateResult,
    CACHE_DIR_NAME,
    ENVIRONMENT_POINTER_FILENAME,
    N_HASH_CHARS,
};
use crate::data::{CanonicalPath, Version};
use crate::flox::{EnvironmentRef, Flox};
use crate::models::container_builder::ContainerBuilder;
use crate::models::env_registry::{
    deregister,
    ensure_registered,
    env_registry_path,
    read_environment_registry,
    EnvRegistry,
    EnvRegistryError,
};
use crate::models::environment_ref::{EnvironmentName, EnvironmentOwner};
use crate::models::floxmeta::{floxmeta_git_options, FloxMeta, FloxMetaError};
use crate::models::lockfile::LockedManifest;
use crate::models::manifest::PackageToInstall;
use crate::models::pkgdb::UpgradeResult;
use crate::providers::git::{
    GitCommandBranchHashError,
    GitCommandError,
    GitProvider,
    GitRemoteCommandError,
};
use crate::utils::mtime_of;

pub const GENERATION_LOCK_FILENAME: &str = "env.lock";

#[derive(Debug)]
pub struct ManagedEnvironment {
    /// Absolute path to the directory containing `env.json`
    // TODO might be better to keep this private
    pub path: CanonicalPath,
    out_link: PathBuf,
    pointer: ManagedPointer,
    floxmeta: FloxMeta,
}

#[derive(Debug, Error)]
pub enum ManagedEnvironmentError {
    #[error("failed to open floxmeta git repo: {0}")]
    OpenFloxmeta(FloxMetaError),
    #[error("failed to fetch environment: {0}")]
    Fetch(GitRemoteCommandError),
    #[error("failed to check for git revision: {0}")]
    CheckGitRevision(GitCommandError),
    #[error("failed to check for branch existence")]
    CheckBranchExists(#[source] GitCommandBranchHashError),
    #[error("can't find local_rev specified in lockfile; local_rev could have been mistakenly committed on another machine")]
    LocalRevDoesNotExist,
    #[error("can't find environment at revision specified in lockfile; this could have been caused by force pushing")]
    RevDoesNotExist,
    #[error("invalid {} file: {0}", GENERATION_LOCK_FILENAME)]
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

    #[error("floxmeta branch name was malformed: {0}")]
    BadBranchName(String),
    #[error("project wasn't found at path {path}: {err}")]
    ProjectNotFound { path: PathBuf, err: std::io::Error },
    #[error("upstream floxmeta branch diverged from local branch")]
    Diverged,
    #[error("access to floxmeta repository was denied")]
    AccessDenied,
    #[error("environment '{0}' does not exist at upstream '{1}'")]
    UpstreamNotFound(EnvironmentRef, String),
    #[error("failed to push environment")]
    Push(#[source] GitRemoteCommandError),
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

    #[error("could not lock environment")]
    Lock(#[source] CoreEnvironmentError),

    #[error("could not build environment")]
    Build(#[source] CoreEnvironmentError),

    #[error("could not read manifest")]
    ReadManifest(#[source] GenerationsError),

    #[error("could not canonicalize environment path")]
    CanonicalizePath(#[source] CanonicalizeError),

    #[error("invalid FloxHub base url")]
    InvalidFloxhubBaseUrl(#[source] url::ParseError),

    #[error("failed to locate project in environment registry")]
    Registry(#[from] EnvRegistryError),
}

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct GenerationLock {
    rev: String,
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
    fn build(&mut self, flox: &Flox) -> Result<(), EnvironmentError> {
        let generations = self
            .generations()
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;
        let mut temporary = generations
            .get_current_generation()
            .map_err(ManagedEnvironmentError::CreateGenerationFiles)?;

        temporary.lock(flox)?;
        let store_path = temporary.build(flox)?;
        temporary.link(flox, &self.out_link, &Some(store_path))?;

        Ok(())
    }

    fn lock(&mut self, flox: &Flox) -> Result<LockedManifest, EnvironmentError> {
        let generations = self
            .generations()
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;
        let mut temporary = generations
            .get_current_generation()
            .map_err(ManagedEnvironmentError::CreateGenerationFiles)?;

        Ok(temporary.lock(flox)?)
    }

    fn build_container(&mut self, flox: &Flox) -> Result<ContainerBuilder, EnvironmentError> {
        let generations = self
            .generations()
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;
        let mut temporary = generations
            .get_current_generation()
            .map_err(ManagedEnvironmentError::CreateGenerationFiles)?;

        let builder = temporary.build_container(flox)?;
        Ok(builder)
    }

    /// Install packages to the environment atomically
    fn install(
        &mut self,
        packages: &[PackageToInstall],
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError> {
        let mut generations = self
            .generations()
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;
        let mut temporary = generations
            .get_current_generation()
            .map_err(ManagedEnvironmentError::CreateGenerationFiles)?;

        let metadata = format!("installed packages: {:?}", &packages);
        let result = temporary.install(packages, flox)?;

        generations
            .add_generation(&mut temporary, metadata)
            .map_err(ManagedEnvironmentError::CommitGeneration)?;
        self.lock_pointer()?;
        temporary.link(flox, &self.out_link, &result.store_path)?;

        Ok(result)
    }

    /// Uninstall packages from the environment atomically
    fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<UninstallationAttempt, EnvironmentError> {
        let mut generations = self
            .generations()
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;
        let mut temporary = generations
            .get_current_generation()
            .map_err(ManagedEnvironmentError::CreateGenerationFiles)?;

        let metadata = format!("uninstalled packages: {:?}", &packages);
        let result = temporary.uninstall(packages, flox)?;

        generations
            .add_generation(&mut temporary, metadata)
            .map_err(ManagedEnvironmentError::CommitGeneration)?;
        self.lock_pointer()?;
        temporary.link(flox, &self.out_link, &result.store_path)?;

        Ok(result)
    }

    /// Atomically edit this environment, ensuring that it still builds
    fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError> {
        let mut generations = self
            .generations()
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;
        let mut temporary = generations
            .get_current_generation()
            .map_err(ManagedEnvironmentError::CreateGenerationFiles)?;

        let result = temporary.edit(flox, contents)?;

        if result == EditResult::Unchanged {
            return Ok(result);
        }

        let store_path = result.store_path();

        debug!("Environment changed, create generation, lock generation, build and link");

        generations
            .add_generation(&mut temporary, "manually edited".to_string())
            .map_err(ManagedEnvironmentError::CommitGeneration)?;
        self.lock_pointer()?;
        temporary.link(flox, &self.out_link, &store_path)?;

        Ok(result)
    }

    /// Atomically update this environment's inputs
    fn update(
        &mut self,
        flox: &Flox,
        inputs: Vec<String>,
    ) -> Result<UpdateResult, EnvironmentError> {
        let mut generations = self
            .generations()
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        let mut temporary = generations
            .get_current_generation()
            .map_err(ManagedEnvironmentError::CreateGenerationFiles)?;

        let result = temporary.update(flox, inputs)?;

        // TODO: better message
        let metadata = "updated environment".to_string();

        generations
            .add_generation(&mut temporary, metadata)
            .map_err(ManagedEnvironmentError::CommitGeneration)?;
        self.lock_pointer()?;
        temporary.link(flox, &self.out_link, &result.store_path)?;

        Ok(result)
    }

    /// Atomically upgrade packages in this environment
    fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[String],
    ) -> Result<UpgradeResult, EnvironmentError> {
        let mut generations = self
            .generations()
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        let mut temporary = generations
            .get_current_generation()
            .map_err(ManagedEnvironmentError::CreateGenerationFiles)?;

        let result = temporary.upgrade(flox, groups_or_iids)?;

        let metadata = format!("upgraded packages: {}", result.packages.join(", "));

        generations
            .add_generation(&mut temporary, metadata)
            .map_err(ManagedEnvironmentError::CommitGeneration)?;

        write_pointer_lockfile(
            self.path.join(GENERATION_LOCK_FILENAME),
            &self.floxmeta,
            remote_branch_name(&self.pointer),
            branch_name(&self.pointer, &self.path).into(),
        )?;
        Ok(result)
    }

    /// Extract the current content of the manifest
    fn manifest_content(&self, _flox: &Flox) -> Result<String, EnvironmentError> {
        let manifest = self
            .generations()
            .current_gen_manifest()
            .map_err(ManagedEnvironmentError::ReadManifest)?;
        Ok(manifest)
    }

    fn activation_path(&mut self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        let pointer_lock_path = self.path.join(GENERATION_LOCK_FILENAME);

        let pointer_lock_modified_at = mtime_of(pointer_lock_path);
        let out_link_modified_at = mtime_of(&self.out_link);

        debug!(
            "pointer_lock_modified_at: {pointer_lock_modified_at:?}
            out_link_modified_at: {out_link_modified_at:?}"
        );

        if pointer_lock_modified_at >= out_link_modified_at || !self.out_link.exists() {
            self.build(flox)?;
        }

        Ok(self.out_link.to_path_buf())
    }

    /// Returns .flox/cache
    fn cache_path(&self) -> Result<PathBuf, EnvironmentError> {
        let cache_dir = self.path.join(CACHE_DIR_NAME);
        if !cache_dir.exists() {
            std::fs::create_dir_all(&cache_dir).map_err(EnvironmentError::CreateCacheDir)?;
        }
        Ok(cache_dir)
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

    /// Path to the environment definition file
    ///
    /// Path will not share a common prefix with the path returned by [`ManagedEnvironment::lockfile_path`]
    fn manifest_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        let path = self.get_current_generation(flox)?.manifest_path();
        Ok(path)
    }

    /// Path to the lockfile. The path may not exist.
    ///
    /// Path will not share a common prefix with the path returned by [`ManagedEnvironment::manifest_path`]
    fn lockfile_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        let path = self.get_current_generation(flox)?.lockfile_path();
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
            .git
            .delete_branch(&branch_name(&self.pointer, &self.path), true)
            .map_err(ManagedEnvironmentError::DeleteBranch)?;

        let out_link_path = self.out_link;
        if out_link_path.exists() {
            std::fs::remove_file(&out_link_path)
                .map_err(|e| ManagedEnvironmentError::DeleteEnvironmentLink(out_link_path, e))?;
        }

        deregister(flox, &self.path, &EnvironmentPointer::Managed(self.pointer))?;

        Ok(())
    }
}

/// Constructors and related functions
impl ManagedEnvironment {
    /// Returns a unique identifier for the location of the project.
    fn encode(path: &CanonicalPath) -> String {
        path_hash(path)
    }

    /// Returns the path to an environment given the branch name in the floxmeta repository.
    ///
    /// Will only error if the symlink doesn't exist, the path the symlink points to doesn't
    /// exist, or if the branch name is malformed.
    #[allow(unused)]
    fn decode(flox: &Flox, branch: &impl AsRef<str>) -> Result<PathBuf, ManagedEnvironmentError> {
        let branch_name = branch.as_ref();
        branch_name
            .split('.')
            .next_back()
            .map(|hash| {
                let reg_path = env_registry_path(flox);
                let reg: EnvRegistry = read_environment_registry(reg_path)?
                    .ok_or(EnvRegistryError::EnvNotRegistered)?;
                let hash = match hash.len() {
                    // Older branches probably still have the longer hashes
                    n if n < N_HASH_CHARS => Err(ManagedEnvironmentError::BadBranchName(
                        String::from(branch_name),
                    )),
                    n if n > N_HASH_CHARS => Ok(hash.split_at(N_HASH_CHARS).0),
                    n => Ok(hash),
                }?;
                reg.path_for_hash(hash)
                    .map_err(ManagedEnvironmentError::Registry)
            })
            .unwrap_or(Err(ManagedEnvironmentError::BadBranchName(String::from(
                branch_name,
            ))))
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
    ) -> Result<Self, ManagedEnvironmentError> {
        let floxmeta = match FloxMeta::open(flox, &pointer) {
            Ok(floxmeta) => floxmeta,
            Err(FloxMetaError::NotFound(_)) => {
                debug!("cloning floxmeta for {}", pointer.owner);
                FloxMeta::clone(flox, &pointer).map_err(ManagedEnvironmentError::OpenFloxmeta)?
            },
            Err(FloxMetaError::CloneBranch(GitRemoteCommandError::AccessDenied))
            | Err(FloxMetaError::FetchBranch(GitRemoteCommandError::AccessDenied)) => {
                return Err(ManagedEnvironmentError::AccessDenied)
            },
            Err(FloxMetaError::CloneBranch(GitRemoteCommandError::RefNotFound(_)))
            | Err(FloxMetaError::FetchBranch(GitRemoteCommandError::RefNotFound(_))) => {
                return Err(ManagedEnvironmentError::UpstreamNotFound(
                    pointer.into(),
                    flox.floxhub.base_url().to_string(),
                ))
            },
            Err(e) => Err(ManagedEnvironmentError::OpenFloxmeta(e))?,
        };

        let dot_flox_path =
            CanonicalPath::new(dot_flox_path).map_err(ManagedEnvironmentError::CanonicalizePath)?;

        let out_link =
            gcroots_dir(flox, &pointer.owner).join(branch_name(&pointer, &dot_flox_path));

        Self::open_with(floxmeta, flox, pointer, dot_flox_path, out_link)
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
        out_link: PathBuf,
    ) -> Result<Self, ManagedEnvironmentError> {
        let lock = Self::ensure_locked(&pointer, &dot_flox_path, &floxmeta)?;

        Self::ensure_branch(&branch_name(&pointer, &dot_flox_path), &lock, &floxmeta)?;

        ensure_registered(
            flox,
            &dot_flox_path,
            &EnvironmentPointer::Managed(pointer.clone()),
        )?;

        Ok(ManagedEnvironment {
            path: dot_flox_path,
            out_link,
            pointer,
            floxmeta,
        })
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
    fn ensure_locked(
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
    ) -> Result<Result<EditResult, CoreEnvironmentError>, EnvironmentError> {
        let mut generations = self
            .generations()
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;
        let mut temporary = generations
            .get_current_generation()
            .map_err(ManagedEnvironmentError::CreateGenerationFiles)?;

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

    fn generations(&self) -> Generations {
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
            .get_current_generation()
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
        debug!("writing pointer lockfile: remote_rev='{rev}', local_rev='{local_rev}', lockfile={lock_path:?}");
    } else {
        debug!("writing pointer lockfile: remote_rev='{rev}', local_rev=<unset>, ,lockfile={lock_path:?}");
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
/// To identify the individual branches per directory,
/// the directory path is encoded using [`ManagedEnvironment::encode`].
///
/// `dot_flox_path` is expected to point to the `.flox/` directory
/// that link to an environment identified by `pointer`.
/// `dot_flox_path` does _not_ need to be passed in its canonicalized form;
/// [`ManagedEnvironment::encode`] will canonicalize the path if necessary.
fn branch_name(pointer: &ManagedPointer, dot_flox_path: &CanonicalPath) -> String {
    format!(
        "{}.{}",
        pointer.name,
        ManagedEnvironment::encode(dot_flox_path)
    )
}

/// The original branch name of an environment that is used to sync an environment with the hub
///
/// In most cases [`branch_name`] should be used over this,
/// within the context of an instance of [ManagedEnvironment].
///
/// [`remote_branch_name`] is primarily used when talking to upstream on FloxHub,
/// during opening to reconciliate with the upsream repo
/// as well as during [`ManagedEnvironment::pull`].
pub fn remote_branch_name(pointer: &ManagedPointer) -> String {
    format!("{}", pointer.name)
}

pub enum PullResult {
    /// The environment was already up to date
    UpToDate,
    /// The environment was reset to the latest upstream version
    Updated,
}

impl ManagedEnvironment {
    /// If access to a remote repository requires authentication,
    /// the FloxHub token must be set in the flox instance.
    /// The caller is responsible for ensuring that the token is present and valid.
    pub fn push_new(
        flox: &Flox,
        path_environment: PathEnvironment,
        owner: EnvironmentOwner,
        force: bool,
    ) -> Result<Self, ManagedEnvironmentError> {
        // path of the original .flox directory
        let dot_flox_path = path_environment.path.clone();
        let name = path_environment.name();

        let mut core_environment = path_environment.into_core_environment();

        // Ensure the environment is locked
        // PathEnvironment may not have a lockfile or an outdated lockfile
        // if the environment was modified primarily through editing the manifest manually.
        core_environment
            .lock(flox)
            .map_err(ManagedEnvironmentError::Lock)?;

        // Ensure the environment builds before we push it
        core_environment
            .build(flox)
            .map_err(ManagedEnvironmentError::Build)?;

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
    ) -> Result<Self, ManagedEnvironmentError> {
        let pointer = ManagedPointer::new(owner, name.clone(), &flox.floxhub);

        let checkedout_floxmeta_path = tempfile::tempdir_in(&flox.temp_dir).unwrap().into_path();
        let temp_floxmeta_path = tempfile::tempdir_in(&flox.temp_dir).unwrap().into_path();

        // Caller decides whether to set token
        let token = flox.floxhub_token.as_ref();

        let git_url = flox.floxhub.git_url();

        let options = floxmeta_git_options(git_url, &pointer.owner, token);

        let generations = Generations::init(
            options,
            checkedout_floxmeta_path,
            temp_floxmeta_path,
            remote_branch_name(&pointer),
            &name,
        )
        .map_err(ManagedEnvironmentError::InitializeFloxmeta)?;

        let temp_floxmeta_git = generations.git().clone();

        let mut generations = generations
            .writable(flox.temp_dir.clone())
            .map_err(ManagedEnvironmentError::CreateFloxmetaDir)?;

        generations
            .add_generation(&mut core_environment, "Add first generation".to_string())
            .map_err(ManagedEnvironmentError::CommitGeneration)?;

        temp_floxmeta_git
            .add_remote(
                "upstream",
                &format!("{}/{}/floxmeta", &git_url, &pointer.owner),
            )
            .unwrap();

        match temp_floxmeta_git.push_ref("upstream", "HEAD", force) {
            Err(GitRemoteCommandError::AccessDenied) => Err(ManagedEnvironmentError::AccessDenied)?,
            Err(GitRemoteCommandError::Diverged) => Err(ManagedEnvironmentError::Diverged)?,
            Err(e) => Err(ManagedEnvironmentError::Push(e))?,
            _ => {},
        }

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

    pub fn push(&mut self, flox: &Flox, force: bool) -> Result<(), ManagedEnvironmentError> {
        let project_branch = branch_name(&self.pointer, &self.path);
        let sync_branch = remote_branch_name(&self.pointer);

        // Ensure the environment builds before we push it
        // Usually we don't create generations unless they build,
        // but that is not always the case.
        // If a user pulls an environment that is broken on their system, we may
        // create a "broken" generation.
        // That generation could have a divergent manifest and lock,
        // or it could fail to build.
        // So we have to verify we don't have a "broken" generation before pushing.
        {
            let mut env = self.get_current_generation(flox)?;
            // [sic] We do not _lock_ the environment here,
            // as a valid lockfile is a precondition for creating a generation.
            env.build(flox).map_err(ManagedEnvironmentError::Build)?;
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
        self.pull(force)?;

        Ok(())
    }

    pub fn pull(&mut self, force: bool) -> Result<PullResult, ManagedEnvironmentError> {
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
                Err(ManagedEnvironmentError::UpstreamNotFound(
                    self.pointer.clone().into(),
                    self.pointer.floxhub_url.to_string(),
                ))?
            },
            Err(e) => Err(ManagedEnvironmentError::FetchUpdates(e))?,
        };

        // Check whether we can fast-forward the remote branch to the local branch,
        // if not the environment has diverged.
        // if `--force` flag is set we skip this check
        if !force {
            let consistent_history = self
                .floxmeta
                .git
                .branch_contains_commit(&project_branch, &sync_branch)
                .map_err(ManagedEnvironmentError::Git)?;
            if !consistent_history {
                Err(ManagedEnvironmentError::Diverged)?;
            }

            let sync_branch_commit = self.floxmeta.git.branch_hash(&sync_branch).ok();
            let project_branch_commit = self.floxmeta.git.branch_hash(&project_branch).ok();

            if sync_branch_commit == project_branch_commit {
                return Ok(PullResult::UpToDate);
            }
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

        Ok(PullResult::Updated)
    }
}

pub mod test_helpers {

    use tempfile::tempdir_in;

    use super::*;
    use crate::flox::{Floxhub, DEFAULT_FLOXHUB_URL};
    use crate::models::environment::core_environment::test_helpers::new_core_environment;
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
            out_link: PathBuf::new(),
            pointer: ManagedPointer::new(
                "owner".parse().unwrap(),
                "test".parse().unwrap(),
                &floxhub,
            ),
            floxmeta: unusable_mock_floxmeta(),
        }
    }

    /// Get a [ManagedEnvironment] that has been pushed to (a mock) FloxHub and
    /// can be built.
    ///
    /// This should be passed a [Flox] instance created with
    /// flox_instance_with_global_lock_and_floxhub()
    ///
    /// If a [ManagedEnvironment] will be unused in tests, use
    /// [unusable_mock_managed_environment] instead.
    pub fn mock_managed_environment(
        flox: &Flox,
        contents: &str,
        owner: EnvironmentOwner,
    ) -> ManagedEnvironment {
        ManagedEnvironment::push_new_without_building(
            flox,
            owner,
            "name".parse().unwrap(),
            false,
            CanonicalPath::new(tempdir_in(&flox.temp_dir).unwrap().into_path()).unwrap(),
            new_core_environment(flox, contents),
        )
        .unwrap()
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;
    use std::time::Duration;

    use fslock::LockFile;
    use url::Url;

    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::env_registry::{
        env_registry_lock_path,
        env_registry_path,
        read_environment_registry,
        write_environment_registry,
        RegisteredEnv,
        RegistryEntry,
    };
    use crate::models::environment::DOT_FLOX;
    use crate::models::floxmeta::floxmeta_dir;
    use crate::providers::git::tests::commit_file;
    use crate::providers::git::{GitCommandProvider, GitProvider};

    fn make_test_pointer(mock_floxhub_git_path: &Path) -> ManagedPointer {
        ManagedPointer {
            owner: EnvironmentOwner::from_str("owner").unwrap(),
            name: EnvironmentName::from_str("name").unwrap(),
            floxhub_url: Url::from_str("https://hub.flox.dev").unwrap(),
            floxhub_git_url_override: Some(
                Url::from_directory_path(mock_floxhub_git_path).unwrap(),
            ),
            version: Version::<1> {},
        }
    }

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
        let remote_base_path = flox.temp_dir.join("remote");
        let test_pointer = make_test_pointer(&remote_base_path);
        let remote_path = remote_base_path
            .join(test_pointer.owner.as_str())
            .join("floxmeta");
        fs::create_dir_all(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).unwrap();

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

        ManagedEnvironment::ensure_locked(&test_pointer, &dot_flox_path, &floxmeta).unwrap();

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
        let remote_base_path = flox.temp_dir.join("remote");
        let test_pointer = make_test_pointer(&remote_base_path);
        let remote_path = remote_base_path
            .join(test_pointer.owner.as_str())
            .join("floxmeta");
        fs::create_dir_all(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).unwrap();

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

        ManagedEnvironment::ensure_locked(&test_pointer, &dot_flox_path, &floxmeta).unwrap();

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
        let remote_base_path = flox.temp_dir.join("remote");
        let test_pointer = make_test_pointer(&remote_base_path);
        let remote_path = remote_base_path
            .join(test_pointer.owner.as_str())
            .join("floxmeta");
        fs::create_dir_all(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).unwrap();

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

        ManagedEnvironment::ensure_locked(
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
        let remote_base_path = flox.temp_dir.join("remote");
        let test_pointer = make_test_pointer(&remote_base_path);
        let remote_path = remote_base_path
            .join(test_pointer.owner.as_str())
            .join("floxmeta");
        fs::create_dir_all(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).unwrap();

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
            ManagedEnvironment::ensure_locked(&test_pointer, &dot_flox_path, &floxmeta),
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
        let remote_base_path = flox.temp_dir.join("remote");
        let test_pointer = make_test_pointer(&remote_base_path);
        let remote_path = remote_base_path
            .join(test_pointer.owner.as_str())
            .join("floxmeta");
        fs::create_dir_all(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).unwrap();

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
            ManagedEnvironment::ensure_locked(&test_pointer, &dot_flox_path, &floxmeta),
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
        let remote_base_path = flox.temp_dir.join("remote");
        let test_pointer = make_test_pointer(&remote_base_path);
        let remote_path = remote_base_path
            .join(test_pointer.owner.as_str())
            .join("floxmeta");
        fs::create_dir_all(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).unwrap();

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

        ManagedEnvironment::ensure_locked(&test_pointer, &dot_flox_path, &floxmeta).unwrap();

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
        let remote_base_path = flox.temp_dir.join("remote");
        let test_pointer = make_test_pointer(&remote_base_path);
        let remote_path = remote_base_path
            .join(test_pointer.owner.as_str())
            .join("floxmeta");
        fs::create_dir_all(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).unwrap();

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
            ManagedEnvironment::ensure_locked(&test_pointer, &dot_flox_path, &floxmeta),
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
        let remote_base_path = flox.temp_dir.join("remote");
        let test_pointer = make_test_pointer(&remote_base_path);
        let remote_path = remote_base_path
            .join(test_pointer.owner.as_str())
            .join("floxmeta");
        fs::create_dir_all(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).unwrap();

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
            ManagedEnvironment::ensure_locked(&test_pointer, &dot_flox_path, &floxmeta).unwrap(),
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
        let remote_base_path = flox.temp_dir.join("remote");
        let test_pointer = make_test_pointer(&remote_base_path);
        let remote_path = remote_base_path
            .join(test_pointer.owner.as_str())
            .join("floxmeta");
        fs::create_dir_all(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).unwrap();

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
    /// ensure_branch resets the branch to commit 2
    #[test]
    fn test_ensure_branch_resets_branch() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let remote_base_path = flox.temp_dir.join("remote");
        let test_pointer = make_test_pointer(&remote_base_path);
        let remote_path = remote_base_path
            .join(test_pointer.owner.as_str())
            .join("floxmeta");
        fs::create_dir_all(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).unwrap();

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
    /// ensure_branch creates branch_2 at commit 1
    #[test]
    fn test_ensure_branch_creates_branch() {
        let (flox, _temp_dir_handle) = flox_instance();

        // create a mock remote
        let remote_base_path = flox.temp_dir.join("remote");
        let test_pointer = make_test_pointer(&remote_base_path);
        let remote_path = remote_base_path
            .join(test_pointer.owner.as_str())
            .join("floxmeta");
        fs::create_dir_all(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).unwrap();

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

    #[test]
    fn stable_encode_name() {
        // Ensure that running the encode function gives you the same results
        // with the same input e.g. doesn't depend on time, etc
        let (_flox, tmp_dir) = flox_instance();
        let path = tmp_dir.path().join("foo");
        std::fs::File::create(&path).unwrap();
        let path = CanonicalPath::new(path).unwrap();

        let encode1 = ManagedEnvironment::encode(&path);
        std::thread::sleep(Duration::from_millis(1_000));
        let encode2 = ManagedEnvironment::encode(&path);
        assert_eq!(encode1, encode2);
    }

    #[test]
    fn decode_branch_name_to_path() {
        let (flox, tmp_dir) = flox_instance();
        let path = tmp_dir.path().join("foo");
        std::fs::File::create(&path).unwrap();
        let path = CanonicalPath::new(path).unwrap();
        // Decode the branch name and assert that it's the same path we created before
        let pointer = ManagedPointer {
            owner: EnvironmentOwner::from_str("owner").unwrap(),
            name: EnvironmentName::from_str("name").unwrap(),
            floxhub_url: Url::from_str("https://hub.flox.dev").unwrap(),
            floxhub_git_url_override: None,
            version: Version::<1>,
        };
        let reg = EnvRegistry {
            version: Version,
            entries: vec![RegistryEntry {
                path_hash: path_hash(&path),
                path: path.to_path_buf(),
                envs: vec![RegisteredEnv {
                    created_at: 0,
                    pointer: EnvironmentPointer::Managed(pointer.clone()),
                }],
            }],
        };
        let lock = LockFile::open(&env_registry_lock_path(&flox)).unwrap();
        write_environment_registry(&reg, &env_registry_path(&flox), lock).unwrap();
        let branch_name = branch_name(&pointer, &path);
        let decoded_path = ManagedEnvironment::decode(&flox, &branch_name).unwrap();
        let canonicalized_decoded_path = std::fs::canonicalize(decoded_path).unwrap();
        let canonicalized_path = std::fs::canonicalize(&path).unwrap();
        assert_eq!(canonicalized_path, canonicalized_decoded_path);
    }

    #[test]
    fn registers_on_open() {
        let (flox, _temp_dir_handle) = flox_instance();
        let dot_flox_path = flox.temp_dir.join(DOT_FLOX);
        std::fs::create_dir_all(&dot_flox_path).unwrap();

        // create a mock remote
        let remote_base_path = flox.temp_dir.join("remote");
        let test_pointer = make_test_pointer(&remote_base_path);
        let remote_path = remote_base_path
            .join(test_pointer.owner.as_str())
            .join("floxmeta");
        fs::create_dir_all(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).unwrap();

        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");

        // create a mock floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &branch);

        let _env = ManagedEnvironment::open_with(
            floxmeta,
            &flox,
            test_pointer,
            CanonicalPath::new(dot_flox_path).unwrap(),
            flox.temp_dir.join("out_link"),
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

        // create a mock remote
        let remote_base_path = flox.temp_dir.join("remote");
        let test_pointer = make_test_pointer(&remote_base_path);
        let remote_path = remote_base_path
            .join(test_pointer.owner.as_str())
            .join("floxmeta");
        fs::create_dir_all(&remote_path).unwrap();
        let remote = GitCommandProvider::init(&remote_path, false).unwrap();

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
            CanonicalPath::new(dot_flox_path).unwrap(),
            flox.temp_dir.join("out_link"),
        )
        .unwrap();
        let reg_path = env_registry_path(&flox);
        assert!(reg_path.exists());

        // Delete the environment from the registry
        env.delete(&flox).unwrap();
        let reg = read_environment_registry(&reg_path).unwrap().unwrap();
        assert!(reg.entries.is_empty());
    }
}
