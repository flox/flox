use std::borrow::Cow;

use flox_types::catalog::CatalogEntry;
use runix::installable::{FlakeAttribute, ParseInstallableError};
use runix::store_path::StorePath;
use thiserror::Error;

use super::flox_package::{FloxPackage, FloxTriple};
use super::floxmeta::environment::GenerationError;
use super::root::transaction::ReadOnly;
use super::{floxmeta, project};
use crate::providers::git::GitProvider;

pub static CATALOG_JSON: &str = "catalog.json";
// don't forget to update the man page
pub const DEFAULT_KEEP_GENERATIONS: usize = 10;
// don't forget to update the man page
pub const DEFAULT_MAX_AGE_DAYS: u32 = 90;

pub enum CommonEnvironment<'flox, Git: GitProvider> {
    Named(floxmeta::environment::Environment<'flox, Git, ReadOnly<Git>>),
    Project(project::environment::Environment<'flox, Git, ReadOnly<Git>>),
}

pub enum InstalledPackage {
    Catalog(FloxTriple, CatalogEntry),
    Installable(FlakeAttribute, CatalogEntry),
    StorePath(StorePath),
}

impl<'flox, Git: GitProvider> CommonEnvironment<'flox, Git> {
    /// get a flake attribute for the environment
    /// todo flake_attribute should be constructed earlier
    pub async fn flake_attribute(
        &self,
    ) -> Result<FlakeAttribute, EnvironmentError<GenerationError<Git>, ParseInstallableError>> {
        match self {
            CommonEnvironment::Named(n) => n
                .flake_attribute(Default::default())
                .await
                .map_err(EnvironmentError::Named),
            CommonEnvironment::Project(p) => p.flake_attribute().map_err(EnvironmentError::Project),
        }
    }

    pub fn system(&self) -> Cow<str> {
        match self {
            CommonEnvironment::Named(n) => n.system(),
            CommonEnvironment::Project(p) => p.system(),
        }
    }

    pub fn packages(&self) -> Vec<InstalledPackage> {
        todo!()
    }

    pub async fn install(
        self,
        packages: &[FloxPackage],
    ) -> Result<CommonEnvironment<'flox, Git>, ()> {
        match self {
            CommonEnvironment::Named(_) => todo!(),
            CommonEnvironment::Project(p) => {
                let (sandbox, mut index) = p.enter_transaction().await.map_err(|_| ())?;
                sandbox.install(packages).await?;
                index.insert(sandbox.flox_nix().await.unwrap(), project::FileAction::Add);
                sandbox
                    .commit_transaction(index, "installed packages")
                    .await
                    .map_err(|_| ())
                    .map(CommonEnvironment::Project)
            },
        }
    }

    pub async fn uninstall(&self, _packages: &[FloxPackage]) {
        todo!()
    }

    pub async fn upgrade(&self, _packages: &[FloxPackage]) {
        todo!()
    }

    pub fn named(self) -> Option<floxmeta::environment::Environment<'flox, Git, ReadOnly<Git>>> {
        match self {
            CommonEnvironment::Named(n) => Some(n),
            CommonEnvironment::Project(_) => None,
        }
    }

    pub fn project(self) -> Option<project::environment::Environment<'flox, Git, ReadOnly<Git>>> {
        match self {
            CommonEnvironment::Named(_) => None,
            CommonEnvironment::Project(p) => Some(p),
        }
    }
}

#[derive(Debug, Error)]
pub enum EnvironmentError<N, P> {
    #[error(transparent)]
    Named(N),
    #[error(transparent)]
    Project(P),
}
