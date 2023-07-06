use flox_types::catalog::CatalogEntry;
use runix::flake_ref::git::GitRef;
use runix::flake_ref::{protocol, FlakeRef};
use runix::installable::AttrPath;
use thiserror::Error;

use crate::flox::Flox;
use crate::providers::git::{GitCommandProvider as Git, GitProvider};

/// State for the publish algorihm
#[allow(dead_code)] // until we implement methods for Publish
pub struct Publish<'flox> {
    flox: &'flox Flox,
    /// The published _upstream_ source
    publish_ref: PublishRef,
    /// The published attrpath
    /// Should be fully resolved to avoid ambiguity
    attr_path: AttrPath,
    analysis: Option<CatalogEntry>, // model as type state?
}

impl<'flox> Publish<'flox> {
    pub async fn new(
        flox: &'flox Flox,
        publish_ref: PublishRef,
        attr_path: AttrPath,
    ) -> PublishResult<Publish<'flox>> {
        Ok(Self {
            flox,
            publish_ref,
            attr_path,
            analysis: None,
        })
    }

    /// run analysis on the package and add to state
    pub async fn analyze(self) -> PublishResult<Publish<'flox>> {
        todo!()
    }

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
    pub fn analysis(&self) -> Option<&CatalogEntry> {
        self.analysis.as_ref()
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
