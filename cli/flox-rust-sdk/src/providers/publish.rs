use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::str::FromStr;

use chrono::{DateTime, Utc};
use flox_catalog::{
    BaseCatalogUrl,
    BaseCatalogUrlError,
    BuildType,
    CatalogClientError,
    CatalogStoreConfig,
    CatalogStoreConfigNixCopy,
    ClientTrait,
    NarInfos,
    PackageOutput,
    PackageOutputs,
    PackageSystem,
    PublishResponse,
    UserBuildPublish,
    UserDerivationInfo,
};
use flox_manifest::lockfile::Lockfile;
use git_url_parse::GitUrl;
use indexmap::IndexSet;
use indoc::{formatdoc, indoc};
use itertools::Itertools;
use nef_lock_catalog::LockOptions;
use nef_lock_catalog::lock::{NixFlakeref, lock_url_with_options};
use serde_json::json;
use thiserror::Error;
use tracing::{debug, instrument};
use url::Url;

use super::auth::{AuthError, AuthProvider, CatalogAuth, NixCopyAuth};
use super::build::{
    BuildResult,
    FloxBuildMk,
    ManifestBuilder,
    ManifestBuilderError,
    PackageTarget,
    PackageTargetError,
    PackageTargetKind,
    find_toplevel_group_nixpkgs,
};
use super::git::{GitCommandError, GitCommandGetOriginError, GitCommandProvider, StatusInfo};
use crate::data::CanonicalPath;
use crate::flox::Flox;
use crate::models::environment::{Environment, EnvironmentError, copy_dir_recursive, open_path};
use crate::providers::auth::catalog_auth_to_envs;
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
    async fn create_package_and_possibly_user_catalog(
        &self,
        client: &impl ClientTrait,
        catalog_name: &str,
    ) -> Result<PackageCreatedGuard, PublishError>;
    /// Publish a built package.
    ///
    /// Returns `true` when the caller should wait for an external publisher
    /// to confirm completion (Publisher mode), or `false` when the CLI has
    /// already populated the catalog directly and no wait is needed
    /// (NixCopy and MetadataOnly modes).
    async fn publish(
        &self,
        client: &impl ClientTrait,
        catalog_name: &str,
        package_created: PackageCreatedGuard,
        build_metadata: &CheckedBuildMetadata,
        key_file: Option<PathBuf>,
        metadata_only: bool,
    ) -> Result<bool, PublishError>;
    async fn wait_for_publish_completion(
        &self,
        client: &impl ClientTrait,
        build_metadata: &CheckedBuildMetadata,
        poll_interval_millis: u64,
        timeout_millis: u64,
    ) -> Result<(), PublishError>;
}

/// Simple struct to hold the information of a locked URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteBuildRepoMetadata {
    pub url: String,
    pub ref_: String,
    pub rev: String,
    pub rev_count: u64,
    pub rev_date: DateTime<Utc>,
}

/// Ensures that the required metadata for publishing is consistent from the environment
#[allow(clippy::manual_non_exhaustive)]
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedEnvironmentMetadata {
    pub lockfile: Lockfile,
    /// The local path of the root of the repo containing the environment.
    pub repo_root_path: PathBuf,

    /// The path to the parent of .flox for the build environment relative to the `repo_root_path`.
    pub rel_project_path: PathBuf,

    /// The path to the directory containing expression builds in
    /// `pkgs/` and locked catalogs in `nix-builds.lock`, typically `.../.flox`.
    ///  Paths are relative to the `repo_root_path`.
    pub rel_expression_build_base_dir: PathBuf,

    /// Metadata about the remote source of the repository.
    /// Used to fill in lock information on publish to allow reproduction,
    /// and accessing recipes through NEF.
    pub build_repo_meta: RemoteBuildRepoMetadata,

    /// A locked Nixpkgs reference for the `toplevel` package group, which
    /// may be absent when the user has no packages installed.
    pub toplevel_catalog_ref: Option<BaseCatalogUrl>,

    // This field isn't "pub", so no one outside this module can construct this struct. That helps
    // ensure that we can only make this struct as a result of doing the "right thing."
    _private: (),
}

impl CheckedEnvironmentMetadata {
    /// Create a canonical flakeref for NEF builds from the metadata collected for the git remote.
    fn remote_flakeref(&self) -> Result<NixFlakeref, PublishError> {
        let value = json! ({
          "type": "git",
          "url": self.build_repo_meta.url,
          "rev": self.build_repo_meta.rev,
          "dir": self.rel_expression_build_base_dir.to_string_lossy()
        });
        NixFlakeref::try_from(value.clone()).map_err(|e| {
            PublishError::Catchall(format!(
                "internal constructed flakeref should be valid.\nvalue: {value}\nerror: {e}"
            ))
        })
    }
}

/// Ensures that the required metadata for publishing is consistent from the build process
#[allow(clippy::manual_non_exhaustive)]
#[derive(Debug, Clone, PartialEq)]
pub struct CheckedBuildMetadata {
    // Define metadata coming from the build, e.g. outpaths
    pub name: String,
    pub pname: String,
    pub outputs: PackageOutputs,
    pub outputs_to_install: Option<Vec<String>>,
    pub drv_path: String,
    pub system: PackageSystem,

