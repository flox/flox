use derive_more::Constructor;
use std::fs;
use std::path::PathBuf;
use thiserror::Error;

use crate::flox::{Flox, FloxNixApi};

#[derive(Constructor)]
pub struct Environment<'flox> {
    flox: &'flox Flox,
    dir: PathBuf,
}

#[derive(Error, Debug)]
pub enum EnvironmentListError {}

#[derive(Error, Debug)]
pub enum EnvironmentEditError {}

#[derive(Error, Debug)]
pub enum EnvironmentInstallError {}

#[derive(Error, Debug)]
pub enum EnvironmentRemoveError {}

impl Environment<'_> {
    pub async fn list<Nix: FloxNixApi>(&self) -> Result<(), EnvironmentListError> {
        todo!()
    }

    pub async fn edit<Nix: FloxNixApi>(&self) -> Result<(), EnvironmentEditError> {
        todo!()
    }

    pub async fn install<Nix: FloxNixApi>(
        &self,
        package: &str,
    ) -> Result<(), EnvironmentInstallError> {
        todo!()
    }

    pub async fn remove<Nix: FloxNixApi>(
        &self,
        package: &str,
    ) -> Result<(), EnvironmentRemoveError> {
        todo!()
    }
}
