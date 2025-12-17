use std::fs;
use std::path::Path;

use fslock::LockFile;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;

use super::{ManagedPointer, path_hash};
use crate::data::CanonicalPath;
use crate::flox::{Flox, RemoteEnvironmentRef};
use crate::models::environment::generations::Generations;
use crate::models::floxmeta::{BRANCH_NAME_PATH_SEPARATOR, FloxMeta, FloxMetaError, floxmeta_dir};
use crate::providers::git::{GitCommandBranchHashError, GitCommandError, GitRemoteCommandError};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GenerationLock {
    pub version: flox_core::Version<1>,
    /// Revision of the environment on FloxHub.
    /// This could be stale if the environment has since been changed.
    pub rev: String,
    /// Revision of the environment in local floxmeta repository.
    /// Since an environment can be pulled into multiple different directories
    /// locally, each could have its own local_rev if the environments are
    /// modified.
    /// This is changed when the environment is modified locally,
    /// so it can diverge from both the remote environment and other copies of
    /// the environment pulled into other directories.
    pub local_rev: Option<String>,
}

impl GenerationLock {
    /// Read lockfile if it exists, returning None if file doesn't exist
    pub fn read_maybe(path: impl AsRef<Path>) -> Result<Option<Self>, FloxmetaBranchError> {
        let lock_contents = match std::fs::read(path) {
            Ok(contents) => contents,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(FloxmetaBranchError::ReadLock(e)),
        };

        let lock: Self =
            serde_json::from_slice(&lock_contents).map_err(FloxmetaBranchError::InvalidLock)?;

        Ok(Some(lock))
    }
}

/// An abstraction over the git backed storage of managed environments.
///
/// [FloxmetaBranch] separates the FloxHub connected storage of environments
/// from the management of the environment itself.
///
/// Environments of a single user are stored as branches
/// in a "[FloxMeta]" repository.
/// Environments can have multiple instances
/// (e.g. if pulled into different directories) which each live on a separate branch.
/// [FloxmetaBranch] implements the management of these branches.
///
/// That includes creating new branches upon first use,
/// locking of local state and restoring from branches from existing locks.
/// Besides that it provides access to [Generations],
/// i.e. the data stored on a branch which in turn
/// can be interpreted as [CoreEnvironment]s allowing environment management.
///
/// [FloxmetaBranch] is meant to separate FloxMeta/FloxHub concerns
/// from the management of environment data itself
/// (i.e. modification and locking of manifests, building of environments
/// and managing environment links).
/// Currently, the latter responsibilities are mixed into
/// the higher level environment abstractions themselves,
/// causing duplication and increasing complexity.
/// That is because we maintain multiple environment implementations
/// that differ mainly in managing "how they are stored".
///
/// The goal of [FloxmetaBranch] is to simplify specific environment implementations further,
/// and possibly remove the need for separate implementations altogether.
#[derive(Debug)]
pub struct FloxmetaBranch {
    floxmeta: FloxMeta,
    branch: String,
}

