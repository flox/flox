use std::borrow::Cow;

use runix::installable::{Installable, ParseInstallableError};
use serde_json::Value;
use thiserror::Error;

use super::flox_package::FloxPackage;
use super::floxmeta::environment::GenerationError;
use super::root::transaction::ReadOnly;
use super::{floxmeta, project};
use crate::providers::git::GitProvider;

pub static CATALOG_JSON: &str = "catalog.json";

pub enum CommonEnvironment<'flox, Git: GitProvider> {
    Named(floxmeta::environment::Environment<'flox, Git, ReadOnly<Git>>),
    Project(project::environment::Environment<'flox, Git, ReadOnly<Git>>),
}

impl<'flox, Git: GitProvider> CommonEnvironment<'flox, Git> {
    /// get an installbale for the environment
    /// todo installable should be constructed earlier
    pub async fn installable(
        &self,
    ) -> Result<Installable, EnvironmentError<GenerationError<Git>, ParseInstallableError>> {
        match self {
            CommonEnvironment::Named(n) => n
                .installable(Default::default())
                .await
                .map_err(EnvironmentError::Named),
            CommonEnvironment::Project(p) => p.installable().map_err(EnvironmentError::Project),
        }
    }

    pub fn system(&self) -> Cow<str> {
        match self {
            CommonEnvironment::Named(n) => n.system(),
            CommonEnvironment::Project(p) => p.system(),
        }
    }

    pub fn packages(&self) -> Value {
        todo!()
    }

    pub async fn install(&self, _packages: &[FloxPackage]) {
        todo!()
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
