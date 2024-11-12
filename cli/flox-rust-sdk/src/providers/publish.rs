use std::error;
use std::str::FromStr;

use catalog_api_v1::types::{Output, Outputs, SystemEnum};
use chrono::{DateTime, Utc};
use flox_core::canonical_path::CanonicalPath;
use thiserror::Error;

use super::build::ManifestBuilder;
use super::catalog::{Client, ClientTrait, UserBuildInfo, UserDerivationInfo};
use super::git::GitCommandProvider;
use crate::flox::{Flox, FloxhubToken};
use crate::models::environment::managed_environment::ManagedEnvironment;
use crate::models::environment::path_environment::PathEnvironment;
use crate::models::environment::Environment;
use crate::models::lockfile::Lockfile;
use crate::providers::git::GitProvider;

pub enum PublishEnvironment {
    Path(PathEnvironment),
    Managed(ManagedEnvironment),
}

#[derive(Debug, Error)]
pub enum PublishError {
    #[error("This type of environment is not supported for publishing")]
    UnsupportedEnvironment,
    #[error("The environment must be locked to publish")]
    UnlockedEnvironment,

    #[error("The outputs from the build could not be identified")]
    UnknownOutputs(#[source] Box<dyn error::Error>),

    #[error("The environment is in an unsupported state for publishing")]
    UnsupportEnvironmentState(#[source] Box<dyn error::Error>),

    #[error("There was an error communicating with the catalog")]
    CatalogError(#[source] Box<dyn error::Error>),

    #[error("Could not identify user from authentication info")]
    Unauthenticated,
}

/// The `Publish` trait describes the high level behavior of publishing a package to a catalog.
/// Authentication, upload, builds etc, are implementation details of the specific provider.
/// Modeling the behavior as a trait allows us to swap out the provider, e.g. a mock for testing.
#[allow(async_fn_in_trait)]
pub trait Publish {
    async fn publish(
        &self,
        client: &Client,
        floxhub_token: &FloxhubToken,
    ) -> Result<(), PublishError>;
}

/// Simple struct to hold the information of a locked URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedUrlInfo {
    pub url: String,
    pub rev: String,
    pub rev_count: u64,
    pub rev_date: Option<DateTime<Utc>>,
}

/// Ensures that the required metadata for publishing is consistent from the environment
#[allow(clippy::manual_non_exhaustive)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedEnvironmentMetadata {
    // There may or may not be a locked base catalog reference in the environment
    pub base_catalog_ref: Option<LockedUrlInfo>,
    // The build repo reference is always present
    pub build_repo_ref: LockedUrlInfo,

    // This field isn't "pub", so no one outside this module can construct this struct. That helps
    // ensure that we can only make this struct as a result of doing the "right thing."
    _private: (),
}

/// Ensures that the required metadata for publishing is consistent from the build process
#[allow(clippy::manual_non_exhaustive)]
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedBuildMetadata {
    // Define metadata coming from the build, e.g. outpaths
    pub package: String,
    pub outputs: Vec<catalog_api_v1::types::Output>,
    pub drv_path: String,
    pub system: SystemEnum,

    // This field isn't "pub", so no one outside this module can construct this struct. That helps
    // ensure that we can only make this struct as a result of doing the "right thing."
    _private: (),
}

/// The `PublishProvider` is a concrete implementation of the `Publish` trait.
/// It is responsible for the actual implementation of the `Publish` trait,
/// i.e. the actual publishing of a package to a catalog.
///
/// The `PublishProvider` is a generic struct, parameterized by a `Builder` type,
/// to build packages before publishing.
pub struct PublishProvider<Builder> {
    env_meta: CheckedEnvironmentMetadata,
    build_meta: CheckedBuildMetadata,