#[derive(Debug, Error)]
pub enum FloxmetaBranchError {
    #[error("failed to create floxmeta directory")]
    CreateFloxmetaDir(#[source] std::io::Error),

    #[error("failed to lock floxmeta git repo")]
    LockFloxmeta(#[source] fslock::Error),

    #[error("failed to open floxmeta git repo: {0}")]
    OpenFloxmeta(#[source] FloxMetaError),

    #[error("access denied to environment")]
    AccessDenied,

    #[error("environment not found: {env_ref} at {upstream}")]
    UpstreamNotFound {
        env_ref: RemoteEnvironmentRef,
        upstream: String,
        user: Option<String>,
    },

    #[error("failed to check for git revision: {0}")]
    CheckGitRevision(#[source] GitCommandError),

    #[error("failed to check for branch existence")]
    CheckBranchExists(#[source] GitCommandBranchHashError),

    #[error(
        "can't find local_rev specified in lockfile; \
         local_rev could have been mistakenly committed on another machine"
    )]
    LocalRevDoesNotExist,

    #[error(
        "can't find rev specified in lockfile; \
         the environment may have been deleted on FloxHub"
    )]
    RevDoesNotExist,

    #[error("failed to fetch environment: {0}")]
    Fetch(#[source] GitRemoteCommandError),

    #[error("failed to get branch hash: {0}")]
    GitBranchHash(#[source] GitCommandBranchHashError),

    #[error("failed to create/update branch: {0}")]
    BranchSetup(#[source] GitCommandError),

    #[error("failed to read lockfile")]
    ReadLock(#[source] std::io::Error),

    #[error("invalid lockfile")]
    InvalidLock(#[source] serde_json::Error),

    #[error("failed to serialize lockfile")]
    SerializeLock(#[source] serde_json::Error),

    #[error("failed to write lockfile")]
    WriteLock(#[source] std::io::Error),
}

impl FloxmetaBranch {
    /// Create validated floxmeta access
    ///
    /// Performs all necessary git validation:
    /// - Opens or clones the floxmeta repository
    /// - Ensures the generation is locked (validates/fetches commit)
    /// - Sets up the branch to track the environment state
    ///
    /// Returns the FloxmetaBranch and the validated GenerationLock that the
    /// caller should write to disk.
    pub fn new(
        flox: &Flox,
        pointer: &ManagedPointer,
        dot_flox_path: &CanonicalPath,
        maybe_lock: Option<GenerationLock>,
    ) -> Result<(Self, GenerationLock), FloxmetaBranchError> {
        // Acquire lock

        let _lock = acquire_floxmeta_lock(&floxmeta_dir(flox, &pointer.owner))?;

        // Open or clone floxmeta
        let floxmeta = open_or_clone_floxmeta(flox, pointer)?;

        // Ensure generation is locked
        let remote_branch = remote_branch_name(pointer);
        let local_branch = branch_name(pointer, dot_flox_path);
        let lock = ensure_generation_locked(&remote_branch, &local_branch, &floxmeta, maybe_lock)?;

        // Set up branch
        ensure_branch(&local_branch, &lock, &floxmeta)?;

        Ok((
            Self {
                floxmeta,
                branch: local_branch,
            },
            lock,
        ))
    }

    /// Access the underlying git provider
    pub fn git(&self) -> &crate::providers::git::GitCommandProvider {
        &self.floxmeta.git
    }

    /// Get the branch name for this environment
    pub fn branch(&self) -> &str {
        &self.branch
    }

    /// Access generations (read-only by default)
    pub fn generations(&self) -> Generations {
        Generations::new(self.floxmeta.git.clone(), self.branch.clone())
    }

    /// Compute the current GenerationLock based on branch state
    ///
    /// Returns a lock with:
    /// - `rev`: hash of the remote branch
    /// - `local_rev`: hash of the local branch (if different from rev)
    pub fn generation_lock(
        &self,
        pointer: &ManagedPointer,
    ) -> Result<GenerationLock, FloxmetaBranchError> {
        let remote_branch = remote_branch_name(pointer);

        // Get rev from remote branch
        let rev = self
            .floxmeta
            .git
            .branch_hash(&remote_branch)
            .map_err(FloxmetaBranchError::GitBranchHash)?;

        // Get local_rev from local branch (only if it differs from rev)
        let local_rev = match self.floxmeta.git.branch_hash(&self.branch) {
            Ok(local_rev) if local_rev == rev => None,
            Ok(local_rev) => Some(local_rev),
            Err(err) => return Err(FloxmetaBranchError::GitBranchHash(err)),
        };

        Ok(GenerationLock {
            rev,
            local_rev,
            version: flox_core::Version::<1>,
        })
    }

    /// Fetch the remote branch into the local sync branch.
    /// The sync branch is always a reset to the remote branch
    /// and it's state should not be depended on.
    pub fn fetch_remote_state(
        &self,
        flox: &Flox,
        pointer: &ManagedPointer,
    ) -> Result<(), FloxmetaBranchError> {
        let sync_branch = remote_branch_name(&pointer);
        self.git()
            .fetch_ref("dynamicorigin", &format!("+{sync_branch}:{sync_branch}"))
            .map_err(|err| match err {
                GitRemoteCommandError::RefNotFound(_) => FloxmetaBranchError::UpstreamNotFound {
                    env_ref: pointer.clone().into(),
                    upstream: pointer.floxhub_base_url.to_string(),
                    user: flox.floxhub_token.as_ref().map(|t| t.handle().to_string()),
                },
                e => FloxmetaBranchError::Fetch(e),
            })
    }
}

/// Acquire exclusive lock on floxmeta directory
#[tracing::instrument(fields(
    progress = "Waiting for lock to open or create Flox remote metadata"
))]
fn acquire_floxmeta_lock(floxmeta_dir: &Path) -> Result<LockFile, FloxmetaBranchError> {
    let parent = floxmeta_dir.parent().expect("path is non-empty");
    std::fs::create_dir_all(parent).map_err(FloxmetaBranchError::CreateFloxmetaDir)?;
    // TODO: use with_extension once we update our rustc
    let mut lock = LockFile::open(
        &floxmeta_dir.with_file_name(
            floxmeta_dir
                .file_name()
                .expect("path is non-empty")
                .to_string_lossy()
                .into_owned()
                + ".lock",
        ),
    )
    .map_err(FloxmetaBranchError::LockFloxmeta)?;
    lock.lock().map_err(FloxmetaBranchError::LockFloxmeta)?;
    Ok(lock)
}

/// Open existing or clone new floxmeta repository
fn open_or_clone_floxmeta(
    flox: &Flox,
    pointer: &ManagedPointer,
) -> Result<FloxMeta, FloxmetaBranchError> {
    // Try to open existing
    let existing_floxmeta = match FloxMeta::open(flox, pointer) {
        Ok(floxmeta) => Some(floxmeta),
        Err(FloxMetaError::NotFound(_)) => None,
        Err(FloxMetaError::FetchBranch(GitRemoteCommandError::AccessDenied)) => {
            return Err(FloxmetaBranchError::AccessDenied);
        },
        Err(FloxMetaError::FetchBranch(GitRemoteCommandError::RefNotFound(_))) => {
            return Err(FloxmetaBranchError::UpstreamNotFound {
                env_ref: pointer.clone().into(),
                upstream: flox.floxhub.base_url().to_string(),
                user: flox.floxhub_token.as_ref().map(|t| t.handle().to_string()),
            });
        },
        Err(e) => return Err(FloxmetaBranchError::OpenFloxmeta(e)),
    };

    // Clone if doesn't exist
    let floxmeta = match existing_floxmeta {
        Some(floxmeta) => floxmeta,
        None => {
            debug!("cloning floxmeta for {}", &pointer.owner);
            match FloxMeta::clone(flox, pointer) {
                Ok(floxmeta) => floxmeta,
                Err(FloxMetaError::CloneBranch(GitRemoteCommandError::AccessDenied)) => {
                    return Err(FloxmetaBranchError::AccessDenied);
                },
                Err(FloxMetaError::CloneBranch(GitRemoteCommandError::RefNotFound(_))) => {
                    return Err(FloxmetaBranchError::UpstreamNotFound {
                        env_ref: pointer.clone().into(),
                        upstream: flox.floxhub.base_url().to_string(),
                        user: flox.floxhub_token.as_ref().map(|t| t.handle().to_string()),
                    });
                },
                Err(e) => return Err(FloxmetaBranchError::OpenFloxmeta(e)),
            }
        },
    };

    Ok(floxmeta)
}

fn remote_branch_name(pointer: &ManagedPointer) -> String {
    format!("{}", pointer.name)
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

/// Ensure generation is locked and commit exists in floxmeta
///
/// Takes an optional GenerationLock (read from disk by caller) and validates that:
/// - If local_rev is set, it exists in the floxmeta repo
/// - If only rev is set, it exists (fetching if necessary)
/// - If no lock provided, fetches latest from remote and creates new lock data
///
/// Returns validated GenerationLock data that caller should write to disk
fn ensure_generation_locked(
    remote_branch: &str,
    local_branch: &str,
    floxmeta: &FloxMeta,
    maybe_lock: Option<GenerationLock>,
) -> Result<GenerationLock, FloxmetaBranchError> {
    Ok(match maybe_lock {
        // Use local_rev if we have it
        Some(lock) if lock.local_rev.is_some() => {
            // Verify local_rev exists in floxmeta
            if !floxmeta
                .git
                .contains_commit(lock.local_rev.as_ref().unwrap())
                .map_err(FloxmetaBranchError::CheckGitRevision)?
            {
                Err(FloxmetaBranchError::LocalRevDoesNotExist)?;
            }
            lock
        },
        // We have rev but not local_rev
        Some(lock) => {
            // Check if commit exists on remote or local branch
            let has_branch = floxmeta
                .git
                .has_branch(local_branch)
                .map_err(FloxmetaBranchError::CheckBranchExists)?;

            let in_local = has_branch
                && floxmeta
                    .git
                    .branch_contains_commit(&lock.rev, local_branch)
                    .map_err(FloxmetaBranchError::CheckGitRevision)?;

            // If not in local, try fetching from remote
            if !in_local {
                let span = tracing::info_span!(
                    "ensure_generation_locked::restore_locked",
                    rev = %lock.rev,
                    progress = "Fetching locked generation"
                );
                let _guard = span.enter();

                floxmeta
                    .git
                    .fetch_ref("dynamicorigin", &format!("+{0}:{0}", remote_branch))
                    .map_err(FloxmetaBranchError::Fetch)?;
            }

            // Verify commit exists after fetch
            let in_remote = floxmeta
                .git
                .branch_contains_commit(&lock.rev, remote_branch)
                .map_err(FloxmetaBranchError::CheckGitRevision)?;

            if !in_remote && !in_local {
                Err(FloxmetaBranchError::RevDoesNotExist)?;
            }

            lock
        },
        // No lockfile, create one from latest remote
        None => {
            let span = tracing::info_span!(
                "ensure_generation_locked::lock_latest",
                progress = "Fetching latest generation"
            );
            let _guard = span.enter();

            floxmeta
                .git
                .fetch_ref("dynamicorigin", &format!("+{0}:{0}", remote_branch))
                .map_err(FloxmetaBranchError::Fetch)?;

            // Get the hash of the remote branch
            let rev = floxmeta
                .git
                .branch_hash(remote_branch)
                .map_err(FloxmetaBranchError::GitBranchHash)?;

            GenerationLock {
                rev,
                local_rev: None,
                version: flox_core::Version::<1> {},
            }
        },
    })
}

/// Ensure the branch exists and points at rev or local_rev
fn ensure_branch(
    branch: &str,
    lock: &GenerationLock,
    floxmeta: &FloxMeta,
) -> Result<(), FloxmetaBranchError> {
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
                    .map_err(FloxmetaBranchError::BranchSetup)?;
            }
        },
        // create branch if it doesn't exist
        Err(GitCommandBranchHashError::DoesNotExist) => {
            floxmeta
                .git
                .create_branch(branch, current_rev)
                .map_err(FloxmetaBranchError::BranchSetup)?;
        },
        Err(err) => Err(FloxmetaBranchError::GitBranchHash(err))?,
    }
    Ok(())
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
/// Write a validated GenerationLock to disk
pub(super) fn write_generation_lock(
    lock_path: impl AsRef<Path>,
    lock: &GenerationLock,
) -> Result<(), FloxmetaBranchError> {
    // Log what we're writing
    if let Some(ref local_rev) = lock.local_rev {
        debug!(
            "writing pointer lockfile: remote_rev='{}', local_rev='{}', lockfile={:?}",
            lock.rev,
            local_rev,
            lock_path.as_ref()
        );
    } else {
        debug!(
            "writing pointer lockfile: remote_rev='{}', local_rev=<unset>, lockfile={:?}",
            lock.rev,
            lock_path.as_ref()
        );
    }

    // Check if lock is unchanged
    {
        let existing_lock = GenerationLock::read_maybe(&lock_path);

        // Convert floxmeta_branch::GenerationLock to managed_environment::GenerationLock for comparison
        let lock_for_comparison = GenerationLock {
            rev: lock.rev.clone(),
            local_rev: lock.local_rev.clone(),
            version: flox_core::Version::<1> {},
        };

        if matches!(existing_lock, Ok(Some(ref existing_lock)) if existing_lock == &lock_for_comparison)
        {
            debug!("skip writing unchanged generation lock");
            return Ok(());
        }
    }

    // Write to disk
    let lock_contents =
        serde_json::to_string_pretty(&lock).map_err(FloxmetaBranchError::SerializeLock)?;

    fs::write(lock_path, lock_contents).map_err(FloxmetaBranchError::WriteLock)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::str::FromStr;

    use flox_core::Version;
    use url::Url;

    use super::*;
    use crate::flox::Flox;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::environment::{EnvironmentName, EnvironmentOwner};
    use crate::models::floxmeta::{FloxMeta, floxmeta_dir};
    use crate::providers::git::tests::commit_file;
    use crate::providers::git::{GitCommandProvider, GitProvider};

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
            version: flox_core::Version::<1> {},
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

    /// Clone a git repo specified by remote_path into the floxmeta dir
    /// corresponding to test_pointer,
    /// and open that as a Floxmeta
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

    /// Test that when ensure_generation_locked has input state of:
    /// - no lock
    /// - floxmeta at commit 1
    /// - remote at commit 2
    ///
    /// It results in output state of:
    /// - lock at commit 2
    /// - [fetches from remote]
    #[test]
    fn test_ensure_generation_locked_no_lockfile() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&remote_branch).unwrap();

        // Create floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);

        // Add a second commit to the remote
        commit_file(&remote, "file 2");
        let hash_2 = remote.branch_hash(&remote_branch).unwrap();

        // Create a .flox directory
        let dot_flox_dir = flox.temp_dir.join(".flox");
        fs::create_dir(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();
        let local_branch = branch_name(&test_pointer, &dot_flox_path);

        // No lockfile, should fetch latest
        let lock =
            ensure_generation_locked(&remote_branch, &local_branch, &floxmeta, None).unwrap();

        let expected = GenerationLock {
            rev: hash_2.clone(),
            local_rev: None,
            version: Version::<1>,
        };
        assert_eq!(lock, expected);
        assert_ne!(hash_1, hash_2);
    }

    /// Test that when ensure_generation_locked has input state of:
    /// - lock at {rev: commit 1, local_rev: commit 1}
    /// - floxmeta at commit 1
    /// - remote at commit 1
    ///
    /// It results in output state of:
    /// - lock at {rev: commit 1, local_rev: commit 1}
    /// - [no fetch, validates local_rev exists]
    #[test]
    fn test_ensure_generation_locked_with_local_rev() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&remote_branch).unwrap();

        // Create floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);

        let dot_flox_dir = flox.temp_dir.join(".flox");
        fs::create_dir(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();
        let local_branch = branch_name(&test_pointer, &dot_flox_path);

        // Provide a lock with local_rev that exists
        let input_lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: Some(hash_1.clone()),
            version: flox_core::Version::<1>,
        };

        let expected_lock = input_lock.clone();
        let lock =
            ensure_generation_locked(&remote_branch, &local_branch, &floxmeta, Some(input_lock))
                .unwrap();

        // Should return unchanged
        assert_eq!(lock, expected_lock);
    }

    /// Test that when ensure_generation_locked has input state of:
    /// - lock at {rev: commit 1, local_rev: None}
    /// - floxmeta with local_branch at commit 1
    /// - remote at commit 1
    ///
    /// It results in output state of:
    /// - lock at {rev: commit 1, local_rev: None}
    /// - [no fetch, finds rev in local branch]
    #[test]
    fn test_ensure_generation_locked_with_rev_in_local() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&remote_branch).unwrap();

