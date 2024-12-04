use std::error;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;

use catalog_api_v1::types::{Output, Outputs, SystemEnum};
use chrono::{DateTime, Utc};
use log::debug;
use thiserror::Error;
use url::Url;

use super::build::{build_symlink_path, ManifestBuilder};
use super::catalog::{Client, ClientTrait, UserBuildInfo, UserDerivationInfo};
use super::git::GitCommandProvider;
use crate::flox::Flox;
use crate::models::environment::{Environment, EnvironmentError};
use crate::providers::buildenv::NIX_BIN;
use crate::providers::git::GitProvider;

#[derive(Debug, Error)]
pub enum PublishError {
    #[error("This type of environment is not supported for publishing")]
    UnsupportedEnvironment,
    #[error("The environment must be locked to publish")]
    UnlockedEnvironment,

    #[error("The outputs from the build do not exist: {0}")]
    NonexistentOutputs(String),

    #[error("The environment is in an unsupported state for publishing: {0}")]
    UnsupportedEnvironmentState(String),

    #[error("There was an error communicating with the catalog")]
    CatalogError(#[source] Box<dyn error::Error + Send>),

    #[error("Could not identify user from authentication info")]
    Unauthenticated,

    #[error("Failed to upload to cache: {0}")]
    CacheUploadError(String),

    #[error(transparent)]
    Environment(#[from] EnvironmentError),
}

/// The `Publish` trait describes the high level behavior of publishing a package to a catalog.
/// Authentication, upload, builds etc, are implementation details of the specific provider.
/// Modeling the behavior as a trait allows us to swap out the provider, e.g. a mock for testing.
#[allow(async_fn_in_trait)]
pub trait Publisher {
    async fn publish(&self, client: &Client, catalog_name: &str) -> Result<(), PublishError>;
}

/// Simple struct to hold the information of a locked URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedUrlInfo {
    pub url: String,
    pub rev: String,
    pub rev_count: u64,
    pub rev_date: DateTime<Utc>,
}

/// Ensures that the required metadata for publishing is consistent from the environment
#[allow(clippy::manual_non_exhaustive)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedEnvironmentMetadata {
    // There may or may not be a locked base catalog reference in the environment
    pub base_catalog_ref: LockedUrlInfo,
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

pub trait BinaryCache {
    fn upload(&self, path: &str) -> Result<(), PublishError>;
    fn cache_url(&self) -> &Url;
}

pub struct NixCopyCache {
    pub url: Url,
    pub key_file: PathBuf,
}

impl BinaryCache for NixCopyCache {
    fn upload(&self, path: &str) -> Result<(), PublishError> {
        let mut url = self.url.clone();
        let url_with_key = url
            .query_pairs_mut()
            .append_pair("secret-key", &self.key_file.to_string_lossy())
            .finish();
        debug!(
            "Uploading {path} to cache {cache}...",
            path = path,
            cache = url_with_key
        );

        let mut copy_command = Command::new(&*NIX_BIN);
        copy_command
            .arg("copy")
            .arg("--to")
            .arg(url_with_key.to_string())
            .arg(path);
        let output = copy_command
            .output()
            .map_err(|e| PublishError::CacheUploadError(e.to_string()))?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(PublishError::CacheUploadError(stderr.to_string()))
        }
    }

    fn cache_url(&self) -> &Url {
        &self.url
    }
}

pub struct MockCache {
    pub url: Url,
    pub error_msg: Option<String>,
}

impl BinaryCache for MockCache {
    fn upload(&self, _path: &str) -> Result<(), PublishError> {
        if let Some(msg) = &self.error_msg {
            Err(PublishError::CacheUploadError(msg.to_string()))
        } else {
            Ok(())
        }
    }

    fn cache_url(&self) -> &Url {
        &self.url
    }
}

/// The `PublishProvider` is a concrete implementation of the `Publish` trait.
/// It is responsible for the actual implementation of the `Publish` trait,
/// i.e. the actual publishing of a package to a catalog.
///
/// The `PublishProvider` is a generic struct, parameterized by a `Builder` type,
/// to build packages before publishing.
pub struct PublishProvider<Builder, Cache> {
    pub env_metadata: CheckedEnvironmentMetadata,
    pub build_metadata: CheckedBuildMetadata,
    pub cache: Option<Cache>,

    pub _builder: Option<Builder>,
}