    _builder: Option<Builder>,
}

/// (default) implementation of the `Publish` trait, i.e. the publish interface to publish.
impl<Builder> Publish for PublishProvider<&Builder>
where
    Builder: ManifestBuilder,
{
    async fn publish(
        &self,
        client: &Client,
        floxhub_token: &FloxhubToken,
    ) -> Result<(), PublishError> {
        // Get metadata from the environment, like locked URLs.

        let catalog_name = floxhub_token.handle().to_string();

        // The create package service call will create the user's own catalog
        // if not already created, and then create (or return) the package noted
        // returning either a 200 or 201.  Either is ok here, as long as it's not an error.
        client
            .create_package(&catalog_name, &self.build_meta.package)
            .await
            .map_err(|e| PublishError::CatalogError(Box::new(e)))?;

        let outputs = Outputs(
            self.build_meta
                .outputs
                .clone()
                .into_iter()
                .map(|o| Output {
                    name: o.name,
                    store_path: o.store_path,
                })
                .collect(),
        );

        let build_info = UserBuildInfo {
            derivation: UserDerivationInfo {
                broken: Some(false),
                description: "".to_string(),
                drv_path: self.build_meta.drv_path.clone(),
                license: None,
                name: self.build_meta.package.to_string().to_owned(),
                outputs,
                outputs_to_install: None,
                pname: Some(self.build_meta.package.to_string()),
                system: self.build_meta.system,
                unfree: None,
                version: None,
            },
            locked_base_catalog_url: Some(self.env_meta.build_repo_ref.url.clone()),
            url: self.env_meta.build_repo_ref.url.clone(),
            rev: self.env_meta.build_repo_ref.rev.clone(),
            rev_count: self.env_meta.build_repo_ref.rev_count as i64,
            rev_date: self.env_meta.build_repo_ref.rev_date.unwrap(),
        };
        client
            .publish_build(&catalog_name, &self.build_meta.package, &build_info)
            .await
            .map_err(|e| PublishError::CatalogError(Box::new(e)))?;

        Ok(())
    }
}

/// Collect metadata needed for publishing that is obtained from the build output
pub fn check_build_metadata(
    env: &PathEnvironment,
    pkg: &str,
    system: &str,
) -> Result<CheckedBuildMetadata, PublishError> {
    // For now assume the build is successful, and present.
    // Look for the output from the build at `results-<pkgname>`
    // Note that the current builds only support a single output at that
    // pre-defined path.  Later work will get structured results from the build
    // process to feed this.
    // See tests in build.rs for examples

    let system = SystemEnum::from_str(system)
        .map_err(|e| PublishError::UnsupportEnvironmentState(Box::new(e)))?;

    let result_dir = env
        .parent_path()
        .map_err(|e| PublishError::UnsupportEnvironmentState(Box::new(e)))?
        .join(format!("result-{pkg}"));
    let store_dir = result_dir
        .read_link()
        .map_err(|e| PublishError::UnknownOutputs(Box::new(e)))?;

    Ok(CheckedBuildMetadata {
        drv_path: store_dir.to_string_lossy().to_string(),
        outputs: vec![catalog_api_v1::types::Output {
            name: "bin".to_string(),
            store_path: store_dir.to_string_lossy().into_owned(),
        }],
        system,
        package: pkg.to_string(),
        _private: (),
    })
}

fn gather_build_repo_meta(environment: &PathEnvironment) -> Result<LockedUrlInfo, PublishError> {
    // Gather build repo info
    let git = match environment.parent_path() {
        Ok(env_path) => GitCommandProvider::discover(env_path)
            .map_err(|e| PublishError::UnsupportEnvironmentState(Box::new(e)))?,
        Err(e) => return Err(PublishError::UnsupportEnvironmentState(Box::new(e))),
    };

    let origin = git
        .get_origin()
        .map_err(|e| PublishError::UnsupportEnvironmentState(Box::new(e)))?;

    let rev = origin
        .revision
        .ok_or(PublishError::UnsupportEnvironmentState(
            "No revision found".to_string().into(),
        ))?;

    let rev_count = git
        .rev_count(rev.as_str())
        .map_err(|e| PublishError::UnsupportEnvironmentState(Box::new(e)))?;

    Ok(LockedUrlInfo {
        url: origin.url,
        rev,
        rev_count,
        rev_date: None,
    })
}

fn gather_base_repo_meta(
    flox: &Flox,
    environment: &PathEnvironment,
) -> Result<Option<LockedUrlInfo>, PublishError> {
    // Gather locked base catalog page info
    let lockfile_path = CanonicalPath::new(
        environment
            .lockfile_path(flox)
            .map_err(|e| PublishError::UnsupportEnvironmentState(Box::new(e)))?,
    )
    .map_err(|e| PublishError::UnsupportEnvironmentState(Box::new(e)))?;

    let lockfile = Lockfile::read_from_file(&lockfile_path)
        .map_err(|e| PublishError::UnsupportEnvironmentState(Box::new(e)))?;

    let install_ids_in_toplevel_group = lockfile
        .manifest
        .pkg_descriptors_in_toplevel_group()
        .into_iter()
        .map(|(pkg, _desc)| pkg);

    // Require a lockfile, but don't require anything in the top level group.
    if install_ids_in_toplevel_group.clone().count() == 0 {
        return Ok(None);
    }

    let top_level_locked_descs = lockfile.packages.iter().filter(|pkg| {
        install_ids_in_toplevel_group
            .clone()
            .any(|id| id == pkg.install_id())
    });
    if let Some(pkg) = top_level_locked_descs.clone().next() {
        Ok(Some(LockedUrlInfo {
            url: pkg.as_catalog_package_ref().unwrap().locked_url.clone(),
            rev: pkg.as_catalog_package_ref().unwrap().rev.clone(),
            rev_count: pkg
                .as_catalog_package_ref()
                .unwrap()
                .rev_count
                .try_into()
                .unwrap(),
            rev_date: Some(pkg.as_catalog_package_ref().unwrap().rev_date),
        }))
    } else {
        Err(PublishError::UnsupportEnvironmentState(
            "Unable to find locked descriptor for toplevel package"
                .to_string()
                .into(),
        ))
    }
}

pub fn check_environment_metadata(
    flox: &Flox,
    environment: &PathEnvironment,
) -> Result<CheckedEnvironmentMetadata, PublishError> {
    // TODO - Ensure current commit is in remote (needed for repeatable builds)
    let build_repo_meta = gather_build_repo_meta(environment)?;

    let base_repo_meta = gather_base_repo_meta(flox, environment)?;

    Ok(CheckedEnvironmentMetadata {
        base_catalog_ref: base_repo_meta,
        build_repo_ref: build_repo_meta,
        _private: (),
    })
}

#[cfg(test)]
pub mod tests {

