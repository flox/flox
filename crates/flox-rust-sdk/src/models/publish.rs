use std::ops::Deref;
use std::path::PathBuf;

use derive_more::{Deref, DerefMut, Display};
use flox_types::catalog::cache::{CacheMeta, SubstituterUrl};
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
use crate::providers::git::{GitCommandProvider as Git, GitProvider};

/// Publish state before analyzing
///
/// Prevents other actions to commence without analyzing the package first
pub struct Empty;

/// Publish state after collecting nix metadata
///
/// Json value (ideally a [flox_types::catalog::CatalogEntry],
/// but that's currently broken on account of some flakerefs)
#[derive(Debug, Deref, DerefMut)]
pub struct NixAnalysis(Value);

/// State for the publish algorihm
///
/// Transitions through typestates to ensure we don't invoke invalid operations
pub struct Publish<'flox, State> {
    flox: &'flox Flox,
    /// The published _upstream_ source
    publish_ref: PublishRef,
    /// The published attrpath
    /// Should be fully resolved to avoid ambiguity
    attr_path: AttrPath,
    stability: Stability,
    analysis: State,
}

impl<'flox> Publish<'flox, Empty> {
    pub fn new(
        flox: &'flox Flox,
        publish_ref: PublishRef,
        attr_path: AttrPath,
        stability: Stability,
    ) -> Publish<'flox, Empty> {
        Self {
            flox,
            publish_ref,
            attr_path,
            stability,
            analysis: Empty,
        }
    }

    /// run analysis on the package and switch to next state
    ///
    /// It uses an analyzer flake to extract eval metadata of the derivation.
    /// The analyzer applies a function to all packages in a `target` flake
    /// and provides the result under `#analysis.eval.<full attrpath of the package>`.
    ///
    /// We evalaute this analysis as json, to which we add
    /// * source urls for reproducibility
    /// * the nixpkgs stability being used to create the package
    pub async fn analyze(self) -> PublishResult<Publish<'flox, NixAnalysis>> {
        let mut drv_metadata_json = self.get_drv_metadata().await?;
        let flake_metadata = self.get_flake_metadata().await?;

        // DEVIATION FROM BASH: using `locked` here instead of `resolved`
        //                      this is used to reproduce the package,
        //                      but is essentially redundant because of the `source.locked`
        drv_metadata_json["element"]["url"] = json!(flake_metadata.locked.to_string());
        drv_metadata_json["source"] = json!({
            "locked": flake_metadata.locked,
            "original": flake_metadata.original,
            "remote": flake_metadata.original,
        });
        drv_metadata_json["eval"]["stability"] = json!(self.stability);

        Ok(Publish {
            flox: self.flox,
            publish_ref: self.publish_ref,
            attr_path: self.attr_path,
            stability: self.stability,
            analysis: NixAnalysis(drv_metadata_json),
        })
    }

    /// extract metadata of the published derivation using the analyzer flake
    async fn get_drv_metadata(&self) -> PublishResult<Value> {
        let nix: NixCommandLine = self.flox.nix(Default::default());

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

        let nixpkgs_flakeref =
            FlakeRef::Indirect(IndirectRef::new("nixpkgs-flox".into(), Default::default()));

        let analyzer_flakeref = FlakeRef::Path(PathRef::new(
            PathBuf::from(env!("FLOX_ANALYZER_SRC")),
            Default::default(),
        ));

        let eval_analysis_command = Eval {
            flake: FlakeArgs {
                override_inputs: [
                    ("target".to_string(), self.publish_ref.clone().into_inner()).into(),
                    (
                        "target/flox-floxpkgs/nixpkgs/nixpkgs".to_string(),
                        nixpkgs_flakeref, // stability overide has already been applied, not duplicating that code here
                    )
                        .into(),
                ]
                .to_vec(),
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
                    self.publish_ref.clone(),
                    nix_error,
                )
            })
            .await
    }

    /// Resolve the metadata of the flake holding the published package
    async fn get_flake_metadata(&self) -> PublishResult<FlakeMetadata> {
        let nix: NixCommandLine = self.flox.nix(Default::default());

        let locked_ref_command = runix::command::FlakeMetadata {
            flake_ref: Some(self.publish_ref.clone().into_inner().into()),
            ..Default::default()
        };

        locked_ref_command
            .run_typed(&nix, &Default::default())
            .map_err(|nix_err| PublishError::FlakeMetadata(self.publish_ref.clone(), nix_err))
            .await
    }
}

