use derive_more::Constructor;
use runix::command::FlakeInit;
use runix::{NixBackend, Run};
use thiserror::Error;

use super::{Closed, Root, RootGuard};
use crate::flox::FloxNixApi;
use crate::providers::git::GitProvider;

#[derive(Constructor, Debug)]
pub struct Floxmeta<T> {
    pub git: T,
}

impl<'flox, Git: GitProvider> Root<'flox, Closed<Git>> {
    /// Guards opening a floxmeta
    pub async fn guard_floxmeta(
        self,
    ) -> Result<RootGuard<'flox, Floxmeta<Git>, Closed<Git>>, OpenFloxmetaError> {
        Ok(RootGuard::Initialized(Root {
            state: Floxmeta {
                git: self.state.inner,
            },
            flox: self.flox,
        }))
    }
}

/// Implementation to upgrade into an open Project
impl<'flox, Git: GitProvider> RootGuard<'flox, Floxmeta<Git>, Closed<Git>> {
    /// Initialize a new project in the workdir of a git root or return
    /// an existing project if it exists.
    pub async fn init_floxmeta<Nix: FloxNixApi>(
        self,
    ) -> Result<Root<'flox, Floxmeta<Git>>, InitFloxmetaError<Nix, Git>>
    where
        FlakeInit: Run<Nix>,
    {
        Ok(self.open().unwrap_or_else(|_| todo!()))
    }
}

/// Errors occuring while trying to upgrade to an [`Open<Git>`] [Root]
#[derive(Error, Debug)]
pub enum OpenFloxmetaError {
    #[error("Could not determine repository root")]
    WorkdirNotFound,
}

#[derive(Error, Debug)]
pub enum InitFloxmetaError<Nix: NixBackend, Git: GitProvider>
where
    FlakeInit: Run<Nix>,
{
    #[error("Could not determine repository root")]
    WorkdirNotFound,

    #[error("Error initializing base template with Nix")]
    NixInitBase(<FlakeInit as Run<Nix>>::Error),
    #[error("Error reading template file contents")]
    ReadTemplateFile(std::io::Error),
    #[error("Error truncating template file")]
    TruncateTemplateFile(std::io::Error),
    #[error("Error writing to template file")]
    WriteTemplateFile(std::io::Error),
    #[error("Error new template file in Git")]
    GitAdd(Git::AddError),
}

#[derive(Error, Debug)]
pub enum FloxmetaListError<Git: GitProvider> {
    // todo: add environment name/path?
    #[error("Failed retrieving 'manifest.json'")]
    RetrieveManifest(Git::ShowError),

    #[error("Failed parsing 'manifest.json'")]
    ParseManifest(serde_json::Error),
}
