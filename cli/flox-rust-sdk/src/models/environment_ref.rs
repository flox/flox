use std::fmt::Display;
use std::str::FromStr;

use derive_more::{AsRef, Deref, Display};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use thiserror::Error;

use super::environment::ManagedPointer;

pub static DEFAULT_NAME: &str = "default";
pub static DEFAULT_OWNER: &str = "local";

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, AsRef, Deref, Display, DeserializeFromStr, SerializeDisplay,
)]
pub struct EnvironmentOwner(String);

impl FromStr for EnvironmentOwner {
    type Err = EnvironmentRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if [' ', '/'].iter().any(|c| s.contains(*c)) {
            Err(EnvironmentRefError::InvalidOwner)?
        }

        Ok(EnvironmentOwner(s.to_string()))
    }
}

#[derive(
    Debug, Clone, PartialEq, Eq, Hash, AsRef, Display, DeserializeFromStr, SerializeDisplay,
)]
pub struct EnvironmentName(String);

impl FromStr for EnvironmentName {
    type Err = EnvironmentRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if [' ', '/'].iter().any(|c| s.contains(*c)) {
            Err(EnvironmentRefError::InvalidName)?
        }

        Ok(EnvironmentName(s.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
pub struct EnvironmentRef {
    owner: EnvironmentOwner,
    name: EnvironmentName,
}

impl Display for EnvironmentRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl FromStr for EnvironmentRef {
    type Err = EnvironmentRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (owner, name) = s.split_once('/').ok_or(EnvironmentRefError::InvalidOwner)?;
        Ok(Self {
            owner: EnvironmentOwner::from_str(owner)?,
            name: EnvironmentName::from_str(name)?,
        })
    }
}

impl From<ManagedPointer> for EnvironmentRef {
    fn from(pointer: ManagedPointer) -> Self {
        Self {
            owner: pointer.owner,
            name: pointer.name,
        }
    }
}

#[derive(Error, Debug)]
pub enum EnvironmentRefError {
    #[error("Name format is invalid")]
    InvalidName,

    #[error("Name format is invalid")]
    InvalidOwner,
}

impl EnvironmentRef {
    pub fn owner(&self) -> &EnvironmentOwner {
        &self.owner
    }

    pub fn name(&self) -> &EnvironmentName {
        &self.name
    }

    pub fn new(owner: impl AsRef<str>, name: impl AsRef<str>) -> Result<Self, EnvironmentRefError> {
        Ok(Self {
            owner: EnvironmentOwner::from_str(owner.as_ref())?,
            name: EnvironmentName::from_str(name.as_ref())?,
        })
    }

    pub fn new_from_parts(owner: EnvironmentOwner, name: EnvironmentName) -> Self {
        Self { owner, name }
    }
}
