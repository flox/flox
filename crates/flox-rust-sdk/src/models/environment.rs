use std::borrow::Cow;

use serde_json::Value;

use super::flox_package::FloxPackage;
use super::floxmeta::{self};
use super::project;
use super::root::transaction::ReadOnly;
use crate::providers::git::GitProvider;

pub enum CommonEnvironment<'flox, Git: GitProvider> {
    Named(floxmeta::environment::Environment<'flox, Git, ReadOnly<Git>>),
    Project(project::environment::Environment<'flox, Git, ReadOnly<Git>>),
}

impl<'flox, Git: GitProvider> CommonEnvironment<'flox, Git> {
    pub async fn installable(&self) -> runix::installable::Installable {
        match self {
            CommonEnvironment::Named(n) => n.installable(Default::default()).await.unwrap(),
            CommonEnvironment::Project(p) => p.installable(),
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

    pub fn to_named(self) -> Option<floxmeta::environment::Environment<'flox, Git, ReadOnly<Git>>> {
        match self {
            CommonEnvironment::Named(n) => Some(n),
            CommonEnvironment::Project(_) => None,
        }
    }

    pub fn to_project(
        self,
    ) -> Option<project::environment::Environment<'flox, Git, ReadOnly<Git>>> {
        match self {
            CommonEnvironment::Named(_) => None,
            CommonEnvironment::Project(p) => Some(p),
        }
    }
}
