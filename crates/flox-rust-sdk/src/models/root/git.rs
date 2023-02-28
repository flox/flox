use std::path::{Path, PathBuf};

use thiserror::Error;

use super::{Closed, Open, Root, RootGuard};
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
    pub async fn init_git(self) -> Result<Root<'flox, Closed<Git>>, ProjectInitGitError<Git>> {
        match self {
            Guard::Initialized(i) => Ok(i),
            Guard::Uninitialized(u) => {
                let repo = Git::init(&u.state.inner)
                    .await
                    .map_err(ProjectInitGitError::InitRepoError)?;

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

    /// Guards opening a project
    ///
    /// - Resolves as initialized if a `flake.nix` is present
    /// - Resolves as uninitialized if not
    pub async fn guard(self) -> Result<RootGuard<'flox, Open<Git>, Closed<Git>>, OpenProjectError> {
        let repo = &self.state.inner;

        let root = repo.workdir().ok_or(OpenProjectError::WorkdirNotFound)?;

        if root.join("flake.nix").exists() {
            Ok(Guard::Initialized(Root {
                flox: self.flox,
                state: Open::new(self.state.inner),
            }))
        } else {
            Ok(Guard::Uninitialized(self))
        }
    }
}

/// Errors possible during initialization of the git repo
#[derive(Error, Debug)]
pub enum ProjectInitGitError<Git: GitProvider> {
    #[error("Error initializing repository: {0}")]
    InitRepoError(Git::InitError),
}

/// Errors occuring while trying to upgrade to an [`Open<Git>`] [Root]
#[derive(Error, Debug)]
pub enum OpenProjectError {
    #[error("Could not determine repository root")]
    WorkdirNotFound,
}