    // Defined in the manifest.toml in
    const EXAMPLE_PACKAGE_NAME: &str = "mypkg";
    const EXAMPLE_MANIFEST: &str = "envs/publish-simple";

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::flox::test_helpers::{create_test_token, flox_instance};
    use crate::models::environment::path_environment::test_helpers::new_path_environment_from_env_files;
    use crate::providers::build::test_helpers::assert_build_status;
    use crate::providers::build::FloxBuildMk;
    use crate::providers::catalog::{MockClient, GENERATED_DATA};

    fn example_remote() -> (tempfile::TempDir, GitCommandProvider, String) {
        let tempdir_handle = tempfile::tempdir_in(std::env::temp_dir()).unwrap();

        let repo = GitCommandProvider::init(tempdir_handle.path(), true).unwrap();

        let remote_uri = format!("file://{}", tempdir_handle.path().display());

        (tempdir_handle, repo, remote_uri)
    }

    fn example_path_environment(
        flox: &Flox,
        remote: Option<&String>,
    ) -> (PathEnvironment, GitCommandProvider) {
        let env = new_path_environment_from_env_files(&flox, GENERATED_DATA.join(EXAMPLE_MANIFEST));

        let git = GitCommandProvider::init(
            env.parent_path().expect("Parent path must be accessible"),
            false,
        )
        .unwrap();

        git.checkout("main", true).expect("checkout main branch");
        git.add(&[&env.dot_flox_path()]).expect("adding flox files");
        git.commit("Initial commit").expect("be able to commit");

        if let Some(uri) = remote {
            git.add_remote("origin", uri.as_str()).unwrap();
            git.push("origin", true).expect("push to origin");
        }

        (env, git)
    }

    #[test]
    fn test_check_env_meta_failure() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (env, _git) = example_path_environment(&flox, None);

        let meta = check_environment_metadata(&flox, &env);
        assert_eq!(meta.is_err(), true);
    }

    #[test]
    fn test_check_env_meta_nominal() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_remote();
        let (env, build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let meta = check_environment_metadata(&flox, &env).unwrap();

        let build_repo_meta = meta.build_repo_ref;
        assert!(build_repo_meta.url.contains(&remote_uri));
        assert!(build_repo
            .contains_commit(build_repo_meta.rev.as_str())
            .is_ok());
        assert_eq!(build_repo_meta.rev_count, 1);

        assert!(meta.base_catalog_ref.is_some());
        let base_repo_meta = meta.base_catalog_ref.unwrap();

        let lockfile_path = CanonicalPath::new(env.lockfile_path(&flox).unwrap());
        let lockfile = Lockfile::read_from_file(&lockfile_path.unwrap()).unwrap();
        // Only the toplevel group in this example, so we can grap the first package
        let locked_base_pkg = lockfile.packages[0].as_catalog_package_ref().unwrap();
        assert_eq!(base_repo_meta.url, locked_base_pkg.locked_url);
        assert_eq!(base_repo_meta.rev, locked_base_pkg.rev);
        assert_eq!(
            base_repo_meta.rev_count,
            TryInto::<u64>::try_into(locked_base_pkg.rev_count).unwrap()
        );
        assert_eq!(base_repo_meta.rev_date.unwrap(), locked_base_pkg.rev_date);
    }

    #[test]
    fn test_check_build_meta_nominal() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_remote();

        let (mut env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        // Do the build to ensure it's been run.  We just want to find the outputs
        assert_build_status(&flox, &mut env, EXAMPLE_PACKAGE_NAME, true);

        let meta = check_build_metadata(&env, EXAMPLE_PACKAGE_NAME, &flox.system).unwrap();
        assert_eq!(meta.outputs.len(), 1);
        assert_eq!(meta.outputs[0].store_path.starts_with("/nix/store/"), true);
    }

    #[test]
    fn test_publish() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_remote();
        let (mut env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        // Do the build to ensure it's been run.  We just want to find the outputs
        assert_build_status(&flox, &mut env, EXAMPLE_PACKAGE_NAME, true);

        let client = Client::Mock(MockClient::new(None::<String>).unwrap());
        let token = create_test_token("test");

        let (env_meta, build_meta) = (
            check_environment_metadata(&flox, &env).unwrap(),
            check_build_metadata(&env, EXAMPLE_PACKAGE_NAME, &flox.system).unwrap(),
        );

        let publish_provider = PublishProvider::<&FloxBuildMk> {
            build_meta,
            env_meta,
            _builder: None,
        };

        let _res = publish_provider.publish(&client, &token);
    }
}
