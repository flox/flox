use std::error;
use std::path::PathBuf;
use std::str::FromStr;

use catalog_api_v1::types::{Output, Outputs, SystemEnum};
use chrono::{DateTime, Utc};
use thiserror::Error;
use tracing::{info, instrument};
use url::Url;

use super::build::{BuildResult, BuildResults, ManifestBuilder};
use super::catalog::{Client, ClientTrait, UserBuildInfo, UserDerivationInfo};
use super::git::GitCommandProvider;
use crate::data::CanonicalPath;
use crate::flox::Flox;
use crate::models::environment::path_environment::PathEnvironment;
use crate::models::environment::{Environment, EnvironmentError, PathPointer};
use crate::models::lockfile::Lockfile;
use crate::providers::build;
use crate::providers::git::GitProvider;
use crate::providers::nix::nix_base_command;
use crate::utils::CommandExt;

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

    #[error("The package could not be built: {0}")]
    BuildError(String),

    #[error("There was an error communicating with the catalog")]
    CatalogError(#[source] Box<dyn error::Error + Send + Sync>),

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
    // This is the local root path of the repo containing the environment
    pub repo_root_path: PathBuf,
    // This is the path to the .flox for the build environment relative to the repo_root_path
    pub rel_dotflox_path: PathBuf,

    // There may or may not be a locked base catalog reference in the environment
    pub base_catalog_ref: LockedUrlInfo,
    // The build repo reference is always present
    pub build_repo_ref: LockedUrlInfo,

    // These are collected from the environment manifest
    pub package: String,
    pub description: Option<String>,

    // This field isn't "pub", so no one outside this module can construct this struct. That helps
    // ensure that we can only make this struct as a result of doing the "right thing."
    _private: (),
}

/// Ensures that the required metadata for publishing is consistent from the build process
#[allow(clippy::manual_non_exhaustive)]
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedBuildMetadata {
    // Define metadata coming from the build, e.g. outpaths
    pub name: String,
    pub pname: String,
    pub outputs: catalog_api_v1::types::Outputs,
    pub outputs_to_install: Vec<String>,
    pub drv_path: String,
    pub system: SystemEnum,

    pub version: Option<String>,

    // This field isn't "pub", so no one outside this module can construct this struct. That helps
    // ensure that we can only make this struct as a result of doing the "right thing."
    _private: (),
}

pub trait BinaryCache {
    fn upload(&self, path: &str) -> Result<(), PublishError>;
    fn cache_url(&self) -> &Url;
}

#[derive(Debug, Clone, PartialEq)]
pub struct NixCopyCache {
    pub url: Url,
    pub key_file: PathBuf,
}

