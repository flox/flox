use std::borrow::Cow;

use runix::installable::Installable;

use super::{Index, Project, TransactionCommitError, TransactionEnterError};
use crate::models::root::transaction::{GitAccess, GitSandBox, ReadOnly};
use crate::providers::git::GitProvider;

pub struct Environment<'flox, Git: GitProvider, Access: GitAccess<Git>> {
    /// aka. Nix attrpath, undr the assumption that they are not nested!
    pub(super) name: String,
    pub(super) system: String,
    pub(super) project: Project<'flox, Git, Access>,
}

/// Implementations for an environment
impl<Git: GitProvider, A: GitAccess<Git>> Environment<'_, Git, A> {
    pub fn name(&self) -> Cow<str> {
        Cow::from(&self.name)
    }

    pub fn system(&self) -> Cow<str> {
        Cow::from(&self.system)
    }

    // pub async fn metadata(&self) -> Result<Metadata, MetadataError<Git>> {
    //    todo!("to be replaced by catalog")
    // }

    /// get an installable for this environment
    // todo: share with named env
    pub fn installable(&self) -> Installable {
        Installable {
            flakeref: self.project.flakeref(),
            attr_path: format!(".floxEnvs.{}.{}", self.system, self.name),
        }
    }
}

/// Implementations for R/O only instances
///
/// Mainly transformation into modifiable sandboxed instances
impl<'flox, Git: GitProvider> Environment<'flox, Git, ReadOnly<Git>> {
    /// Enter into editable mode by creating a git sandbox for the floxmeta
    pub async fn enter_transaction(
        self,
    ) -> Result<(Environment<'flox, Git, GitSandBox<Git>>, Index), TransactionEnterError> {
        let (project, index) = self.project.enter_transaction().await?;
        Ok((
            Environment {
                name: self.name,
                system: self.system,
                project,
            },
            index,
        ))
    }
}

/// Implementations for sandboxed only Environments
impl<'flox, Git: GitProvider> Environment<'flox, Git, GitSandBox<Git>> {
    /// Commit changes to environment by closing the underlying transaction
    pub async fn commit_transaction(
        self,
        index: Index,
        message: &'flox str,
    ) -> Result<Environment<'_, Git, ReadOnly<Git>>, TransactionCommitError<Git>> {
        let project = self.project.commit_transaction(index, message).await?;
        Ok(Environment {
            name: self.name,
            system: self.system,
            project,
        })
    }
}
