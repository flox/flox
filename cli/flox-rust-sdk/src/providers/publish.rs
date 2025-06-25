use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::str::FromStr;

use catalog_api_v1::types::{NarInfo, NarInfos, Output, Outputs, PublishResponse, SystemEnum};
use chrono::{DateTime, Utc};
use indoc::{formatdoc, indoc};
use thiserror::Error;
use tracing::{debug, instrument};
use url::Url;

use super::auth::{AuthError, AuthProvider, CatalogAuth, NixCopyAuth};
use super::build::{
    BuildResult,
    BuildResults,
    FloxBuildMk,
    ManifestBuilder,
    ManifestBuilderError,
    PackageTarget,
    PackageTargetError,
    PackageTargetKind,
    find_toplevel_group_nixpkgs,
    nix_expression_dir,
};
use super::catalog::{
    BaseCatalogUrl,
    BaseCatalogUrlError,
    CatalogClientError,
    Client,
    ClientTrait,
    UserBuildPublish,
    UserDerivationInfo,
    mock_base_catalog_url,
};
use super::git::{GitCommandError, GitCommandProvider, StatusInfo};
use crate::data::CanonicalPath;
use crate::flox::Flox;
use crate::models::environment::{Environment, EnvironmentError, open_path};
use crate::models::lockfile::Lockfile;
use crate::models::manifest::typed::Inner;
use crate::providers::auth::catalog_auth_to_envs;
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

    #[error(transparent)]
    PackageTargetError(#[from] PackageTargetError),

    #[error(transparent)]
    ManifestBuildError(#[from] ManifestBuilderError),

    #[error(transparent)]
    CatalogError(CatalogClientError),

    #[error("invalid nixpkgs base url")]
    InvalidNixpkgsBaseUrl(
        #[source]
        #[from]
        BaseCatalogUrlError,
    ),

    #[error("build of package(s) failed ({status})")]
    PackageBuildError { status: ExitStatus },

    #[error("Could not identify user from authentication info")]
    Unauthenticated,

    #[error("Failed to upload to cache: {0}")]
    CacheUploadError(String),

    #[error("Failed to get additional artifact metadata: {0}")]
    GetNarInfo(String),

    #[error("Failed to parse artifact metadata")]
    ParseNarInfo(#[source] serde_json::Error),

    #[error(transparent)]
    Environment(#[from] EnvironmentError),

    #[error("{0}")]
    Catchall(String),

    #[error(transparent)]
    Git(#[from] GitCommandError),

    #[error(transparent)]
    Auth(#[from] AuthError),

    #[error("Timed out waiting for publish completion")]
    PublishTimeout,
}

/// The `Publish` trait describes the high level behavior of publishing a package to a catalog.
/// Authentication, upload, builds etc, are implementation details of the specific provider.
/// Modeling the behavior as a trait allows us to swap out the provider, e.g. a mock for testing.
#[allow(async_fn_in_trait)]
pub trait Publisher {
    async fn create_package(
        &self,
        client: &Client,
        catalog_name: &str,
    ) -> Result<PackageCreatedGuard, PublishError>;
    async fn publish(
        &self,
        client: &Client,
        catalog_name: &str,
        package_created: PackageCreatedGuard,
        build_metadata: &CheckedBuildMetadata,
        key_file: Option<PathBuf>,
        metadata_only: bool,
    ) -> Result<(), PublishError>;
    async fn wait_for_publish_completion(
        &self,
        client: &Client,
        build_metadata: &CheckedBuildMetadata,
        poll_interval_millis: u64,
        timeout_millis: u64,
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
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedEnvironmentMetadata {
    pub lockfile: Lockfile,
    // This is the local root path of the repo containing the environment
    pub repo_root_path: PathBuf,
    // This is the path to the parent of .flox for the build environment relative to the repo_root_path
    pub rel_project_path: PathBuf,

    // The build repo reference is always present
    pub build_repo_ref: LockedUrlInfo,

    // There may or may not be a locked base catalog reference in the environment
    pub toplevel_catalog_ref: Option<BaseCatalogUrl>,

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

#[allow(clippy::manual_non_exhaustive)]
#[derive(Debug, Clone, PartialEq)]
pub struct PackageMetadata {
    pub base_catalog_ref: BaseCatalogUrl,

    // These are collected from the environment manifest
    pub package: PackageTarget,
    pub description: String,

    // This field isn't "pub", so no one outside this module can construct this struct. That helps
    // ensure that we can only make this struct as a result of doing the "right thing."
    _private: (),
}

/// Ensures that a package has been created (or rather registered) before
/// attempting to publish the build.
#[derive(Debug)]
pub struct PackageCreatedGuard {
    // This field isn't "pub", so no one outside this module can construct this struct. That helps
    // ensure that we can only make this struct as a result of doing the "right thing."
    _private: (),
}

/// Configuration for uploading to or downloading from a catalog store.
#[allow(clippy::large_enum_variant)] // TODO: Remove after implementing `Publisher`.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientSideCatalogStoreConfig {
    /// A `nix copy`-compatible Catalog Store (typically an S3 bucket).
    NixCopy {
        /// Where signed artifacts are uploaded to.
        ingress_uri: Url,
        /// Where artifacts are downloaded from.
        egress_uri: Url,
        /// The path to the key used to sign artifacts before uploading them.
        signing_private_key_path: PathBuf,
        /// Authentication file used when interacting with the store.
        auth_netrc_path: PathBuf,
    },
    /// A Catalog Store which only accepts metadata uploads.
    MetadataOnly,
    /// A Catalog with no Catalog Store configured.
    Null,
    /// A Catalog Store that doesn't require a signing key from the user doing
    /// the upload.
    Publisher {
        /// Where signed artifacts are uploaded to.
        ingress_uri: Url,
        /// The path to the key used to sign artifacts before uploading them.  Optional here.
        signing_private_key_path: Option<PathBuf>,
        /// Authentication provided by the catalog
        ingress_auth: Option<CatalogAuth>,
    },
}

impl ClientSideCatalogStoreConfig {
    /// Returns the URL to which a client would upload new artifacts.
    pub fn upload_url(&self) -> Option<Url> {
        match self {
            ClientSideCatalogStoreConfig::NixCopy { ingress_uri, .. } => Some(ingress_uri.clone()),
            ClientSideCatalogStoreConfig::MetadataOnly => None,
            ClientSideCatalogStoreConfig::Null => None,
            ClientSideCatalogStoreConfig::Publisher { ingress_uri, .. } => {
                Some(ingress_uri.clone())
            },
        }
    }

    /// Returns the URL from which a client would download artifacts.
    pub fn download_url(&self) -> Option<Url> {
        match self {
            ClientSideCatalogStoreConfig::NixCopy { egress_uri, .. } => Some(egress_uri.clone()),
            ClientSideCatalogStoreConfig::MetadataOnly => None,
            ClientSideCatalogStoreConfig::Null => None,
            ClientSideCatalogStoreConfig::Publisher { .. } => None,
        }
    }

    /// Returns the path of the local signing key if one is configured.
    pub fn local_signing_key_path(&self) -> Option<PathBuf> {
        if let ClientSideCatalogStoreConfig::NixCopy {
            signing_private_key_path,
            ..
        } = self
        {
            Some(signing_private_key_path.clone())
        } else if let ClientSideCatalogStoreConfig::Publisher {
            signing_private_key_path,
            ..
        } = self
        {
            signing_private_key_path.clone()
        } else {
            None
        }
    }

    /// Depending on whether the catalog store is configured to accept uploaded artifacts,
    /// upload the build outputs and their NAR infos or skip the upload entirely.
    pub fn maybe_upload_artifacts(
        &self,
        build_outputs: &[Output],
    ) -> Result<Option<NarInfos>, PublishError> {
        if build_outputs.is_empty() {
            debug!(reason = "no build outputs", "skipping artifact upload");
            return Ok(None);
        }

        match self {
            ClientSideCatalogStoreConfig::NixCopy {
                ingress_uri,
                egress_uri,
                signing_private_key_path,
                auth_netrc_path,
            } => {
                debug!(
                    reason = "nix-copy catalog store",
                    ?ingress_uri,
                    "uploading artifacts to cache"
                );
                Self::upload_build_outputs(
                    ingress_uri,
                    Some(signing_private_key_path.as_path()),
                    &Some(NixCopyAuth::Netrc(auth_netrc_path.clone())),
                    build_outputs,
                )?;
                let nar_infos =
                    Self::get_build_output_nar_infos(egress_uri, auth_netrc_path, build_outputs)?;
                Ok(Some(nar_infos))
            },
            ClientSideCatalogStoreConfig::MetadataOnly => {
                debug!(
                    reason = "metadata-only catalog store",
                    "skipping artifact upload"
                );
                Ok(None)
            },
            ClientSideCatalogStoreConfig::Null => {
                debug!(reason = "null catalog store", "skipping artifact upload");
                Ok(None)
            },
            ClientSideCatalogStoreConfig::Publisher {
                ingress_uri,
                signing_private_key_path,
                ingress_auth,
            } => {
                debug!(
                    reason = "publisher catalog store",
                    ?ingress_uri,
                    "uploading artifacts to cache"
                );
                Self::upload_build_outputs(
                    ingress_uri,
                    signing_private_key_path.as_deref(),
                    &ingress_auth.to_owned().map(NixCopyAuth::CatalogProvided),
                    build_outputs,
                )?;
                // No narinfos for publisher type
                Ok(None)
            },
        }
    }

    /// Uploads the store paths corresponding to each build output. Note that
    /// NAR info is uploaded in a different method.
    fn upload_build_outputs(
        destination_url: &Url,
        signing_key_path: Option<&Path>,
        nix_copy_auth: &Option<NixCopyAuth>,
        build_outputs: &[Output],
    ) -> Result<(), PublishError> {
        for output in build_outputs.iter() {
            debug!(
                ?output,
                %destination_url,
                "Uploading output...",
            );
            Self::upload_store_path(
                destination_url,
                signing_key_path,
                nix_copy_auth,
                &output.store_path,
            )?;
        }
        Ok(())
    }

    /// Upload a single store path to a Catalog Store.
    ///
    /// Note: this is only public because it's used in the private `flox upload`
    ///       command.
    #[instrument(skip_all, fields(progress = format!("Uploading '{store_path}'")))]
    pub fn upload_store_path(
        destination_url: &Url,
        signing_key_path: Option<&Path>,
        nix_copy_auth: &Option<NixCopyAuth>,
        store_path: &str,
    ) -> Result<(), PublishError> {
        let mut url_with_query = destination_url.clone();
        let mut query = url_with_query.query_pairs_mut();
        if let Some(key_path) = signing_key_path {
            // If the signing key is None, we don't want to add it to the URL.
            query.append_pair("secret-key", key_path.to_string_lossy().as_ref());
        }
        query.append_pair("compression", "zstd");
        query.append_pair("write-nar-listing", "true");
        if destination_url.scheme() == "s3" {
            // https://nix.dev/manual/nix/2.24/command-ref/new-cli/nix3-help-stores#store-s3-binary-cache-store-ls-compression
            query.append_pair("ls-compression", "zstd");
        }
        drop(query);

        let mut copy_command = nix_base_command();
        match nix_copy_auth {
            Some(NixCopyAuth::Netrc(path)) => {
                copy_command.arg("--netrc-file").arg(path);
            },
            Some(NixCopyAuth::CatalogProvided(auth)) => {
                copy_command.envs(catalog_auth_to_envs(auth)?);
            },
            // A publisher might not provide auth for a public store
            _ => {},
        }
        copy_command
            .arg("copy")
            .arg("--to")
            .arg(url_with_query.to_string())
            .arg(store_path);

        debug!(
            %store_path,
            %url_with_query,
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

    /// Constructs a `nix path-info` command that will get the NAR info for a
    /// store path from the specified store, including the optional information
    /// about the closure size of the store path.
    fn nar_info_cmd(store_url: &Url, store_path: &str, auth_netrc_path: &Path) -> Command {
        let mut cmd = nix_base_command();
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.arg("--netrc-file").arg(auth_netrc_path);
        cmd.args([
            "path-info",
            "--store",
            store_url.as_str(),
            "--closure-size",
            "--json",
            store_path,
        ]);
        cmd
    }

    /// Gets the NAR info for a single store path and returns it in the format that the
    /// catalog server expects e.g. one that is tolerant of the different NAR info formats
    /// that the `nix` CLI can return.
    #[instrument(skip_all, fields(progress = format!("Collecting extra build metadata for '{store_path}'")))]
    fn get_nar_info(
        source_url: &Url,
        store_path: &str,
        auth_netrc_path: &Path,
    ) -> Result<NarInfo, PublishError> {
        let mut cmd = Self::nar_info_cmd(source_url, store_path, auth_netrc_path);
        debug!(cmd = %cmd.display(), "running nix path-info command");
        let output = cmd.output().map_err(|e| {
            PublishError::Catchall(format!("failed to execute NAR info command: {e}"))
        })?;
        if !output.status.success() {
            return Err(PublishError::GetNarInfo(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }
        serde_json::from_slice(&output.stdout).map_err(PublishError::ParseNarInfo)
    }

    /// Gets the NAR info for each build output and returns it in the format
    /// that the catalog server expects e.g. one that is tolerant of the different
    /// NAR info formats that the `nix` CLI can return.
    fn get_build_output_nar_infos(
        source_url: &Url,
        auth_netrc_path: &Path,
        build_outputs: &[Output],
    ) -> Result<NarInfos, PublishError> {
        let mut nar_infos = HashMap::new();
        for output in build_outputs.iter() {
            debug!(
                output = output.name,
                store_path = output.store_path,
                store = source_url.as_str(),
                "querying NAR info for build output"
            );
            let nar_info = Self::get_nar_info(source_url, &output.store_path, auth_netrc_path)?;
            nar_infos.insert(output.name.clone(), nar_info);
        }
        Ok(nar_infos.into())
    }
}

/// The `PublishProvider` is a concrete implementation of the `Publish` trait.
/// It is responsible for the actual implementation of the `Publish` trait,
/// i.e. the actual publishing of a package to a catalog.
///
/// The `PublishProvider` is a generic struct, parameterized by a `Builder` type,
/// to build packages before publishing.
pub struct PublishProvider<A> {
    pub env_metadata: CheckedEnvironmentMetadata,
    pub package_metadata: PackageMetadata,
    auth: A,
}

impl<A> PublishProvider<A> {
    pub fn new(
        env_metadata: CheckedEnvironmentMetadata,
        package_metadata: PackageMetadata,
        auth: A,
    ) -> Self {
        Self {
            env_metadata,
            package_metadata,
            auth,
        }
    }
}

/// (default) implementation of the `Publish` trait, i.e. the publish interface to publish.
impl<A> Publisher for PublishProvider<A>
where
    A: AuthProvider,
{
    /// Ensure that a package is created and, by proxy, that the user has
    /// permission to publish it.
    async fn create_package(
        &self,
        client: &Client,
        catalog_name: &str,
    ) -> Result<PackageCreatedGuard, PublishError> {
        // Step 1 hit /packages
        // The create package service call will create the user's own catalog
        // if not already created, and then create (or return) the package noted
        // returning either a 200 or 201.  Either is ok here, as long as it's not an error.
        // "Creating a package" is just registering a "attr_path" with the catalog, nothing more.
        tracing::debug!("Creating package in catalog...");
        client
            .create_package(
                &catalog_name,
                self.package_metadata.package.name().as_ref(),
                &self.env_metadata.build_repo_ref.url,
            )
            .await
            .map_err(PublishError::CatalogError)?;

        Ok(PackageCreatedGuard { _private: () })
    }

    /// Publish a built package.
    ///
    /// [PackageCreatedGuard] must be obtained from [Self::create_package].
    async fn publish(
        &self,
        client: &Client,
        catalog_name: &str,
        _package_created: PackageCreatedGuard,
        build_metadata: &CheckedBuildMetadata,
        key_file: Option<PathBuf>,
        metadata_only: bool,
    ) -> Result<(), PublishError> {
        // Step 2 hit /publish
        // Catalogs are configured with their "store".
        // We must request upload information for _this_ catalog to know where
        // to upload store paths.
        // For now calling publish just gets information about cache,
        // but in the future this will also provide access tokens and other info
        // needed.
        tracing::debug!("Beginning publish of package...");
        let publish_response = client
            .publish_info(catalog_name, self.package_metadata.package.name().as_ref())
            .await
            .map_err(PublishError::CatalogError)?;

        let netrc_path = self.auth.create_netrc().map_err(PublishError::Auth)?;
        let catalog_store_config = get_client_side_catalog_store_config(
            metadata_only,
            key_file,
            &netrc_path,
            publish_response,
        )?;
        let narinfos = catalog_store_config.maybe_upload_artifacts(&build_metadata.outputs)?;

        let build_info = UserBuildPublish {
            derivation: UserDerivationInfo {
                broken: Some(false),
                description: self.package_metadata.description.clone(), // TODO: extract from expr build result
                drv_path: build_metadata.drv_path.clone(),
                license: None,
                name: build_metadata.name.clone(),
                outputs: build_metadata.outputs.clone(),
                outputs_to_install: Some(build_metadata.outputs_to_install.clone()),
                pname: Some(build_metadata.pname.clone()),
                system: build_metadata.system,
                unfree: None,
                version: build_metadata.version.clone(),
            },
            locked_base_catalog_url: Some(self.package_metadata.base_catalog_ref.to_string()),
            url: self.env_metadata.build_repo_ref.url.clone(),
            rev: self.env_metadata.build_repo_ref.rev.clone(),
            rev_count: self.env_metadata.build_repo_ref.rev_count as i64,
            rev_date: self.env_metadata.build_repo_ref.rev_date,
            cache_uri: catalog_store_config.upload_url().map(|url| url.to_string()),
            narinfos,
            // The URL where the narinfos were downloaded from.
            narinfos_source_url: catalog_store_config
                .download_url()
                .map(|url| url.to_string()),
            // This is the version of the narinfo being submitted.  Until we
            // define changes, we'll use the service defaults.
            narinfos_source_version: None,
        };

        tracing::debug!("Publishing build in catalog...");
        client
            .publish_build(
                &catalog_name,
                self.package_metadata.package.name().as_ref(),
                &build_info,
            )
            .await
            .map_err(PublishError::CatalogError)?;

        Ok(())
    }

    /// Waits until the narinfos for all store paths are present in the catalog,
    /// or errors on timeout.
    async fn wait_for_publish_completion(
        &self,
        client: &Client,
        build_metadata: &CheckedBuildMetadata,
        poll_interval_millis: u64,
        timeout_millis: u64,
    ) -> Result<(), PublishError> {
        let store_paths = build_metadata
            .outputs
            .0
            .iter()
            .map(|output| output.store_path.clone())
            .collect::<Vec<_>>();
        let loop_poll_and_wait = async {
            let start = tokio::time::Instant::now();
            let wait_duration = tokio::time::Duration::from_millis(poll_interval_millis);
            loop {
                let now = tokio::time::Instant::now();
                let elapsed = now.duration_since(start);
                debug!(
                    elapsed_millis = elapsed.as_millis(),
                    "polling publish completion"
                );
                if client
                    .is_publish_complete(&store_paths)
                    .await
                    .map_err(PublishError::CatalogError)?
                {
                    break;
                }
                debug!("not complete, sleeping");
                tokio::time::sleep(wait_duration).await;
            }
            let now = tokio::time::Instant::now();
            let elapsed = now.duration_since(start);
            debug!(elapsed_millis = elapsed.as_millis(), "publish complete");
            Ok::<_, PublishError>(())
        };
        let timeout = tokio::time::Duration::from_millis(timeout_millis);
        tokio::time::timeout(timeout, loop_poll_and_wait)
            .await
            .map_err(|_| PublishError::PublishTimeout)??;
        Ok(())
    }
}

/// Get the complete configuration for client-side interactions with the provided
/// Catalog Store.
fn get_client_side_catalog_store_config(
    metadata_only: bool,
    key_file: Option<PathBuf>,
    auth_netrc_path: impl AsRef<Path>,
    publish_response: PublishResponse,
) -> Result<ClientSideCatalogStoreConfig, PublishError> {
    if metadata_only {
        return Ok(ClientSideCatalogStoreConfig::MetadataOnly);
    }
    let config = match publish_response.catalog_store_config {
        CatalogStoreConfig::Null => ClientSideCatalogStoreConfig::Null,
        CatalogStoreConfig::MetaOnly => ClientSideCatalogStoreConfig::MetadataOnly,
        CatalogStoreConfig::NixCopy(nix_copy_config) => {
            let CatalogStoreConfigNixCopy {
                ingress_uri,
                egress_uri,
                ..
            } = nix_copy_config;
            if let Some(path) = key_file {
                ClientSideCatalogStoreConfig::NixCopy {
                    ingress_uri: Url::parse(&ingress_uri).map_err(|e| {
                        PublishError::Catchall(format!("failed to parse ingress URI: {e}"))
                    })?,
                    egress_uri: Url::parse(&egress_uri).map_err(|e| {
                        PublishError::Catchall(format!("failed to parse egress URI: {e}"))
                    })?,
                    signing_private_key_path: path,
                    auth_netrc_path: auth_netrc_path.as_ref().to_path_buf(),
                }
            } else {
                return Err(PublishError::Catchall(
                    indoc! { "
                       A signing key is required to upload artifacts.

                       You can supply a signing key by either:
                       - Providing a path to a key with the '--signing-private-key' option.
                       - Setting it in the config via 'flox config --set publish.signing_private_key <path>'

                       Or you can publish without uploading artifacts via the '--metadata-only' option.
                    "}
                    .to_string(),
                ));
            }
        },
        CatalogStoreConfig::Publisher(_) => ClientSideCatalogStoreConfig::Publisher {
            ingress_uri: Url::parse(
                publish_response
                    .ingress_uri
                    .as_deref()
                    .ok_or_else(|| PublishError::Catchall("ingress URI is missing".to_string()))?,
            )
            .map_err(|e| PublishError::Catchall(format!("failed to parse ingress URI: {e}")))?,
            signing_private_key_path: key_file,
            ingress_auth: publish_response.ingress_auth,
        },
    };
    Ok(config)
}

fn check_build_metadata_from_build_result(
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
    pkg: &PackageTarget,
) -> Result<CheckedBuildMetadata, PublishError> {
    // git clone into a temp directory
    let clean_repo_path = tempfile::tempdir_in(flox.temp_dir.clone())
        .map_err(|err| PublishError::Catchall(format!("could not create tempdir: {err}")))?;
    let git = <GitCommandProvider as GitProvider>::clone(
        env_metadata.repo_root_path.as_path(),
        &clean_repo_path,
        false,
    )
    .map_err(|e| PublishError::UnsupportedEnvironmentState(e.to_string()))?;

    // checkout the rev we want to publish
    git.checkout(env_metadata.build_repo_ref.rev.as_str(), true)
        .map_err(|e| PublishError::UnsupportedEnvironmentState(e.to_string()))?;

    let project_path = CanonicalPath::new(
        clean_repo_path
            .path()
            .join(env_metadata.rel_project_path.as_path()),
    )
    .map_err(|_err| {
        PublishError::UnsupportedEnvironmentState(
            "Flox project not found in clean checkout, is it tracked in the repository?"
                .to_string(),
        )
    })?;

    let mut clean_build_env = open_path(flox, &project_path)
        .map_err(|e| PublishError::UnsupportedEnvironmentState(e.to_string()))?;
    let base_dir = clean_build_env.parent_path()?;
    let expression_dir = nix_expression_dir(&clean_build_env);
    let built_environments = clean_build_env.build(flox)?;

    let builder = FloxBuildMk::new(flox, &base_dir, &expression_dir, &built_environments);

    // Build the package and collect the outputs
    let output_stream = builder.build(
        // todo: use a non-hardcoded nixpkgs url
        &mock_base_catalog_url().as_flake_ref()?,
        env_metadata
            .toplevel_catalog_ref
            .as_ref()
            .map(BaseCatalogUrl::as_flake_ref)
            .transpose()?
            .as_ref(),
        &built_environments.develop,
        &[pkg.name()],
        Some(false),
    )?;

    let mut output_build_results: Option<BuildResults> = None;

    for message in output_stream {
        match message {
            build::Output::Success(build_results) => {
                output_build_results = Some(build_results);
            },
            build::Output::Failure(status) => {
                return Err(PublishError::PackageBuildError { status });
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

pub fn check_environment_metadata(
    flox: &Flox,
    environment: &impl Environment,
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
    let project_path = match environment.parent_path() {
        Ok(env_path) => env_path,
        Err(e) => return Err(PublishError::UnsupportedEnvironmentState(e.to_string())),
    };
    let git = GitCommandProvider::discover(&project_path)
        .map_err(|e| PublishError::UnsupportedEnvironmentState(format!("Git discover {e}")))?;

    let rel_project_path = project_path.strip_prefix(git.path()).map_err(|e| {
        PublishError::UnsupportedEnvironmentState(format!("Flox project path not in git repo: {e}"))
    })?;

    let build_repo_meta = gather_build_repo_meta(&git)?;

    let toplevel_catalog_ref = find_toplevel_group_nixpkgs(&lockfile);

    Ok(CheckedEnvironmentMetadata {
        lockfile,
        build_repo_ref: build_repo_meta,
        toplevel_catalog_ref,
        repo_root_path: git.path().to_path_buf(),
        rel_project_path: rel_project_path.to_path_buf(),
        _private: (),
    })
}

pub fn check_package_metadata(
    lockfile: &Lockfile,
    expression_build_ref: &BaseCatalogUrl,
    toplevel_catalog_ref: Option<&BaseCatalogUrl>,
    pkg: PackageTarget,
) -> Result<PackageMetadata, PublishError> {
    // When publishing a manifest build the toplevel nixpkgs is required as the base url.
    // for expression builds we want to use the extenally determined base url, i.e. stability.
    //
    // We should not need this, and allow for no base catalog page dependency.
    // But for now, requiring it simplifies resolution and model updates
    // significantly.
    let base_catalog_ref = if pkg.kind() == PackageTargetKind::ManifestBuild {
        toplevel_catalog_ref.cloned().ok_or_else(|| {
            PublishError::UnsupportedEnvironmentState("No packages in toplevel group".to_string())
        })?
    } else {
        expression_build_ref.clone()
    };

    let description = lockfile
        .manifest
        .build
        .inner()
        .get(*pkg.name().as_ref())
        .and_then(|desc| desc.description.clone())
        .unwrap_or_default();

    Ok(PackageMetadata {
        package: pkg,
        description,
        base_catalog_ref,
        _private: (),
    })
}

#[cfg(test)]
pub mod tests {

    // Defined in the manifest.toml in
    const EXAMPLE_PACKAGE_NAME: &str = "mypkg";
    static EXAMPLE_MANIFEST_PACKAGE_TARGET: LazyLock<PackageTarget> = LazyLock::new(|| {
        PackageTarget::new_unchecked(EXAMPLE_PACKAGE_NAME, PackageTargetKind::ManifestBuild)
    });

    const EXAMPLE_MANIFEST: &str = "envs/publish-simple";

    use std::io::Write;
    use std::sync::LazyLock;

    use catalog_api_v1::types::CatalogStoreConfigNixCopy;
    use chrono::Utc;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::flox::FloxhubToken;
    use crate::flox::test_helpers::{create_test_token, flox_instance};
    use crate::models::environment::path_environment::PathEnvironment;
    use crate::models::environment::path_environment::test_helpers::new_path_environment_from_env_files_in;
    use crate::models::lockfile::Lockfile;
    use crate::providers::auth::{Auth, write_floxhub_netrc};
    use crate::providers::catalog::test_helpers::reset_mocks;
    use crate::providers::catalog::{
        GENERATED_DATA,
        MockClient,
        PublishResponse,
        Response,
        mock_base_catalog_url,
    };
    use crate::providers::git::tests::{
        commit_file,
        create_remotes,
        get_remote_url,
        init_temp_repo,
    };
    use crate::providers::nix::test_helpers::known_store_path;

    fn example_git_remote_repo() -> (tempfile::TempDir, GitCommandProvider, String) {
        let tempdir_handle = tempfile::tempdir_in(std::env::temp_dir()).unwrap();

        let repo = GitCommandProvider::init(tempdir_handle.path(), true).unwrap();

        let remote_uri = format!("file://{}", tempdir_handle.path().display());

        (tempdir_handle, repo, remote_uri)
    }

    fn local_nix_cache(
        token: &FloxhubToken,
    ) -> (tempfile::NamedTempFile, ClientSideCatalogStoreConfig) {
        // Returns a temp local cache and signing key file to use in testing publish
        let temp_dir = tempfile::tempdir_in(std::env::temp_dir()).unwrap();
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

        let cache_url = format!("file://{}", temp_dir.path().display());
        let parsed_cache_url = Url::parse(&cache_url).unwrap();
        let key_file_path = temp_key_file.path().to_path_buf();
        let auth_file = write_floxhub_netrc(temp_dir.path(), token).unwrap();
        let catalog_store = ClientSideCatalogStoreConfig::NixCopy {
            ingress_uri: parsed_cache_url.clone(),
            egress_uri: parsed_cache_url.clone(),
            auth_netrc_path: auth_file.to_path_buf(),
            signing_private_key_path: key_file_path,
        };
        (temp_key_file, catalog_store)
    }

    fn example_path_environment(
        flox: &Flox,
        remote: Option<&String>,
    ) -> (PathEnvironment, GitCommandProvider) {
        let repo_root = tempfile::tempdir_in(&flox.temp_dir).unwrap().keep();
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

        let meta = check_environment_metadata(&flox, &env);
        meta.expect_err("Should fail due to not being a git repo");
    }

    #[test]
    fn test_check_env_meta_dirty() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();
        let (env, _git) = example_path_environment(&flox, Some(&remote_uri));

        let meta = check_environment_metadata(&flox, &env);
        assert!(meta.is_ok());

        std::fs::write(env.manifest_path(&flox).unwrap(), "dirty content")
            .expect("to write some additional text to the .flox");

        let meta = check_environment_metadata(&flox, &env);
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

        let meta = check_environment_metadata(&flox, &env);
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

        let meta = check_environment_metadata(&flox, &env);
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

        let meta = check_environment_metadata(&flox, &env).unwrap();

        let build_repo_meta = meta.build_repo_ref;
        assert!(build_repo_meta.url.contains(&remote_uri));
        assert!(
            build_repo
                .contains_commit(build_repo_meta.rev.as_str())
                .is_ok()
        );
        assert_eq!(build_repo_meta.rev_count, 1);
    }

    #[test]
    fn test_check_package_meta_nominal() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();
        let (mut env, _) = example_path_environment(&flox, Some(&remote_uri));

        let lockfile = env.lockfile(&flox).unwrap().into();

        let toplevel_catalog_url = find_toplevel_group_nixpkgs(&lockfile);

        let meta = check_package_metadata(
            &lockfile,
            &mock_base_catalog_url(),
            toplevel_catalog_url.as_ref(),
            EXAMPLE_MANIFEST_PACKAGE_TARGET.clone(),
        )
        .unwrap();
        let description_in_manifest = "Some sample package description from our tests";

        // Only the toplevel group in this example, so we can grab the first package
        let locked_base_pkg = lockfile.packages[0].as_catalog_package_ref().unwrap();
        assert_eq!(
            meta.base_catalog_ref.to_string(),
            locked_base_pkg.locked_url
        );
        assert_eq!(&meta.package, &*EXAMPLE_MANIFEST_PACKAGE_TARGET);
        assert_eq!(meta.description, description_in_manifest.to_string());
    }

    #[test]
    fn test_check_build_meta_nominal() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();

        let (env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let env_metadata = check_environment_metadata(&flox, &env).unwrap();

        // This will actually run the build
        let meta =
            check_build_metadata(&flox, &env_metadata, &EXAMPLE_MANIFEST_PACKAGE_TARGET).unwrap();

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
        let (mut flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();
        let (env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let token = create_test_token("test");
        let catalog_name = token.handle().to_string();
        flox.floxhub_token = Some(token);

        let env_metadata = check_environment_metadata(&flox, &env).unwrap();
        let package_metadata = check_package_metadata(
            &env_metadata.lockfile,
            &mock_base_catalog_url(),
            env_metadata.toplevel_catalog_ref.as_ref(),
            EXAMPLE_MANIFEST_PACKAGE_TARGET.clone(),
        )
        .unwrap();

        let build_metadata =
            check_build_metadata(&flox, &env_metadata, &package_metadata.package).unwrap();

        let auth = Auth::from_flox(&flox).unwrap();
        let publish_provider = PublishProvider::new(env_metadata, package_metadata, auth);

        reset_mocks(&mut flox.catalog_client, vec![
            Response::CreatePackage,
            Response::Publish(PublishResponse {
                ingress_uri: None,
                ingress_auth: None,
                catalog_store_config: CatalogStoreConfig::MetaOnly,
            }),
            Response::PublishBuild,
        ]);

        let package_created = publish_provider
            .create_package(&flox.catalog_client, &catalog_name)
            .await
            .unwrap();
        let res = publish_provider
            .publish(
                &flox.catalog_client,
                &catalog_name,
                package_created,
                &build_metadata,
                None,
                false,
            )
            .await;

        assert!(res.is_ok(), "Expected publish to succeed, got: {:?}", res);
    }

    /// Generate dummy CheckedBuildMetadata and CheckedEnvironmentMetadata that
    /// can be passed to publish()
    ///
    /// It is dummy in the sense that no human thought about it ;)
    fn dummy_publish_metadata() -> (
        CheckedBuildMetadata,
        CheckedEnvironmentMetadata,
        PackageMetadata,
    ) {
        let build_metadata = CheckedBuildMetadata {
            name: "dummy".to_string(),
            pname: "dummy".to_string(),
            outputs: Outputs(vec![Output {
                name: "out".to_string(),
                store_path: known_store_path().to_string_lossy().to_string(),
            }]),
            outputs_to_install: vec![],
            drv_path: "dummy".to_string(),
            system: SystemEnum::X8664Linux,
            version: Some("1.0.0".to_string()),
            _private: (),
        };

        let env_metadata = CheckedEnvironmentMetadata {
            lockfile: Lockfile::default(),
            repo_root_path: PathBuf::new(),
            rel_project_path: PathBuf::new(),

            toplevel_catalog_ref: Some(mock_base_catalog_url()),
            build_repo_ref: LockedUrlInfo {
                url: "dummy".to_string(),
                rev: "dummy".to_string(),
                rev_count: 0,
                rev_date: Utc::now(),
            },

            _private: (),
        };

        let package_metadata = PackageMetadata {
            base_catalog_ref: mock_base_catalog_url(),
            package: EXAMPLE_MANIFEST_PACKAGE_TARGET.clone(),
            description: "dummy".to_string(),
            _private: (),
        };

        (build_metadata, env_metadata, package_metadata)
    }

    #[tokio::test]
    async fn publish_errors_without_key() {
        let (mut flox, _tempdir) = flox_instance();
        let mut client = Client::Mock(MockClient::new());

        let token = create_test_token("test");
        let catalog_name = token.handle().to_string();
        flox.floxhub_token = Some(token);

        // Don't do a build because it's slow
        let (build_metadata, env_metadata, package_metadata) = dummy_publish_metadata();

        let auth = Auth::from_flox(&flox).unwrap();
        let publish_provider = PublishProvider::new(env_metadata, package_metadata, auth);

        reset_mocks(&mut client, vec![
            Response::CreatePackage,
            Response::Publish(PublishResponse {
                ingress_uri: Some("https://example.com".to_string()),
                ingress_auth: None,
                catalog_store_config: CatalogStoreConfig::NixCopy(CatalogStoreConfigNixCopy {
                    ingress_uri: "https://example.com".to_string(),
                    egress_uri: "https://example.com".to_string(),
                    store_type: "nix-copy".to_string(),
                }),
            }),
        ]);

        let package_created = publish_provider
            .create_package(&client, &catalog_name)
            .await
            .unwrap();

        let result = publish_provider
            .publish(
                &client,
                &catalog_name,
                package_created,
                &build_metadata,
                None,
                false,
            )
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

    // TODO: Replace with readable/writable mocks from fixture file.
    // publish() passes the error details from the server through
    // #[tokio::test]
    // async fn publish_passes_error_details_through() {
    //     let (mut flox, _tempdir) = flox_instance();
    //     let server = MockServer::start();

    //     let token = create_test_token("test");
    //     let catalog_name = token.handle().to_string();
    //     flox.floxhub_token = Some(token.clone());

    //     // Don't do a build because it's slow
    //     let (build_metadata, env_metadata) = dummy_publish_metadata();
    //     let package_name = &env_metadata.package;
    //     let original_url = &env_metadata.build_repo_ref.url;

    //     let packages_mock = server
    //         .create_catalog_package_api_v1_catalog_catalogs_catalog_name_packages_post(
    //             |when, then| {
    //                 when.catalog_name(&str_to_catalog_name(&catalog_name).unwrap())
    //                     .name(&Name::from_str(package_name).unwrap());
    //                 then.ok(&UserPackage {
    //                     catalog: catalog_name.clone(),
    //                     name: package_name.clone(),
    //                     original_url: Some(original_url.clone()),
    //                 });
    //             },
    //         );

    //     let publish_mock = server.publish_request_api_v1_catalog_catalogs_catalog_name_packages_package_name_publish_info_post(|when, then| {
    //         when.catalog_name(&str_to_catalog_name(&catalog_name).unwrap())
    //             .package_name(&str_to_package_name(package_name).unwrap());
    //         then.unprocessable_entity(&ErrorResponse { detail: "Some\nlong\nresponse\nfrom\nthe\nserver".to_string() });
    //     });

    //     let client = Client::Catalog(CatalogClient::new(CatalogClientConfig {
    //         catalog_url: server.base_url(),
    //         floxhub_token: Some(token.secret().to_string()),
    //         extra_headers: Default::default(),
    //     }));

    //     let auth = Auth::from_flox(&flox).unwrap();
    //     let publish_provider = PublishProvider::new(env_metadata, build_metadata, auth);

    //     // We should error even if metadata_only is true
    //     let result = publish_provider
    //         .publish(&client, &catalog_name, None, true)
    //         .await;

    //     packages_mock.assert();
    //     publish_mock.assert();

    //     let err = result.unwrap_err();
    //     assert_eq!(
    //         err.to_string(),
    //         indoc! {"
    //             422 Unprocessable Entity: Some
    //             long
    //             response
    //             from
    //             the
    //             server"}
    //         .to_string()
    //     );
    // }

    #[tokio::test]
    async fn upload_to_local_cache() {
        let (mut flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();
        let (env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let token = create_test_token("test");
        let catalog_name = token.handle().to_string();
        flox.floxhub_token = Some(token.clone());

        let env_metadata = check_environment_metadata(&flox, &env).unwrap();
        let package_metadata = check_package_metadata(
            &env_metadata.lockfile,
            &mock_base_catalog_url(),
            env_metadata.toplevel_catalog_ref.as_ref(),
            EXAMPLE_MANIFEST_PACKAGE_TARGET.clone(),
        )
        .unwrap();

        let build_metadata =
            check_build_metadata(&flox, &env_metadata, &package_metadata.package).unwrap();

        let (_key_file, cache) = local_nix_cache(&token);
        let auth = Auth::from_flox(&flox).unwrap();
        let publish_provider = PublishProvider::new(env_metadata, package_metadata, auth);

        // the 'cache' should be non existent before the publish
        let cache_url = cache.upload_url().unwrap();
        let cache_path = cache_url.to_file_path().unwrap();
        assert!(std::fs::read_dir(&cache_path).is_err());

        reset_mocks(&mut flox.catalog_client, vec![
            Response::CreatePackage,
            Response::Publish(PublishResponse {
                ingress_uri: Some(cache_url.to_string()),
                ingress_auth: None,
                catalog_store_config: CatalogStoreConfig::NixCopy(CatalogStoreConfigNixCopy {
                    ingress_uri: cache_url.to_string(),
                    egress_uri: cache_url.to_string(),
                    store_type: "nix-copy".to_string(),
                }),
            }),
            Response::PublishBuild,
        ]);

        let package_created = publish_provider
            .create_package(&flox.catalog_client, &catalog_name)
            .await
            .unwrap();

        publish_provider
            .publish(
                &flox.catalog_client,
                &catalog_name,
                package_created,
                &build_metadata,
                cache.local_signing_key_path(),
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