        let dot_flox_dir = flox.temp_dir.join(".flox");
        fs::create_dir(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();

        // Create the local branch on the remote first
        let local_branch = branch_name(&test_pointer, &dot_flox_path);
        remote.checkout(&local_branch, true).unwrap();

        // Create floxmeta and fetch the local branch
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);
        floxmeta
            .git
            .fetch_ref("origin", &format!("+{}:{}", local_branch, local_branch))
            .ok();

        // Provide a lock with rev (no local_rev)
        let input_lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: None,
            version: flox_core::Version::<1>,
        };

        let expected_lock = input_lock.clone();
        let lock =
            ensure_generation_locked(&remote_branch, &local_branch, &floxmeta, Some(input_lock))
                .unwrap();

        // Should return unchanged, no fetch needed
        assert_eq!(lock, expected_lock);
    }

    /// Test that when ensure_generation_locked has input state of:
    /// - lock at {rev: commit 1, local_rev: nonexistent commit}
    /// - floxmeta at commit 1
    /// - remote at commit 1
    ///
    /// It results in output state of:
    /// - error: LocalRevDoesNotExist
    #[test]
    fn test_ensure_generation_locked_local_rev_missing() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&remote_branch).unwrap();

        // Create floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);

        let dot_flox_dir = flox.temp_dir.join(".flox");
        fs::create_dir(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();
        let local_branch = branch_name(&test_pointer, &dot_flox_path);

        // Provide a lock with local_rev that doesn't exist
        let input_lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: Some("nonexistent_commit_hash".to_string()),
            version: flox_core::Version::<1>,
        };

        let result =
            ensure_generation_locked(&remote_branch, &local_branch, &floxmeta, Some(input_lock));

        assert!(matches!(
            result,
            Err(FloxmetaBranchError::LocalRevDoesNotExist)
        ));
    }

    /// Test that when ensure_generation_locked has input state of:
    /// - lock at {rev: commit 1, local_rev: None}
    /// - floxmeta with local_branch at commit 2 (different commit)
    /// - remote at commit 1
    ///
    /// It results in output state of:
    /// - lock at {rev: commit 1, local_rev: None}
    /// - [fetches from remote, finds rev there]
    #[test]
    fn test_ensure_generation_locked_rev_not_in_local_but_in_remote() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&remote_branch).unwrap();

        let dot_flox_dir = flox.temp_dir.join(".flox");
        fs::create_dir(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();

        // Create local branch on remote with different commit
        let local_branch = branch_name(&test_pointer, &dot_flox_path);
        remote.checkout(&local_branch, true).unwrap();
        commit_file(&remote, "different file");

        // Create floxmeta and fetch the local branch
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);
        floxmeta
            .git
            .fetch_ref("origin", &format!("+{}:{}", local_branch, local_branch))
            .ok();

        // Provide a lock with rev that's only in remote branch
        let input_lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: None,
            version: flox_core::Version::<1>,
        };

        let expected_lock = input_lock.clone();
        let lock =
            ensure_generation_locked(&remote_branch, &local_branch, &floxmeta, Some(input_lock))
                .unwrap();

        // Should fetch and find the rev in remote
        assert_eq!(lock, expected_lock);
    }

    /// Test that when ensure_generation_locked has input state of:
    /// - lock at {rev: nonexistent commit, local_rev: None}
    /// - floxmeta with local_branch at commit 1
    /// - remote at commit 1
    ///
    /// It results in output state of:
    /// - error: RevDoesNotExist
    /// - [fetches from remote, but rev not found anywhere]
    #[test]
    fn test_ensure_generation_locked_rev_not_found() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");

        let dot_flox_dir = flox.temp_dir.join(".flox");
        fs::create_dir(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();

        // Create local branch
        let local_branch = branch_name(&test_pointer, &dot_flox_path);
        remote.checkout(&local_branch, true).unwrap();

        // Create floxmeta and fetch the local branch
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);
        floxmeta
            .git
            .fetch_ref("origin", &format!("+{}:{}", local_branch, local_branch))
            .ok();

        // Provide a lock with rev that doesn't exist
        let input_lock = GenerationLock {
            rev: "nonexistent_commit_hash".to_string(),
            local_rev: None,
            version: flox_core::Version::<1>,
        };

        let result =
            ensure_generation_locked(&remote_branch, &local_branch, &floxmeta, Some(input_lock));

        assert!(matches!(result, Err(FloxmetaBranchError::RevDoesNotExist)));
    }

    /// Test that when ensure_generation_locked has input state of:
    /// - lock at {rev: commit 1, local_rev: None}
    /// - floxmeta without local_branch
    /// - remote at commit 1
    ///
    /// It results in output state of:
    /// - lock at {rev: commit 1, local_rev: None}
    /// - [fetches from remote, finds rev there]
    #[test]
    fn test_ensure_generation_locked_no_local_branch_rev_in_remote() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&remote_branch).unwrap();

        // Create floxmeta - no local branch exists
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);

        let dot_flox_dir = flox.temp_dir.join(".flox");
        fs::create_dir(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();
        let local_branch = branch_name(&test_pointer, &dot_flox_path);

        // Provide a lock with rev that's in remote
        let input_lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: None,
            version: flox_core::Version::<1>,
        };

        let expected_lock = input_lock.clone();
        let lock =
            ensure_generation_locked(&remote_branch, &local_branch, &floxmeta, Some(input_lock))
                .unwrap();

        // Should fetch and find the rev in remote
        assert_eq!(lock, expected_lock);
    }

    /// Test that when ensure_generation_locked has input state of:
    /// - lock at {rev: nonexistent commit, local_rev: None}
    /// - floxmeta without local_branch
    /// - remote at commit 1
    ///
    /// It results in output state of:
    /// - error: RevDoesNotExist
    /// - [fetches from remote, but rev not found]
    #[test]
    fn test_ensure_generation_locked_no_local_branch_rev_not_in_remote() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");

        // Create floxmeta - no local branch exists
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);

        let dot_flox_dir = flox.temp_dir.join(".flox");
        fs::create_dir(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();
        let local_branch = branch_name(&test_pointer, &dot_flox_path);

        // Provide a lock with rev that doesn't exist
        let input_lock = GenerationLock {
            rev: "nonexistent_commit_hash".to_string(),
            local_rev: None,
            version: flox_core::Version::<1>,
        };

        let result =
            ensure_generation_locked(&remote_branch, &local_branch, &floxmeta, Some(input_lock));

        assert!(matches!(result, Err(FloxmetaBranchError::RevDoesNotExist)));
    }

    /// Test that ensure_branch is a no-op with input state:
    /// - branch at commit 1
    /// - lock at {rev: commit 1, local_rev: None}
    #[test]
    fn test_ensure_branch_noop() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&remote_branch).unwrap();

        // Create floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);

        let dot_flox_dir = flox.temp_dir.join(".flox");
        fs::create_dir(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();
        let local_branch = branch_name(&test_pointer, &dot_flox_path);

        // Create the branch at the correct commit
        floxmeta.git.create_branch(&local_branch, &hash_1).unwrap();

        let lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: None,
            version: Version::<1>,
        };

        // Should be a no-op
        ensure_branch(&local_branch, &lock, &floxmeta).unwrap();

        // Verify branch still at same commit
        let branch_hash = floxmeta.git.branch_hash(&local_branch).unwrap();
        assert_eq!(branch_hash, hash_1);
    }

    /// Test that with input state:
    /// - branch at commit 1
    /// - lock at {rev: commit 1, local_rev: commit 2}
    ///
    /// ensure_branch resets the branch to commit 2
    #[test]
    fn test_ensure_branch_resets_to_local_rev() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&remote_branch).unwrap();
        commit_file(&remote, "file 2");
        let hash_2 = remote.branch_hash(&remote_branch).unwrap();

        // Create floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);

        let dot_flox_dir = flox.temp_dir.join(".flox");
        fs::create_dir(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();
        let local_branch = branch_name(&test_pointer, &dot_flox_path);

        // Create branch at commit 1
        floxmeta.git.create_branch(&local_branch, &hash_1).unwrap();

        // Lock points to commit 2 via local_rev
        let lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: Some(hash_2.clone()),
            version: Version::<1>,
        };

        // Should reset branch to commit 2
        ensure_branch(&local_branch, &lock, &floxmeta).unwrap();

        // Verify branch now at commit 2
        let branch_hash = floxmeta.git.branch_hash(&local_branch).unwrap();
        assert_eq!(branch_hash, hash_2);
    }

    /// Test that with input state:
    /// - branch does not exist
    /// - lock at {rev: commit 1, local_rev: None}
    ///
    /// ensure_branch creates branch at commit 1
    #[test]
    fn test_ensure_branch_creates_new() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&remote_branch).unwrap();

        // Create floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);

        let dot_flox_dir = flox.temp_dir.join(".flox");
        fs::create_dir(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();
        let local_branch = branch_name(&test_pointer, &dot_flox_path);

        // Branch doesn't exist yet
        let lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: None,
            version: Version::<1>,
        };

        // Should create the branch
        ensure_branch(&local_branch, &lock, &floxmeta).unwrap();

        // Verify branch created at correct commit
        let branch_hash = floxmeta.git.branch_hash(&local_branch).unwrap();
        assert_eq!(branch_hash, hash_1);
    }

    /// Test that with input state:
    /// - branch at commit 1
    /// - lock at {rev: commit 2, local_rev: None}
    ///
    /// ensure_branch resets the branch to commit 2
    #[test]
    fn test_ensure_branch_resets_wrong_commit() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&remote_branch).unwrap();
        commit_file(&remote, "file 2");
        let hash_2 = remote.branch_hash(&remote_branch).unwrap();

        // Create floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);

        let dot_flox_dir = flox.temp_dir.join(".flox");
        fs::create_dir(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();
        let local_branch = branch_name(&test_pointer, &dot_flox_path);

        // Create branch at commit 1
        floxmeta.git.create_branch(&local_branch, &hash_1).unwrap();

        // Lock points to commit 2
        let lock = GenerationLock {
            rev: hash_2.clone(),
            local_rev: None,
            version: Version::<1>,
        };

        // Should reset branch to commit 2
        ensure_branch(&local_branch, &lock, &floxmeta).unwrap();

        // Verify branch now at commit 2
        let branch_hash = floxmeta.git.branch_hash(&local_branch).unwrap();
        assert_eq!(branch_hash, hash_2);
    }

    #[test]
    fn test_open_or_clone_opens_existing() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");

        // Clone the floxmeta (simulating it already exists locally)
        let _floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &branch);

        // Should open the existing floxmeta
        let result = open_or_clone_floxmeta(&flox, &test_pointer);
        assert!(result.is_ok());
    }

    #[test]
    fn test_open_or_clone_clones_new() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, _remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let branch = remote_branch_name(&test_pointer);
        remote.checkout(&branch, true).unwrap();
        commit_file(&remote, "file 1");

        // Don't create local floxmeta - should clone it
        let result = open_or_clone_floxmeta(&flox, &test_pointer);
        assert!(result.is_ok());

        // Verify it was cloned
        let floxmeta_path = floxmeta_dir(&flox, &test_pointer.owner);
        assert!(floxmeta_path.exists());
    }

    /// Test that FloxmetaBranch::new() completes successfully with existing repo
    #[test]
    fn test_new_complete_flow_existing_repo() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&remote_branch).unwrap();

        // Clone the floxmeta (simulating it already exists locally)
        let _existing_floxmeta =
            create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);

        // Create .flox directory
        let dot_flox_dir = flox.temp_dir.join("project").join(".flox");
        fs::create_dir_all(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();

        // Should successfully create FloxmetaBranch (no existing lock)
        let result = FloxmetaBranch::new(&flox, &test_pointer, &dot_flox_path, None);
        assert!(result.is_ok());

        let (branch_access, lock) = result.unwrap();
        let expected_branch = branch_name(&test_pointer, &dot_flox_path);
        assert_eq!(branch_access.branch, expected_branch);
        assert_eq!(lock.rev, hash_1);
        assert_eq!(lock.local_rev, None);
    }

    /// Test that FloxmetaBranch::new() completes successfully by cloning new repo
    #[test]
    fn test_new_complete_flow_clone_repo() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, _remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&remote_branch).unwrap();

        // Create the floxmeta parent directory so lock can be acquired
        let user_floxmeta_dir = floxmeta_dir(&flox, &test_pointer.owner);
        fs::create_dir_all(user_floxmeta_dir.parent().unwrap()).unwrap();

        // Create .flox directory
        let dot_flox_dir = flox.temp_dir.join("project").join(".flox");
        fs::create_dir_all(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();

        // Should successfully create FloxmetaBranch by cloning
        let result = FloxmetaBranch::new(&flox, &test_pointer, &dot_flox_path, None);
        assert!(result.is_ok(), "error: {:?}", result.unwrap_err());

        let (branch_access, lock) = result.unwrap();
        let expected_branch = branch_name(&test_pointer, &dot_flox_path);
        assert_eq!(branch_access.branch, expected_branch);
        assert_eq!(lock.rev, hash_1);
        assert_eq!(lock.local_rev, None);

        // Verify floxmeta was cloned
        let floxmeta_path = floxmeta_dir(&flox, &test_pointer.owner);
        assert!(floxmeta_path.exists());
    }

    /// Test that FloxmetaBranch::new() handles upstream not found error
    #[test]
    fn test_new_upstream_not_found() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote with a valid git repo but without the expected branch
        let (test_pointer, _remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        // Create a different branch name, not the one we're looking for
        remote.checkout("different_branch", true).unwrap();
        commit_file(&remote, "file 1");

        // Create the floxmeta parent directory so lock can be acquired
        let user_floxmeta_dir = floxmeta_dir(&flox, &test_pointer.owner);
        fs::create_dir_all(user_floxmeta_dir.parent().unwrap()).unwrap();

        // Create .flox directory
        let dot_flox_dir = flox.temp_dir.join("project").join(".flox");
        fs::create_dir_all(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();

        // Should fail with UpstreamNotFound (branch doesn't exist)
        let result = FloxmetaBranch::new(&flox, &test_pointer, &dot_flox_path, None);
        assert!(
            matches!(result, Err(FloxmetaBranchError::UpstreamNotFound { .. })),
            "got instead: {result:?}"
        );
    }

    /// Test that FloxmetaBranch::new() accepts existing lock and validates it
    #[test]
    fn test_new_with_existing_lock() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&remote_branch).unwrap();

        // Clone the floxmeta
        let _existing_floxmeta =
            create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);

        // Create .flox directory
        let dot_flox_dir = flox.temp_dir.join("project").join(".flox");
        fs::create_dir_all(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();

        // Provide an existing lock
        let input_lock = GenerationLock {
            rev: hash_1.clone(),
            local_rev: None,
            version: Version::<1>,
        };

        // Should successfully create FloxmetaBranch with existing lock
        let result = FloxmetaBranch::new(
            &flox,
            &test_pointer,
            &dot_flox_path,
            Some(input_lock.clone()),
        );
        assert!(result.is_ok());

        let (branch_access, lock) = result.unwrap();
        let expected_branch = branch_name(&test_pointer, &dot_flox_path);
        assert_eq!(branch_access.branch, expected_branch);
        assert_eq!(lock, input_lock);
    }

    /// Test that generation_lock() computes the correct lock from branch state
    #[test]
    fn test_generation_lock() {
        let (flox, _temp_dir_handle) = flox_instance();

        // Create a mock remote
        let (test_pointer, remote_path, remote) = create_mock_remote(flox.temp_dir.join("remote"));
        let remote_branch = remote_branch_name(&test_pointer);
        remote.checkout(&remote_branch, true).unwrap();
        commit_file(&remote, "file 1");
        let hash_1 = remote.branch_hash(&remote_branch).unwrap();

        // Create floxmeta
        let floxmeta = create_floxmeta(&flox, &remote_path, &test_pointer, &remote_branch);

        let dot_flox_dir = flox.temp_dir.join(".flox");
        fs::create_dir_all(&dot_flox_dir).unwrap();
        let dot_flox_path = CanonicalPath::new(dot_flox_dir).unwrap();
        let local_branch = branch_name(&test_pointer, &dot_flox_path);

        // Case 1: Local branch at same commit as remote - local_rev should be None
        floxmeta.git.create_branch(&local_branch, &hash_1).unwrap();

        let branch_access = FloxmetaBranch {
            floxmeta: floxmeta.clone(),
            branch: local_branch.clone(),
        };

        let lock = branch_access.generation_lock(&test_pointer).unwrap();
        assert_eq!(lock.rev, hash_1);
        assert_eq!(lock.local_rev, None);

        // Case 2: Local branch at different commit - local_rev should be set
        commit_file(&remote, "file 2");
        let hash_2 = remote.branch_hash(&remote_branch).unwrap();

        // Fetch the new commit into floxmeta
        floxmeta
            .git
            .fetch_ref("origin", &format!("+{}:{}", remote_branch, remote_branch))
            .unwrap();

        let lock2 = branch_access.generation_lock(&test_pointer).unwrap();
        assert_eq!(lock2.rev, hash_2);
        assert_eq!(lock2.local_rev, Some(hash_1));
    }
}
