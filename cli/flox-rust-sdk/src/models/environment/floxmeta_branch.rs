use std::path::Path;

use fslock::LockFile;
use thiserror::Error;

use crate::models::floxmeta::FloxMeta;

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
