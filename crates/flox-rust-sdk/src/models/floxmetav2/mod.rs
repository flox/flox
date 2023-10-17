use std::path::{Path, PathBuf};

use thiserror::Error;

use super::environment::managed_environment::remote_branch_name;
use super::environment::ManagedPointer;
use super::environment_ref::EnvironmentOwner;
use crate::flox::Flox;
use crate::providers::git::{
    GitCommandBranchHashError,
    GitCommandError,
    GitCommandOpenError,
    GitCommandProvider,
};

pub const FLOXMETA_DIR_NAME: &str = "meta";

#[derive(Debug)]
pub struct FloxmetaV2 {
    pub(super) git: GitCommandProvider,
}

#[derive(Error, Debug)]
pub enum FloxmetaV2Error {
    #[error("Currently only hub.flox.dev is supported as a remote")]
    UnsupportedRemote,
    #[error("Could not open user environment directory {0}")]
    Open(GitCommandOpenError),
    #[error("Failed to check for branch: {0}")]
    CheckForBranch(GitCommandBranchHashError),
    #[error("Failed to fetch environment: {0}")]
    FetchBranch(GitCommandError),
}

impl FloxmetaV2 {
    fn open_path(path: impl AsRef<Path>) -> Result<Self, FloxmetaV2Error> {
        let git = GitCommandProvider::open(path).map_err(FloxmetaV2Error::Open)?;
        Ok(FloxmetaV2 { git })
    }

    fn clone_in(
        _path: impl AsRef<Path>,
        _pointer: &ManagedPointer,
    ) -> Result<Self, FloxmetaV2Error> {
        todo!()
    }

    pub fn open(flox: &Flox, pointer: &ManagedPointer) -> Result<Self, FloxmetaV2Error> {
        let user_floxmeta_dir = floxmeta_dir(flox, &pointer.owner);
        if user_floxmeta_dir.exists() {
            let floxmeta = FloxmetaV2::open_path(user_floxmeta_dir)?;
            let branch = remote_branch_name(&flox.system, pointer);
            if !floxmeta
                .git
                .has_branch(&branch)
                .map_err(FloxmetaV2Error::CheckForBranch)?
            {
                floxmeta
                    .git
                    .fetch_branch("origin", &branch)
                    .map_err(FloxmetaV2Error::FetchBranch)?;
            }
            Ok(floxmeta)
        } else {
            FloxmetaV2::clone_in(user_floxmeta_dir, pointer)
        }
    }
}

pub(super) fn floxmeta_dir(flox: &Flox, owner: &EnvironmentOwner) -> PathBuf {
    flox.data_dir
        .join(FLOXMETA_DIR_NAME)
        .join(owner.to_string())
}
