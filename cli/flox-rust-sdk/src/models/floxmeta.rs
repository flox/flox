use std::path::{Path, PathBuf};

use flox_core::data::environment_ref::EnvironmentOwner;
use thiserror::Error;
use tracing::{debug, instrument};
use url::Url;
use uuid::Uuid;

use super::environment::ManagedPointer;
use crate::flox::{Flox, Floxhub, FloxhubError, FloxhubToken};
use crate::models::environment::floxmeta_branch::remote_branch_name;
use crate::providers::git::{
    GitCommandBranchHashError,
    GitCommandError,
    GitCommandOpenError,
    GitCommandOptions,
    GitCommandProvider,
    GitProvider,
    GitRemoteCommandError,
};
use crate::utils::{HEADER_DEVICE_UUID, HEADER_INVOCATION_SOURCE, INVOCATION_SOURCES};

pub const FLOXMETA_DIR_NAME: &str = "meta";
pub const BRANCH_NAME_PATH_SEPARATOR: &str = ".";
pub const FLOXHUB_TOKEN_ENV_VAR: &str = "FLOX_FLOXHUB_TOKEN";

#[derive(Debug, Clone)]
pub struct FloxMeta {
    pub(super) git: GitCommandProvider,
}

#[derive(Error, Debug)]
pub enum FloxMetaError {
    #[error("floxmeta for {0} not found")]
    NotFound(String),
    #[error("Could not open user environment directory {0}")]
    Open(GitCommandOpenError),

    #[error("Failed to check for branch: {0}")]
    CheckForBranch(GitCommandBranchHashError),
    #[error("Failed to fetch environment: {0}")]
    FetchBranch(GitRemoteCommandError),
    #[error("Failed to clone environment: {0}")]
    CloneBranch(GitRemoteCommandError),
    #[error("Failed to list branches: {0}")]
    ListBranch(GitCommandError),
    #[error("Failed to delete branch: {0}")]
    DeleteBranch(GitCommandError),

    #[error(transparent)]
    FloxhubError(FloxhubError),
}

impl FloxMeta {
    /// Clone the floxmeta repository for the given user to the given path
    ///
    /// If access to a remote repository requires authentication,
    /// the FloxHub token must be set in the flox instance.
    /// The caller is responsible for ensuring that the token is present and valid.
    ///
    /// Most of the time, you want to use [`FloxmetaV2::clone`] instead.
    /// This is useful for testing and isolated remote operations.
    pub fn clone_to(
        path: impl AsRef<Path>,
        flox: &Flox,
        pointer: &ManagedPointer,
    ) -> Result<Self, FloxMetaError> {
        let token = flox.floxhub_token.as_ref();

        let floxhub = Floxhub::new(
            pointer.floxhub_base_url.to_owned(),
            pointer.floxhub_git_url_override.clone(),
        )
        .map_err(FloxMetaError::FloxhubError)?;

        let git_url = floxhub.git_url();

        let git_options = floxmeta_git_options(
            git_url,
            &pointer.owner,
            token,
            flox.metrics_device_uuid.as_ref(),
        );
        let branch = remote_branch_name(pointer);

        let git = GitCommandProvider::clone_branch_with(
            git_options,
            format!("{}/{}/floxmeta", git_url, pointer.owner),
            path,
            branch,
            true,
        )
        .map_err(FloxMetaError::CloneBranch)?;

        Ok(FloxMeta { git })
    }

