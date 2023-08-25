use std::collections::BTreeMap;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{fs, io};

use derive_more::{Deref, DerefMut, Display};
use flox_types::catalog::cache::{CacheMeta, SubstituterUrl};
use flox_types::catalog::System;
use flox_types::stability::Stability;
use futures::TryFutureExt;
use itertools::Itertools;
use log::{debug, error};
use runix::arguments::common::NixCommonArgs;
use runix::arguments::eval::EvaluationArgs;
use runix::arguments::flake::{FlakeArgs, OverrideInput};
use runix::arguments::{CopyArgs, NixArgs, StoreSignArgs};
use runix::command::{Build, Eval, StoreSign};
use runix::command_line::{NixCommandLine, NixCommandLineRunError, NixCommandLineRunJsonError};
use runix::flake_metadata::FlakeMetadata;
use runix::flake_ref::git::{GitAttributes, GitRef};
use runix::flake_ref::git_service::{service, GitServiceRef};
use runix::flake_ref::indirect::IndirectRef;
use runix::flake_ref::path::PathRef;
use runix::flake_ref::protocol::{WrappedUrl, WrappedUrlParseError};
use runix::flake_ref::{protocol, FlakeRef};
use runix::installable::{AttrPath, FlakeAttribute, Installable};
use runix::store_path::{StorePath, StorePathError};
use runix::url_parser::{InstallableOutputs, UrlParseError};
use runix::{Run, RunJson, RunTyped};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::flox::Flox;
use crate::providers::git::{
    GitCommandError,
    GitCommandGetOriginError,
    GitCommandProvider as Git,
    GitProvider,
};

/// Publish state before analyzing
///
/// Prevents other actions to commence without analyzing the package first
pub struct Empty;

/// Publish state after collecting nix metadata
///
/// JSON value (ideally a [flox_types::catalog::CatalogEntry],
/// but that's currently broken on account of some flakerefs)
#[derive(Debug, Deref, DerefMut)]
pub struct NixAnalysis(Value);

/// State for the publish algorihm
///
/// The analysis field tracks the transition from Empty -> NixAnalysis to ensure we don't invoke invalid operations
pub struct Publish<'flox, State> {
    /// A shared flox session
    ///
    /// Nearly all commands require shared state from the [Flox] object.
    /// Save a reference to it to simplify the method signatures.
    flox: &'flox Flox,
    /// The published _upstream_ source
    publish_flake_ref: PublishFlakeRef,
    /// The attr_path of the published package in the source flake (`publish_flake_ref`)
    ///
    /// E.g. when publishing `git+https://github.com/flox/flox#packages.aarch64-darwin.flox`
    /// this is: `packages.aarch64-darwin.flox`
    ///
    /// Should be fully resolved to avoid ambiguity
    attr_path: AttrPath,
    stability: Stability,
    analysis: State,
}

impl<'flox> Publish<'flox, Empty> {
    /// Create a new [Publish] instance at first without any metadata
    pub fn new(
        flox: &'flox Flox,
        publish_flake_ref: PublishFlakeRef,
        attr_path: AttrPath,
        stability: Stability,
    ) -> Publish<'flox, Empty> {
        Self {
            flox,
            publish_flake_ref,
            attr_path,
            stability,
            analysis: Empty,
        }
    }

    /// Run analysis on the package and switch to next state.
    ///
    /// We evaluate package metadata as JSON, to which we add
    /// * source URLs for reproducibility
    /// * the nixpkgs stability being used to create the package
    pub async fn analyze(self) -> Result<Publish<'flox, NixAnalysis>, PublishError> {
        let mut drv_metadata_json = self.get_drv_metadata().await?;
        let flake_metadata = self.get_flake_metadata().await?;

        // DEVIATION FROM BASH: using `locked` here instead of `resolved`
        //                      this is used to reproduce the package,
        //                      but is essentially redundant because of the `source.locked`
        // TODO it would be better if we didn't have to do post processing of the analysis for parity with calls to readPackage in https://github.com/flox/floxpkgs/blob/master/modules/common.nix
        drv_metadata_json["element"]["url"] = json!(flake_metadata.locked.to_string());
        drv_metadata_json["source"] = json!({
            "locked": flake_metadata.locked,
            "original": flake_metadata.original,
            "remote": flake_metadata.original,
        });
        drv_metadata_json["eval"]["stability"] = json!(self.stability);

        Ok(Publish {
            flox: self.flox,
            publish_flake_ref: self.publish_flake_ref,
            attr_path: self.attr_path,
            stability: self.stability,
            analysis: NixAnalysis(drv_metadata_json),
        })
    }

    /// Extract metadata of the published derivation using the analyzer flake.
    ///
    /// It uses an analyzer flake to extract eval metadata of the derivation.
    /// The analyzer applies a function to all packages in a `target` flake
    /// and provides the result under `#analysis.eval.<full attrpath of the package>`.
    async fn get_drv_metadata(&self) -> Result<Value, PublishError> {
        let nix: NixCommandLine = self.flox.nix(Default::default());

        // Create the analysis.eval.<full attrpath of the package> attr path
        // taking care to remove any leading `""` from the original attr_path
        // used to signal strict paths (a flox concept, to be upstreamed)
        let analysis_attr_path = {
            let mut attrpath = AttrPath::try_from(["", "analysis", "eval"]).unwrap();
            attrpath.extend(
                self.attr_path
                    .clone()
                    .into_iter()
                    .peekable()
                    .skip_while(|attr| attr.as_ref() == ""),
            );
            attrpath
        };

        // We bundle the analyzer flake with flox (see the package definition for flox)
        let analyzer_flakeref = FlakeRef::Path(PathRef::new(
            PathBuf::from(env!("FLOX_ANALYZER_SRC")),
            Default::default(),
        ));

        let mut override_inputs = [
            // The analyzer flake provides analysis outputs for the flake input `target`
            // Here, we're setting the target flake to our source flake.
            OverrideInput {
                from: "target".to_string(),
                to: self.publish_flake_ref.clone().into_inner(),
            },
        ]
        .to_vec();

        let nixpkgs_flakeref = FlakeRef::Indirect(IndirectRef::new(
            format!("nixpkgs-{}", self.stability),
            Default::default(),
        ));

        // Stabilities are managed by overriding the `flox-floxpkgs/nixpkgs/nixpkgs` input to
        // `nixpkgs-<stability>`.
        // The analyzer flake adds an additional indirection,
        // so we have to do the override manually.
        // However, since https://github.com/flox/flox/pull/182,
        // we only set this when a stability is specified
        // This is the `nixpkgs-<stability>` portion.
        override_inputs.push(OverrideInput {
            from: "target/flox-floxpkgs/nixpkgs/nixpkgs".to_string(),
            to: nixpkgs_flakeref,
        });

        let eval_analysis_command = Eval {
            flake: FlakeArgs {
                override_inputs,
                // The analyzer flake is bundled with flox as a nix store path and thus read-only.
                no_write_lock_file: true.into(),
            },
            eval_args: runix::arguments::EvalArgs {
                installable: Some(
                    FlakeAttribute {
                        flakeref: analyzer_flakeref,
                        attr_path: analysis_attr_path,
                        outputs: InstallableOutputs::Default,
                    }
                    .into(),
                ),
                ..Default::default()
            },
            ..Default::default()
        };

        eval_analysis_command
            .run_json(&nix, &Default::default())
            .map_err(|nix_error| {
                PublishError::DrvMetadata(
                    self.attr_path.clone(),
                    self.publish_flake_ref.to_string(),
                    nix_error,
                )
            })
            .await
    }

    /// Resolve the metadata of the flake holding the published package
    async fn get_flake_metadata(&self) -> Result<FlakeMetadata, PublishError> {
        let nix: NixCommandLine = self.flox.nix(Default::default());

        let locked_ref_command = runix::command::FlakeMetadata {
            flake_ref: Some(self.publish_flake_ref.clone().into_inner().into()),
            ..Default::default()
        };

        locked_ref_command
            .run_typed(&nix, &Default::default())
            .map_err(|nix_err| {
                PublishError::FlakeMetadata(self.publish_flake_ref.to_string(), nix_err)
            })
            .await
    }
}