    pub description: Option<String>,
    pub license: Option<String>,
    pub broken: Option<bool>,
    pub insecure: Option<bool>,
    pub unfree: Option<bool>,

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
    ///
    /// Returns the narinfos and the URL string identifying where the narinfos
    /// were collected from, or `None` when no narinfos are available.
    pub fn maybe_upload_artifacts(
        &self,
        build_outputs: &[PackageOutput],
    ) -> Result<Option<(NarInfos, String)>, PublishError> {
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
                let nar_infos = Self::get_build_output_nar_infos(
                    Some(egress_uri.as_str()),
                    Some(auth_netrc_path.as_path()),
                    build_outputs,
                )?;
                Ok(Some((nar_infos, egress_uri.to_string())))
            },
            ClientSideCatalogStoreConfig::MetadataOnly => {
                debug!(
                    reason = "metadata-only catalog store",
                    "collecting narinfo from local store (no artifact upload)"
                );
                // MetadataOnly populates the catalog directly from the local
                // daemon store. If narinfo collection fails, the publish would
                // silently submit without NAR info, leaving the catalog entry
                // incomplete. Propagate the error so the user sees a clear
                // failure instead of a silent no-op.
                // Pass None so nix honors NIX_REMOTE / its own store config
                // rather than being forced to use a local daemon socket that
                // may not exist (e.g. environments using a remote SSH-NG store).
                let nar_infos = Self::get_build_output_nar_infos(None, None, build_outputs)?;
                Ok(Some((nar_infos, "daemon://".to_string())))
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
        build_outputs: &[PackageOutput],
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
    ///
    /// Uses `--recursive` to collect narinfo for the full closure (the store
    /// path and all its transitive dependencies), matching the behavior of
    /// the catalog-publisher.
    fn nar_info_cmd(
        store_url: Option<&str>,
        store_path: &str,
        auth_netrc_path: Option<&Path>,
    ) -> Command {
        let mut cmd = nix_base_command();
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        if let Some(netrc) = auth_netrc_path {
            cmd.arg("--netrc-file").arg(netrc);
        }
        cmd.args(["path-info", "--recursive", "--closure-size", "--json"]);
        // Only pass --store when an explicit store URL is provided. Omitting it
        // lets nix honor NIX_REMOTE (or its own config), which is required for
        // environments that use a remote store rather than a local daemon.
        if let Some(url) = store_url {
            cmd.args(["--store", url]);
        }
        cmd.arg(store_path);
        cmd
    }

    /// Gets the NAR info for **the closure** of a store path from the given store.
    #[instrument(skip_all, fields(progress = format!("Collecting extra build metadata for '{store_path}'")))]
    fn get_nar_info(
        source_url: Option<&str>,
        store_path: &str,
        auth_netrc_path: Option<&Path>,
    ) -> Result<NarInfos, PublishError> {
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
        let narinfos = serde_json::from_slice::<NarInfos>(&output.stdout)
            .map_err(PublishError::ParseNarInfo)?;
        if !narinfos.contains_key(store_path) {
            return Err(PublishError::GetNarInfo(formatdoc! {
                "NAR info for store path '{store_path}' not found in response: {narinfos:?}"
            }));
        }
        Ok(narinfos)
    }

    /// Retrieves and merges the [NarInfos] closures of the provided
    /// build outputs from the given store.
    fn get_build_output_nar_infos(
        source_url: Option<&str>,
        auth_netrc_path: Option<&Path>,
        build_outputs: &[PackageOutput],
    ) -> Result<NarInfos, PublishError> {
        let mut nar_infos = HashMap::new();
        for output in build_outputs.iter() {
            debug!(
                output = output.name,
                store_path = output.store_path,
                store = source_url.unwrap_or("local"),
                "querying NAR info for build output"
            );
            let output_nar_infos =
                Self::get_nar_info(source_url, &output.store_path, auth_netrc_path)?;
            nar_infos.extend(output_nar_infos.0.into_iter());
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
    async fn create_package_and_possibly_user_catalog(
        &self,
        client: &impl ClientTrait,
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
                &self.env_metadata.build_repo_meta.url,
            )
            .await
            .map_err(PublishError::CatalogError)?;

        Ok(PackageCreatedGuard { _private: () })
    }

    /// Publish a built package.
    ///
    /// [PackageCreatedGuard] must be obtained from [Self::create_package].
    ///
    /// Returns `true` when the caller should poll for publisher confirmation,
    /// `false` when the CLI already populated the catalog (NixCopy/MetadataOnly).
    async fn publish(
        &self,
        client: &impl ClientTrait,
        catalog_name: &str,
        _package_created: PackageCreatedGuard,
        build_metadata: &CheckedBuildMetadata,
        key_file: Option<PathBuf>,
        metadata_only: bool,
    ) -> Result<bool, PublishError> {
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

        let catalog_store_config = get_client_side_catalog_store_config(
            metadata_only,
            key_file,
            &self.auth,
            publish_response,
        )?;
        // Only the Publisher mode requires the caller to poll for confirmation.
        // NixCopy and MetadataOnly populate the catalog directly, so no wait needed.
        let needs_publisher_wait = matches!(
            catalog_store_config,
            ClientSideCatalogStoreConfig::Publisher { .. }
        );
        let upload_result = catalog_store_config.maybe_upload_artifacts(&build_metadata.outputs)?;
        let (narinfos, narinfos_source_url) = match upload_result {
            Some((nar_infos, source_url)) => (Some(nar_infos), Some(source_url)),
            None => (None, None),
        };

        let build_info = UserBuildPublish {
            derivation: UserDerivationInfo {
                description: build_metadata.description.clone(),
                drv_path: build_metadata.drv_path.clone(),
                license: build_metadata.license.clone(),
                // TODO: populate licenses
                licenses: None,
                name: build_metadata.name.clone(),
                outputs: build_metadata.outputs.clone(),
                outputs_to_install: build_metadata.outputs_to_install.clone(),
                pname: Some(build_metadata.pname.clone()),
                system: build_metadata.system,
                broken: build_metadata.broken,
                unfree: build_metadata.unfree,
                version: build_metadata.version.clone(),
            },
            locked_base_catalog_url: Some(self.package_metadata.base_catalog_ref.to_string()),
            base_catalog_rev_count: None,
            base_catalog_rev_date: None,
            url: self.env_metadata.build_repo_meta.url.clone(),
            rev: self.env_metadata.build_repo_meta.rev.clone(),
            rev_count: self.env_metadata.build_repo_meta.rev_count as i64,
            rev_date: self.env_metadata.build_repo_meta.rev_date,
            ref_: Some(self.env_metadata.build_repo_meta.ref_.clone()),
            cache_uri: catalog_store_config.upload_url().map(|url| url.to_string()),
            narinfos,
            narinfos_source_url,
            // This is the version of the narinfo being submitted.  Until we
            // define changes, we'll use the service defaults.
            narinfos_source_version: None,
            build_type: match self.package_metadata.package.kind() {
                PackageTargetKind::ExpressionBuild(_) => BuildType::Nef,
                PackageTargetKind::ManifestBuild => BuildType::Manifest,
            }
            .into(),
            dot_flox_dir: self
                .env_metadata
                .rel_expression_build_base_dir
                .to_string_lossy()
                .into_owned(),
        };

        tracing::debug!(?build_info, "Publishing build in catalog...");
        client
            .publish_build(
                &catalog_name,
                self.package_metadata.package.name().as_ref(),
                &build_info,
            )
            .await
            .map_err(PublishError::CatalogError)?;

        Ok(needs_publisher_wait)
    }

    /// Waits until the narinfos for all store paths are present in the catalog,
    /// or errors on timeout.
    async fn wait_for_publish_completion(
        &self,
        client: &impl ClientTrait,
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
    auth: &dyn AuthProvider,
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
                let netrc = auth.create_netrc().map_err(PublishError::Auth)?;
                ClientSideCatalogStoreConfig::NixCopy {
                    ingress_uri: Url::parse(&ingress_uri).map_err(|e| {
                        PublishError::Catchall(format!("failed to parse ingress URI: {e}"))
                    })?,
                    egress_uri: Url::parse(&egress_uri).map_err(|e| {
                        PublishError::Catchall(format!("failed to parse egress URI: {e}"))
                    })?,
                    signing_private_key_path: path,
                    auth_netrc_path: netrc.to_path_buf(),
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

/// Convert a [BuildResult] into a form that can more easily fill the publish api request.
///
/// Note: This function is an implementation detail of [check_build_metadata].
/// In almost any case [check_build_metadata] should be used to obtain [CheckedBuildMetadata]!
fn convert_build_result_to_build_metadata(
    build_result: &BuildResult,
) -> Result<CheckedBuildMetadata, PublishError> {
    let outputs = build_result
        .outputs
        .clone()
        .into_iter()
        .map(|(output_name, output_path)| PackageOutput {
            name: output_name,
            store_path: output_path.to_string_lossy().to_string(),
        })
        .collect::<Vec<_>>()
        .into();

    // Get outputs to install from the build result, or default to all outputs.
    let outputs_to_install = build_result.meta.outputs_to_install.clone();

    // Wrapping `outputs_to_install` in an option to satisfy the API.
    // In practice outputs_to_install are required / must be always `Some`
    // so a future change will update the API to reflect that.
    let outputs_to_install = Some(outputs_to_install);

    let license = match &build_result.meta.license {
        Some(lic) => Some(lic.to_catalog_license()?),
        None => None,
    };

    Ok(CheckedBuildMetadata {
        drv_path: build_result.drv_path.clone(),
        name: build_result.name.clone(),
        pname: build_result.pname.clone(),

        description: build_result.meta.description.clone(),
        license,
        broken: build_result.meta.broken,
        insecure: build_result.meta.insecure,
        unfree: build_result.meta.unfree,

        outputs,
        outputs_to_install,
        system: PackageSystem::from_str(build_result.system.as_str()).map_err(|_e| {
            PublishError::UnsupportedEnvironmentState("Invalid system".to_string())
        })?,
        version: Some(build_result.version.clone()),
        _private: (),
    })
}

/// Collect metadata needed for publishing that is obtained from the build output
///
/// Notably, [CheckedBuildMetadata] obtained from this function testifies:
/// * That the remote source is accessible
/// * That the package can be built
pub fn check_build_metadata(
    flox: &Flox,
    base_nixpkgs_url: &BaseCatalogUrl,
    system_override: Option<String>,
    env_metadata: &CheckedEnvironmentMetadata,
    pkg: &PackageTarget,
) -> Result<CheckedBuildMetadata, PublishError> {
    // Fetch remote sources based on the source info collected in `CheckedEnvironmentMetadata`.
    // This serves several purposes:
    // 1. It ensures that the source info we have is indeed valid and accessible
    // 2. It provides a guaranteed clean checkout that is consistent
    //    with reproduction/catalog import.
    // 3. It reduces coupling of publish to _the_ local repo
    let expression_ref = env_metadata.remote_flakeref()?;
    let expression_ref_fetched = lock_url_with_options(&expression_ref, &LockOptions::default())
        .map_err(|e| PublishError::Catchall(e.to_string()))?;
    let expression_ref_locked = expression_ref_fetched.locked_flakeref();

    // git clone into a temp directory
    let clean_repo_path = tempfile::tempdir_in(&flox.temp_dir)
        .map_err(|err| PublishError::Catchall(format!("could not create tempdir: {err}")))?
        .keep();

    // base dir and buildtime environments **for manifest builds**
    // both are inferred from the fetched source,
    // based on relative directories of the local environment.
    // Similar assumptions are made by the NEF at  eval time.
    let (base_dir, built_environments) = {
        copy_dir_recursive(expression_ref_fetched.store_path(), &clean_repo_path, false)
            .map_err(|e| PublishError::Catchall(e.to_string()))?;
        let project_path =
            CanonicalPath::new(clean_repo_path.join(env_metadata.rel_project_path.as_path()))
                .map_err(|_err| {
                    PublishError::UnsupportedEnvironmentState(
                    "Flox project not found in clean checkout, is it tracked in the repository?"
                        .to_string(),
                )
                })?;
        let mut clean_build_env = open_path(flox, &project_path, None)
            .map_err(|e| PublishError::UnsupportedEnvironmentState(e.to_string()))?;
        (clean_build_env.parent_path()?, clean_build_env.build(flox)?)
    };

    let builder = FloxBuildMk::new(flox, &base_dir, &expression_ref_locked, &built_environments);

    // Build the package and collect the outputs
    let build_results = builder.build(
        &base_nixpkgs_url.as_flake_ref()?,
        &built_environments.develop,
        &[pkg.name()],
        Some(false),
        system_override.clone(),
    )?;

    if build_results.len() != 1 {
        return Err(PublishError::NonexistentOutputs(
            "No results returned from build command.".to_string(),
        ));
    }
    let build_result = &build_results[0];
    convert_build_result_to_build_metadata(build_result)
}

/// Creates an error for a build repo that's in an invalid state.
pub fn build_repo_err(msg: &str) -> PublishError {
    PublishError::UnsupportedEnvironmentState(msg.to_string())
}

/// Verify that the critical environment files are tracked by git.
/// Publishing creates a clean checkout, so untracked files won't be available.
fn check_env_files_tracked(
    git: &impl GitProvider,
    dot_flox_path: &impl AsRef<Path>,
) -> Result<(), PublishError> {
    // Find files in `.flox/` that are untracked and not ignored according to
    // the rules generated in `.flox/.gitignore`.
    let untracked_files = git
        .list_files_untracked(dot_flox_path.as_ref())
        .map_err(|e| {
            PublishError::UnsupportedEnvironmentState(format!("Failed to check git tracking: {e}"))
        })?;

    if !untracked_files.is_empty() {
        let listing = untracked_files
            .iter()
            .map(|path| format!("- {path}"))
            .join("\n");
        return Err(build_repo_err(&formatdoc! {"
            The following environment files are not tracked by git:
            {listing}",
        }));
    }
    Ok(())
}

/// Check the local repo that the build source is in to make sure that it's in
/// a state amenable to publishing an artifact built from it.
///
/// This entails checking that:
/// - The repo has a remote configured.
/// - The tracked source files are clean.
/// - The current revision exists on the tracked remote branch.
#[instrument(skip_all, fields(progress = "Checking repository state"))]
fn gather_build_repo_meta(
    git: &GitCommandProvider,
) -> Result<RemoteBuildRepoMetadata, PublishError> {
    let status = git
        .status()
        .map_err(|_e| build_repo_err("Unable to get repository status."))?;

    if status.is_dirty {
        return Err(build_repo_err(
            "Build repository must be clean, but has dirty tracked files.",
        ));
    }

    let remote_info = git.get_current_branch_remote_info().map_err(|e| match e {
        GitCommandGetOriginError::NoUpstream => {
            let remote_hint = git
                .remotes()
                .ok()
                .and_then(|r| match r.as_slice() {
                    [single] if !single.is_empty() => Some(single.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "<remote>".to_string());

            if let Some(branch) = status
                .ref_
                .as_deref()
                .and_then(|r| r.strip_prefix("refs/heads/"))
            {
                build_repo_err(&formatdoc! {"
                    Current branch '{branch}' has no upstream remote configured.
                    Set one with 'git branch --set-upstream-to={remote_hint}/{branch}'"
                })
            } else {
                build_repo_err(&formatdoc! {"
                    Repository is in detached HEAD state and has no upstream remote configured.
                    Check out a branch before publishing: \
                        git checkout -b <branch-name>"
                })
            }
        },
        GitCommandGetOriginError::AccessDenied(ref cmd_err) => build_repo_err(&formatdoc! {"
            Could not access the remote repository: {cmd_err}
            Check your SSH agent (`ssh-add -l`) or credential configuration."
        }),
        GitCommandGetOriginError::Command(ref cmd_err) => build_repo_err(&cmd_err.to_string()),
    })?;

    let rev_on_remote = match git.rev_exists_on_remote(&status.rev, &remote_info.name) {
        Ok(exists) => exists,
        Err(ref cmd_err) if cmd_err.is_access_denied() => {
            return Err(build_repo_err(&formatdoc! {"
                Could not access remote '{remote_name}' while verifying the local revision: {cmd_err}
                Check your SSH agent (`ssh-add -l`) or credential configuration.",
                remote_name = remote_info.name,
            }));
        },
        Err(cmd_err) => {
            return Err(build_repo_err(&formatdoc! {"
                Failed to check whether local revision exists on remote '{remote_name}/{remote_branch}': {cmd_err}",
                remote_name = remote_info.name,
                remote_branch = remote_info.reference,
            }));
        },
    };
    if !rev_on_remote {
        return Err(build_repo_err(&formatdoc! {"
            Local revision is not present on remote '{remote_name}/{remote_branch}'.
            Push your commits with 'git push'",
            remote_name = remote_info.name,
            remote_branch = remote_info.reference,
        }));
    }

    let url =
        GitUrl::parse_to_url(&remote_info.url).map_err(|err| build_repo_err(&err.to_string()))?;

    Ok(RemoteBuildRepoMetadata {
        url: url.to_string(),
        rev: status.rev,
        rev_count: status.rev_count,
        rev_date: status.rev_date,
        ref_: remote_info.reference,
    })
}

// TODO: remove after discussion reg UX of this change.
#[allow(unused)]
fn url_for_remote_containing_current_rev(
    git: &impl GitProvider,
    status: &StatusInfo,
) -> Result<String, PublishError> {
    let remote_names = git.remotes()?;
    if remote_names.is_empty() {
        return Err(build_repo_err(
            "The repository must have at least one remote configured.",
        ));
    }

    // A revision may be present on multiple remotes so we want to take the one
    // that's most canonical and likely to persist, e.g. if a repo has been
    // forked without adding any commits then we should favour the upstream
    // instead of the fork.
    //
    // Check the configured remotes, once each, in order of..
    let mut ordered_remotes = IndexSet::new();
    // 1. Tracked remote for branch, if configured.
    if let Ok(tracked_remote) = git.get_current_branch_remote_info() {
        ordered_remotes.insert(tracked_remote.name);
    }
    // 2. Preferred remotes, if they are present.
    for preferred_remote in ["upstream", "origin"] {
        let name = preferred_remote.to_string();
        if remote_names.contains(&name) {
            ordered_remotes.insert(name);
        }
    }
    // 3. Any remaining remotes that haven't already been added.
    //    The order of these is not guaranteed to be stable.
    ordered_remotes.extend(remote_names.clone());

    for remote_name in ordered_remotes {
        if git
            .rev_exists_on_remote(&status.rev, &remote_name)
            // Continue checking other remotes if this remote is misconfigured.
            .unwrap_or_else(|err| {
                debug!(%err, remote_name, "Failed to check if current revision exists on remote");
                false
            })
        {
            return git
                .remote_url(&remote_name)
                .map_err(|_| build_repo_err("Failed to get remote URL for the current revision."));
        }
    }

    Err(build_repo_err(
        "Current revision not found on any remote repositories.",
    ))
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

    let dot_flox_path = environment.dot_flox_path();
    let rel_dot_flox_dir = dot_flox_path.strip_prefix(git.path()).map_err(|e| {
        PublishError::UnsupportedEnvironmentState(format!(".flox/ dir not in git repo: {e}"))
    })?;

    check_env_files_tracked(&git, &dot_flox_path)?;

    let build_repo_meta = gather_build_repo_meta(&git)?;
    let toplevel_catalog_ref = find_toplevel_group_nixpkgs(&lockfile);

    Ok(CheckedEnvironmentMetadata {
        lockfile,
        build_repo_meta,
        toplevel_catalog_ref,
        repo_root_path: git.path().to_path_buf(),
        rel_project_path: rel_project_path.to_path_buf(),
        rel_expression_build_base_dir: rel_dot_flox_dir.to_path_buf(),
        _private: (),
    })
}

pub fn check_package_metadata(
    expression_build_ref: &BaseCatalogUrl,
    toplevel_catalog_ref: Option<&BaseCatalogUrl>,
    pkg: PackageTarget,
) -> Result<PackageMetadata, PublishError> {
    // When publishing a manifest build the toplevel nixpkgs is required as the base url.
    // for expression builds we want to use the externally determined base url, i.e. stability.
    //
    // We should not need this, and allow for no base catalog page dependency.
    // But for now, requiring it simplifies resolution and model updates
    // significantly.
    let base_catalog_ref = if pkg.kind() == &PackageTargetKind::ManifestBuild {
        toplevel_catalog_ref.cloned().ok_or_else(|| {
            PublishError::UnsupportedEnvironmentState("No packages in toplevel group".to_string())
        })?
    } else {
        expression_build_ref.clone()
    };

    Ok(PackageMetadata {
        package: pkg,
        base_catalog_ref,
        _private: (),
    })
}

#[cfg(test)]
pub mod tests {

    // Defined in the manifest.toml in
    const EXAMPLE_PACKAGE_NAME: &str = "mypkg";
    const EXAMPLE_PACKAGE_NAME_MISSING_FIELDS: &str = "mypkg_missing_fields";
    static EXAMPLE_MANIFEST_PACKAGE_TARGET: LazyLock<PackageTarget> = LazyLock::new(|| {
        PackageTarget::new_unchecked(EXAMPLE_PACKAGE_NAME, PackageTargetKind::ManifestBuild)
    });
    static EXAMPLE_MANIFEST_PACKAGE_TARGET_MISSING_FIELDS: LazyLock<PackageTarget> =
        LazyLock::new(|| {
            PackageTarget::new_unchecked(
                EXAMPLE_PACKAGE_NAME_MISSING_FIELDS,
                PackageTargetKind::ManifestBuild,
            )
        });

    const EXAMPLE_MANIFEST: &str = "envs/publish-simple";

    use std::io::Write;
    use std::sync::LazyLock;

    use chrono::Utc;
    use flox_manifest::interfaces::{AsWritableManifest, WriteManifest};
    use flox_test_utils::GENERATED_DATA;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::flox::FloxhubToken;
    use crate::flox::test_helpers::{
        PublishTestUser,
        create_test_token,
        flox_instance,
        set_test_auth,
    };
    use crate::models::environment::ENVIRONMENT_POINTER_FILENAME;
    use crate::models::environment::path_environment::PathEnvironment;
    use crate::models::environment::path_environment::test_helpers::new_path_environment_from_env_files_in;
    use crate::providers::auth::{Auth, write_floxhub_netrc};
    use crate::providers::catalog::test_helpers::{
        TEST_READ_ONLY_CATALOG_NAME,
        TEST_READ_WRITE_CATALOG_NAME,
        UNIT_TEST_GENERATED,
        auto_recording_catalog_client_for_authed_local_services,
        reset_mocks,
    };
    use crate::providers::catalog::{Response, get_base_nixpkgs_url, mock_base_catalog_url};
    use crate::providers::git::tests::{
        commit_file,
        create_remotes,
        get_remote_url,
        init_temp_repo,
        test_git_options,
    };
    use crate::providers::nix::test_helpers::known_store_path;

    fn example_git_remote_repo() -> (tempfile::TempDir, GitCommandProvider, String) {
        let tempdir_handle = tempfile::tempdir_in(std::env::temp_dir()).unwrap();

        let repo =
            GitCommandProvider::init_with(test_git_options(), tempdir_handle.path(), true).unwrap();

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

        let git = GitCommandProvider::init_with(test_git_options(), repo_root, false).unwrap();

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
            format!(
                "{}\n",
                env.manifest_without_migrating(&flox)
                    .unwrap()
                    .as_writable()
                    .to_string()
            ),
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

        let build_repo_meta = meta.build_repo_meta;
        assert!(build_repo_meta.url.contains(&remote_uri));
        assert!(
            build_repo
                .contains_commit(build_repo_meta.rev.as_str())
                .is_ok()
        );
        assert_eq!(build_repo_meta.rev_count, 1);
    }

    #[test]
    fn test_check_env_files_tracked_success() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();
        let (env, git) = example_path_environment(&flox, Some(&remote_uri));

        check_env_files_tracked(&git, &env.dot_flox_path())
            .expect("all env files should be tracked");
    }

    #[test]
    fn test_check_env_files_tracked_untracked_file() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();
        let (env, git) = example_path_environment(&flox, Some(&remote_uri));

        let env_json_path = env.dot_flox_path().join(ENVIRONMENT_POINTER_FILENAME);
        git.rm(&[env_json_path.as_path()], false, false, true)
            .expect("cached remove of env.json");

        let result = check_env_files_tracked(&git, &env.dot_flox_path());
        match result {
            Err(PublishError::UnsupportedEnvironmentState(msg)) => {
                assert_eq!(
                    msg,
                    indoc! {"
                    The following environment files are not tracked by git:
                    - subdir_for_flox_stuff/.flox/env.json"}
                    .to_string()
                );
            },
            _ => panic!("Expected UnsupportedEnvironmentState error"),
        }
    }

    #[test]
    fn test_check_package_meta_nominal() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();
        let (mut env, _) = example_path_environment(&flox, Some(&remote_uri));

        let lockfile = env.lockfile(&flox).unwrap().into();

        let toplevel_catalog_url = find_toplevel_group_nixpkgs(&lockfile);

        let meta = check_package_metadata(
            &mock_base_catalog_url(),
            toplevel_catalog_url.as_ref(),
            EXAMPLE_MANIFEST_PACKAGE_TARGET.clone(),
        )
        .unwrap();

        // Only the toplevel group in this example, so we can grab the first package
        let locked_base_pkg = lockfile.packages[0].as_catalog_package_ref().unwrap();
        assert_eq!(
            meta.base_catalog_ref.to_string(),
            locked_base_pkg.locked_url
        );
        assert_eq!(&meta.package, &*EXAMPLE_MANIFEST_PACKAGE_TARGET);
    }

    #[test]
    fn test_check_build_meta_nominal() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();

        let (env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let env_metadata = check_environment_metadata(&flox, &env).unwrap();

        // This will actually run the build
        let meta = check_build_metadata(
            &flox,
            env_metadata.toplevel_catalog_ref.as_ref().unwrap(),
            None,
            &env_metadata,
            &EXAMPLE_MANIFEST_PACKAGE_TARGET,
        )
        .unwrap();

        let version_in_manifest = "1.0.2a";
        let description_in_manifest = "Some sample package description from our tests";
        let _license_in_manifest = "[\"my very private license\"]";

        assert_eq!(meta.outputs.len(), 1);
        assert_eq!(meta.outputs_to_install, Some(vec!["out".to_string()]));
        assert_eq!(meta.outputs[0].store_path.starts_with("/nix/store/"), true);
        assert_eq!(meta.drv_path.starts_with("/nix/store/"), true);
        assert_eq!(meta.version, Some(version_in_manifest.to_string()));
        assert_eq!(meta.pname, EXAMPLE_PACKAGE_NAME.to_string());
        assert_eq!(meta.system.to_string(), flox.system);

        assert_eq!(meta.description, Some(description_in_manifest.to_string()));
        // Note that this is different than what's in the manifest.  This is a
        // result of the processing through the build process into build
        // results, and processing from there as a NixyLicense.  The formatting
        // of the license between nix and the catalog is very inconsistent and
        // lossy unfortunately.  We'll need to address that, but for now, we
        // choose to be consistent in the processing between them.  The
        // processing is to join the licenses, without quotes and spaces around
        // the brackets.   i.e. - "[ {<licenses joined with commas>} ]"
        assert_eq!(
            meta.license,
            Some("[ my very private license ]".to_string())
        );
    }

    #[test]
    fn test_check_build_meta_null_fields() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();

        let (env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let env_metadata = check_environment_metadata(&flox, &env).unwrap();

        // This will actually run the build
        let meta = check_build_metadata(
            &flox,
            env_metadata.toplevel_catalog_ref.as_ref().unwrap(),
            None,
            &env_metadata,
            &EXAMPLE_MANIFEST_PACKAGE_TARGET_MISSING_FIELDS,
        )
        .unwrap();

        assert_eq!(meta.outputs.len(), 1);
        assert_eq!(meta.outputs_to_install, Some(vec!["out".to_string()]));
        assert_eq!(meta.outputs[0].store_path.starts_with("/nix/store/"), true);
        assert_eq!(meta.drv_path.starts_with("/nix/store/"), true);
        assert_eq!(meta.pname, EXAMPLE_PACKAGE_NAME_MISSING_FIELDS.to_string());
        assert_eq!(meta.system.to_string(), flox.system);

        // We apply a default version if none is specified, set in flox-build.mk
        assert_eq!(meta.version, Some("0.0.0".to_string()));
        assert_eq!(meta.description, None);
        assert_eq!(meta.license, None);
    }

    #[tokio::test]
    async fn publish_meta_only() {
        let (mut flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();
        let (env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        set_test_auth(&mut flox, "test");
        let catalog_name = "test".to_string();

        let env_metadata = check_environment_metadata(&flox, &env).unwrap();
        let package_metadata = check_package_metadata(
            &mock_base_catalog_url(),
            env_metadata.toplevel_catalog_ref.as_ref(),
            EXAMPLE_MANIFEST_PACKAGE_TARGET.clone(),
        )
        .unwrap();

        let build_metadata = check_build_metadata(
            &flox,
            env_metadata.toplevel_catalog_ref.as_ref().unwrap(),
            None,
            &env_metadata,
            &package_metadata.package,
        )
        .unwrap();

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
            .create_package_and_possibly_user_catalog(&flox.catalog_client, &catalog_name)
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
        // MetadataOnly submits narinfos directly — no external publisher to wait for.
        assert_eq!(
            res.unwrap(),
            false,
            "MetadataOnly should not require publisher wait"
        );
    }

    #[test]
    fn metadata_only_collects_narinfos_from_local_store() {
        let (flox, _temp_dir_handle) = flox_instance();
        let (_tempdir_handle, _remote_repo, remote_uri) = example_git_remote_repo();
        let (env, _build_repo) = example_path_environment(&flox, Some(&remote_uri));

        let env_metadata = check_environment_metadata(&flox, &env).unwrap();
        let package_metadata = check_package_metadata(
            &mock_base_catalog_url(),
            env_metadata.toplevel_catalog_ref.as_ref(),
            EXAMPLE_MANIFEST_PACKAGE_TARGET.clone(),
        )
        .unwrap();

        let build_metadata = check_build_metadata(
            &flox,
            env_metadata.toplevel_catalog_ref.as_ref().unwrap(),
            None,
            &env_metadata,
            &package_metadata.package,
        )
        .unwrap();

        let config = ClientSideCatalogStoreConfig::MetadataOnly;

        let result = config
            .maybe_upload_artifacts(&build_metadata.outputs)
            .expect("metadata-only narinfo collection should succeed");

        // MetadataOnly should return Some((narinfos, source_url))
        let (narinfos, source_url) = result.expect("narinfos should be Some for metadata-only");

        // The source URL should be "daemon://" for metadata-only
        assert_eq!(
            source_url, "daemon://",
            "MetadataOnly should report 'daemon://' as narinfos source"
        );

        // Should contain at least the build output store path
        assert!(
            !narinfos.is_empty(),
            "Expected narinfos to be non-empty for metadata-only"
        );

        // The build output's store path should be in the narinfos
        for output in build_metadata.outputs.iter() {
            assert!(
                narinfos.contains_key(&output.store_path),
                "Expected narinfos to contain build output store path: {}",
                output.store_path
            );
        }

        // With --recursive, there should be more entries than just the
        // build outputs (transitive dependencies)
        assert!(
            narinfos.len() > build_metadata.outputs.len(),
            "Expected recursive narinfos to include transitive dependencies, \
             got {} entries for {} outputs",
            narinfos.len(),
            build_metadata.outputs.len()
        );
    }

    #[test]
    fn metadata_only_returns_daemon_source_url() {
        let config = ClientSideCatalogStoreConfig::MetadataOnly;
        // MetadataOnly with no build outputs returns None (no narinfos to collect)
        let result = config.maybe_upload_artifacts(&[]).unwrap();
        assert!(result.is_none(), "empty outputs should return None");

        // With actual outputs we cannot test in a unit test (requires nix store),
        // but the source URL is verified in
        // metadata_only_collects_narinfos_from_local_store below.
    }

    #[test]
    fn null_store_returns_no_narinfos() {
        let config = ClientSideCatalogStoreConfig::Null;
        let result = config
            .maybe_upload_artifacts(&[PackageOutput {
                name: "out".to_string(),
                store_path: "/nix/store/fake-path".to_string(),
            }])
            .unwrap();
        assert!(result.is_none(), "Null store should return None");
    }

    /// MetadataOnly must propagate narinfo collection failures rather than
    /// silently submitting without NAR info and leaving the catalog entry
    /// incomplete.
    #[test]
    fn metadata_only_propagates_narinfo_error() {
        let config = ClientSideCatalogStoreConfig::MetadataOnly;
        // A store path that does not exist in the local daemon store will cause
        // get_build_output_nar_infos to fail. The error must surface to the
        // caller instead of being swallowed.
        let result = config.maybe_upload_artifacts(&[PackageOutput {
            name: "out".to_string(),
            store_path: "/nix/store/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA-nonexistent".to_string(),
        }]);
        assert!(
            result.is_err(),
            "MetadataOnly should return Err when narinfo collection fails, got: {:?}",
            result
        );
    }

    /// Generate dummy CheckedBuildMetadata and CheckedEnvironmentMetadata that
    /// can be passed to publish()
    ///
    /// It is dummy in the sense that no human thought about it ;)
    fn dummy_publish_metadata(
        pkg_name: &str,
    ) -> (
        CheckedBuildMetadata,
        CheckedEnvironmentMetadata,
        PackageMetadata,
    ) {
        // A bare revision is written to this file when generating the mocks.
        let nixpkgs_rev =
            std::fs::read_to_string(UNIT_TEST_GENERATED.join("latest_dev_catalog_rev.txt"))
                .expect("failed to read catalog rev file")
                .trim()
                .to_string();
        let base_catalog_url = format!("https://github.com/flox/nixpkgs?rev={nixpkgs_rev}");
        let catalog_page_nixpkgs_https_url = BaseCatalogUrl::from(base_catalog_url);

        let build_metadata = CheckedBuildMetadata {
            name: pkg_name.to_string(),
            pname: pkg_name.to_string(),
            outputs: vec![PackageOutput {
                name: "out".to_string(),
                store_path: "/nix/store/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA-foo".to_string(),
            }]
            .into(),
            outputs_to_install: None,
            drv_path: "dummy".to_string(),
            system: PackageSystem::X8664Linux,
            version: Some("1.0.0".to_string()),
            description: Some("dummy".to_string()),
            license: None,
            broken: Some(false),
            insecure: None,
            unfree: None,
            _private: (),
        };

        let env_metadata = CheckedEnvironmentMetadata {
            lockfile: Lockfile::default(),
            repo_root_path: PathBuf::new(),
            rel_project_path: PathBuf::new(),
            rel_expression_build_base_dir: PathBuf::new(),

            toplevel_catalog_ref: Some(catalog_page_nixpkgs_https_url.clone()),
            build_repo_meta: RemoteBuildRepoMetadata {
                url: "https://dummy.local".to_string(),
                rev: "dummy".to_string(),
                ref_: "dummy".to_string(),
                rev_count: 0,
                rev_date: "2025-01-01T12:00:00Z".parse::<DateTime<Utc>>().unwrap(),
            },

            _private: (),
        };

        let package_metadata = PackageMetadata {
            base_catalog_ref: catalog_page_nixpkgs_https_url,
            package: PackageTarget::new_unchecked(pkg_name, PackageTargetKind::ManifestBuild),
            _private: (),
        };

        (build_metadata, env_metadata, package_metadata)
    }

    #[tokio::test]
    async fn publish_errors_without_key() {
        let (mut flox, _tempdir) = flox_instance();

        set_test_auth(&mut flox, "test");
        let catalog_name = "test".to_string();

        // Don't do a build because it's slow
        let (build_metadata, env_metadata, package_metadata) = dummy_publish_metadata("mypkg1");

        let auth = Auth::from_flox(&flox).unwrap();
        let publish_provider = PublishProvider::new(env_metadata, package_metadata, auth);

        reset_mocks(&mut flox.catalog_client, vec![
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
            .create_package_and_possibly_user_catalog(&flox.catalog_client, &catalog_name)
            .await
            .unwrap();

        let result = publish_provider
            .publish(
                &flox.catalog_client,
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

        set_test_auth(&mut flox, "test");
        let catalog_name = "test".to_string();

        let env_metadata = check_environment_metadata(&flox, &env).unwrap();
        let package_metadata = check_package_metadata(
            &mock_base_catalog_url(),
            env_metadata.toplevel_catalog_ref.as_ref(),
            EXAMPLE_MANIFEST_PACKAGE_TARGET.clone(),
        )
        .unwrap();

        let build_metadata = check_build_metadata(
            &flox,
            env_metadata.toplevel_catalog_ref.as_ref().unwrap(),
            None,
            &env_metadata,
            &package_metadata.package,
        )
        .unwrap();

        let (_key_file, cache) = local_nix_cache(flox.floxhub_token.as_ref().unwrap());
        let auth = Auth::from_flox(&flox).unwrap();
        let publish_provider = PublishProvider::new(env_metadata, package_metadata, auth);

        // the 'cache' should be nonexistent before the publish
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
            .create_package_and_possibly_user_catalog(&flox.catalog_client, &catalog_name)
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
        cmd.args([
            "branch",
            "--set-upstream-to",
            &format!("{}/{}", remote_name, branch_name),
        ]);
        GitCommandProvider::run_command(&mut cmd).unwrap();
        let remote_url =
            url_for_remote_containing_current_rev(&build_repo, &build_repo.status().unwrap())
                .unwrap();
        assert_eq!(remote_url, get_remote_url(&remotes, remote_name));
    }

    #[test]
    fn falls_back_to_untracked_remote() {
        let remote_name = "some_remote";
        let branch_name = "some_branch";
        let (build_repo, _tempdir) = init_temp_repo(false);
        let remotes = create_remotes(&build_repo, &[remote_name, "other_remote"]);

        // Push to a tracking remote, to mimic a freshly cloned non-fork.
        commit_file(&build_repo, "foo");
        build_repo
            .create_branch(branch_name, &build_repo.status().unwrap().rev)
            .unwrap();
        build_repo.checkout(branch_name, false).unwrap();
        build_repo
            .push_ref("other_remote", branch_name, false)
            .unwrap();
        let mut tracking_cmd = build_repo.new_command();
        tracking_cmd.args([
            "branch",
            "--set-upstream-to",
            &format!("{}/{}", "other_remote", branch_name),
        ]);
        GitCommandProvider::run_command(&mut tracking_cmd).unwrap();

        // Commit and push a new rev to a different remote, to mimic a fork.
        commit_file(&build_repo, "bar");
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
    fn falls_back_to_any_pushed_remote() {
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

    #[test]
    fn error_when_not_pushed_to_any_remote() {
        let branch_name = "some_branch";
        let (build_repo, _tempdir) = init_temp_repo(false);
        commit_file(&build_repo, "foo");
        let status = build_repo.status().unwrap();
        build_repo.create_branch(branch_name, &status.rev).unwrap();
        create_remotes(&build_repo, &["some_remote", "other_remote"]);
        let err = url_for_remote_containing_current_rev(&build_repo, &build_repo.status().unwrap())
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            build_repo_err("Current revision not found on any remote repositories.").to_string()
        )
    }

    #[test]
    fn test_get_nar_info() {
        let token = create_test_token("test");

        let (flox, _temp_dir_handle) = flox_instance();
        let auth_file = write_floxhub_netrc(flox.temp_dir.as_path(), &token).unwrap();

        // The known_store_path includes `/bin/nix`.
        let store_path = {
            let mut full_path = known_store_path();
            full_path.pop(); // drop `nix`
            full_path.pop(); // drop `bin/`
            full_path
        };
        let store_path_str = store_path.to_str().unwrap();
        let narinfos =
            ClientSideCatalogStoreConfig::get_nar_info(None, store_path_str, Some(&auth_file))
                .unwrap();
        // With --recursive, narinfos contains the queried path and its
        // transitive dependencies.
        assert!(
            narinfos.contains_key(store_path_str),
            "Expected narinfos to contain the queried store path"
        );
        let narinfo = &narinfos[store_path_str];
        assert!(
            narinfo.closure_size.is_some(),
            "Expected narinfo to have a closure size"
        );
        assert!(
            narinfo.nar_hash.is_some(),
            "Expected narinfo to have a nar hash"
        );
        assert!(
            narinfo.nar_size.is_some(),
            "Expected narinfo to have a nar size"
        );
        assert!(
            narinfo.references.is_some(),
            "Expected narinfo to have a references field"
        );
        // --recursive should return more entries than just the queried path
        assert!(
            narinfos.len() > 1,
            "Expected --recursive narinfo to include transitive dependencies, got {} entries",
            narinfos.len()
        );
    }

    // This test isn't really for testing publish functionality, but instead
    // for testing that we've hooked up local services correctly for generating
    // publish test mocks.
    #[tokio::test(flavor = "multi_thread")]
    async fn retrieves_base_catalog_url() {
        let (_build_meta, env_meta, _pkg_meta) = dummy_publish_metadata("mypkg2");
        let (flox, _tmpdir) = flox_instance();
        let (flox, _auth) = auto_recording_catalog_client_for_authed_local_services(
            flox,
            PublishTestUser::NoCatalogs,
            "get_base_catalog_nixpkgs_url",
        );
        let _url = get_base_nixpkgs_url(&flox, Some("stable"), &env_meta)
            .await
            .unwrap();
    }

    // This test ensures that a user's default catalog gets created inline
    // if it doesn't already exist so that individual users can publish
    // without first needing to pay and create an organization.
    #[tokio::test(flavor = "multi_thread")]
    async fn publishes_new_package_for_users_default_catalog_and_creates_catalog() {
        let (build_meta, env_meta, pkg_meta) = dummy_publish_metadata("mypkg3");
        let (flox, _tmpdir) = flox_instance();
        let (flox, auth) = auto_recording_catalog_client_for_authed_local_services(
            flox,
            PublishTestUser::NoCatalogs,
            "publish_provider_publishes_package_in_users_catalog",
        );
        let user_handle = flox
            .floxhub_token
            .expect("expected token to be present")
            .handle()
            .to_string();
        let publish_provider = PublishProvider::new(env_meta, pkg_meta, auth);
        let packaged_created_guard = publish_provider
            .create_package_and_possibly_user_catalog(&flox.catalog_client, &user_handle)
            .await
            .unwrap();
        publish_provider
            .publish(
                &flox.catalog_client,
                &user_handle,
                packaged_created_guard,
                &build_meta,
                None,
                // Server returns Null store config so no narinfo collection
                false,
            )
            .await
            .expect("failed to do publish");
    }

    // This test ensures that a user is able to publish to a catalog other than
    // their personal catalog assuming (1) the catalog already exists, and
    // (2) they have write permissions.
    #[tokio::test(flavor = "multi_thread")]
    async fn publishes_new_package_for_org_catalog() {
        let (build_meta, env_meta, pkg_meta) = dummy_publish_metadata("mypkg4");
        let (flox, _tmpdir) = flox_instance();
        let (flox, auth) = auto_recording_catalog_client_for_authed_local_services(
            flox,
            PublishTestUser::WithCatalogs,
            "publish_provider_creates_package_in_org_catalog",
        );
        let publish_provider = PublishProvider::new(env_meta, pkg_meta, auth);
        let packaged_created_guard = publish_provider
            .create_package_and_possibly_user_catalog(
                &flox.catalog_client,
                // This catalog name matches one that the test user has r/w
                // access to as defined in _FLOXHUB_TEST_USERS.json from the floxhub repo.
                TEST_READ_WRITE_CATALOG_NAME,
            )
            .await
            .unwrap();
        publish_provider
            .publish(
                &flox.catalog_client,
                TEST_READ_WRITE_CATALOG_NAME,
                packaged_created_guard,
                &build_meta,
                None,
                // Server returns Null store config so no narinfo collection
                false,
            )
            .await
            .expect("failed to do publish");
    }

    // This test was intended to ensure that a user gets an error if they try to
    // publish to a catalog that exists but that they don't have write access to.
    // However, ownership (the user targeting their personal catalog) takes precedence
    // over roles, and so the user WILL be able to publish.
    // To properly test this, we would need two user, one to create the catalog,
    // and another to try and publish to it where they have only READER access.
    // The mocks currently do not support this.
    //
    // Additionally, this is covered by the service and integration tests.
    // For now we can leave this with a should_panic attribute.
    #[tokio::test(flavor = "multi_thread")]
    #[should_panic]
    async fn error_publishing_to_read_only_catalog() {
        let (build_meta, env_meta, pkg_meta) = dummy_publish_metadata("mypkg5");
        let (flox, _tmpdir) = flox_instance();
        let (flox, auth) = auto_recording_catalog_client_for_authed_local_services(
            flox,
            PublishTestUser::WithCatalogs,
            "publish_provider_error_when_user_only_has_read_access_to_catalog",
        );
        let publish_provider = PublishProvider::new(env_meta, pkg_meta, auth);
        let guard = publish_provider
            .create_package_and_possibly_user_catalog(
                &flox.catalog_client,
                // This catalog name matches one that the test user has read-only
                // access to as defined in _FLOXHUB_TEST_USERS.json from the floxhub repo.
                TEST_READ_ONLY_CATALOG_NAME,
            )
            .await
            .unwrap();
        let res = publish_provider
            .publish(
                &flox.catalog_client,
                TEST_READ_ONLY_CATALOG_NAME,
                guard,
                &build_meta,
                None,
                // Server returns Null store config so no narinfo collection
                false,
            )
            .await;
        assert!(res.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn repeat_publish_succeeds() {
        // This test is ensuring that you can publish a package with the same
        // metadata more than once. Whether that makes sense is a separate
        // concern, so this test is just identifying current behavior.
        let (build_meta, env_meta, pkg_meta) = dummy_publish_metadata("mypkg6");
        let (flox, _tmpdir) = flox_instance();
        let (flox, auth) = auto_recording_catalog_client_for_authed_local_services(
            flox,
            PublishTestUser::WithCatalogs,
            "repeat_publish_of_existing_package_succeeds",
        );
        let publish_provider = PublishProvider::new(env_meta, pkg_meta, auth);
        let packaged_created_guard = publish_provider
            .create_package_and_possibly_user_catalog(
                &flox.catalog_client,
                TEST_READ_WRITE_CATALOG_NAME,
            )
            .await
            .unwrap();
        publish_provider
            .publish(
                &flox.catalog_client,
                TEST_READ_WRITE_CATALOG_NAME,
                packaged_created_guard,
                &build_meta,
                None,
                // Server returns Null store config so no narinfo collection
                false,
            )
            .await
            .expect("failed to do publish");
        publish_provider
            .publish(
                &flox.catalog_client,
                TEST_READ_WRITE_CATALOG_NAME,
                // The guard is consumed by the publish, so we need to create
                // a new one.
                PackageCreatedGuard { _private: () },
                &build_meta,
                None,
                // Server returns Null store config so no narinfo collection
                false,
            )
            .await
            .expect("failed to do publish");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn error_publishing_nonexistent_package() {
        // This test is ensuring that you get an error if you haven't called
        // `create_package`. We have a guard that prevents this, so again this
        // test is just ensuring that we've correctly identified the current
        // behavior.
        let (build_meta, env_meta, pkg_meta) = dummy_publish_metadata("mypkg7");
        let (flox, _tmpdir) = flox_instance();
        let (flox, auth) = auto_recording_catalog_client_for_authed_local_services(
            flox,
            PublishTestUser::WithCatalogs,
            "error_from_publish_provider_when_publishing_package_not_yet_created",
        );
        let publish_provider = PublishProvider::new(env_meta, pkg_meta, auth);
        let res = publish_provider
            .publish(
                &flox.catalog_client,
                flox.floxhub_token
                    .expect("expected token to exist")
                    .handle(),
                PackageCreatedGuard { _private: () },
                &build_meta,
                None,
                // Server returns Null store config so no narinfo collection
                false,
            )
            .await;
        assert!(res.is_err());
    }

    // ---- gather_build_repo_meta error differentiation tests ----

    #[test]
    fn gather_repo_meta_no_upstream_suggests_set_upstream() {
        // A repo with a commit and a remote but no upstream tracking
        // branch should produce an error using the actual remote name.
        let (_remote_tempdir, _remote_repo, remote_uri) = example_git_remote_repo();
        let (git, _tempdir) = init_temp_repo(false);
        git.checkout("main", true).unwrap();
        commit_file(&git, "init.txt");
        git.add_remote("upstream", &remote_uri).unwrap();

        let err = gather_build_repo_meta(&git).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("--set-upstream-to=upstream/main"),
            "Expected suggestion with actual remote name, got: {msg}"
        );
    }

    #[test]
    fn gather_repo_meta_no_upstream_no_remote_uses_placeholder() {
        // A repo with no remotes at all should use a placeholder in
        // the set-upstream-to suggestion.
        let (git, _tempdir) = init_temp_repo(false);
        git.checkout("main", true).unwrap();
        commit_file(&git, "init.txt");

        let err = gather_build_repo_meta(&git).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("--set-upstream-to=<remote>/main"),
            "Expected placeholder <remote> in suggestion, got: {msg}"
        );
    }

    #[test]
    fn gather_repo_meta_revision_not_on_remote_suggests_push() {
        // When the local revision is not present on the remote, the
        // error should mention the remote/branch and suggest `git push`.
        let (_remote_tempdir, _remote_repo, remote_uri) = example_git_remote_repo();
        let (git, _tempdir) = init_temp_repo(false);
        git.checkout("main", true).unwrap();
        commit_file(&git, "first.txt");
        git.add_remote("origin", &remote_uri).unwrap();
        git.push("origin", true).unwrap();

        // Create a local commit that hasn't been pushed
        commit_file(&git, "local_only.txt");

        let err = gather_build_repo_meta(&git).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("origin/main"),
            "Expected 'origin/main' in message, got: {msg}"
        );
        assert!(
            msg.contains("git push"),
            "Expected 'git push' suggestion, got: {msg}"
        );
    }

    #[test]
    fn gather_repo_meta_dirty_repo_mentions_dirty_files() {
        // A repo with uncommitted changes should produce an error
        // about dirty tracked files.
        let (_remote_tempdir, _remote_repo, remote_uri) = example_git_remote_repo();
        let (git, _tempdir) = init_temp_repo(false);
        git.checkout("main", true).unwrap();
        commit_file(&git, "init.txt");
        git.add_remote("origin", &remote_uri).unwrap();
        git.push("origin", true).unwrap();

        // Dirty the repo by modifying a tracked file without committing
        std::fs::write(git.path().join("init.txt"), "modified").unwrap();

        let err = gather_build_repo_meta(&git).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("dirty"),
            "Expected 'dirty' in message, got: {msg}"
        );
    }
}