    /// Clone the floxmeta repository for the given user to the default path
    ///
    /// If access to a remote repository requires authentication,
    /// the FloxHub token must be set in the flox instance.
    /// The caller is responsible for ensuring that the token is present and valid.
    ///
    /// Like [`FloxmetaV2::clone_to`], but uses the system path for floxmeta repositories in XDG_DATA_HOME
    #[instrument(skip(flox), fields(progress = format!("Retrieving environment metadata for {}/{}", pointer.owner, pointer.name)))]
    pub fn clone(flox: &Flox, pointer: &ManagedPointer) -> Result<Self, FloxMetaError> {
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
    /// If access to a remote repository requires authentication,
    /// the FloxHub token must be set in the flox instance.
    /// The caller is responsible for ensuring that the token is present and valid.
    ///
    /// In most cases, you want to use [`FloxmetaV2::open`] instead which provides the flox defaults.
    pub fn open_at(
        user_floxmeta_dir: impl AsRef<Path>,
        flox: &Flox,
        pointer: &ManagedPointer,
    ) -> Result<Self, FloxMetaError> {
        let token = flox.floxhub_token.as_ref();

        let floxhub = Floxhub::new(
            pointer.floxhub_base_url.to_owned(),
            pointer.floxhub_git_url_override.clone(),
        )
        .map_err(FloxMetaError::FloxhubError)?;

        let git_url = floxhub.git_url();

        let git_options = floxmeta_git_options(
            git_url,
            &pointer.owner,
            token,
            flox.metrics_device_uuid.as_ref(),
        );

        if !user_floxmeta_dir.as_ref().exists() {
            Err(FloxMetaError::NotFound(pointer.owner.to_string()))?
        }

        let git = GitCommandProvider::open_with(git_options, user_floxmeta_dir)
            .map_err(FloxMetaError::Open)?;
        let branch: String = remote_branch_name(pointer);
        if !git
            .has_branch(&branch)
            .map_err(FloxMetaError::CheckForBranch)?
        {
            git.fetch_branch("dynamicorigin", &branch)
                .map_err(FloxMetaError::FetchBranch)?;
        }

        Ok(FloxMeta { git })
    }

    /// Open a floxmeta repository for a given user
    ///
    /// Like [`FloxmetaV2::open_at`], but uses the system path for floxmeta repositories in XDG_DATA_HOME.
    #[instrument(skip(flox), fields(progress = format!("Updating environment metadata for {}/{}", pointer.owner, pointer.name)))]
    pub fn open(flox: &Flox, pointer: &ManagedPointer) -> Result<Self, FloxMetaError> {
        let user_floxmeta_dir = floxmeta_dir(flox, &pointer.owner);
        Self::open_at(user_floxmeta_dir, flox, pointer)
    }

    pub fn new_in(
        user_floxmeta_dir: impl AsRef<Path>,
        flox: &Flox,
        pointer: &ManagedPointer,
    ) -> Result<Self, FloxMetaError> {
        let token = flox.floxhub_token.as_ref();

        let floxhub = Floxhub::new(
            pointer.floxhub_base_url.to_owned(),
            pointer.floxhub_git_url_override.clone(),
        )
        .map_err(FloxMetaError::FloxhubError)?;

        let git_url = floxhub.git_url();

        let git_options = floxmeta_git_options(
            git_url,
            &pointer.owner,
            token,
            flox.metrics_device_uuid.as_ref(),
        );

        let git = GitCommandProvider::init_with(git_options, user_floxmeta_dir, false).unwrap();
        git.rename_branch(&remote_branch_name(pointer)).unwrap();

        Ok(FloxMeta { git })
    }
}

/// Returns the git options for interacting with floxmeta repositories
///
/// * Disable global and system config
///   to avoid user config interfering with flox operations
/// * Set required user config (name and email)
/// * Configure a dynamic origin for the FloxHub repository
///   to allow cloning and fetching from different FloxHub hosts per user.
///   The FloxHub host is derived from the FloxHub url in the environment pointer.
/// * Set authentication with the FloxHub token using an inline credential helper
///   if a token is provided.
pub fn floxmeta_git_options(
    floxhub_git_url: &Url,
    floxhub_owner: &str,
    floxhub_token: Option<&FloxhubToken>,
    metrics_device_uuid: Option<&Uuid>,
) -> GitCommandOptions {
    let mut options = GitCommandOptions::default();

    // set the user config
    // todo: eventually use the user's name and email once integrated with FloxHub
    options.add_config_flag("user.name", "Flox User");
    options.add_config_flag("user.email", "floxuser@example.invalid");

    // unset the global and system config
    options.add_env_var("GIT_CONFIG_GLOBAL", "/dev/null");
    options.add_env_var("GIT_CONFIG_SYSTEM", "/dev/null");

    // provides a "dynamic" remote "dynamicorigin".
    //
    // either the FloxHub url from the environment pointer
    // or the default FloxHub url if the current operation does not operate on a managed environment.
    //
    // Local floxmeta repositories may contain environments from different FloxHub hosts.
    // The dynamic origin allows to fetch from different FloxHub hosts per environment
    // and reduces the amount of state stored in the local floxmeta repository.
    options.add_config_flag(
        "remote.dynamicorigin.url",
        format!("{floxhub_git_url}/{floxhub_owner}/floxmeta"),
    );

    let token = if let Some(token) = floxhub_token {
        if token.is_expired() {
            debug!("FloxHub token is expired, sending for identification");
        } else {
            debug!("using valid FloxHub token");
        }
        token.secret()
    } else {
        debug!("no FloxHub token configured");
        ""
    };

    // Set authentication with the FloxHub token using an inline credential helper.
    // The credential helper should help avoiding a leak of the token in the process list.
    //
    // If no token is provided, we still set the credential helper and pass an empty string as password
    // to enforce authentication failures and avoid falling back to pinentry
    options.add_env_var(FLOXHUB_TOKEN_ENV_VAR, token);
    options.add_config_flag(
        &format!("credential.{floxhub_git_url}.helper"),
        format!(r#"!f(){{ echo "username=oauth"; echo "password=${FLOXHUB_TOKEN_ENV_VAR}"; }}; f"#),
    );

    if let Some(uuid) = metrics_device_uuid {
        options.add_http_header(HEADER_DEVICE_UUID, &uuid.to_string());
    }

    if !INVOCATION_SOURCES.is_empty() {
        options.add_http_header(HEADER_INVOCATION_SOURCE, &INVOCATION_SOURCES.join(","));
    }

    options
}

pub(super) fn floxmeta_dir(flox: &Flox, owner: &EnvironmentOwner) -> PathBuf {
    flox.data_dir
        .join(FLOXMETA_DIR_NAME)
        .join(owner.to_string())
}

#[cfg(test)]
mod header_tests {
    use httpmock::MockServer;
    use url::Url;
    use uuid::Uuid;

    use super::*;
    use crate::flox::FLOX_VERSION;

    /// Issue a git clone against the mock server using the given options.
    /// The mock returns 403, so git stops after one request. We assert the
    /// clone fails to prove the mock was actually hit.
    fn clone_against_mock(server: &MockServer, owner: &str, options: GitCommandOptions) {
        let result = GitCommandProvider::clone_branch_with(
            options,
            format!("{}/{owner}/floxmeta", server.base_url()),
            tempfile::tempdir().unwrap().path(),
            "main",
            true,
        );
        assert!(
            matches!(result, Err(GitRemoteCommandError::Command(_))),
            "Expected clone to fail with a command error, got: {result:?}"
        );
    }

    #[test]
    fn git_request_includes_telemetry_headers() {
        let source = "test.mock-source";
        temp_env::with_var("FLOX_INVOCATION_SOURCE", Some(source), || {
            let server = MockServer::start();
            let uuid = Uuid::nil();

            let mock = server.mock(|when, then| {
                when.header(HEADER_DEVICE_UUID, uuid.to_string())
                    .header_includes(HEADER_INVOCATION_SOURCE, source)
                    .header("user-agent", format!("flox-cli/{}", &*FLOX_VERSION));
                then.status(403);
            });

            let owner = "testowner";
            let server_url = Url::parse(&server.base_url()).unwrap();
            let options = floxmeta_git_options(&server_url, owner, None, Some(&uuid));
            clone_against_mock(&server, owner, options);
            mock.assert();
        });
    }

    #[test]
    fn git_request_includes_telemetry_headers_without_device_uuid() {
        let source = "test.mock-source";
        temp_env::with_var("FLOX_INVOCATION_SOURCE", Some(source), || {
            let server = MockServer::start();
            let uuid = None;

            let mock = server.mock(|when, then| {
                when.header_missing(HEADER_DEVICE_UUID)
                    .header_includes(HEADER_INVOCATION_SOURCE, source)
                    .header("user-agent", format!("flox-cli/{}", &*FLOX_VERSION));
                then.status(403);
            });

            let owner = "testowner";
            let server_url = Url::parse(&server.base_url()).unwrap();
            let options = floxmeta_git_options(&server_url, owner, None, uuid);
            clone_against_mock(&server, owner, options);
            mock.assert();
        });
    }
}

#[cfg(test)]
#[cfg(feature = "impure-unit-tests")]
mod tests {
    use std::fs;

    use super::*;
    use crate::flox::DEFAULT_FLOXHUB_URL;
    use crate::flox::test_helpers::flox_instance;
    use crate::providers::git::GitProvider;

    /// Create an upstream floxmeta repository with an environment under a given base path
    fn create_fake_floxmeta(
        floxhub_base_path: &Path,
        _flox: &Flox,
        pointer: &ManagedPointer,
    ) -> GitCommandProvider {
        let floxmeta_path = floxhub_base_path.join(format!("{}/floxmeta", pointer.owner));
        fs::create_dir_all(&floxmeta_path).unwrap();
        let git = GitCommandProvider::init(floxmeta_path, false).unwrap();
        git.rename_branch(&remote_branch_name(pointer)).unwrap();
        fs::write(git.path().join("test.txt"), "test").unwrap();
        git.add(&[Path::new("test.txt")]).unwrap();
        git.commit("test").unwrap();
        git
    }

    /// Test whether a floxmeta repository can be successfully cloned
    /// from a given FloxHub host (here a git file:// url pointing to a fake floxmeta repo)
    /// and opened from an existing clone.
    #[test]
    fn clone_repo() {
        let (flox, tempdir) = flox_instance();
        let source_path = tempdir.path().join("source");

        let floxhub = Floxhub::new(
            DEFAULT_FLOXHUB_URL.clone(),
            Some(Url::from_directory_path(&source_path).unwrap()),
        )
        .unwrap();

        let pointer = ManagedPointer::new(
            "floxtest".parse().unwrap(),
            "test".parse().unwrap(),
            &floxhub,
        );

        create_fake_floxmeta(&source_path, &flox, &pointer);

        FloxMeta::clone_to(tempdir.path().join("dest"), &flox, &pointer)
            .expect("Cloning a floxmeta repo should succeed");
        FloxMeta::open_at(tempdir.path().join("dest"), &flox, &pointer)
            .expect("Opening a floxmeta repo should succeed");
    }
}