impl<'flox> Publish<'flox, NixAnalysis> {
    /// Construct an installable type from the upstream flakeref and attrpath
    fn installable(&self) -> Installable {
        FlakeAttribute {
            flakeref: self.publish_flake_ref.clone().into_inner(),
            attr_path: self.attr_path.clone(),
            outputs: InstallableOutputs::All,
        }
        .into()
    }

    /// Extract the store paths from the snapshot
    ///
    /// This should be equivalent to `<installable>^*` without evaluation.
    /// Since the publish evaluates purely, nix's eval cache may serve the same purpose now.
    ///
    /// Todo: https://github.com/flox/runix/issues/41
    fn store_paths(&self) -> Result<Vec<Installable>, PublishError> {
        let store_paths = self.analysis()["element"]["storePaths"]
            .as_array()
            // TODO use CatalogEntry and then we don't need to unwrap
            .unwrap()
            .iter()
            .map(|value| {
                // TODO use CatalogEntry and then we don't need to unwrap
                StorePath::from_path(value.as_str().unwrap())
                    .map_err(PublishError::ParseStorePath)
                    .map(Installable::StorePath)
            })
            .collect::<Result<Vec<Installable>, _>>()?;
        Ok(store_paths)
    }

    /// Read out the current publish state
    pub fn analysis(&self) -> &Value {
        self.analysis.deref()
    }

    pub async fn build(&self) -> Result<(), PublishError> {
        let nix = self.flox.nix(Default::default());

        let command = Build {
            installables: [self.installable()].into(),
            flake: FlakeArgs {
                override_inputs: [self.stability.as_override()].into(),
                ..Default::default()
            },
            ..Default::default()
        };

        command
            .run(&nix, &Default::default())
            .await
            .map_err(PublishError::Build)?;
        Ok(())
    }

    /// Sign the binary
    ///
    /// The current implementation does not involve any state transition,
    /// making signing an optional operation.
    ///
    /// Requires a valid signing key
    pub async fn sign_binary(&self, key_file: impl AsRef<Path>) -> Result<(), PublishError> {
        let nix = self.flox.nix(Default::default());

        let sign_command = StoreSign {
            store_sign: StoreSignArgs {
                key_file: key_file.as_ref().into(),
                recursive: Some(true.into()),
            },
            installables: self.store_paths()?.into(),
            eval: Default::default(),
            flake: Default::default(),
        };

        sign_command
            .run(&nix, &Default::default())
            .await
            .map_err(PublishError::SignPackage)?;

        Ok(())
    }

    /// Copy the outputs and dependencies of the package to binary store
    pub async fn upload_binary(
        &self,
        substituter: Option<SubstituterUrl>,
    ) -> Result<(), PublishError> {
        let nix: NixCommandLine = self.flox.nix(Default::default());
        let copy_command = runix::command::NixCopy {
            installables: self.store_paths()?.into(),
            eval: EvaluationArgs {
                eval_store: Some("auto".to_string().into()),
                ..Default::default()
            },
            copy_args: CopyArgs {
                to: substituter.clone().map(|url| url.to_string().into()),
                ..Default::default()
            },
            ..Default::default()
        };

        let nix_args = Default::default();

        copy_command
            .run(&nix, &nix_args)
            .map_err(PublishError::Copy)
            .await?;
        Ok(())
    }

    pub async fn check_substituter(
        &mut self,
        substituter: SubstituterUrl,
    ) -> Result<(), PublishError> {
        let cache_meta = self.get_binary_cache_metadata(Some(substituter)).await?;
        let cache_meta_json =
            serde_json::to_value(cache_meta).map_err(PublishError::SerializeCacheMeta)?;
        let mut caches = self
            .analysis
            .0
            .get("cache")
            .map(|v| v.as_array().expect("invalid metadata").to_owned())
            .unwrap_or_default();

        caches.push(cache_meta_json);

        self.analysis.0["cache"] = Value::Array(caches);

        Ok(())
    }

