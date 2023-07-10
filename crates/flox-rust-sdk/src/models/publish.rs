use std::ops::Deref;
use std::path::PathBuf;

use derive_more::{Deref, DerefMut};
use flox_types::stability::Stability;
use runix::arguments::flake::FlakeArgs;
use runix::command::Eval;
use runix::command_line::NixCommandLine;
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
struct Empty;

/// Publish state after collecting nix metadata
///
/// Json value (ideally a [flox_types::catalog::CatalogEntry],
/// but that's currently broken on account of some flakerefs)
#[derive(Debug, Deref, DerefMut)]
struct NixAnalysis(Value);

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
    pub async fn new(
        flox: &'flox Flox,
        publish_ref: PublishRef,
        attr_path: AttrPath,
        stability: Stability,
    ) -> PublishResult<Publish<'flox, Empty>> {
        Ok(Self {
            flox,
            publish_ref,
            attr_path,
            stability,
            analysis: Empty,
        })
    }

    /// run analysis on the package and switch to next state
    pub async fn analyze(self) -> PublishResult<Publish<'flox, NixAnalysis>> {
        let nix: NixCommandLine = self.flox.nix(Default::default());

        let analysis_attr_path = {
            let mut attrpath = AttrPath::try_from(["", "analysis", "eval"]).unwrap();
            attrpath.extend(self.attr_path.clone());
            attrpath
        };

        let nixpkgs_flakeref =
            FlakeRef::Indirect(IndirectRef::new("nixpkgs-flox".into(), Default::default()));

        let analyzer_flakeref =
            FlakeRef::Path(PathRef::new(PathBuf::from("asd"), Default::default()));

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

        let mut analytics_json = eval_analysis_command
            .run_json(&nix, &Default::default())
            .await
            .unwrap();

        let locked_ref = {
            let locked_ref_command: runix::command::FlakeMetadata = runix::command::FlakeMetadata {
                flake_ref: Some(self.publish_ref.clone().into_inner().into()),
                ..Default::default()
            };

            locked_ref_command
                .run_typed(&nix, &Default::default())
                .await
                .unwrap()
        };

        // DEVIATION FROM BASH: using `locked` here instead of `resolved`
        //                      this is used to reproduce the package,
        //                        but is essentially redundant because of the `source.locked`
        analytics_json["element"]["url"] = json!(locked_ref.locked.to_string());
        analytics_json["source"] = json!({
            "locked": locked_ref.locked,
            "original": locked_ref.original,
            "remote": locked_ref.original,
        });
        analytics_json["eval"]["stability"] = json!(self.stability);

        Ok(Publish {
            flox: self.flox,
            publish_ref: self.publish_ref,
            attr_path: self.attr_path,
            stability: self.stability,
            analysis: NixAnalysis(analytics_json),
        })
    }
}

impl<'flox> Publish<'flox, NixAnalysis> {
    /// copy the outputs and dependencies of the package to binary store
    pub async fn upload_binary(&self) -> PublishResult<()> {
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
pub enum PublishError {}

type PublishResult<T> = Result<T, PublishError>;

/// Publishable FlakeRefs
///
/// `publish` modifies branches of the source repository.
/// Thus we can only publish to repositories in (remote*) git repositories.
/// This enum represents the subset of flakerefs we can use,
/// so we can avoid parsing and converting flakerefs within publish.
/// [GitRef<protocol::File>] should in most cases be resolved to a remote type.
#[derive(PartialEq, Eq, Clone)]
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
