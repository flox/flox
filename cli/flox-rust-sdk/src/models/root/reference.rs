//! This module provides implementations on root references
//! Use the methods here to "upgrade" [Root] to sages with more context.
use std::path::PathBuf;

use thiserror::Error;

use super::{Closed, Root, RootGuard};
use crate::providers::git::{
    GitCommandDiscoverError,
    GitCommandProvider,
    GitDiscoverError,
    GitProvider,
};
use crate::utils::guard::Guard;

/// Methods on a reference to a [Root] object
///
/// At this stage the root has not yet been verified.
/// This state should be handled as a mere reference to a potential root of any kind
impl<'flox> Root<'flox, Closed<PathBuf>> {
    pub fn guard(
        self,
    ) -> Result<
        RootGuard<'flox, Closed<GitCommandProvider>, Closed<PathBuf>>,
        ProjectDiscoverGitError,
    > {
        match GitCommandProvider::discover(&self.state.inner) {
            Ok(repo) => Ok(Guard::Initialized(Root {
                flox: self.flox,
                state: Closed::new(repo),
            })),
            Err(err) if err.not_found() => Ok(Guard::Uninitialized(Root {
                flox: self.flox,
                state: Closed::new(self.state.inner),
            })),
            Err(err) => Err(ProjectDiscoverGitError::DiscoverRepoError(err)),
        }
    }
}

#[derive(Error, Debug)]
pub enum ProjectDiscoverGitError {
    #[error("Error attempting to discover repository: {0}")]
    DiscoverRepoError(GitCommandDiscoverError),
}