    /// Check whether store paths are substitutable by a given substituter and
    /// return the associated metadata.
    ///
    /// If substituter is None, the local store will be used, which is probably
    /// only useful for testing.
    async fn get_binary_cache_metadata(
        &self,
        substituter: Option<SubstituterUrl>,
    ) -> Result<CacheMeta, PublishError> {
        let nix: NixCommandLine = self.flox.nix(Default::default());
        let path_info_command = runix::command::PathInfo {
            installables: self.store_paths()?.into(),
            eval: EvaluationArgs {
                eval_store: Some("auto".to_string().into()),
                ..Default::default()
            },
            ..Default::default()
        };

        let nix_args = NixArgs {
            common: NixCommonArgs {
                store: substituter.clone().map(|url| url.to_string().into()),
            },
            ..Default::default()
        };

        let narinfos = path_info_command
            .run_typed(&nix, &nix_args)
            .map_err(PublishError::PathInfo)
            .await?;

        Ok(CacheMeta {
            cache_url: substituter.unwrap_or(SubstituterUrl::parse("file:///nix/store").unwrap()),
            narinfo: narinfos,
            _other: BTreeMap::new(),
        })
    }

    /// Write snapshot to catalog and push to origin
    pub async fn push_snapshot(&self) -> Result<(), PublishError> {
        let mut upstream_repo =
            UpstreamRepo::clone_repo(self.publish_flake_ref.clone_url(), &self.flox.temp_dir)
                .await?;
        self.push_snapshot_to(&mut upstream_repo).await
    }

    /// Write snapshot to a catalog and push to 'origin'
    ///
    /// Internal method to test
    async fn push_snapshot_to(&self, upstream_repo: &mut UpstreamRepo) -> Result<(), PublishError> {
        let catalog = upstream_repo
            .get_or_create_catalog(&self.flox.system)
            .await?;
        if let Ok(Some(_)) = catalog.get_snapshot(self.analysis()) {
            Err(PublishError::SnapshotExists)?;
        }
        catalog.add_snapshot(self.analysis()).await?;
        catalog.push_catalog().await?;
        Ok(())
    }
}

/// Representation of an exclusive clone of an upstream repo
///
/// [UpstreamRepo] and [UpstreamCatalog] ensure safe access to individual catalog branches.
/// Every [UpstreamRepo] instance represents an exclusive clone
/// and can only ever create a single [UpstreamCatalog] instance at a time.
struct UpstreamRepo(Git);

impl UpstreamRepo {
    /// Clone an upstream repo
    async fn clone_repo(
        url: impl AsRef<str>,
        temp_dir: impl AsRef<Path>,
    ) -> Result<Self, PublishError> {
        let repo_dir = tempfile::tempdir_in(temp_dir).unwrap().into_path(); // todo catch error
        let repo = <Git as GitProvider>::clone(url.as_ref(), &repo_dir, false).await?;

        Ok(Self(repo))
    }

    fn catalog_branch_name(system: &System) -> String {
        format!("catalog/{system}")
    }

    /// Create an [UpstreamCatalog] by checking out or creating a catalog branch.
    ///
    /// `Git` objects can switch branches at any time leaving the repo in an unknown state.
    /// [get_catalog] ensures that only one [UpstreamCatalog] exists at a time by requiring a `&mut self`.
    async fn get_or_create_catalog(
        &mut self,
        system: &System,
    ) -> Result<UpstreamCatalog, PublishError> {
        if self.0.list_branches().await? // todo: catch error
            .into_iter().any(|info| info.name == Self::catalog_branch_name(system))
        {
            self.0
                .checkout(&Self::catalog_branch_name(system), false)
                .await?; // todo: catch error
        } else {
            self.0
                .checkout(&Self::catalog_branch_name(system), true)
                .await?;
        }
        Ok(UpstreamCatalog(&self.0))
    }
}

/// Representation of a specific catalog branch in an exclusive clone of an upstream repo.
///
/// [UpstreamCatalog] guaranteesd that during its lifetime all operations on the underlying git repo
/// are performed on a single branch,
/// and that the branch is pushed to upstream before a new branch can be checked out.
struct UpstreamCatalog<'a>(&'a Git);

