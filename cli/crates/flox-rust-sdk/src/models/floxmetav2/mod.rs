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
    GitProvider,
};

pub const FLOXMETA_DIR_NAME: &str = "meta";

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
        path: impl AsRef<Path>,
        flox: &Flox,
        pointer: &ManagedPointer,
    ) -> Result<Self, FloxmetaV2Error> {
        let host = flox.floxhub_host.as_str();
        let token = flox
            .floxhub_token
            .as_ref()
            .ok_or(FloxmetaV2Error::LoggedOut)?;

        let git_options = floxmeta_git_options(&flox.floxhub_host, token);
        let branch = remote_branch_name(&flox.system, pointer);

        let git = GitCommandProvider::clone_branch_with(
            git_options,
            format!("{host}/{}/floxmeta", pointer.owner),
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
        Self::clone_to(floxmeta_dir(flox, &pointer.owner), flox, pointer)
    }

    /// Open a floxmeta repository at a given path
    /// and ensure a branch exists for a given environment.
    ///
    /// This is useful for testing and isolated remote operations.
    /// Branch name, token and host are however still derived from the environment pointer
    /// and metadata provided by the flox reference.
    /// Ideally, these could be passed as parameters.
    ///
    /// In most cases, you want to use [`FloxmetaV2::open`] instead which provides the flox defaults.
    pub fn open_at(
        user_floxmeta_dir: impl AsRef<Path>,
        flox: &Flox,
        pointer: &ManagedPointer,
    ) -> Result<Self, FloxmetaV2Error> {
        let token = flox
            .floxhub_token
            .as_ref()
            .ok_or(FloxmetaV2Error::LoggedOut)?;

        let git_options = floxmeta_git_options(&flox.floxhub_host, token);

        if !user_floxmeta_dir.as_ref().exists() {
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

    /// Open a floxmeta repository for a given user
    ///
    /// Like [`FloxmetaV2::open_at`], but uses the system path for floxmeta repositories in XDG_DATA_HOME.
    pub fn open(flox: &Flox, pointer: &ManagedPointer) -> Result<Self, FloxmetaV2Error> {
        let user_floxmeta_dir = floxmeta_dir(flox, &pointer.owner);
        Self::open_at(user_floxmeta_dir, flox, pointer)
    }

    pub fn new_in(
        user_floxmeta_dir: impl AsRef<Path>,
        flox: &Flox,
        pointer: &ManagedPointer,
    ) -> Result<Self, FloxmetaV2Error> {
        let token = flox
            .floxhub_token
            .as_ref()
            .ok_or(FloxmetaV2Error::LoggedOut)?;

        let git_options = floxmeta_git_options(&flox.floxhub_host, token);

        let git = GitCommandProvider::init_with(git_options, user_floxmeta_dir, false).unwrap();
        git.rename_branch(&remote_branch_name(&flox.system, pointer))
            .unwrap();

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
        &format!("credential.{floxhub_host}.helper"),
        r#"!f(){ echo "username=oauth"; echo "password=$FLOX_FLOXHUB_TOKEN"; }; f"#,
    );

    options
}

pub(super) fn floxmeta_dir(flox: &Flox, owner: &EnvironmentOwner) -> PathBuf {
    flox.data_dir
        .join(FLOXMETA_DIR_NAME)
        .join(owner.to_string())
}

#[cfg(test)]
#[cfg(feature = "impure-unit-tests")]
mod tests {
    use std::fs;
    use std::str::FromStr;

    use super::*;
    use crate::flox::tests::flox_instance;
    use crate::flox::EnvironmentName;
    use crate::providers::git::GitProvider;

    /// Create an upstream floxmeta repository with an environment under a given base path
    fn create_fake_floxmeta(
        floxhub_base_path: &Path,
        flox: &Flox,
        pointer: &ManagedPointer,
    ) -> GitCommandProvider {
        let floxmeta_path = floxhub_base_path.join(format!("{}/floxmeta", pointer.owner));
        fs::create_dir_all(&floxmeta_path).unwrap();
        let git = GitCommandProvider::init(floxmeta_path, false).unwrap();
        git.rename_branch(&remote_branch_name(&flox.system, pointer))
            .unwrap();
        fs::write(git.path().join("test.txt"), "test").unwrap();
        git.add(&[Path::new("test.txt")]).unwrap();
        git.commit("test").unwrap();
        git
    }

    /// Test whether a floxmeta repository can be successfully cloned
    /// from a given floxhub host (here a git file:// url pointing to a fake floxmeta repo)
    /// and opened from an existing clone.
    #[test]
    fn clone_repo() {
        let _ = env_logger::try_init();

        let (mut flox, tempdir) = flox_instance();

        let pointer = ManagedPointer::new("floxtest".parse().unwrap(), "test".parse().unwrap());
        let source_path = tempdir.path().join("source");

        flox.floxhub_token = Some("no token needed here".to_string());
        flox.floxhub_host = format!("file://{}", source_path.to_string_lossy());

        create_fake_floxmeta(&source_path, &flox, &pointer);

        FloxmetaV2::clone_to(tempdir.path().join("dest"), &flox, &pointer)
            .expect("Cloning a floxmeta repo should succeed");
        FloxmetaV2::open_at(tempdir.path().join("dest"), &flox, &pointer)
            .expect("Opening a floxmeta repo should succeed");
    }

    /// Test whether a floxmeta repository can be successfully cloned from floxhub
    /// and other branches are fetched lazily when opened.
    ///
    /// Finally, verify that non-existent environments correctly fail to be opened.
    ///
    /// Uses the environments `floxtest/default` and `floxtest/nondefault`
    /// which are prepared on the host `https://git.hub.flox.dev`.
    /// Tries to authenticate with a test token that grants access to floxtest.
    #[test]
    fn clone_from_floxhub() {
        let _ = env_logger::try_init();

        let (mut flox, _) = flox_instance();

        let pointer = ManagedPointer::new(
            EnvironmentOwner::from_str("floxtest").unwrap(),
            EnvironmentName::from_str("default").unwrap(),
        );

        flox.floxhub_token = Some("flox_testOAuthToken".to_string());
        flox.floxhub_host = "https://git.hub.flox.dev".to_string();

        FloxmetaV2::clone(&flox, &pointer)
            .expect("Cloning a floxmeta repo from floxhub should succeed");

        let pointer_other_success = ManagedPointer::new(
            EnvironmentOwner::from_str("floxtest").unwrap(),
            EnvironmentName::from_str("nondefault").unwrap(),
        );

        FloxmetaV2::open(&flox, &pointer_other_success)
            .expect("Should pull other branch 'nondefault' from floxhub");

        let pointer_other_failure = ManagedPointer::new(
            EnvironmentOwner::from_str("floxtest").unwrap(),
            EnvironmentName::from_str("nonexistent").unwrap(),
        );

        FloxmetaV2::open(&flox, &pointer_other_failure)
            .expect_err("Should fail pulling branch 'nonexistent' from floxhub");
    }
}
