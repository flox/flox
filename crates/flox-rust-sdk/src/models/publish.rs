use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::{fs, io};

use derive_more::{Deref, DerefMut, Display};
use flox_types::catalog::cache::{CacheMeta, SubstituterUrl};
use flox_types::catalog::System;
use flox_types::stability::Stability;
use futures::TryFutureExt;
use runix::arguments::flake::FlakeArgs;
use runix::command::Eval;
use runix::command_line::{NixCommandLine, NixCommandLineRunJsonError};
use runix::flake_metadata::FlakeMetadata;
use runix::flake_ref::git::GitRef;
use runix::flake_ref::indirect::IndirectRef;
use runix::flake_ref::path::PathRef;
use runix::flake_ref::{protocol, FlakeRef};
use runix::installable::{AttrPath, Installable};
use runix::{RunJson, RunTyped};
use serde_json::{json, Value};
use thiserror::Error;

use crate::flox::Flox;
use crate::providers::git::{GitCommandError, GitCommandProvider as Git, GitProvider};

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

        let nixpkgs_flakeref = FlakeRef::Indirect(IndirectRef::new(
            format!("nixpkgs-{}", self.stability),
            Default::default(),
        ));

        // We bundle the analyzer flake with flox (see the package definition for flox)
        let analyzer_flakeref = FlakeRef::Path(PathRef::new(
            PathBuf::from(env!("FLOX_ANALYZER_SRC")),
            Default::default(),
        ));

        let eval_analysis_command = Eval {
            flake: FlakeArgs {
                override_inputs: [
                    // The analyzer flake provides analysis outputs for the flake input `target`
                    // Here, we're setting the target flake to our source flake.
                    (
                        "target".to_string(),
                        self.publish_flake_ref.clone().into_inner(),
                    )
                        .into(),
                    // Stabilities are managed by overriding the `flox-floxpkgs/nixpkgs/nixpkgs` input to
                    // `nixpkgs-<stability>`.
                    // The analyzer flake adds an additional indirection,
                    // so we have to do the override manually.
                    // This is the `nixpkgs-<stability>` portion.
                    (
                        "target/flox-floxpkgs/nixpkgs/nixpkgs".to_string(),
                        nixpkgs_flakeref,
                    )
                        .into(),
                ]
                .to_vec(),
                // The analyzer flake is bundled with flox as a nix store path and thus read-only.
                no_write_lock_file: true.into(),
            },
            eval_args: runix::arguments::EvalArgs {
                installable: Some(
                    Installable {
                        flakeref: analyzer_flakeref,
                        attr_path: analysis_attr_path,
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
                    self.publish_flake_ref.clone(),
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
            .map_err(|nix_err| PublishError::FlakeMetadata(self.publish_flake_ref.clone(), nix_err))
            .await
    }
}

impl<'flox> Publish<'flox, NixAnalysis> {
    /// Copy the outputs and dependencies of the package to binary store
    pub async fn upload_binary(self) -> Result<Publish<'flox, NixAnalysis>, PublishError> {
        todo!()
    }

    /// Check whether a store path is substitutable by a given substituter
    /// and return the associated metadata.
    #[allow(unused)] // until implemented
    async fn get_binary_cache_metadata(
        &self,
        substituter: SubstituterUrl,
    ) -> Result<CacheMeta, PublishError> {
        todo!()
    }

    /// Write snapshot to catalog and push to origin
    pub async fn push_snapshot(&self) -> Result<(), PublishError> {
        let mut upstream_repo = UpstreamRepo::clone(&self.publish_ref, &self.flox.temp_dir).await?;
        if let Ok(Some(_)) = catalog.get_snapshot(self.analysis()) {
        let catalog = upstream_repo.get_catalog(&self.flox.system).await?;
            Err(PublishError::SnapshotExists)?;
        }
        catalog.add_snapshot(self.analysis()).await?;
        catalog.push_catalog().await?;

        Ok(())
    }

    /// Read out the current publish state
    pub fn analysis(&self) -> &Value {
        self.analysis.deref()
    }
}

/// Representation of an exclusive clone of an upstream repo
///
/// [UpstreamRepo] and [UpstreamCatalog] ensure safe access to individual catalog branches.
/// Every [UpstreamRepo] instance represents an exclusive clone
/// and can only ever create a single [UpstreamCatalog] instance at a time.
struct UpstreamRepo(Git);

impl UpstreamRepo {
    /// Clone the upstream repo
    async fn clone(
        publish_ref: &PublishRef,
        temp_dir: impl AsRef<Path>,
    ) -> Result<Self, PublishError> {
        let url = publish_ref.clone_url();
        let repo_dir = tempfile::tempdir_in(temp_dir).unwrap().into_path(); // todo catch error
        let repo = <Git as GitProvider>::clone(&url, &repo_dir, false).await?;

        Ok(Self(repo))
    }

    fn catalog_branch_name(system: &System) -> String {
        format!("catalog/{system}")
    }

    /// Create an [UpstreamCatalog] by checking out or creating a catalog branch.
    ///
    /// `Git` objects can switch branches at any time leaving the repo in an unknown state.
    /// [get_catalog] ensures that only one [UpstreamCatalog] exists at a time by requiring a `&mut self`.
    async fn get_catalog(&mut self, system: &System) -> Result<UpstreamCatalog, PublishError> {
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
            self.0
                .set_origin(&Self::catalog_branch_name(system), "origin")
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
    /// TODO: fix before releasing this PR!
    fn get_snapshot_path(&self, snapshot: &Value) -> PathBuf {
        let mut path = self
            .0
            .workdir()
            .unwrap()
            .join("packages")
            .join(snapshot["eval"]["meta"]["pname"].to_string());
        snapshot["eval"]["meta"]["version"].to_string();

        path.set_file_name(format!("{}.json", snapshot["eval"]["meta"]["version"]));
        path
    }

    /// Try retrieving a snapshot from the catalog
    /// TODO: better addressing (attrpath + version + drv hash?)
    fn get_snapshot(&self, snapshot: &Value) -> Result<Option<Value>, PublishError> {
        let path = self.get_snapshot_path(snapshot);
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
        let mut snapshot_file = fs::OpenOptions::new()
            .create_new(true)
            .open(self.get_snapshot_path(snapshot))?;
        serde_json::to_writer(&mut snapshot_file, snapshot)?;

        self.0.add(&[&self.get_snapshot_path(snapshot)]).await?;
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
    DrvMetadata(AttrPath, PublishFlakeRef, NixCommandLineRunJsonError),

    #[error("Failed to load metadata for flake '{0}': {1}")]
    FlakeMetadata(String, NixCommandLineRunJsonError),

    #[error("Failed reading snapshot data: {0}")]
    ReadSnapshot(#[from] serde_json::Error),

    #[error("Failed to run git operation: {0}")]
    GitOperation(#[from] GitCommandError),

    #[error("Failed to run IO operation: {0}")]
    IoOperation(#[from] std::io::Error),

    #[error("Already published")]
    SnapshotExists,
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
    // File(GitRef<protocol::File>),
}

impl PublishFlakeRef {
    /// Extract a URL for cloning with git
    fn clone_url(&self) -> String {
        match self {
            PublishFlakeRef::Ssh(ref ssh_ref) => ssh_ref.url.as_str().to_owned(),
            PublishFlakeRef::Https(ref https_ref) => https_ref.url.as_str().to_owned(),
        }
    }

    /// Return the [FlakeRef] type for the wrapped refs
    fn into_inner(self) -> FlakeRef {
        match self {
            PublishFlakeRef::Ssh(ssh_ref) => FlakeRef::GitSsh(ssh_ref),
            PublishFlakeRef::Https(https_ref) => FlakeRef::GitHttps(https_ref),
        }
    }
}

impl TryFrom<FlakeRef> for PublishFlakeRef {
    type Error = ConvertFlakeRefError;

    fn try_from(value: FlakeRef) -> Result<Self, Self::Error> {
        let publish_flake_ref = match value {
            FlakeRef::GitSsh(ssh_ref) => Self::Ssh(ssh_ref),
            FlakeRef::GitHttps(https_ref) => Self::Https(https_ref),
            // resolve upstream for local git repo
            FlakeRef::GitPath(_) => todo!(),
            // resolve indirect ref to direct ref (recursively)
            FlakeRef::Indirect(_) => todo!(),
            FlakeRef::Github(_) => todo!(),
            FlakeRef::Gitlab(_) => todo!(),
            _ => Err(ConvertFlakeRefError::UnsupportedTarget(value))?,
        };
        Ok(publish_flake_ref)
    }
}

/// Errors arising from convert
#[derive(Error, Debug)]
pub enum ConvertFlakeRefError {
    #[error("Unsupported flakeref for publish: {0}")]
    UnsupportedTarget(FlakeRef),
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::flox::tests::flox_instance;
    use crate::prelude::Channel;

    #[cfg(feature = "impure-unit-tests")] // disabled for offline builds, TODO fix tests to work with local repos
    #[tokio::test]
    async fn creates_catalog_entry() {
        env_logger::init();

        let (mut flox, _temp_dir_handle) = flox_instance();

        flox.channels.register_channel(
            "nixpkgs-stable",
            Channel::from("github:flox/nixpkgs/stable".parse::<FlakeRef>().unwrap()),
        );

        let publish_flake_ref: PublishFlakeRef = "git+ssh://git@github.com/flox/flox"
            .parse::<FlakeRef>()
            .unwrap()
            .try_into()
            .unwrap();

        let attr_path = ["", "packages", "aarch64-darwin", "flox"]
            .try_into()
            .unwrap();
        let stability = Stability::Stable;
        let publish = Publish::new(&flox, publish_flake_ref, attr_path, stability);

        let value = publish.analyze().await.unwrap().analysis().to_owned();

        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    }
}