impl UpstreamCatalog<'_> {
    /// Mostly naïve approxiaton of a snapshot path
    ///
    ///  /catalog/<namespace '/'-separated>/<version>-<(filename outPaths[0])[..8]>.json
    ///
    /// see [tests::test_get_snapshot_path] for an example
    fn get_snapshot_path(snapshot: &Value) -> PathBuf {
        let mut path = PathBuf::from("catalog");

        let attr_path = snapshot["eval"]["attrPath"]
            .as_array()
            .unwrap()
            .iter()
            .skip(2)
            .map(|v| v.as_str().unwrap());

        path.extend(attr_path);

        let version = &snapshot["eval"]["version"]
            .as_str()
            .expect("invalid metadata");

        // Add a hash of all storePaths to the snapshot path.
        // This prevents collisions between publishes of different outputs, and
        // it prevents collisions between multiple publishes of the same version
        // of a package when the package has changed.
        let nix_out_hash = &{
            let hasher = snapshot["element"]["storePaths"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().expect("Invalid metadata"))
                .sorted()
                .fold(Sha256::new(), |mut hasher, path| {
                    hasher.update(path);
                    hasher
                });
            format!("{:x}", hasher.finalize())
        }[0..8];

        path.push(format!("{version}-{nix_out_hash}.json"));

        path
    }

    /// Get the workdir of the checked-out repo
    fn workdir(&self) -> &Path {
        self.0.workdir().unwrap()
    }

    /// Try retrieving a snapshot from the catalog
    fn get_snapshot(&self, snapshot: &Value) -> Result<Option<Value>, PublishError> {
        let path = self.workdir().join(Self::get_snapshot_path(snapshot));
        let read_snapshot = match fs::read_to_string(path) {
            Ok(s) => Some(serde_json::from_str(&s)?),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => Err(e)?,
        };

        Ok(read_snapshot)
    }

    /// [Value] to be [flox_types::catalog::CatalogEntry]
    ///
    /// Consumers should check if a snapshot already exists with [Self::get_snapshot]
    async fn add_snapshot(&self, snapshot: &Value) -> Result<(), PublishError> {
        let path = self.workdir().join(Self::get_snapshot_path(snapshot));

        fs::create_dir_all(path.parent().unwrap())?; // only an issue for a git repo in /

        let mut snapshot_file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)?;

        serde_json::to_writer(&mut snapshot_file, snapshot)?;

        self.0.add(&[&path]).await?;
        self.0.commit("Added snapshot").await?; // TODO: pass message in here? commit in separate method?
        Ok(())
    }

    /// Push the catalog branch to origin
    ///
    /// Pushing a catalog consumes the catalog instance,
    /// which in turn enables any other methods on the [UpstreamRepo] that created this instance.
    async fn push_catalog(self) -> Result<(), PublishError> {
        self.0.push("origin").await?;
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum PublishError {
    #[error("Failed to load metadata for the package '{0}' in '{1}': {2}")]
    DrvMetadata(AttrPath, String, NixCommandLineRunJsonError),

    #[error("Failed to load metadata for flake '{0}': {1}")]
    FlakeMetadata(String, NixCommandLineRunJsonError),

    #[error("Failed to sign package: {0}")]
    SignPackage(NixCommandLineRunError),

    #[error("Failed reading snapshot data: {0}")]
    ReadSnapshot(#[from] serde_json::Error),

    #[error("Failed to run git operation: {0}")]
    GitOperation(#[from] GitCommandError),

    #[error("Failed to run IO operation: {0}")]
    IoOperation(#[from] std::io::Error),

    #[error("Already published")]
    SnapshotExists,

    #[error("Failed to parse store path {0}")]
    ParseStorePath(StorePathError),

    #[error("Failed to invoke path-info: {0}")]
    PathInfo(NixCommandLineRunJsonError),

    #[error("Failed to invoke build: {0}")]
    Build(NixCommandLineRunError),

    #[error("Failed to invoke copy: {0}")]
    Copy(NixCommandLineRunError),

    #[error("Failed to serialize cache metadata: {0}")]
    SerializeCacheMeta(serde_json::Error),
}

/// Publishable FlakeRefs
///
/// `publish` modifies branches of the source repository.
/// Thus we can only publish to repositories in (remote*) git repositories.
/// This enum represents the subset of flakerefs we can use,
/// so we can avoid parsing and converting flakerefs within publish.
/// [GitRef<protocol::File>] should in most cases be resolved to a remote type.
///
/// \* A publish allows other users to substitute or build a package
/// (as long as repo name and references remain available).
/// If you publish a local repository, all urls will refer to local paths,
/// so a snapshot can't be reproduced anywhere but the local machine.
///
/// `flox publish git+file:///somewhere/local#package` would actually resolve git+file:///somewhere/local
/// to the upstream repo defined by the current branch's remote.
#[derive(PartialEq, Eq, Clone, Debug, Display)]
pub enum PublishFlakeRef {
    Ssh(GitRef<protocol::SSH>),
    Https(GitRef<protocol::HTTPS>),
    File(GitRef<protocol::File>),
}

impl PublishFlakeRef {
    /// Extract a URL for cloning with git
    pub fn clone_url(&self) -> String {
        match self {
            PublishFlakeRef::Ssh(ref ssh_ref) => ssh_ref.url.as_str().to_owned(),
            PublishFlakeRef::Https(ref https_ref) => https_ref.url.as_str().to_owned(),
            PublishFlakeRef::File(ref file_ref) => file_ref.url.as_str().to_owned(),
        }
    }

    /// Return the [FlakeRef] type for the wrapped refs
    fn into_inner(self) -> FlakeRef {
        match self {
            PublishFlakeRef::Ssh(ssh_ref) => FlakeRef::GitSsh(ssh_ref),
            PublishFlakeRef::Https(https_ref) => FlakeRef::GitHttps(https_ref),
            PublishFlakeRef::File(file_ref) => FlakeRef::GitPath(file_ref),
        }
    }

    /// Resolve `github:` flakerefs to a "proper" git url.
    ///
    /// Unlike nix operations publish is writing to the targeted git repository.
    /// To allow native git operations on the remote,
    /// the `github:` shorthand must be exanded to an `ssh` or `https` url.
    ///
    /// By default we resolve ssh urls,
    /// since (publishing) users will more likely have ssh configured,
    /// rather than (often insecure) token access.
    /// In cases where https token access is granted by another tool (e.g. gh),
    /// or within github actions we can prefer https URLs,
    /// so not to require an ssh setup to be present.
    #[allow(unused)]
    fn from_github_ref(
        GitServiceRef {
            owner,
            repo,
            attributes,
            ..
        }: GitServiceRef<service::Github>,
        prefer_https: bool,
    ) -> Result<Self, ConvertFlakeRefError> {
        let host = attributes.host.unwrap_or("github.com".to_string());

        let only_rev = attributes.rev.is_some() && attributes.reference.is_none();

        let git_attributes = GitAttributes {
            rev: attributes.rev,
            reference: attributes.reference,
            dir: attributes.dir,
            // Nix needs either a `?ref=` or `allRefs=1` set to fetch a revision
            all_refs: only_rev.then_some(true),
            ..Default::default()
        };

        let publish_flake_ref = if prefer_https {
            let url_str = format!("https://{host}/{owner}/{repo}");
            let url = WrappedUrl::from_str(&url_str)
                .map_err(|e| ConvertFlakeRefError::InvalidResultUrl(url_str, e))?;

            Self::Https(GitRef {
                url,
                attributes: git_attributes,
            })
        } else {
            let url_str = format!("ssh://git@{host}/{owner}/{repo}");
            let url = WrappedUrl::from_str(&url_str)
                .map_err(|e| ConvertFlakeRefError::InvalidResultUrl(url_str, e))?;

            Self::Ssh(GitRef {
                url,
                attributes: git_attributes,
            })
        };

        Ok(publish_flake_ref)
    }

    /// Resolve a git+file flake ref to a git+https reference
    ///
    /// Reproducing a snapshot from source requires access to the original repo.
    /// Including a local file reference in the snapshot
    /// means it's practically only possible to reproduce for the original creator.
    /// Thus, we resolve a local branch to its upstream remote and branch.
    ///
    /// For local repositories we also check
    /// whether the repository contains any uncommitted changes
    /// and is in sync with its upstream branch.
    async fn from_git_file_flake_ref(
        file_ref: GitRef<protocol::File>,
        nix: &NixCommandLine,
    ) -> Result<Self, ConvertFlakeRefError> {
        // Get nix metadata for the referred flake
        // Successfully acquiring metadata proves the path
        // - is in fact a flake
        // - is a git repository
        // - the path is not dirty (no uncommitted changes)
        let local_metadata = {
            let command = runix::command::FlakeMetadata {
                flake_ref: Some(FlakeRef::GitPath(file_ref.clone()).into()),
                ..Default::default()
            };
            command
                .run_typed(nix, &Default::default())
                .await
                .map_err(ConvertFlakeRefError::LocalFlakeMetadata)?
        };

        if local_metadata.revision.is_none() {
            Err(ConvertFlakeRefError::LocalFlakeDirty)?;
        }

        // Create a handle to the git repo
        let repo = Git::discover(file_ref.url.path())
            .await
            .map_err(|_| ConvertFlakeRefError::RepoNotFound(file_ref.url.path().into()))?;

        // Get the upstream branch information.
        // This is essentialy
        //
        //   upstream_ref = git rev-parse @{u}
        //   (remote_name, branch_name) = split_once "/" upstream_ref
        //   upstream_url = git remote get-url ${remote_name}
        //   upstream_rev = git ls-remote ${remote_name} ${branch_name}
        //
        // The current branch MUST have an upstream ref configured
        // which resolves to a valid rev on the remote.
        let remote = repo
            .get_origin()
            .await
            .map_err(ConvertFlakeRefError::NoRemote)?;

        // Ensure the remote branch exists
        let remote_revision = remote
            .revision
            .ok_or(ConvertFlakeRefError::RemoteBranchNotFound)?;

        debug!(
            "Resolved local flake to remote '{name}:{reference}' at '{url}'",
            name = remote.name,
            reference = remote.reference,
            url = remote.url
        );

        // Check whether the local branch is in sync with its upstream branch,
        // to ensure we publish the intended revision
        // by comparing the local revision to the one found upstream.
        //
        // Dirty branches are already permitted due to the filter above.
        if let Some(local_rev) = local_metadata.revision {
            if local_rev.as_ref() != remote_revision {
                Err(ConvertFlakeRefError::RemoteBranchNotSynced(
                    local_rev.to_string(),
                    remote_revision.clone(),
                ))?
            }
        }

        // Git supports special urls for e.g. ssh repos, e.g.
        //
        //   git@github.com:flox/flox
        //
        // Normalize these urls to proper URLs for the use with nix.
        let remote_url = git_url_parse::normalize_url(&remote.url)
            .map_err(|e| ConvertFlakeRefError::UnknownRemoteUrl(e.to_string()))?;

        // Copy the flakeref attributes but unlock it in support of preferred upstream refs
        let mut attributes = file_ref.attributes;
        attributes.last_modified = None;
        attributes.rev_count = None;
        attributes.nar_hash = None;
        // safe unwrap: remote revision is provided by git and is thus expected to match the revision regex
        attributes.rev = Some(
            remote_revision
                .parse()
                .expect("failed parsing revision returned by git"),
        );
        attributes.reference = Some(remote.reference);

        let remote_flake_ref = match remote_url.scheme() {
            "ssh" => Self::Ssh(GitRef::new(
                WrappedUrl::try_from(remote_url).unwrap(),
                attributes,
            )),
            "https" => Self::Https(GitRef::new(
                WrappedUrl::try_from(remote_url).unwrap(),
                attributes,
            )),
            // Resolving to a local remote is an error case most of the time
            // but technically valid and required for testing.
            // `File` variants are filtered out at a higher level
            "file" => Self::File(GitRef::new(
                WrappedUrl::try_from(remote_url).unwrap(),
                attributes,
            )),
            _ => Err(ConvertFlakeRefError::UnsupportedGitUrl(remote_url))?,
        };
        Ok(remote_flake_ref)
    }

    pub async fn from_flake_ref(
        flake_ref: FlakeRef,
        flox: &Flox,
        git_service_prefer_https: bool,
    ) -> Result<Self, ConvertFlakeRefError> {
        // This should really be a recursive function, but that requires two things:
        // - Making this return type Pin<Box<impl Future<Output = ...>>>
        // - That all Futures that this one depends on are Send
        // It turns out we can't do that because the GitProvider trait doesn't require the Futures it
        // returns to be Send. Instead of a recursive call we do this loop until we no longer have
        // an indirect flake reference. It should run a maximum of twice (once if `flake_ref` isn't indirect,
        // twice if it is indirect).
        let flake_ref = if let FlakeRef::Indirect(indirect) = flake_ref {
            indirect.resolve()?
        } else {
            flake_ref
        };
        match flake_ref {
            FlakeRef::GitSsh(ssh_ref) => Ok(Self::Ssh(ssh_ref)),
            FlakeRef::GitHttps(https_ref) => Ok(Self::Https(https_ref)),
            // resolve upstream for local git repo
            FlakeRef::GitPath(file_ref) => {
                Self::from_git_file_flake_ref(file_ref, &flox.nix(Default::default())).await
            },
            FlakeRef::Github(github_ref) => {
                Self::from_github_ref(github_ref, git_service_prefer_https)
            },
            FlakeRef::Gitlab(_) => todo!(),
            // Resolving an indirect reference shouldn't give you back
            // another indirect reference.
            FlakeRef::Indirect(_) => unreachable!(),
            _ => Err(ConvertFlakeRefError::UnsupportedTarget(flake_ref.clone())),
        }
    }
}

impl PartialEq<FlakeRef> for PublishFlakeRef {
    fn eq(&self, other: &FlakeRef) -> bool {
        match (self, other) {
            (PublishFlakeRef::Https(this), FlakeRef::GitHttps(other)) => this == other,
            (PublishFlakeRef::Ssh(this), FlakeRef::GitSsh(other)) => this == other,
            (PublishFlakeRef::File(this), FlakeRef::GitPath(other)) => this == other,
            _ => false,
        }
    }
}

impl PartialEq<PublishFlakeRef> for FlakeRef {
    fn eq(&self, other: &PublishFlakeRef) -> bool {
        other == self
    }
}

/// Errors arising from convert
#[derive(Error, Debug)]
pub enum ConvertFlakeRefError {
    #[error("Unsupported flakeref for publish: {0}")]
    UnsupportedTarget(FlakeRef),

    #[error("Invalid URL after conversion: {0}: {1}")]
    InvalidResultUrl(String, WrappedUrlParseError),

    #[error("Couldn't find a local git repository in: {0}")]
    RepoNotFound(PathBuf),

    #[error("Couldn't find remote")]
    NoRemote(GitCommandGetOriginError),

    #[error("Couldn't get metadata for local flake")]
    LocalFlakeMetadata(NixCommandLineRunJsonError),

    #[error("Local flake contains uncommitted changes")]
    LocalFlakeDirty,

    #[error("Current branch in local flake does not have a remote configured")]
    RemoteBranchNotFound,

    #[error("Local repo out of sync with remote: local: {0}, remote: {1}")]
    RemoteBranchNotSynced(String, String),

    #[error("Failed normalizing git url: {0}")]
    UnknownRemoteUrl(String),

    #[error("Unsupported git remote URL: {0}")]
    UnsupportedGitUrl(url::Url),

    #[error("Failed to parse URL")]
    URLParseFailed(#[from] UrlParseError),
}

#[cfg(test)]
mod tests {

    use std::str::FromStr;

    use once_cell::sync::Lazy;
    use runix::url_parser::PARSER_UTIL_BIN_PATH;

    use super::*;
    use crate::flox::tests::flox_instance;
    #[cfg(feature = "impure-unit-tests")]
    use crate::prelude::Channel;

    static EXAMPLE_CATALOG_ENTRY: Lazy<Value> = Lazy::new(|| {
        json!({
            "eval": {
                "attrPath": ["packages", "aarch64-darwin", "flox"],
                "version": "0.0.0-r42"
                // other entries omitted
            },
            "element": {
                "storePaths": [
                    "/nix/store/2rrfpkq6cr8ppip9szl0z1qfdlskdinq-flox-0.0.0-r42",
                    "/nix/store/k11z2k4iyf3sjfca1vfg935vss9v8y03-flox-0.0.0-r42-man"
                ]
            }
        })
    });

    #[cfg(feature = "impure-unit-tests")] // /nix/store is not accessible in the sandbox
    #[tokio::test]
    /// Check that adds_substituter_metadata correctly returns the validity of
    /// a bad path and a good path, judging against the local /nix/store.
    async fn adds_substituter_metadata() {
        let (flox, _temp_dir_handle) = flox_instance();

        let flake_ref = "git+ssh://git@github.com/flox/dummy"
            .parse::<FlakeRef>()
            .unwrap();

        let publish_flake_ref = PublishFlakeRef::from_flake_ref(flake_ref, &flox, false)
            .await
            .unwrap();

        let bad_path = "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-a";
        let flox_sh_path = env!("FLOX_SH_PATH");
        let publish: Publish<'_, NixAnalysis> = Publish {
            flox: &flox,
            publish_flake_ref,
            attr_path: [""].try_into().unwrap(),
            stability: Stability::Stable,
            analysis: NixAnalysis(json!({
                "element": {
                    "storePaths": [
                        flox_sh_path,
                        bad_path,
                    ],
                },
            })),
        };

        let narinfos = publish
            .get_binary_cache_metadata(None)
            .await
            .unwrap()
            .narinfo;

        // flox is valid
        assert_eq!(narinfos.len(), 2);
        let flox_narinfo = narinfos
            .iter()
            .find(|narinfo| narinfo.path.to_string_lossy() == flox_sh_path)
            .unwrap();
        assert!(flox_narinfo.valid);
        // bad path is not valid
        let bad_path_narinfo = narinfos
            .iter()
            .find(|narinfo| narinfo.path.to_string_lossy() == bad_path)
            .unwrap();
        assert!(!bad_path_narinfo.valid);
    }

    #[cfg(feature = "impure-unit-tests")] // disabled for offline builds, TODO fix tests to work with local repos
    #[tokio::test]
    async fn creates_catalog_entry() {
        let _ = env_logger::try_init();

        let (mut flox, _temp_dir_handle) = flox_instance();

        flox.channels.register_channel(
            "nixpkgs-stable",
            Channel::from("github:flox/nixpkgs/stable".parse::<FlakeRef>().unwrap()),
        );

        let flake_ref = "git+ssh://git@github.com/flox/flox"
            .parse::<FlakeRef>()
            .unwrap();

        let publish_flake_ref = PublishFlakeRef::from_flake_ref(flake_ref, &flox, false)
            .await
            .unwrap();

        let attr_path = ["", "packages", "aarch64-darwin", "flox"]
            .try_into()
            .unwrap();
        let stability = Stability::Stable;
        let publish = Publish::new(&flox, publish_flake_ref, attr_path, stability);

        let value = publish.analyze().await.unwrap().analysis().to_owned();

        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    }

    #[test]
    fn convert_github_ref() {
        // simple github references
        let flake_ref = GitServiceRef::<service::Github>::from_str("github:flox/flox").unwrap();
        let publish_flake_ref = PublishFlakeRef::from_github_ref(flake_ref, false).unwrap();
        assert_eq!(
            publish_flake_ref.to_string(),
            "git+ssh://git@github.com/flox/flox"
        );

        // github references with explicit host param
        let flake_ref = GitServiceRef::<service::Github>::from_str(
            "github:flox/flox?host=github.myenterprise.com",
        )
        .unwrap();
        let publish_flake_ref = PublishFlakeRef::from_github_ref(flake_ref, false).unwrap();
        assert_eq!(
            publish_flake_ref.to_string(),
            "git+ssh://git@github.myenterprise.com/flox/flox"
        );

        // github references with dir param
        let flake_ref =
            GitServiceRef::<service::Github>::from_str("github:flox/flox?dir=somwhere/inside")
                .unwrap();
        let publish_flake_ref = PublishFlakeRef::from_github_ref(flake_ref, false).unwrap();
        assert_eq!(
            publish_flake_ref.to_string(),
            "git+ssh://git@github.com/flox/flox?dir=somwhere%2Finside"
        );

        // github references with git ref
        let flake_ref =
            GitServiceRef::<service::Github>::from_str("github:flox/flox/feat/test").unwrap();
        let publish_flake_ref = PublishFlakeRef::from_github_ref(flake_ref, false).unwrap();
        assert_eq!(
            publish_flake_ref.to_string(),
            "git+ssh://git@github.com/flox/flox?ref=feat%2Ftest"
        );

        // github references with git rev
        let flake_ref = GitServiceRef::<service::Github>::from_str(
            "github:flox/flox/49335c4bade5b3feb7378f9af8e9a528d9c4103e",
        )
        .unwrap();
        let publish_flake_ref = PublishFlakeRef::from_github_ref(flake_ref, false).unwrap();
        assert_eq!(
            publish_flake_ref.to_string(),
            "git+ssh://git@github.com/flox/flox?allRefs=1&rev=49335c4bade5b3feb7378f9af8e9a528d9c4103e"
        );

        // simple github references
        let flake_ref = GitServiceRef::<service::Github>::from_str("github:flox/flox").unwrap();
        let publish_flake_ref = PublishFlakeRef::from_github_ref(flake_ref, true).unwrap();
        assert_eq!(
            publish_flake_ref.to_string(),
            "git+https://github.com/flox/flox"
        );
    }

    /// Red path test: expect error if no upstream set for the current branch
    #[cfg(feature = "impure-unit-tests")] // disabled for offline builds, TODO fix tests to work with local repos
    #[tokio::test]
    async fn git_file_error_if_dirty() {
        let _ = env_logger::try_init();

        let (flox, _temp_dir_handle) = flox_instance();
        let repo_dir = _temp_dir_handle.path().join("repo");

        fs::create_dir(&repo_dir).unwrap();
        let repo = Git::init(&repo_dir, false).await.unwrap();

        // create a file and stage it without committing so that the repo is dirty
        fs::write(repo_dir.join("flake.nix"), "{ outputs = _: {}; }").unwrap();
        repo.add(&[Path::new(".")]).await.unwrap();

        let flake_ref =
            GitRef::from_str(&format!("git+file://{}", repo_dir.to_string_lossy())).unwrap();

        assert!(matches!(
            PublishFlakeRef::from_git_file_flake_ref(
                flake_ref.clone(),
                &flox.nix(Default::default())
            )
            .await,
            Err(ConvertFlakeRefError::LocalFlakeDirty)
        ));
    }

    /// Green path test: resolve a branch and revision of upstream repo
    /// Here, the "upstream" repo is just "the repo itself" (git remote add upstream .)
    /// Note that there are many steps to this.
    /// However in practice this operates on clones of upstream repos, where remote
    /// (and often remote branches) are already set.
    #[cfg(feature = "impure-unit-tests")] // disabled for offline builds, TODO fix tests to work with local repos
    #[tokio::test]
    async fn git_file_resolve_branch_and_rev() {
        let _ = env_logger::try_init();

        let (flox, _temp_dir_handle) = flox_instance();
        let repo_dir = _temp_dir_handle.path().join("repo");

        // create a repo
        fs::create_dir(&repo_dir).unwrap();
        let repo = Git::init(&repo_dir, false).await.unwrap();

        // use a custom name as the default branch name might be affected by the user's git conf
        repo.rename_branch("test/branch").await.unwrap();

        // commit a file
        fs::write(repo_dir.join("flake.nix"), "{ outputs = _: {}; }").unwrap();
        repo.add(&[Path::new(".")]).await.unwrap();
        repo.commit("Commit flake").await.unwrap();

        // add a remote
        repo.add_remote("upstream", &repo_dir.to_string_lossy())
            .await
            .unwrap();
        repo.fetch().await.unwrap();

        // set the origin
        repo.set_origin("test/branch", "upstream").await.unwrap();

        let flake_ref =
            GitRef::from_str(&format!("git+file://{}", repo_dir.to_string_lossy())).unwrap();

        let publish_flake_ref = PublishFlakeRef::from_git_file_flake_ref(
            flake_ref.clone(),
            &flox.nix(Default::default()),
        )
        .await
        .unwrap();
        assert!(matches!(
            publish_flake_ref,
            PublishFlakeRef::File(GitRef {
                url: _,
                attributes: GitAttributes {
                    rev: Some(_),
                    reference: Some(reference),
                    ..
                }}
            )
            if reference == "test/branch"
        ))
    }

    #[cfg(feature = "impure-unit-tests")] // disabled for offline builds, TODO fix tests to work with local repos
    #[tokio::test]
    async fn git_file_error_if_no_upstream() {
        let _ = env_logger::try_init();

        let (flox, _temp_dir_handle) = flox_instance();
        let repo_dir = _temp_dir_handle.path().join("repo");

        fs::create_dir(&repo_dir).unwrap();
        let repo = Git::init(&repo_dir, false).await.unwrap();

        fs::write(repo_dir.join("flake.nix"), "{ outputs = _: {}; }").unwrap();
        repo.add(&[Path::new(".")]).await.unwrap();
        repo.commit("Commit flake").await.unwrap();

        let flake_ref =
            GitRef::from_str(&format!("git+file://{}", repo_dir.to_string_lossy())).unwrap();

        let result = PublishFlakeRef::from_git_file_flake_ref(
            flake_ref.clone(),
            &flox.nix(Default::default()),
        )
        .await;

        assert!(matches!(result, Err(ConvertFlakeRefError::NoRemote(_))));
    }

    /// Check if we successfully clone a repo
    #[tokio::test]
    async fn upstream_repo_from_url() {
        let _ = env_logger::try_init();

        let (_flox, temp_dir_handle) = flox_instance();
        let repo_dir = temp_dir_handle.path().join("repo");

        // create a repo
        fs::create_dir(&repo_dir).unwrap();
        let repo = Git::init(&repo_dir, false).await.unwrap();

        UpstreamRepo::clone_repo(
            repo.workdir().unwrap().to_string_lossy(),
            temp_dir_handle.path(),
        )
        .await
        .expect("Should clone repo");
    }

    // disabled because nix build does not have git user/email config,
    // TODO fix tests to work with local repos
    #[cfg(feature = "impure-unit-tests")]
    /// Check if we successfully clone a repo
    #[tokio::test]
    async fn create_catalog_branch() {
        let _ = env_logger::try_init();
        let (_flox, temp_dir_handle) = flox_instance();
        let repo_dir = temp_dir_handle.path().join("repo");

        // create a repo
        fs::create_dir(&repo_dir).unwrap();
        let repo = Git::init(&repo_dir, false).await.unwrap();
        let mut repo = UpstreamRepo::clone_repo(
            repo.workdir().unwrap().to_string_lossy(),
            temp_dir_handle.path(),
        )
        .await
        .expect("Should clone repo");

        assert!(repo.0.list_branches().await.unwrap().is_empty());

        let catalog = repo
            .get_or_create_catalog(&"aarch64-darwin".to_string())
            .await
            .expect("Should create branch");

        // commit a file to the branch to crate the first reference on the orphan branch
        fs::write(catalog.0.workdir().unwrap().join(".tag"), "").unwrap();
        catalog.0.add(&[Path::new(".tag")]).await.unwrap();
        catalog.0.commit("root commit").await.unwrap();

        assert_eq!(catalog.0.list_branches().await.unwrap().len(), 1);
    }

    #[test]
    fn test_get_snapshot_path() {
        let snapshot = EXAMPLE_CATALOG_ENTRY.deref();

        let expected = PathBuf::from("catalog/flox/0.0.0-r42-828d710f.json");
        let actual = UpstreamCatalog::get_snapshot_path(snapshot);

        assert_eq!(actual, expected);
    }

    // disabled because nix build does not have git user/email config,
    // TODO fix tests to work with local repos
    #[cfg(feature = "impure-unit-tests")]
    /// Check if we successfully clone a repo
    #[tokio::test]
    async fn test_add_snapshot() {
        let _ = env_logger::try_init();
        let (_flox, temp_dir_handle) = flox_instance();
        let repo_dir = temp_dir_handle.path().join("repo");

        let snapshot = EXAMPLE_CATALOG_ENTRY.deref();

        // create a repo
        fs::create_dir(&repo_dir).unwrap();
        let repo = Git::init(&repo_dir, false).await.unwrap();
        let mut repo = UpstreamRepo::clone_repo(
            repo.workdir().unwrap().to_string_lossy(),
            temp_dir_handle.path(),
        )
        .await
        .expect("Should clone repo");

        let catalog = repo
            .get_or_create_catalog(&"aarch64-darwin".to_string())
            .await
            .expect("Should create branch");

        catalog
            .add_snapshot(snapshot)
            .await
            .expect("Should add snapshot");

        assert_eq!(
            &catalog
                .get_snapshot(snapshot)
                .unwrap()
                .expect("should find written snapshot"),
            snapshot
        );
    }

    // disabled because nix build does not have git user/email config,
    // TODO fix tests to work with local repos
    #[cfg(feature = "impure-unit-tests")]
    /// Check if we successfully clone a repo
    #[tokio::test]
    async fn test_push_snapshot() {
        let _ = env_logger::try_init();
        let (_flox, temp_dir_handle) = flox_instance();
        let repo_dir = temp_dir_handle.path().join("repo");

        let snapshot = EXAMPLE_CATALOG_ENTRY.deref();

        // create an "upstream" repo
        fs::create_dir(&repo_dir).unwrap();
        let upstream_repo = Git::init(&repo_dir, false).await.unwrap();

        // clone a "downstream"
        let mut repo = UpstreamRepo::clone_repo(
            upstream_repo.workdir().unwrap().to_string_lossy(),
            temp_dir_handle.path(),
        )
        .await
        .expect("Should clone repo");

        // get a catalog, write a snapshot and push to upstream
        let catalog = repo
            .get_or_create_catalog(&"aarch64-darwin".to_string())
            .await
            .expect("Should create branch");

        catalog
            .add_snapshot(snapshot)
            .await
            .expect("Should add snapshot");
        catalog.push_catalog().await.expect("Should push catalog");

        // checkout the new branch upstream to check if the file got written
        upstream_repo
            .checkout(
                &UpstreamRepo::catalog_branch_name(&"aarch64-darwin".to_string()),
                false,
            )
            .await
            .expect("catalog branch should exist upstream");

        let snapshot_path = repo_dir.join(UpstreamCatalog::get_snapshot_path(snapshot));
        assert!(snapshot_path.exists());

        let snapshot_actual: Value =
            serde_json::from_str(&fs::read_to_string(&snapshot_path).unwrap()).unwrap();

        assert_eq!(snapshot_actual, snapshot_actual);
    }

    #[tokio::test]
    async fn resolves_indirect_ref_to_git_https() {
        let indirect_ref = FlakeRef::from_url("flake:flox", PARSER_UTIL_BIN_PATH).unwrap();
        let (flox, _temp_dir_handle) = flox_instance();
        // Need to expose the custom registry to `parser-util` via environment variable
        flox.nix::<NixCommandLine>(vec![]).export_env_vars();
        let publishable = PublishFlakeRef::from_flake_ref(indirect_ref, &flox, true)
            .await
            .unwrap();
        let expected = PublishFlakeRef::Https(
            GitRef::from_str("git+https://github.com/flox/floxpkgs?ref=master").unwrap(),
        );
        assert_eq!(publishable, expected);
    }
}