/// (default) implementation of the `Publish` trait, i.e. the publish interface to publish.
impl<Builder, Cache> Publisher for PublishProvider<&Builder, &Cache>
where
    Builder: ManifestBuilder,
    Cache: BinaryCache,
{
    async fn publish(&self, client: &Client, catalog_name: &str) -> Result<(), PublishError> {
        // Get metadata from the environment, like locked URLs.

        // The create package service call will create the user's own catalog
        // if not already created, and then create (or return) the package noted
        // returning either a 200 or 201.  Either is ok here, as long as it's not an error.
        debug!("Creating package in catalog...");
        client
            .create_package(
                &catalog_name,
                &self.build_metadata.package,
                &self.env_metadata.build_repo_ref.url,
            )
            .await
            .map_err(|e| PublishError::CatalogError(Box::new(e)))?;

        let outputs = Outputs(
            self.build_metadata
                .outputs
                .clone()
                .into_iter()
                .map(|o| Output {
                    name: o.name,
                    store_path: o.store_path,
                })
                .collect(),
        );
        let outputs_to_install = Some(
            self.build_metadata
                .outputs
                .clone()
                .into_iter()
                .map(|o| o.name.clone())
                .collect(),
        );

        let build_info = UserBuildInfo {
            derivation: UserDerivationInfo {
                broken: Some(false),
                description: "".to_string(),
                drv_path: self.build_metadata.drv_path.clone(),
                license: None,
                name: self.build_metadata.package.to_string().to_owned(),
                outputs,
                outputs_to_install,
                pname: Some(self.build_metadata.package.to_string()),
                system: self.build_metadata.system,
                unfree: None,
                version: Some("unknown".to_string()),
            },
            locked_base_catalog_url: Some(self.env_metadata.base_catalog_ref.url.clone()),
            url: self.env_metadata.build_repo_ref.url.clone(),
            rev: self.env_metadata.build_repo_ref.rev.clone(),
            rev_count: self.env_metadata.build_repo_ref.rev_count as i64,
            rev_date: self.env_metadata.build_repo_ref.rev_date,
            cache_uri: self.cache.map(|c| c.cache_url().to_string()),
        };

        if let Some(cache) = self.cache {
            cache.upload(&self.build_metadata.drv_path)?
        }

        debug!("Publishing build in catalog...");
        client
            .publish_build(&catalog_name, &self.build_metadata.package, &build_info)
            .await
            .map_err(|e| PublishError::CatalogError(Box::new(e)))?;

        Ok(())
    }
}

