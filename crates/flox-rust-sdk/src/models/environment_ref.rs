use std::fmt::Display;
use std::str::FromStr;

use derive_more::{AsRef, Deref, Display};
use runix::installable::FlakeAttribute;
use thiserror::Error;

use super::environment::{DotFloxDir, Environment, EnvironmentError2, Read};
use crate::flox::Flox;
use crate::providers::git::GitProvider;

pub static DEFAULT_NAME: &str = "default";
pub static DEFAULT_OWNER: &str = "local";

#[derive(Debug, Clone, PartialEq, AsRef, Deref, Display)]
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

#[derive(Debug, Clone, PartialEq, AsRef, Display)]
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

#[allow(unused)]
impl EnvironmentRef {
    /// Returns a list of all matches for a user specified environment
    pub fn find(
        flox: &Flox,
        environment_name: Option<&str>,
    ) -> Result<(Vec<EnvironmentRef>), EnvironmentError2> {
        let dot_flox_dir = DotFloxDir::discover(std::env::current_dir().unwrap())?;

        let env_ref = environment_name
            .map(|n| n.parse::<EnvironmentRef>())
            .transpose()?;

        let mut environment_refs = dot_flox_dir.environments()?;

        if let Some(env_ref) = env_ref {
            environment_refs.retain(|env| {
                if env_ref.owner.is_some() {
                    env_ref.owner == env.environment_ref().owner
                        && env_ref.name == env.environment_ref().name
                } else {
                    env_ref.name == env.environment_ref().name
                        || Some(env_ref.name.as_ref()) == env.environment_ref().owner.as_deref()
                }
            });
        }

        Ok(environment_refs
            .into_iter()
            .map(|env| env.environment_ref().clone())
            .collect())
    }

    // only used by some autocompletion logic
    // TODO: remove?
    pub async fn get_latest_flake_attribute<'flox, Git: GitProvider>(
        &self,
        flox: &'flox Flox,
    ) -> Result<FlakeAttribute, EnvironmentError2> {
        let env = self.to_env()?;
        Ok(env.flake_attribute(&flox.system))
    }

    pub fn to_env(&self) -> Result<Environment<Read>, EnvironmentError2> {
        let dot_flox_dir = DotFloxDir::discover(std::env::current_dir().unwrap())?;
        let env = dot_flox_dir.environment(self.clone())?;
        Ok(env)
    }

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
