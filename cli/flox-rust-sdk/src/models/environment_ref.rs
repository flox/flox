use std::fmt::Display;
use std::path::PathBuf;
use std::str::FromStr;

use derive_more::{AsRef, Deref, Display};
use schemars::{JsonSchema, json_schema};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use shell_escape::escape;
use thiserror::Error;

use super::environment::{ConcreteEnvironment, Environment, EnvironmentError, ManagedPointer};

pub static DEFAULT_NAME: &str = "default";
pub static DEFAULT_OWNER: &str = "local";

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    AsRef,
    Deref,
    Display,
    DeserializeFromStr,
    SerializeDisplay,
    JsonSchema,
)]
pub struct EnvironmentOwner(String);

impl FromStr for EnvironmentOwner {
    type Err = RemoteEnvironmentRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if [' ', '/'].iter().any(|c| s.contains(*c)) {
            Err(RemoteEnvironmentRefError::InvalidOwner(s.to_string()))?
        }

        Ok(EnvironmentOwner(s.to_string()))
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    AsRef,
    Display,
    DeserializeFromStr,
    SerializeDisplay,
    JsonSchema,
)]
pub struct EnvironmentName(String);

impl FromStr for EnvironmentName {
    type Err = RemoteEnvironmentRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if [' ', '/'].iter().any(|c| s.contains(*c)) {
            Err(RemoteEnvironmentRefError::InvalidName(s.to_string()))?
        }

        Ok(EnvironmentName(s.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
#[cfg_attr(test, derive(proptest_derive::Arbitrary))]
pub struct RemoteEnvironmentRef {
    owner: EnvironmentOwner,
    name: EnvironmentName,
}

impl Display for RemoteEnvironmentRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl FromStr for RemoteEnvironmentRef {
    type Err = RemoteEnvironmentRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (owner, name) = s
            .split_once('/')
            .ok_or(RemoteEnvironmentRefError::InvalidOwner(s.to_string()))?;
        Ok(Self {
            owner: EnvironmentOwner::from_str(owner)?,
            name: EnvironmentName::from_str(name)?,
        })
    }
}

impl From<ManagedPointer> for RemoteEnvironmentRef {
    fn from(pointer: ManagedPointer) -> Self {
        Self {
            owner: pointer.owner,
            name: pointer.name,
        }
    }
}

impl JsonSchema for RemoteEnvironmentRef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "EnvironmentRef".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        json_schema!({
            "description": "Environment Reference",
            "type": "string",
        })
    }
}

#[derive(Error, Debug)]
pub enum RemoteEnvironmentRefError {
    #[error(
        "Name '{0}' is invalid.\nEnvironment names may only contain alphanumeric characters, '.', '_', and '-'."
    )]
    InvalidName(String),

    #[error(
        "Owner '{0}' is invalid.\nEnvironment owners may only contain alphanumeric characters, '.', '_', and '-'."
    )]
    InvalidOwner(String),
}

impl RemoteEnvironmentRef {
    pub fn owner(&self) -> &EnvironmentOwner {
        &self.owner
    }

    pub fn name(&self) -> &EnvironmentName {
        &self.name
    }

    pub fn new(
        owner: impl AsRef<str>,
        name: impl AsRef<str>,
    ) -> Result<Self, RemoteEnvironmentRefError> {
        Ok(Self {
            owner: EnvironmentOwner::from_str(owner.as_ref())?,
            name: EnvironmentName::from_str(name.as_ref())?,
        })
    }

    pub fn new_from_parts(owner: EnvironmentOwner, name: EnvironmentName) -> Self {
        Self { owner, name }
    }
}

/// An environment that can be activated.
/// ConcreteEnvironment::{Path,Managed} uses a local path that's the parent of `.flox`
/// ConcreteEnvironment::Remote uses a remote reference on FloxHub
//
// TODO: Support pinned generation for managed and remote environments?
#[derive(Debug, Clone)]
pub enum ActivateEnvironmentRef {
    Local(PathBuf),
    Remote(RemoteEnvironmentRef),
}

impl ActivateEnvironmentRef {
    /// Render the activation arguments (`-d`/`-r`) used by `flox activate`.
    pub fn activate_target_arg(&self) -> String {
        match self {
            ActivateEnvironmentRef::Local(path) => {
                format!("-d {}", escape(path.to_string_lossy()))
            },
            ActivateEnvironmentRef::Remote(remote) => {
                format!("-r {}", escape(remote.to_string().into()))
            },
        }
    }
}

impl TryFrom<&ConcreteEnvironment> for ActivateEnvironmentRef {
    type Error = EnvironmentError;

    fn try_from(env: &ConcreteEnvironment) -> Result<Self, Self::Error> {
        let env_ref = match env {
            ConcreteEnvironment::Path(env) => ActivateEnvironmentRef::Local(env.parent_path()?),
            ConcreteEnvironment::Managed(env) => ActivateEnvironmentRef::Local(env.parent_path()?),
            ConcreteEnvironment::Remote(env) => {
                ActivateEnvironmentRef::Remote(RemoteEnvironmentRef::from(env.pointer().clone()))
            },
        };
        Ok(env_ref)
    }
}

#[cfg(test)]
mod test {
    use proptest::arbitrary::Arbitrary;
    use proptest::strategy::{BoxedStrategy, Strategy};

    use super::*;

    impl Arbitrary for EnvironmentOwner {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            "[^ /]"
                .prop_map(|s| EnvironmentOwner(s.to_string()))
                .boxed()
        }
    }

    impl Arbitrary for EnvironmentName {
        type Parameters = ();
        type Strategy = BoxedStrategy<Self>;

        fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
            "[^ /]".prop_map(|s| EnvironmentName(s.to_string())).boxed()
        }
    }
}
