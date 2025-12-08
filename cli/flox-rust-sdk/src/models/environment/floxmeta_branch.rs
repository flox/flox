use std::path::Path;

use fslock::LockFile;
use thiserror::Error;
use tracing::debug;

use super::{ManagedPointer, path_hash};
use crate::data::CanonicalPath;
use crate::flox::{Flox, RemoteEnvironmentRef};
use crate::models::floxmeta::{BRANCH_NAME_PATH_SEPARATOR, FloxMeta, FloxMetaError};
use crate::providers::git::GitRemoteCommandError;

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
}

impl FloxmetaBranch {}

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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::str::FromStr;

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
}
