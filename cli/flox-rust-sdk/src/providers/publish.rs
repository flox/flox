use std::path::PathBuf;
use std::str::FromStr;

use catalog_api_v1::types::{Output, Outputs, SystemEnum};
use chrono::{DateTime, Utc};
use indoc::{formatdoc, indoc};
use thiserror::Error;
use tracing::instrument;
use url::Url;

use super::build::{BuildResult, BuildResults, ManifestBuilder};
use super::catalog::{
    CatalogClientError,
    Client,
    ClientTrait,
    UserBuildPublish,
    UserDerivationInfo,
};
use super::git::{GitCommandError, GitCommandProvider, StatusInfo};
use crate::data::CanonicalPath;
use crate::flox::Flox;
use crate::models::environment::path_environment::PathEnvironment;
use crate::models::environment::{Environment, EnvironmentError, PathPointer};
use crate::models::lockfile::Lockfile;
use crate::models::manifest::typed::Inner;
use crate::providers::build;
use crate::providers::catalog::{CatalogStoreConfig, CatalogStoreConfigNixCopy};
use crate::providers::git::GitProvider;
use crate::providers::nix::nix_base_command;
use crate::utils::CommandExt;

#[derive(Debug, Error)]
pub enum PublishError {
    #[error("The outputs from the build do not exist: {0}")]
    NonexistentOutputs(String),

    #[error("The environment is in an unsupported state for publishing: {0}")]
    UnsupportedEnvironmentState(String),

    #[error("The package could not be built: {0}")]
    BuildError(String),

    #[error(transparent)]
    CatalogError(CatalogClientError),

    #[error("Could not identify user from authentication info")]
    Unauthenticated,

    #[error("Failed to upload to cache: {0}")]
    CacheUploadError(String),

