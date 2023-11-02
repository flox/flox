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
    GitCommandOptions,
    GitCommandProvider,
};

pub const FLOXMETA_DIR_NAME: &str = "meta";
pub const DEFAULT_FLOXHUB_URL: &str = "https://git.hub.flox.dev";

#[derive(Debug)]
pub struct FloxmetaV2 {
    pub(super) git: GitCommandProvider,
}

#[derive(Error, Debug)]
pub enum FloxmetaV2Error {
    #[error("No login token provided")]
    LoggedOut,
    #[error("floxmeta for {0} not found")]
    NotFound(String),
    #[error("Currently only hub.flox.dev is supported as a remote")]
    UnsupportedRemote,
    #[error("Could not open user environment directory {0}")]
    Open(GitCommandOpenError),
    #[error("Failed to check for branch: {0}")]
    CheckForBranch(GitCommandBranchHashError),
    #[error("Failed to fetch environment: {0}")]
    FetchBranch(GitCommandError),
    #[error("Failed to clone environment: {0}")]
    CloneBranch(GitCommandError),
}

impl FloxmetaV2 {
    /// Clone the floxmeta repository for the given user to the given path
    ///
    /// Most of the time, you want to use [`FloxmetaV2::clone`] instead.
    /// This is useful for testing and isolated remote operations.
    pub fn clone_to(
        flox: &Flox,
        pointer: &ManagedPointer,
        path: impl AsRef<Path>,
    ) -> Result<Self, FloxmetaV2Error> {
        let token = flox
            .floxhub_token
            .as_ref()
            .ok_or(FloxmetaV2Error::LoggedOut)?;

        let git_options = floxmeta_git_options(DEFAULT_FLOXHUB_URL, token);
        let branch: String = remote_branch_name(&flox.system, pointer);

        let git = GitCommandProvider::clone_branch_with(
            git_options,
            DEFAULT_FLOXHUB_URL,
            path,
            branch,
            true,
        )
        .map_err(FloxmetaV2Error::CloneBranch)?;

        Ok(FloxmetaV2 { git })
    }

    /// Clone the floxmeta repository for the given user to the default path
    ///
    /// Like [`FloxmetaV2::clone_to`], but uses the system path for floxmeta repositories in XDG_DATA_HOME
    pub fn clone(flox: &Flox, pointer: &ManagedPointer) -> Result<Self, FloxmetaV2Error> {
        Self::clone_to(flox, pointer, floxmeta_dir(flox, &pointer.owner))
    }

    /// Open the floxmeta repository for the given user
    /// and ensure a branch exists for the given environment.
    pub fn open(flox: &Flox, pointer: &ManagedPointer) -> Result<Self, FloxmetaV2Error> {
        let token = flox
            .floxhub_token
            .as_ref()
            .ok_or(FloxmetaV2Error::LoggedOut)?;

        let git_options = floxmeta_git_options(DEFAULT_FLOXHUB_URL, token);

        let user_floxmeta_dir = floxmeta_dir(flox, &pointer.owner);

        if !user_floxmeta_dir.exists() {
            Err(FloxmetaV2Error::NotFound(pointer.owner.to_string()))?
        }

        let git = GitCommandProvider::open_with(git_options, user_floxmeta_dir)
            .map_err(FloxmetaV2Error::Open)?;
        let branch: String = remote_branch_name(&flox.system, pointer);
        if !git
            .has_branch(&branch)
            .map_err(FloxmetaV2Error::CheckForBranch)?
        {
            git.fetch_branch("origin", &branch)
                .map_err(FloxmetaV2Error::FetchBranch)?;
        }

        Ok(FloxmetaV2 { git })
    }
}

/// Returns the git options for interacting with floxmeta repositories
// todo: move floxhub host and token to Flox, or integrate config...
fn floxmeta_git_options(floxhub_host: &str, floxhub_token: &str) -> GitCommandOptions {
    let mut options = GitCommandOptions::default();

    // set the user config
    // todo: eventually use the user's name and email once integrated with floxhub
    options.add_config_flag("user.name", "Flox User");
    options.add_config_flag("user.email", "floxuser@example.invalid");

    // unset the global and system config
    options.add_env_var("GIT_CONFIG_GLOBAL", "/dev/null");
    options.add_env_var("GIT_CONFIG_SYSTEM", "/dev/null");

    // Set authentication with the floxhub token using an inline credential helper.
    // The credential helper should help avoinding a leak of the token in the process list.
    options.add_env_var("FLOX_FLOXHUB_TOKEN", floxhub_token);
    options.add_config_flag(
        &format!("credentials.{floxhub_host}.helper"),
        "!f(){ echo ${FLOX_FLOXHUB_TOKEN}; }; f",
    );

    options
}

pub(super) fn floxmeta_dir(flox: &Flox, owner: &EnvironmentOwner) -> PathBuf {
    flox.data_dir
        .join(FLOXMETA_DIR_NAME)
        .join(owner.to_string())
}
