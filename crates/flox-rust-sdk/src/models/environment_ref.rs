use std::fmt::Display;
use std::str::FromStr;

use derive_more::{AsRef, Deref, Display};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use thiserror::Error;

pub static DEFAULT_NAME: &str = "default";
pub static DEFAULT_OWNER: &str = "local";

#[derive(Debug, Clone, PartialEq, AsRef, Deref, Display, DeserializeFromStr, SerializeDisplay)]
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

#[derive(Debug, Clone, PartialEq, AsRef, Display, DeserializeFromStr, SerializeDisplay)]
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

#[derive(Debug, Clone, PartialEq)]
pub struct EnvironmentRef {
    owner: Option<EnvironmentOwner>,
    name: EnvironmentName,
}

impl Display for EnvironmentRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref owner) = self.owner {
            write!(f, "{owner}/")?;
        }
        write!(f, "{}", self.name)
    }
}

impl FromStr for EnvironmentRef {
    type Err = EnvironmentRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((owner, name)) = s.split_once('/') {
            Ok(Self {
                owner: Some(EnvironmentOwner::from_str(owner)?),
                name: EnvironmentName::from_str(name)?,
            })
        } else {
            Ok(Self {
                owner: None,
                name: EnvironmentName::from_str(s)?,
            })
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
    pub fn owner(&self) -> Option<&EnvironmentOwner> {
        self.owner.as_ref()
    }

    pub fn name(&self) -> &EnvironmentName {
        &self.name
    }

    pub fn new(owner: Option<&str>, name: impl AsRef<str>) -> Result<Self, EnvironmentRefError> {
        Ok(Self {
            name: EnvironmentName::from_str(name.as_ref())?,
            owner: owner
                .as_ref()
                .map(|o| EnvironmentOwner::from_str(o))
                .transpose()?,
        })
    }

    pub fn new_from_parts(owner: Option<EnvironmentOwner>, name: EnvironmentName) -> Self {
        Self { owner, name }
    }
}
