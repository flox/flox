use std::convert::Infallible;
use std::fmt::Display;
use std::str::FromStr;

use runix::installable::FlakeAttribute;
use thiserror::Error;

use super::environment::{DotFloxDir, Environment, EnvironmentError2, Read, State};
use crate::flox::Flox;
use crate::providers::git::GitProvider;

pub static DEFAULT_NAME: &str = "default";
pub static DEFAULT_OWNER: &str = "local";

#[derive(Debug, Clone)]
pub struct EnvironmentRef {
    owner: Option<String>,
    name: String,
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
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some((owner, name)) = s.split_once('/') {
            Ok(Self {
                owner: Some(owner.to_string()),
                name: name.to_string(),
            })
        } else {
            Ok(Self {
                owner: None,
                name: s.to_string(),
            })
        }
    }
}

impl<S: State> From<Environment<S>> for EnvironmentRef {
    fn from(env: Environment<S>) -> Self {
        EnvironmentRef {
            name: env.name().to_string(),
            owner: env.owner().map(ToString::to_string),
        }
    }
}

#[derive(Error, Debug)]
pub enum EnvironmentRefError {
    #[error(transparent)]
    Environment(EnvironmentError2),

    #[error("Name format is invalid")]
    Invalid,
}

#[allow(unused)]
impl EnvironmentRef {
    /// Returns a list of all matches for a user specified environment
    pub fn find(
        flox: &Flox,
        environment_name: Option<&str>,
    ) -> Result<(Vec<EnvironmentRef>), EnvironmentRefError> {
        let dot_flox_dir = DotFloxDir::discover(std::env::current_dir().unwrap())
            .map_err(EnvironmentRefError::Environment)?;

        let env_ref = environment_name.map(
            |n| n.parse::<EnvironmentRef>().unwrap(), /* infallible */
        );

        let mut environment_refs = dot_flox_dir
            .environments()
            .map_err(EnvironmentRefError::Environment)?;
        if let Some(env_ref) = env_ref {
            environment_refs.retain(|env| {
                if env_ref.owner.is_some() {
                    env_ref.owner.as_deref() == env.owner() && env_ref.name == env.name()
                } else {
                    env_ref.name == env.owner().unwrap_or_else(|| env.name())
                }
            });
        }

        Ok(environment_refs.into_iter().map(|env| env.into()).collect())
    }

    pub async fn get_latest_flake_attribute<'flox, Git: GitProvider>(
        &self,
        flox: &'flox Flox,
    ) -> Result<FlakeAttribute, EnvironmentRefError> {
        let env = self.to_env()?;
        Ok(env.flake_attribute(&flox.system))
    }

    pub fn to_env(&self) -> Result<Environment<Read>, EnvironmentRefError> {
        let dot_flox_dir = DotFloxDir::discover(std::env::current_dir().unwrap())
            .map_err(EnvironmentRefError::Environment)?;
        let env = dot_flox_dir
            .environment(self.owner.clone(), &self.name)
            .map_err(EnvironmentRefError::Environment)?;
        Ok(env)
    }
}