/// Collect metadata needed for publishing that is obtained from the build output
pub fn check_build_metadata(
    env: &impl Environment,
    pkg: &str,
    system: &str,
) -> Result<CheckedBuildMetadata, PublishError> {
    // For now assume the build is successful, and present.
    // Look for the output from the build at `results-<pkgname>`
    // Note that the current builds only support a single output at that
    // pre-defined path.  Later work will get structured results from the build
    // process to feed this.

    let system = SystemEnum::from_str(system).map_err(|e| {
        PublishError::UnsupportedEnvironmentState(format!("Unable to identify system: {e}"))
    })?;

    let result_dir = build_symlink_path(env, pkg)?;
    let store_dir = result_dir
        .read_link()
        .map_err(|e| PublishError::NonexistentOutputs(e.to_string()))?;

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

fn gather_build_repo_meta(environment: &impl Environment) -> Result<LockedUrlInfo, PublishError> {
    // Gather build repo info
    let git = match environment.parent_path() {
        Ok(env_path) => GitCommandProvider::discover(env_path)
            .map_err(|e| PublishError::UnsupportedEnvironmentState(format!("Git error {e}")))?,
        Err(e) => return Err(PublishError::UnsupportedEnvironmentState(e.to_string())),
    };

    let origin = git
        .get_origin()
        .map_err(|e| PublishError::UnsupportedEnvironmentState(format!("Git error {e}")))?;

    let status = git
        .status()
        .map_err(|e| PublishError::UnsupportedEnvironmentState(e.to_string()))?;

    // TODO - check is_dirty and warn?
    // TODO - check if REV is in remote?

    Ok(LockedUrlInfo {
        url: origin.url,
        rev: status.rev,
        rev_count: status.rev_count,
        rev_date: status.rev_date,
    })
}

fn gather_base_repo_meta(
    flox: &Flox,
    environment: &mut impl Environment,
) -> Result<LockedUrlInfo, PublishError> {
    // Gather locked base catalog page info
    let lockfile = environment
        .lockfile(flox)
        .map_err(|e| PublishError::UnsupportedEnvironmentState(e.to_string()))?;
    let install_ids_in_toplevel_group = lockfile
        .manifest
        .pkg_descriptors_in_toplevel_group()
        .into_iter()
        .map(|(pkg, _desc)| pkg);

    // We should not need this, and allow for no base catalog page dependency.
    // But for now, requiring it simplifies resolution and model updates
    // significantly.
    if install_ids_in_toplevel_group.clone().count() == 0 {
        return Err(PublishError::UnsupportedEnvironmentState(
            "No packages in toplevel group".to_string(),
        ));
    }

    let top_level_locked_descs = lockfile.packages.iter().filter(|pkg| {
        install_ids_in_toplevel_group
            .clone()
            .any(|id| id == pkg.install_id())
    });
    if let Some(pkg) = top_level_locked_descs.clone().next() {
        Ok(LockedUrlInfo {
            url: pkg.as_catalog_package_ref().unwrap().locked_url.clone(),
            rev: pkg.as_catalog_package_ref().unwrap().rev.clone(),
            rev_count: pkg
                .as_catalog_package_ref()
                .unwrap()
                .rev_count
                .try_into()
                .unwrap(),
            rev_date: pkg.as_catalog_package_ref().unwrap().rev_date,
        })
    } else {
        Err(PublishError::UnsupportedEnvironmentState(
            "Unable to find locked descriptor for toplevel package".to_string(),
        ))
    }
}

pub fn check_environment_metadata(
    flox: &Flox,
    environment: &mut impl Environment,
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

    use std::io::Write;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::data::CanonicalPath;
    use crate::flox::test_helpers::{create_test_token, flox_instance};
    use crate::models::environment::path_environment::test_helpers::new_path_environment_from_env_files;
    use crate::models::environment::path_environment::PathEnvironment;
    use crate::models::lockfile::Lockfile;
    use crate::providers::build::test_helpers::assert_build_status;
    use crate::providers::build::FloxBuildMk;
    use crate::providers::catalog::{MockClient, GENERATED_DATA};

    fn example_remote() -> (tempfile::TempDir, GitCommandProvider, String) {
        let tempdir_handle = tempfile::tempdir_in(std::env::temp_dir()).unwrap();

        let repo = GitCommandProvider::init(tempdir_handle.path(), true).unwrap();

        let remote_uri = format!("file://{}", tempdir_handle.path().display());

        (tempdir_handle, repo, remote_uri)
    }

    fn local_nix_cache() -> (tempfile::NamedTempFile, NixCopyCache) {
        // Returns a temp local cache and signing key file to use in testing publish
        let tempdir_handle = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
        let mut temp_key_file =
            tempfile::NamedTempFile::new().expect("Should create named temp file");

        let mut key_command = Command::new(&*NIX_BIN);
        key_command
            .arg("key")
            .arg("generate-secret")
            .arg("--key-name")
            .arg("cli-test");
        let output = key_command
            .output()
            .map_err(|e| PublishError::CacheUploadError(e.to_string()))
            .expect("Should generate key");
        // write the key to the file
        temp_key_file
            .write_all(&output.stdout)
            .expect("Should write key to file");
        temp_key_file.flush().expect("Should flush key file");

        let cache_url = format!("file://{}", tempdir_handle.path().display());
        let key_file_path = temp_key_file.path().to_path_buf();
        (temp_key_file, NixCopyCache {
            url: Url::parse(&cache_url).unwrap(),
            key_file: key_file_path,
        })
    }

    fn example_path_environment(
        flox: &Flox,
        remote: Option<&String>,
    ) -> (PathEnvironment, GitCommandProvider) {
        let env = new_path_environment_from_env_files(flox, GENERATED_DATA.join(EXAMPLE_MANIFEST));

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
        let (mut env, _git) = example_path_environment(&flox, None);

        let meta = check_environment_metadata(&flox, &mut env);
        meta.expect_err("Should fail due to not being a git repo");
    }

    #[test]
    fn test_check_env_meta_nominal() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_remote();
        let (mut env, build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let meta = check_environment_metadata(&flox, &mut env).unwrap();

        let build_repo_meta = meta.build_repo_ref;
        assert!(build_repo_meta.url.contains(&remote_uri));
        assert!(build_repo
            .contains_commit(build_repo_meta.rev.as_str())
            .is_ok());
        assert_eq!(build_repo_meta.rev_count, 1);

        let lockfile_path = CanonicalPath::new(env.lockfile_path(&flox).unwrap());
        let lockfile = Lockfile::read_from_file(&lockfile_path.unwrap()).unwrap();
        // Only the toplevel group in this example, so we can grab the first package
        let locked_base_pkg = lockfile.packages[0].as_catalog_package_ref().unwrap();
        assert_eq!(meta.base_catalog_ref.url, locked_base_pkg.locked_url);
        assert_eq!(meta.base_catalog_ref.rev, locked_base_pkg.rev);
        assert_eq!(
            meta.base_catalog_ref.rev_count,
            TryInto::<u64>::try_into(locked_base_pkg.rev_count).unwrap()
        );
        assert_eq!(meta.base_catalog_ref.rev_date, locked_base_pkg.rev_date);
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

    #[tokio::test]
    async fn test_publish() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_remote();
        let (mut env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        // Do the build to ensure it's been run.  We just want to find the outputs
        assert_build_status(&flox, &mut env, EXAMPLE_PACKAGE_NAME, true);

        let client = Client::Mock(MockClient::new(None::<String>).unwrap());
        let token = create_test_token("test");
        let catalog_name = token.handle().to_string();

        let (env_metadata, build_metadata) = (
            check_environment_metadata(&flox, &mut env).unwrap(),
            check_build_metadata(&env, EXAMPLE_PACKAGE_NAME, &flox.system).unwrap(),
        );

        let publish_provider = PublishProvider::<&FloxBuildMk, &MockCache> {
            build_metadata,
            env_metadata,
            cache: None,
            _builder: None,
        };

        let res = publish_provider.publish(&client, &catalog_name).await;

        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_upload_to_cache_failed() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_remote();
        let (mut env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        // Do the build to ensure it's been run.  We just want to find the outputs
        assert_build_status(&flox, &mut env, EXAMPLE_PACKAGE_NAME, true);

        let client = Client::Mock(MockClient::new(None::<String>).unwrap());
        let token = create_test_token("test");
        let catalog_name = token.handle().to_string();

        let (env_metadata, build_metadata) = (
            check_environment_metadata(&flox, &mut env).unwrap(),
            check_build_metadata(&env, EXAMPLE_PACKAGE_NAME, &flox.system).unwrap(),
        );

        // Test an expected failure from the Mock
        let cache = Some(MockCache {
            url: Url::parse("s3://my-cool-cache").unwrap(),
            error_msg: Some("Something went wrong".to_string()),
        });

        let publish_provider = PublishProvider::<&FloxBuildMk, &MockCache> {
            build_metadata,
            env_metadata,
            cache: cache.as_ref(),
            _builder: None,
        };

        let res = publish_provider.publish(&client, &catalog_name).await;
        let err = res.expect_err("Should fail due to cache error");
        assert_eq!(
            err.to_string(),
            "Failed to upload to cache: Something went wrong"
        );
    }

    #[tokio::test]
    async fn test_upload_to_local_cache() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_remote();
        let (mut env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        // Do the build to ensure it's been run.  We just want to find the outputs
        assert_build_status(&flox, &mut env, EXAMPLE_PACKAGE_NAME, true);

        let client = Client::Mock(MockClient::new(None::<String>).unwrap());
        let token = create_test_token("test");
        let catalog_name = token.handle().to_string();

        let (env_metadata, build_metadata) = (
            check_environment_metadata(&flox, &mut env).unwrap(),
            check_build_metadata(&env, EXAMPLE_PACKAGE_NAME, &flox.system).unwrap(),
        );

        let (_key_file, cache) = local_nix_cache();
        let publish_provider = PublishProvider::<&FloxBuildMk, &NixCopyCache> {
            build_metadata,
            env_metadata,
            cache: Some(&cache),
            _builder: None,
        };

        // the 'cache' should be non existent before the publish
        let cache_path = cache.url.to_file_path().unwrap();
        assert!(std::fs::read_dir(&cache_path).is_err());

        let res = publish_provider.publish(&client, &catalog_name).await;
        assert!(res.is_ok());

        // The 'cache' should be non-empty after the publish
        let entries = std::fs::read_dir(&cache_path).unwrap();
        assert!(entries.count() != 0);
    }
}
