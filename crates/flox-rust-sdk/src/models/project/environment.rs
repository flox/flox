use std::borrow::Cow;
use std::fmt::Display;
use std::path::PathBuf;

use flox_types::catalog::{EnvCatalog, StorePath};
use runix::arguments::eval::EvaluationArgs;
use runix::arguments::{BuildArgs, EvalArgs};
use runix::command::{Build, Eval};
use runix::command_line::{NixCommandLine, NixCommandLineRunJsonError};
use runix::installable::{FlakeAttribute, ParseInstallableError};
use runix::{NixBackend, Run, RunJson, RunTyped};
use thiserror::Error;

use super::{Index, Project, TransactionCommitError, TransactionEnterError};
use crate::flox::{Flox, FloxNixApi};
use crate::models::root::transaction::{GitAccess, GitSandBox, ReadOnly};
use crate::utils::errors::IoError;

pub struct Environment<'flox, Access: GitAccess> {
    /// aka. Nix attrpath, undr the assumption that they are not nested!
    pub(super) name: String,
    pub(super) system: String,
    pub(super) project: Project<'flox, Access>,
}

#[derive(Error, Debug)]
pub enum ProjectEnvironmentError {
    #[error(transparent)]
    ParseInstallable(#[from] ParseInstallableError),
    #[error(transparent)]
    Io(#[from] IoError),
    #[error("Failed to eval environment catalog: {0}")]
    EvalCatalog(NixCommandLineRunJsonError),
    #[error("Failed parsing environment catalog: {0}")]
    ParseCatalog(serde_json::Error),
    #[error("Failed parsing store paths installed in environment: {0}")]
    ParseStorePaths(serde_json::Error),
}

#[derive(Error, Debug)]
pub enum EnvironmentBuildError<Nix: NixBackend>
where
    Build: Run<Nix>,
{
    #[error("Could not create installable: {0}")]
    ParseInstallable(#[from] ParseInstallableError),
    #[error(transparent)]
    Build(<Build as Run<Nix>>::Error),
}

/// Implementations for an environment
impl<A: GitAccess> Environment<'_, A> {
    pub fn name(&self) -> Cow<str> {
        Cow::from(&self.name)
    }

    pub fn system(&self) -> Cow<str> {
        Cow::from(&self.system)
    }

    // pub async fn metadata(&self) -> Result<Metadata, MetadataError<Git>> {
    //    todo!("to be replaced by catalog")
    // }

    /// get a flake_attribute for this environment
    // todo: share with named env
    pub fn flake_attribute(&self) -> Result<FlakeAttribute, ParseInstallableError> {
        Ok(FlakeAttribute {
            flakeref: self.project.flakeref(),
            attr_path: ["", "floxEnvs", &self.system, &self.name].try_into()?,
            outputs: Default::default(),
        })
    }

    pub async fn installed_store_paths(
        &self,
        flox: &Flox,
    ) -> Result<Vec<StorePath>, ProjectEnvironmentError> {
        let nix = flox.nix::<NixCommandLine>(Default::default());

        let mut flake_attribute = self.flake_attribute()?;
        flake_attribute.attr_path.push_attr("installedStorePaths")?;

        let eval = Eval {
            eval: EvaluationArgs {
                impure: true.into(),
                ..Default::default()
            },
            eval_args: EvalArgs {
                installable: Some(flake_attribute.into()),
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

        let mut flake_attribute = self.flake_attribute()?;
        flake_attribute.attr_path.push_attr("catalog")?;

        let eval = Eval {
            eval: EvaluationArgs {
                impure: true.into(),
                ..Default::default()
            },
            eval_args: EvalArgs {
                installable: Some(flake_attribute.into()),
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

    pub fn systematized_name(&self) -> String {
        format!("{0}.{1}", self.system, self.name)
    }

    /// Where to link a built environment to
    ///
    /// When used as a lookup signals whether the environment has *at some point* been built before
    /// and is "activatable". Note that the environment may have been modified since it was last built.
    ///
    /// Mind that an existing out link does not necessarily imply that the environment
    /// can in fact be built.
    pub fn out_link(&self) -> PathBuf {
        self.project
            .environment_out_link_dir()
            .join(self.systematized_name())
    }

    /// Try building the environment and optionally linking it to the associated out_link
    ///
    /// [try_build]'s only external effect is having nix build
    /// and create a gcroot/out_link for an environment derivation.
    pub async fn try_build<Nix>(&self) -> Result<(), EnvironmentBuildError<Nix>>
    where
        Nix: FloxNixApi,
        Build: RunTyped<Nix>,
    {
        let nix: Nix = self.project.flox.nix([].to_vec());

        let build = Build {
            installables: [self.flake_attribute()?.into()].into(),
            eval: runix::arguments::eval::EvaluationArgs {
                impure: true.into(),
                ..Default::default()
            },
            build: BuildArgs {
                out_link: Some(self.out_link().into()),
                ..Default::default()
            },
            ..Default::default()
        };

        build
            .run(&nix, &Default::default())
            .await
            .map_err(EnvironmentBuildError::Build)?;
        Ok(())
    }
}

#[derive(Debug, Error)]
#[error(transparent)]
pub struct BuildError<Nix: NixBackend>(pub(crate) <Build as Run<Nix>>::Error)
where
    Build: Run<Nix>;

/// Implementations for R/O only instances
///
/// Mainly transformation into modifiable sandboxed instances
impl<'flox> Environment<'flox, ReadOnly> {
    /// Enter into editable mode by creating a git sandbox for the floxmeta
    pub async fn enter_transaction(
        self,
    ) -> Result<(Environment<'flox, GitSandBox>, Index), TransactionEnterError> {
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
impl<'flox> Environment<'flox, GitSandBox> {
    /// Commit changes to environment by closing the underlying transaction
    pub async fn commit_transaction(
        self,
        index: Index,
        message: &'flox str,
    ) -> Result<Environment<'_, ReadOnly>, TransactionCommitError> {
        let project = self.project.commit_transaction(index, message).await?;
        Ok(Environment {
            name: self.name,
            system: self.system,
            project,
        })
    }
}

impl<A: GitAccess> Display for Environment<'_, A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // this assumes self.project.flakeref is the current working directory
        write!(f, "environment .#{}", self.name)
    }
}

#[cfg(test)]
#[cfg(feature = "impure-unit-tests")]
mod tests {
    use std::env;

    use tempfile::TempDir;

    use crate::flox::Flox;
    use crate::models::environment::MANIFEST_FILENAME;
    use crate::prelude::ChannelRegistry;
    use crate::providers::git::{GitCommandProvider, GitProvider};

    fn flox_instance() -> (Flox, TempDir) {
        let tempdir_handle = tempfile::tempdir_in(std::env::temp_dir()).unwrap();

        let cache_dir = tempdir_handle.path().join("caches");
        let temp_dir = tempdir_handle.path().join("temp");
        let config_dir = tempdir_handle.path().join("config");

        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();

        let mut channels = ChannelRegistry::default();
        channels.register_channel("flox", "github:flox/floxpkgs/master".parse().unwrap());

        let flox = Flox {
            system: "aarch64-darwin".to_string(),
            cache_dir,
            temp_dir,
            config_dir,
            channels,
            ..Default::default()
        };

        (flox, tempdir_handle)
    }
}
