use std::borrow::Cow;
use std::fmt::Display;

use flox_types::catalog::{EnvCatalog, StorePath};
use runix::arguments::eval::EvaluationArgs;
use runix::arguments::EvalArgs;
use runix::command::Eval;
use runix::command_line::{NixCommandLine, NixCommandLineRunJsonError};
use runix::installable::Installable;
use runix::RunJson;
use thiserror::Error;

use super::{Index, Project, TransactionCommitError, TransactionEnterError};
use crate::flox::Flox;
use crate::models::root::transaction::{GitAccess, GitSandBox, ReadOnly};
use crate::providers::git::GitProvider;
use crate::utils::errors::IoError;

pub struct Environment<'flox, Git: GitProvider, Access: GitAccess<Git>> {
    /// aka. Nix attrpath, undr the assumption that they are not nested!
    pub(super) name: String,
    pub(super) system: String,
    pub(super) project: Project<'flox, Git, Access>,
}

#[derive(Error, Debug)]
pub enum ProjectEnvironmentError {
    #[error(transparent)]
    Io(#[from] IoError),
    #[error("Failed to eval environment catalog: {0}")]
    EvalCatalog(NixCommandLineRunJsonError),
    #[error("Failed parsing environment catalog: {0}")]
    ParseCatalog(serde_json::Error),
    #[error("Failed parsing store paths installed in environment: {0}")]
    ParseStorePaths(serde_json::Error),
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

    pub async fn installed_store_paths(
        &self,
        flox: &Flox,
    ) -> Result<Vec<StorePath>, ProjectEnvironmentError> {
        let nix = flox.nix::<NixCommandLine>(Default::default());

        let mut installable = self.installable();
        installable.attr_path.push_str(".installedStorePaths");

        let eval = Eval {
            eval: EvaluationArgs {
                impure: true.into(),
            },
            eval_args: EvalArgs {
                installable: Some(installable.into()),
                apply: None,
            },
            ..Eval::default()
        };

        let installed_store_paths_value: serde_json::Value = eval
            .run_json(&nix, &Default::default())
            .await
            .map_err(ProjectEnvironmentError::EvalCatalog)?;

        serde_json::from_value(installed_store_paths_value)
            .map_err(ProjectEnvironmentError::ParseStorePaths)
    }

    pub async fn catalog(&self, flox: &Flox) -> Result<EnvCatalog, ProjectEnvironmentError> {
        let nix = flox.nix::<NixCommandLine>(Default::default());

        let mut installable = self.installable();
        installable.attr_path.push_str(".catalog");

        let eval = Eval {
            eval: EvaluationArgs {
                impure: true.into(),
            },
            eval_args: EvalArgs {
                installable: Some(installable.into()),
                apply: None,
            },
            ..Eval::default()
        };

        let catalog_value: serde_json::Value = eval
            .run_json(&nix, &Default::default())
            .await
            .map_err(ProjectEnvironmentError::EvalCatalog)?;

        serde_json::from_value(catalog_value).map_err(ProjectEnvironmentError::ParseCatalog)
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

impl<Git: GitProvider, A: GitAccess<Git>> Display for Environment<'_, Git, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // this assumes self.project.flakeref is the current working directory
        write!(f, "environment .#{}", self.name)
    }
}
