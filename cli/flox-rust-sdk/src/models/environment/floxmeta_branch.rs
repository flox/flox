use std::path::Path;

use fslock::LockFile;
use thiserror::Error;

use crate::models::floxmeta::FloxMeta;

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

#[cfg(test)]
mod tests {
    use super::*;
}