    #[error(transparent)]
    Environment(#[from] EnvironmentError),

    #[error("{0}")]
    Catchall(String),

    #[error(transparent)]
    Git(#[from] GitCommandError),
}

/// The `Publish` trait describes the high level behavior of publishing a package to a catalog.
/// Authentication, upload, builds etc, are implementation details of the specific provider.
/// Modeling the behavior as a trait allows us to swap out the provider, e.g. a mock for testing.
#[allow(async_fn_in_trait)]
pub trait Publisher {
    async fn publish(
        &self,
        client: &Client,
        catalog_name: &str,
        ingress_uri_override: Option<Url>,
        key_file: Option<PathBuf>,
        metadata_only: bool,
    ) -> Result<(), PublishError>;
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
    pub description: String,

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

#[derive(Debug, Clone, PartialEq)]
pub struct NixCopyCache {
    pub url: Url,
    pub key_file: PathBuf,
}

impl NixCopyCache {
    #[instrument(skip(self), fields(progress = format!("Uploading '{path}' to '{}'", self.url)))]
    pub fn upload(&self, path: &str) -> Result<(), PublishError> {
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
}

/// The `PublishProvider` is a concrete implementation of the `Publish` trait.
/// It is responsible for the actual implementation of the `Publish` trait,
/// i.e. the actual publishing of a package to a catalog.
///
/// The `PublishProvider` is a generic struct, parameterized by a `Builder` type,
/// to build packages before publishing.
pub struct PublishProvider {
    pub env_metadata: CheckedEnvironmentMetadata,
    pub build_metadata: CheckedBuildMetadata,
}

/// (default) implementation of the `Publish` trait, i.e. the publish interface to publish.
impl Publisher for PublishProvider {
    async fn publish(
        &self,
        client: &Client,
        catalog_name: &str,
        ingress_uri_override: Option<Url>,
        key_file: Option<PathBuf>,
        metadata_only: bool,
    ) -> Result<(), PublishError> {
        // Step 1 hit /packages
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
            .map_err(PublishError::CatalogError)?;

        // Step 2 hit /publish
        // For now calling publish just gets information about cache,
        // but in the future it will get information about a publisher
        tracing::debug!("Beginning publish of package...");
        let publish_response = client
            .publish(catalog_name, &self.env_metadata.package)
            .await
            .map_err(PublishError::CatalogError)?;

        let cache = determine_cache(
            metadata_only,
            ingress_uri_override,
            key_file,
            publish_response.catalog_store_config,
        )?;

        // Step 3: optionally upload to cache
        if let Some(cache) = &cache {
            for output in self.build_metadata.outputs.iter() {
                tracing::debug!(
                    "Uploading output {} ({}) to cache...",
                    output.name,
                    output.store_path
                );
                cache.upload(&output.store_path)?
            }
        } else {
            tracing::debug!("No cache configured so skipping upload");
        }

        let build_info = UserBuildPublish {
            derivation: UserDerivationInfo {
                broken: Some(false),
                description: self.env_metadata.description.clone(),
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
            cache_uri: cache.map(|cache| cache.url.to_string()),
            narinfos: None,
        };

        // Step 4: tell the catalog the publish is complete
        tracing::debug!("Publishing build in catalog...");
        client
            .publish_build(&catalog_name, &self.env_metadata.package, &build_info)
            .await
            .map_err(PublishError::CatalogError)?;

        Ok(())
    }
}

/// Construct a cache if one should be used.
///
/// ingress_uri_override is used regardless of what [CatalogStoreConfig] is set
/// to.
/// If an ingress_uri is provided either by way of an override or the
/// [CatalogStoreConfig], a key_file must be provided as well, unless
/// metadata_only is true.
fn determine_cache(
    metadata_only: bool,
    ingress_uri_override: Option<Url>,
    key_file: Option<PathBuf>,
    store_config: CatalogStoreConfig,
) -> Result<Option<NixCopyCache>, PublishError> {
    if metadata_only {
        return Ok(None);
    }
    let ingress_uri = match (ingress_uri_override, store_config) {
        (Some(ingress_uri), _) => ingress_uri,
        (None, CatalogStoreConfig::NixCopy(CatalogStoreConfigNixCopy { ingress_uri, .. })) => {
            Url::parse(&ingress_uri)
                .map_err(|e| PublishError::Catchall(format!("failed to parse ingress URI: {e}")))?
        },
        (None, CatalogStoreConfig::Null) => {
            unreachable!("publish endpoint should error for CatalogStoreConfig::Null")
        },
        // No cache for CatalogStoreConfig::MetaOnly
        (None, CatalogStoreConfig::MetaOnly) => return Ok(None),
        (None, CatalogStoreConfig::Publisher) => {
            unimplemented!("publisher store type is not implemented")
        },
    };

    if let Some(key_file) = key_file {
        Ok(Some(NixCopyCache {
            url: ingress_uri,
            key_file,
        }))
    } else {
        Err(PublishError::Catchall(
            indoc! { "
               A signing key is required to upload artifacts.

               You can supply a signing key by either:
               - Providing a path to a key with the '--signing-private-key' option.
               - Setting it in the config via 'flox config --set publish.signing_private_key <path>'

               Or you can publish without uploading artifacts via the '--metadata-only' option.
            "}
            .to_string(),
        ))
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
    .map_err(|_err| {
        PublishError::UnsupportedEnvironmentState(
            ".flox folder not found in clean checkout, is it tracked in the repository?"
                .to_string(),
        )
    })?;
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
            Some(false),
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
                println!("{line}");
            },
            build::Output::Stderr(line) => {
                eprintln!("{line}");
            },
        }
    }

    let build_results = output_build_results.ok_or(PublishError::NonexistentOutputs(
        "No results returned from build command.".to_string(),
    ))?;
    if build_results.len() != 1 {
        return Err(PublishError::NonexistentOutputs(
            "No results returned from build command.".to_string(),
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

/// Creates the error message for a build repo that's in an invalid state
/// by filling out a template with a provided specific error message.
fn build_repo_err_msg(msg: &str) -> String {
    formatdoc! {"
        \n{msg}

        The build repository must satisfy a few requirements in order to use the 'flox publish' command:
        - It must be a git repository.
        - All of the tracked files must be in a clean state.
        - A remote must be configured.
        - The current revision must be pushed to a remote.
    "}
}

pub fn build_repo_err(msg: &str) -> PublishError {
    PublishError::UnsupportedEnvironmentState(build_repo_err_msg(msg))
}

/// Check the local repo that the build source is in to make sure that it's in
/// a state amenable to publishing an artifact built from it.
///
/// This entails checking that:
/// - The repo has a remote configured.
/// - The tracked source files are clean.
/// - The current revision is the latest one on the remote.
fn gather_build_repo_meta(git: &impl GitProvider) -> Result<LockedUrlInfo, PublishError> {
    let status = git
        .status()
        .map_err(|_e| build_repo_err("Unable to get repository status."))?;

    if status.is_dirty {
        return Err(build_repo_err(
            "Build repository must be clean, but has dirty tracked files.",
        ));
    }

    // Check whether the current branch is tracking a remote branch, and if so,
    // get information about that tracked remote.
    let remote_url = url_for_remote_containing_current_rev(git, &status)?;

    Ok(LockedUrlInfo {
        url: remote_url,
        rev: status.rev,
        rev_count: status.rev_count,
        rev_date: status.rev_date,
    })
}

fn url_for_remote_containing_current_rev(
    git: &impl GitProvider,
    status: &StatusInfo,
) -> Result<String, PublishError> {
    match git.get_current_branch_remote_info() {
        Ok(tracked_remote_info) => {
            match git.rev_exists_on_remote(&status.rev, &tracked_remote_info.name) {
                Ok(exists) => {
                    // Note: strictly speaking this checks that there is a tracked
                    //       remote branch, and that the revision exists on the
                    //       remote that the tracked branch is on, but does not check
                    //       that the revision is on the tracked branch.
                    if exists {
                        git.remote_url(&tracked_remote_info.name)
                            .map_err(|_| build_repo_err("Failed to get URL for remote."))
                    } else {
                        Err(build_repo_err(
                            "Current revision not found on tracked remote branch.",
                        ))
                    }
                },
                // Something failed while trying to talk to the remote.
                Err(_) => Err(build_repo_err(
                    "Failed while trying to locate current revision on remote repository.",
                )),
            }
        },
        Err(_) => {
            // Try to identify whether the revision exists on a remote even though
            // it's not tracking a remote branch.
            let remote_names = git.remotes()?;
            match remote_names.len() {
                // If there are no remotes configured, that's an error we want the
                // user to address.
                0 => Err(build_repo_err(
                    "The repository must have at least one remote configured.",
                )),
                // If there's only a single remote configured, use that.
                1 => {
                    let only_remote = remote_names
                        .first()
                        .expect("already check that at least one remote exists");
                    match git.rev_exists_on_remote(&status.rev, only_remote) {
                        Ok(exists) => {
                            if exists {
                                git.remote_url(only_remote)
                                    .map_err(|_| build_repo_err("Failed to get URL for remote."))
                            } else {
                                Err(build_repo_err(
                                    "Current revision not found on remote repository.",
                                ))
                            }
                        },
                        // Something failed while trying to talk to the remote.
                        Err(_) => Err(build_repo_err(
                            "Failed while trying to locate current revision on remote.",
                        )),
                    }
                },
                // Otherwise, we need to inspect the remotes and apply some heuristics
                // to determine which one to use (if it contains the revision). One
                // heuristic is to prefer remotes named "upstream" and "origin" since
                // those are more likely to be what the canonical repository is.
                _ => {
                    const PREFERRED_REMOTE_NAMES: [&str; 2] = ["upstream", "origin"];
                    let mut chosen_remote = None;
                    for remote_name in PREFERRED_REMOTE_NAMES.iter() {
                        if let Ok(true) = git.rev_exists_on_remote(&status.rev, remote_name) {
                            chosen_remote = Some(remote_name);
                            break;
                        }
                    }
                    if let Some(remote_name) = chosen_remote {
                        Ok(git.remote_url(remote_name)?.to_string())
                    } else {
                        // If the user doesn't have a remote named "upstream" or "origin",
                        // we don't really have any other information we can use to decide
                        // which remote to use, so just pick one.
                        Ok(git.remote_url(&remote_names[0])?.to_string())
                    }
                },
            }
        },
    }
}

fn gather_base_repo_meta(lockfile: &Lockfile) -> Result<LockedUrlInfo, PublishError> {
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
    // We want to make sure we don't incur a lock operation, it must be locked and committed to the repo
    // So we do so with an immutable Environment reference.
    let Some(lockfile) = environment
        .existing_lockfile(flox)
        .map_err(|e| PublishError::UnsupportedEnvironmentState(e.to_string()))?
    else {
        unreachable!("It should have been verified the environment was locked");
    };

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
    let base_repo_meta = gather_base_repo_meta(&lockfile)?;

    let description = lockfile
        .manifest
        .build
        .inner()
        .get(pkg)
        .and_then(|desc| desc.description.clone());

    Ok(CheckedEnvironmentMetadata {
        base_catalog_ref: base_repo_meta,
        build_repo_ref: build_repo_meta,
        package: pkg.to_string(),
        repo_root_path: git.path().to_path_buf(),
        rel_dotflox_path: rel_dotflox_path.to_path_buf(),
        description: description.unwrap_or_else(|| "Not Provided".to_string()),
        _private: (),
    })
}

#[cfg(test)]
pub mod tests {

    // Defined in the manifest.toml in
    const EXAMPLE_PACKAGE_NAME: &str = "mypkg";
    const EXAMPLE_MANIFEST: &str = "envs/publish-simple";

    use std::io::Write;

    use catalog_api_v1::mock::MockServerExt;
    use catalog_api_v1::types::{ErrorResponse, Name, UserPackage};
    use httpmock::prelude::*;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::data::CanonicalPath;
    use crate::flox::test_helpers::{create_test_token, flox_instance};
    use crate::models::environment::path_environment::PathEnvironment;
    use crate::models::environment::path_environment::test_helpers::new_path_environment_from_env_files_in;
    use crate::models::lockfile::Lockfile;
    use crate::providers::build::FloxBuildMk;
    use crate::providers::catalog::test_helpers::reset_mocks;
    use crate::providers::catalog::{
        CatalogClient,
        CatalogClientConfig,
        GENERATED_DATA,
        MockClient,
        PublishResponse,
        Response,
        str_to_catalog_name,
        str_to_package_name,
    };
    use crate::providers::git::tests::{
        commit_file,
        create_remotes,
        get_remote_url,
        init_temp_repo,
    };

    fn example_git_remote_repo() -> (tempfile::TempDir, GitCommandProvider, String) {
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
            None,
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
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();
        let (env, _git) = example_path_environment(&flox, Some(&remote_uri));

        let meta = check_environment_metadata(&flox, &env, EXAMPLE_PACKAGE_NAME);
        assert!(meta.is_ok());

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
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();
        let (env, git) = example_path_environment(&flox, Some(&remote_uri));

        let meta = check_environment_metadata(&flox, &env, EXAMPLE_PACKAGE_NAME);
        assert!(meta.is_ok());

        let manifest_path = env
            .manifest_path(&flox)
            .expect("to be able to get manifest path");
        std::fs::write(
            &manifest_path,
            format!("{}\n", env.manifest_contents(&flox).unwrap()),
        )
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
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();
        let (env, build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let meta = check_environment_metadata(&flox, &env, EXAMPLE_PACKAGE_NAME).unwrap();
        let description_in_manifest = "Some sample package description from our tests";

        let build_repo_meta = meta.build_repo_ref;
        assert!(build_repo_meta.url.contains(&remote_uri));
        assert!(
            build_repo
                .contains_commit(build_repo_meta.rev.as_str())
                .is_ok()
        );
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
        assert_eq!(meta.description, description_in_manifest.to_string());
    }

    #[test]
    fn test_check_build_meta_nominal() {
        let builder = FloxBuildMk;
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();

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
    async fn publish_meta_only() {
        let builder = FloxBuildMk;
        let (mut flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();
        let (env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let token = create_test_token("test");
        let catalog_name = token.handle().to_string();

        let env_metadata = check_environment_metadata(&flox, &env, EXAMPLE_PACKAGE_NAME).unwrap();
        let build_metadata =
            check_build_metadata(&flox, &env_metadata, &env, &builder, EXAMPLE_PACKAGE_NAME)
                .unwrap();

        let publish_provider = PublishProvider {
            build_metadata,
            env_metadata,
        };

        reset_mocks(&mut flox.catalog_client, vec![
            Response::CreatePackage,
            Response::Publish(PublishResponse {
                ingress_uri: None,
                catalog_store_config: CatalogStoreConfig::MetaOnly,
            }),
            Response::PublishBuild,
        ]);

        let res = publish_provider
            .publish(&flox.catalog_client, &catalog_name, None, None, false)
            .await;

        assert!(res.is_ok());
    }

    /// Generate dummy CheckedBuildMetadata and CheckedEnvironmentMetadata that
    /// can be passed to publish()
    ///
    /// It is dummy in the sense that no human thought about it ;)
    fn dummy_publish_metadata() -> (CheckedBuildMetadata, CheckedEnvironmentMetadata) {
        let build_metadata = CheckedBuildMetadata {
            name: "dummy".to_string(),
            pname: "dummy".to_string(),
            outputs: Outputs(vec![]),
            outputs_to_install: vec![],
            drv_path: "dummy".to_string(),
            system: SystemEnum::X8664Linux,
            version: Some("1.0.0".to_string()),
            _private: (),
        };

        let env_metadata = CheckedEnvironmentMetadata {
            repo_root_path: PathBuf::new(),
            rel_dotflox_path: PathBuf::new(),
            base_catalog_ref: LockedUrlInfo {
                url: "dummy".to_string(),
                rev: "dummy".to_string(),
                rev_count: 0,
                rev_date: Utc::now(),
            },
            build_repo_ref: LockedUrlInfo {
                url: "dummy".to_string(),
                rev: "dummy".to_string(),
                rev_count: 0,
                rev_date: Utc::now(),
            },
            package: "dummy".to_string(),
            description: "dummy".to_string(),
            _private: (),
        };

        (build_metadata, env_metadata)
    }

    #[tokio::test]
    async fn publish_errors_without_key() {
        let mut client = Client::Mock(MockClient::new(None::<String>).unwrap());

        let token = create_test_token("test");
        let catalog_name = token.handle().to_string();

        // Don't do a build because it's slow
        let (build_metadata, env_metadata) = dummy_publish_metadata();

        let publish_provider = PublishProvider {
            build_metadata,
            env_metadata,
        };

        reset_mocks(&mut client, vec![
            Response::CreatePackage,
            Response::Publish(PublishResponse {
                ingress_uri: Some("https://example.com".to_string()),
                catalog_store_config: CatalogStoreConfig::NixCopy(CatalogStoreConfigNixCopy {
                    ingress_uri: "https://example.com".to_string(),
                    egress_uri: "https://example.com".to_string(),
                }),
            }),
        ]);

        let result = publish_provider
            .publish(&client, &catalog_name, None, None, false)
            .await;

        let err = result.unwrap_err();
        assert_eq!(
            err.to_string(),
            indoc! { "
                A signing key is required to upload artifacts.

                You can supply a signing key by either:
                - Providing a path to a key with the '--signing-private-key' option.
                - Setting it in the config via 'flox config --set publish.signing_private_key <path>'

                Or you can publish without uploading artifacts via the '--metadata-only' option.
            " }
            .to_string()
        );
    }

    /// publish() passes the error details from the server through
    #[tokio::test]
    async fn publish_passes_error_details_through() {
        let server = MockServer::start();

        let token = create_test_token("test");
        let catalog_name = token.handle().to_string();

        // Don't do a build because it's slow
        let (build_metadata, env_metadata) = dummy_publish_metadata();
        let package_name = &env_metadata.package;
        let original_url = &env_metadata.build_repo_ref.url;

        let packages_mock = server
            .create_catalog_package_api_v1_catalog_catalogs_catalog_name_packages_post(
                |when, then| {
                    when.catalog_name(&str_to_catalog_name(&catalog_name).unwrap())
                        .name(&Name::from_str(package_name).unwrap());
                    then.ok(&UserPackage {
                        catalog: catalog_name.clone(),
                        name: package_name.clone(),
                        original_url: Some(original_url.clone()),
                    });
                },
            );

        let publish_mock = server.publish_request_api_v1_catalog_catalogs_catalog_name_packages_package_name_publish_post(|when, then| {
            when.catalog_name(&str_to_catalog_name(&catalog_name).unwrap())
                .package_name(&str_to_package_name(package_name).unwrap());
            then.unprocessable_entity(&ErrorResponse { detail: "Some\nlong\nresponse\nfrom\nthe\nserver".to_string() });
        });

        let client = Client::Catalog(CatalogClient::new(CatalogClientConfig {
            catalog_url: server.base_url(),
            floxhub_token: Some(token.secret().to_string()),
            extra_headers: Default::default(),
        }));

        let publish_provider = PublishProvider {
            build_metadata,
            env_metadata,
        };

        // We should error even if metadata_only is true and ingress_uri_override is set
        let result = publish_provider
            .publish(
                &client,
                &catalog_name,
                Some("https://example.com".parse().unwrap()),
                None,
                true,
            )
            .await;

        packages_mock.assert();
        publish_mock.assert();

        let err = result.unwrap_err();
        assert_eq!(
            err.to_string(),
            indoc! {"
                422 Unprocessable Entity: Some
                long
                response
                from
                the
                server"}
            .to_string()
        );
    }

    #[tokio::test]
    async fn upload_to_local_cache() {
        let builder = FloxBuildMk;
        let (mut flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();
        let (env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let token = create_test_token("test");
        let catalog_name = token.handle().to_string();

        let env_metadata = check_environment_metadata(&flox, &env, EXAMPLE_PACKAGE_NAME).unwrap();
        let build_metadata =
            check_build_metadata(&flox, &env_metadata, &env, &builder, EXAMPLE_PACKAGE_NAME)
                .unwrap();

        let (_key_file, cache) = local_nix_cache();
        let publish_provider = PublishProvider {
            build_metadata,
            env_metadata,
        };

        // the 'cache' should be non existent before the publish
        let cache_path = cache.url.to_file_path().unwrap();
        assert!(std::fs::read_dir(&cache_path).is_err());

        reset_mocks(&mut flox.catalog_client, vec![
            Response::CreatePackage,
            Response::Publish(PublishResponse {
                ingress_uri: Some(cache.url.to_string()),
                catalog_store_config: CatalogStoreConfig::NixCopy(CatalogStoreConfigNixCopy {
                    ingress_uri: cache.url.to_string(),
                    egress_uri: cache.url.to_string(),
                }),
            }),
            Response::PublishBuild,
        ]);

        publish_provider
            .publish(
                &flox.catalog_client,
                &catalog_name,
                None,
                Some(cache.key_file),
                false,
            )
            .await
            .unwrap();

        // The 'cache' should be non-empty after the publish
        let entries = std::fs::read_dir(&cache_path).unwrap();
        assert!(entries.count() != 0);
    }

    #[test]
    fn prefers_tracked_remote() {
        let remote_name = "some_remote";
        let branch_name = "some_branch";
        let (build_repo, _tempdir) = init_temp_repo(false);
        commit_file(&build_repo, "foo");
        let status = build_repo.status().unwrap();
        build_repo.create_branch(branch_name, &status.rev).unwrap();
        let remotes = create_remotes(&build_repo, &[remote_name, "other_remote"]);
        build_repo
            .push_ref(remote_name, branch_name, false)
            .unwrap();
        build_repo
            .push_ref("other_remote", branch_name, false)
            .unwrap();
        // Make this repo track the upstream `main` branch
        let mut cmd = build_repo.new_command();
        cmd.args(["branch", "-u", &format!("{}/{}", remote_name, branch_name)]);
        GitCommandProvider::run_command(&mut cmd).unwrap();
        let remote_url =
            url_for_remote_containing_current_rev(&build_repo, &build_repo.status().unwrap())
                .unwrap();
        assert_eq!(remote_url, get_remote_url(&remotes, remote_name));
    }

    #[test]
    fn finds_single_untracked_remote() {
        let remote_name = "some_remote";
        let branch_name = "some_branch";
        let (build_repo, _tempdir) = init_temp_repo(false);
        commit_file(&build_repo, "foo");
        let status = build_repo.status().unwrap();
        build_repo.create_branch(branch_name, &status.rev).unwrap();
        let remotes = create_remotes(&build_repo, &[remote_name]);
        build_repo
            .push_ref(remote_name, branch_name, false)
            .unwrap();
        let remote_url =
            url_for_remote_containing_current_rev(&build_repo, &build_repo.status().unwrap())
                .unwrap();
        assert_eq!(remote_url, get_remote_url(&remotes, remote_name));
    }

    #[test]
    fn prefers_upstream_to_other_remote() {
        let remote_name = "upstream";
        let branch_name = "some_branch";
        let (build_repo, _tempdir) = init_temp_repo(false);
        commit_file(&build_repo, "foo");
        let status = build_repo.status().unwrap();
        build_repo.create_branch(branch_name, &status.rev).unwrap();
        let remotes = create_remotes(&build_repo, &[remote_name, "other_remote"]);
        build_repo
            .push_ref(remote_name, branch_name, false)
            .unwrap();
        build_repo
            .push_ref("other_remote", branch_name, false)
            .unwrap();
        let remote_url =
            url_for_remote_containing_current_rev(&build_repo, &build_repo.status().unwrap())
                .unwrap();
        assert_eq!(remote_url, get_remote_url(&remotes, remote_name));
    }

    #[test]
    fn prefers_upstream_to_origin() {
        let preferred_remote_name = "upstream";
        let branch_name = "some_branch";
        let (build_repo, _tempdir) = init_temp_repo(false);
        commit_file(&build_repo, "foo");
        let status = build_repo.status().unwrap();
        build_repo.create_branch(branch_name, &status.rev).unwrap();
        let remotes = create_remotes(&build_repo, &[preferred_remote_name, "origin"]);
        build_repo
            .push_ref(preferred_remote_name, branch_name, false)
            .unwrap();
        build_repo.push_ref("origin", branch_name, false).unwrap();
        let retrieved_remote_url =
            url_for_remote_containing_current_rev(&build_repo, &build_repo.status().unwrap())
                .unwrap();
        assert_eq!(
            retrieved_remote_url,
            get_remote_url(&remotes, preferred_remote_name)
        );
    }

    #[test]
    fn prefers_origin_to_other_remote() {
        let remote_name = "origin";
        let branch_name = "some_branch";
        let (build_repo, _tempdir) = init_temp_repo(false);
        commit_file(&build_repo, "foo");
        let status = build_repo.status().unwrap();
        build_repo.create_branch(branch_name, &status.rev).unwrap();
        let remotes = create_remotes(&build_repo, &[remote_name, "other_remote"]);
        build_repo
            .push_ref(remote_name, branch_name, false)
            .unwrap();
        build_repo
            .push_ref("other_remote", branch_name, false)
            .unwrap();
        let remote_url =
            url_for_remote_containing_current_rev(&build_repo, &build_repo.status().unwrap())
                .unwrap();
        let expected_remote_url = get_remote_url(&remotes, remote_name);
        assert_eq!(remote_url, expected_remote_url);
    }

    #[test]
    fn falls_back_to_some_remote() {
        let branch_name = "some_branch";
        let (build_repo, _tempdir) = init_temp_repo(false);
        commit_file(&build_repo, "foo");
        let status = build_repo.status().unwrap();
        build_repo.create_branch(branch_name, &status.rev).unwrap();
        let remotes = create_remotes(&build_repo, &["some_remote", "other_remote"]);
        build_repo
            .push_ref("some_remote", branch_name, false)
            .unwrap();
        build_repo
            .push_ref("other_remote", branch_name, false)
            .unwrap();
        let remote_url =
            url_for_remote_containing_current_rev(&build_repo, &build_repo.status().unwrap())
                .unwrap();
        let is_some_remote = remote_url == get_remote_url(&remotes, "some_remote");
        let is_other_remote = remote_url == get_remote_url(&remotes, "other_remote");
        assert!(is_some_remote || is_other_remote);
    }
}