impl<'flox> Publish<'flox, NixAnalysis> {
    /// copy the outputs and dependencies of the package to binary store
    pub async fn upload_binary(self) -> PublishResult<Publish<'flox, NixAnalysis>> {
        todo!()
    }

    #[allow(unused)] // until implemented
    async fn get_binary_cache_metadata(
        &self,
        substituter: SubstituterUrl,
    ) -> PublishResult<CacheMeta> {
        todo!()
    }

    /// write snapshot to catalog and push to origin
    pub async fn push_catalog(self) -> PublishResult<()> {
        let url = self.publish_ref.clone_url();
        let repo_dir = tempfile::tempdir_in(&self.flox.temp_dir)
            .unwrap()
            .into_path(); // todo catch error
        let catalog = <Git as GitProvider>::clone(url, &repo_dir, false)
            .await
            .unwrap(); // todo: catch error

        if catalog.list_branches().await.unwrap() // todo: catch error
            .into_iter().any(|info| info.name == "catalog")
        {
            catalog.checkout("catalog", false).await.unwrap(); // todo: catch error
        } else {
            todo!();
            // catalog.checkout("catalog", true).await.unwrap(); // todo: catch error

            // catalog.set_upstream("origin", "catalog").await.unwrap();  // todo: implement
            //                                                               todo: catch error
        }
        todo!()
    }

    /// read out the current publish state
    pub fn analysis(&self) -> &Value {
        self.analysis.deref()
    }
}

#[derive(Error, Debug)]
pub enum PublishError {
    #[error("Failed to load metadata for the package '{0}' in '{1}': {2}")]
    DrvMetadata(AttrPath, PublishRef, NixCommandLineRunJsonError),

    #[error("Failed to load metadata for flake '{0}': {1}")]
    FlakeMetadata(PublishRef, NixCommandLineRunJsonError),
}

type PublishResult<T> = Result<T, PublishError>;

/// Publishable FlakeRefs
///
/// `publish` modifies branches of the source repository.
/// Thus we can only publish to repositories in (remote*) git repositories.
/// This enum represents the subset of flakerefs we can use,
/// so we can avoid parsing and converting flakerefs within publish.
/// [GitRef<protocol::File>] should in most cases be resolved to a remote type.
#[derive(PartialEq, Eq, Clone, Debug, Display)]
pub enum PublishRef {
    Ssh(GitRef<protocol::SSH>),
    Https(GitRef<protocol::HTTPS>),
    // File(GitRef<protocol::File>),
}

impl PublishRef {
    /// extract an url for cloning with git
    fn clone_url(&self) -> String {
        match self {
            PublishRef::Ssh(ref ssh_ref) => ssh_ref.url.as_str().to_owned(),
            PublishRef::Https(ref https_ref) => https_ref.url.as_str().to_owned(),
        }
    }

    /// return the a flakeref type for the wrapped refs
    fn into_inner(self) -> FlakeRef {
        match self {
            PublishRef::Ssh(ssh_ref) => FlakeRef::GitSsh(ssh_ref),
            PublishRef::Https(https_ref) => FlakeRef::GitHttps(https_ref),
        }
    }
}

impl TryFrom<FlakeRef> for PublishRef {
    type Error = ConvertFlakeRefError;

    fn try_from(value: FlakeRef) -> Result<Self, Self::Error> {
        let publish_ref = match value {
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
        Ok(publish_ref)
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

    #[tokio::test]
    async fn creates_catalog_entry() {
        env_logger::init();

        let (mut flox, _temp_dir_handle) = flox_instance();

        flox.channels.register_channel(
            "nixpkgs-flox",
            Channel::from("github:flox/nixpkgs/stable".parse::<FlakeRef>().unwrap()),
        );

        let publish_ref: PublishRef = "git+ssh://git@github.com/flox/flox"
            .parse::<FlakeRef>()
            .unwrap()
            .try_into()
            .unwrap();

        let attr_path = ["", "packages", "aarch64-darwin", "flox"]
            .try_into()
            .unwrap();
        let stability = Stability::Stable;
        let publish = Publish::new(&flox, publish_ref, attr_path, stability);

        let value = publish.analyze().await.unwrap().analysis().to_owned();

        println!("{}", serde_json::to_string_pretty(&value).unwrap());
    }
}