impl BinaryCache for NixCopyCache {
    #[instrument(skip(self), fields(progress = format!("Uploading '{path}' to '{}'", self.url)))]
    fn upload(&self, path: &str) -> Result<(), PublishError> {
        let mut url = self.url.clone();
        let url_with_key = url
            .query_pairs_mut()
            .append_pair("secret-key", &self.key_file.to_string_lossy())
            .append_pair("ls-compression", "zstd")
            .append_pair("compression", "zstd")
            .append_pair("write-nar-listing", "true")
            .finish();

        let mut copy_command = nix_base_command();
        copy_command
            .arg("copy")
            .arg("--to")
            .arg(url_with_key.to_string())
            .arg(path);

        tracing::debug!(
            %path,
            %url_with_key,
            cmd = %copy_command.display(),
            "Uploading store path to cache"
        );

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
pub struct PublishProvider<Cache> {
    pub env_metadata: CheckedEnvironmentMetadata,
    pub build_metadata: CheckedBuildMetadata,
    pub cache: Option<Cache>,
}

/// (default) implementation of the `Publish` trait, i.e. the publish interface to publish.
impl<Cache> Publisher for PublishProvider<&Cache>
where
    Cache: BinaryCache,
{
    async fn publish(&self, client: &Client, catalog_name: &str) -> Result<(), PublishError> {
        // The create package service call will create the user's own catalog
        // if not already created, and then create (or return) the package noted
        // returning either a 200 or 201.  Either is ok here, as long as it's not an error.
        tracing::debug!("Creating package in catalog...");
        client
            .create_package(
                &catalog_name,
                &self.env_metadata.package,
                &self.env_metadata.build_repo_ref.url,
            )
            .await
            .map_err(|e| PublishError::CatalogError(Box::new(e)))?;

        let build_info = UserBuildInfo {
            derivation: UserDerivationInfo {
                broken: Some(false),
                description: "".to_string(),
                drv_path: self.build_metadata.drv_path.clone(),
                license: None,
                name: self.build_metadata.name.clone(),
                outputs: self.build_metadata.outputs.clone(),
                outputs_to_install: Some(self.build_metadata.outputs_to_install.clone()),
                pname: Some(self.build_metadata.pname.clone()),
                system: self.build_metadata.system,
                unfree: None,
                version: self.build_metadata.version.clone(),
            },
            locked_base_catalog_url: Some(self.env_metadata.base_catalog_ref.url.clone()),
            url: self.env_metadata.build_repo_ref.url.clone(),
            rev: self.env_metadata.build_repo_ref.rev.clone(),
            rev_count: self.env_metadata.build_repo_ref.rev_count as i64,
            rev_date: self.env_metadata.build_repo_ref.rev_date,
            cache_uri: self.cache.map(|c| c.cache_url().to_string()),
        };

        if let Some(cache) = self.cache {
            for output in self.build_metadata.outputs.iter() {
                tracing::debug!(
                    "Uploading {}:{} to cache...",
                    output.name,
                    output.store_path
                );
                cache.upload(&output.store_path)?
            }
        }

        tracing::debug!("Publishing build in catalog...");
        client
            .publish_build(&catalog_name, &self.env_metadata.package, &build_info)
            .await
            .map_err(|e| PublishError::CatalogError(Box::new(e)))?;

        Ok(())
    }
}

pub fn check_build_metadata_from_build_result(
    build_result: &BuildResult,
    system: SystemEnum,
) -> Result<CheckedBuildMetadata, PublishError> {
    let outputs = Outputs(
        build_result
            .outputs
            .clone()
            .into_iter()
            .map(|(output_name, output_path)| Output {
                name: output_name,
                store_path: output_path.to_string_lossy().to_string(),
            })
            .collect(),
    );

    let outputs_to_install: Vec<String> = build_result.outputs.clone().into_keys().collect();

    Ok(CheckedBuildMetadata {
        drv_path: build_result.drv_path.clone(),
        name: build_result.name.clone(),
        pname: build_result.pname.clone(),
        outputs,
        outputs_to_install,
        system,
        version: Some(build_result.version.clone()),
        _private: (),
    })
}

/// Collect metadata needed for publishing that is obtained from the build output
pub fn check_build_metadata(
    flox: &Flox,
    env_metadata: &CheckedEnvironmentMetadata,
    env: &PathEnvironment,
    builder: &impl ManifestBuilder,
    pkg: &str,
) -> Result<CheckedBuildMetadata, PublishError> {
    // git clone into a temp directory
    let clean_repo_path = tempfile::tempdir_in(flox.temp_dir.clone()).unwrap();
    let git = <GitCommandProvider as GitProvider>::clone(
        env_metadata.repo_root_path.as_path(),
        &clean_repo_path,
        false,
    )
    .map_err(|e| PublishError::UnsupportedEnvironmentState(e.to_string()))?;
    // checkout the rev we want to publish
    git.checkout(env_metadata.build_repo_ref.rev.as_str(), true)
        .map_err(|e| PublishError::UnsupportedEnvironmentState(e.to_string()))?;

    let dot_flox_path = CanonicalPath::new(
        clean_repo_path
            .path()
            .join(env_metadata.rel_dotflox_path.as_path()),
    )
    .map_err(|err| EnvironmentError::DotFloxNotFound(err.path))?;
    let mut clean_build_env =
        PathEnvironment::open(flox, PathPointer::new(env.name()), dot_flox_path)?;

    // Build the package and collect the outputs
    let output_stream = builder
        .build(
            flox,
            &clean_build_env.parent_path()?,
            &clean_build_env.build(flox)?,
            &clean_build_env
                .rendered_env_links(flox)
                .unwrap()
                .development,
            &[pkg.to_owned()],
        )
        .map_err(|e| PublishError::BuildError(e.to_string()))?;

    let mut output_build_results: Option<BuildResults> = None;
    for message in output_stream {
        match message {
            build::Output::Success(build_results) => {
                output_build_results = Some(build_results);
            },
            build::Output::Failure(_) => {
                panic!("expected build to succeed");
            },
            build::Output::Stdout(line) => {
                info!("stdout: {line}");
            },
            build::Output::Stderr(line) => {
                info!("stderr: {line}");
            },
        }
    }

    let build_results = output_build_results.ok_or(PublishError::NonexistentOutputs(
        "No build results".to_string(),
    ))?;
    if build_results.len() != 1 {
        return Err(PublishError::NonexistentOutputs(
            "No build results".to_string(),
        ));
    }
    let build_result = &build_results[0];

    let metadata = check_build_metadata_from_build_result(
        build_result,
        SystemEnum::from_str(flox.system.as_str()).map_err(|_e| {
            PublishError::UnsupportedEnvironmentState("Invalid system".to_string())
        })?,
    )?;
    Ok(metadata)
}

fn gather_build_repo_meta(git: &impl GitProvider) -> Result<LockedUrlInfo, PublishError> {
    // Gather build repo info

    // This call will fail if the local head is not in the remote
    let origin = git
        .get_origin()
        .map_err(|e| PublishError::UnsupportedEnvironmentState(format!("Git get origin {e}")))?;

    let status = git
        .status()
        .map_err(|e| PublishError::UnsupportedEnvironmentState(format!("Git get status {e}")))?;

    if status.is_dirty {
        return Err(PublishError::UnsupportedEnvironmentState(
            "Build repo is dirty".to_string(),
        ));
    }

    Ok(LockedUrlInfo {
        url: origin.url,
        rev: status.rev,
        rev_count: status.rev_count,
        rev_date: status.rev_date,
    })
}

fn gather_base_repo_meta(
    flox: &Flox,
    environment: &impl Environment,
) -> Result<LockedUrlInfo, PublishError> {
    // Gather locked base catalog page info
    // We want to make sure we don't incur a lock operation, it must be locked and committed to the repo
    // So we do so with an immutable Environment reference.
    let lockfile_path = CanonicalPath::new(environment.lockfile_path(flox)?);
    let lockfile = Lockfile::read_from_file(&lockfile_path.unwrap())
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
    environment: &impl Environment,
    pkg: &str,
) -> Result<CheckedEnvironmentMetadata, PublishError> {
    // Gather build repo info
    let git = match environment.parent_path() {
        Ok(env_path) => GitCommandProvider::discover(env_path)
            .map_err(|e| PublishError::UnsupportedEnvironmentState(format!("Git discover {e}")))?,
        Err(e) => return Err(PublishError::UnsupportedEnvironmentState(e.to_string())),
    };

    let dot_flox_path = environment.dot_flox_path();
    let rel_dotflox_path = dot_flox_path.strip_prefix(git.path()).map_err(|e| {
        PublishError::UnsupportedEnvironmentState(format!("Flox path not in git repo: {e}"))
    })?;

    let build_repo_meta = gather_build_repo_meta(&git)?;

    let base_repo_meta = gather_base_repo_meta(flox, environment)?;

    let manifest = environment.manifest(flox)?;
    let description = manifest
        .build
        .get(pkg)
        .and_then(|desc| desc.description.clone());

    Ok(CheckedEnvironmentMetadata {
        base_catalog_ref: base_repo_meta,
        build_repo_ref: build_repo_meta,
        package: pkg.to_string(),
        repo_root_path: git.path().to_path_buf(),
        rel_dotflox_path: rel_dotflox_path.to_path_buf(),
        description,
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
    use crate::models::environment::path_environment::test_helpers::new_path_environment_from_env_files_in;
    use crate::models::environment::path_environment::PathEnvironment;
    use crate::models::lockfile::Lockfile;
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

        let mut key_command = nix_base_command();
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
        let repo_root = tempfile::tempdir_in(&flox.temp_dir).unwrap().into_path();
        let repo_subdir = repo_root.join("subdir_for_flox_stuff");

        let env = new_path_environment_from_env_files_in(
            flox,
            GENERATED_DATA.join(EXAMPLE_MANIFEST),
            repo_subdir,
        );

        let git = GitCommandProvider::init(repo_root, false).unwrap();

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

        let meta = check_environment_metadata(&flox, &env, EXAMPLE_PACKAGE_NAME);
        meta.expect_err("Should fail due to not being a git repo");
    }

    #[test]
    fn test_check_env_meta_dirty() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (env, _git) = example_path_environment(&flox, None);

        std::fs::write(env.manifest_path(&flox).unwrap(), "dirty content")
            .expect("to write some additional text to the .flox");

        let meta = check_environment_metadata(&flox, &env, EXAMPLE_PACKAGE_NAME);
        match meta {
            Err(PublishError::UnsupportedEnvironmentState(_msg)) => {},
            _ => panic!("Expected error to be of type UnsupportedEnvironmentState"),
        }
    }

    #[test]
    fn test_check_env_meta_not_in_remote() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (env, git) = example_path_environment(&flox, None);

        let manifest_path = env
            .manifest_path(&flox)
            .expect("to be able to get manifest path");
        std::fs::write(&manifest_path, "dirty content")
            .expect("to write some additional text to the .flox");
        git.add(&[manifest_path.as_path()])
            .expect("adding flox files");
        git.commit("dirty comment").expect("be able to commit");

        let meta = check_environment_metadata(&flox, &env, EXAMPLE_PACKAGE_NAME);
        match meta {
            Err(PublishError::UnsupportedEnvironmentState(_msg)) => {},
            _ => panic!("Expected error to be of type UnsupportedEnvironmentState"),
        }
    }

    #[test]
    fn test_check_env_meta_nominal() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_remote();
        let (env, build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let meta = check_environment_metadata(&flox, &env, EXAMPLE_PACKAGE_NAME).unwrap();
        let description_in_manifest = "Some sample package description from our tests";

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
        assert_eq!(meta.package, EXAMPLE_PACKAGE_NAME);
        assert_eq!(meta.description, Some(description_in_manifest.to_string()));
    }

    #[test]
    fn test_check_build_meta_nominal() {
        let builder = FloxBuildMk;
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_remote();

        let (env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let env_metadata = check_environment_metadata(&flox, &env, EXAMPLE_PACKAGE_NAME).unwrap();

        // This will actually run the build
        let meta = check_build_metadata(&flox, &env_metadata, &env, &builder, EXAMPLE_PACKAGE_NAME)
            .unwrap();

        let version_in_manifest = "1.0.2a";

        assert_eq!(meta.outputs.len(), 1);
        assert_eq!(meta.outputs_to_install.len(), 1);
        assert_eq!(meta.outputs[0].store_path.starts_with("/nix/store/"), true);
        assert_eq!(meta.drv_path.starts_with("/nix/store/"), true);
        assert_eq!(meta.version, Some(version_in_manifest.to_string()));
        assert_eq!(meta.pname, EXAMPLE_PACKAGE_NAME.to_string());
        assert_eq!(meta.system.to_string(), flox.system);
    }

    #[tokio::test]
    async fn test_publish() {
        let builder = FloxBuildMk;
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_remote();
        let (env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let client = Client::Mock(MockClient::new(None::<String>).unwrap());
        let token = create_test_token("test");
        let catalog_name = token.handle().to_string();

        let env_metadata = check_environment_metadata(&flox, &env, EXAMPLE_PACKAGE_NAME).unwrap();
        let build_metadata =
            check_build_metadata(&flox, &env_metadata, &env, &builder, EXAMPLE_PACKAGE_NAME)
                .unwrap();

        let publish_provider = PublishProvider::<&MockCache> {
            build_metadata,
            env_metadata,
            cache: None,
        };

        let res = publish_provider.publish(&client, &catalog_name).await;

        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn test_upload_to_cache_failed() {
        let builder = FloxBuildMk;
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_remote();
        let (env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let client = Client::Mock(MockClient::new(None::<String>).unwrap());
        let token = create_test_token("test");
        let catalog_name = token.handle().to_string();

        let env_metadata = check_environment_metadata(&flox, &env, EXAMPLE_PACKAGE_NAME).unwrap();
        let build_metadata =
            check_build_metadata(&flox, &env_metadata, &env, &builder, EXAMPLE_PACKAGE_NAME)
                .unwrap();

        // Test an expected failure from the Mock
        let cache = Some(MockCache {
            url: Url::parse("s3://my-cool-cache").unwrap(),
            error_msg: Some("Something went wrong".to_string()),
        });

        let publish_provider = PublishProvider::<&MockCache> {
            build_metadata,
            env_metadata,
            cache: cache.as_ref(),
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
        let builder = FloxBuildMk;
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_remote();
        let (env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let client = Client::Mock(MockClient::new(None::<String>).unwrap());
        let token = create_test_token("test");
        let catalog_name = token.handle().to_string();

        let env_metadata = check_environment_metadata(&flox, &env, EXAMPLE_PACKAGE_NAME).unwrap();
        let build_metadata =
            check_build_metadata(&flox, &env_metadata, &env, &builder, EXAMPLE_PACKAGE_NAME)
                .unwrap();

        let (_key_file, cache) = local_nix_cache();
        let publish_provider = PublishProvider::<&NixCopyCache> {
            build_metadata,
            env_metadata,
            cache: Some(&cache),
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
