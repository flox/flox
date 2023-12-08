use std::path::{Path, PathBuf};

use thiserror::Error;

use super::{Closed, Root, RootGuard};
use crate::providers::git::GitProvider;
use crate::utils::guard::Guard;

/// methods to initialize a [`Closed<Git>`] [`Root`] from a [`PathBuf`].
///
/// Usually retrieved from [`Root<Closed<PathBuf>>::open()`]
impl<'flox, Git: GitProvider> RootGuard<'flox, Closed<Git>, Closed<PathBuf>> {
    /// get the path of either the git repo or non git directory
    pub fn path(&self) -> &Path {
        match self {
            Guard::Initialized(i) => i.path(),
            Guard::Uninitialized(u) => &u.state.inner,
        }
    }

    /// Retrieve the initialized repo or try to create one
    // todo add `bare` option
    pub fn init_git(self) -> Result<Root<'flox, Closed<Git>>, ProjectInitGitError<Git>> {
        match self {
            Guard::Initialized(i) => Ok(i),
            Guard::Uninitialized(u) => {
                let repo =
                    Git::init(&u.state.inner, false).map_err(ProjectInitGitError::InitRepoError)?;

                Ok(Root {
                    flox: u.flox,
                    state: Closed::new(repo),
                })
            },
        }
    }
}

impl<'flox, Git: GitProvider> Root<'flox, Closed<Git>> {
    /// Get the git root directory
    pub fn workdir(&self) -> Option<&Path> {
        self.state.inner.workdir()
    }

    /// get the path originally used to discover the repo
    pub fn path(&self) -> &Path {
        self.state.inner.path()
    }
}

/// Errors possible during initialization of the git repo
#[derive(Error, Debug)]
pub enum ProjectInitGitError<Git: GitProvider> {
    #[error("Error initializing repository: {0}")]
    InitRepoError(Git::InitError),
}
