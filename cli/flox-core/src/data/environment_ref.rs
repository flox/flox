use std::fmt::Display;
use std::path::PathBuf;
use std::str::FromStr;

use derive_more::{AsRef, Deref, Display};
use schemars::{JsonSchema, json_schema};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use shell_escape::escape;
use thiserror::Error;

pub static DEFAULT_NAME: &str = "default";
pub static DEFAULT_OWNER: &str = "local";

/// Whether `s` is usable as a single owner or name component.
///
/// Owners and names are interpolated directly into filesystem paths — most
/// visibly the local checkout at `<cache>/remote/<owner>/<name>` — so the
/// components that carry meaning to the filesystem are rejected here, at the
/// parse boundary, rather than left for each consumer to re-check. `..` is the
/// one that matters: `PathBuf::join` does not normalize it, so it would
/// otherwise resolve outside the directory the reference names.
fn is_valid_component(s: &str) -> bool {
    !s.is_empty() && !s.contains([' ', '/']) && s != "." && s != ".."
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
        if !is_valid_component(s) {
            Err(RemoteEnvironmentRefError::InvalidOwner(s.to_string()))?
        }

        Ok(EnvironmentOwner(s.to_string()))
    }
}

#[cfg(any(test, feature = "tests"))]
impl proptest::arbitrary::Arbitrary for EnvironmentName {
    type Parameters = ();
    type Strategy = proptest::strategy::BoxedStrategy<Self>;

    fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
        use proptest::prelude::Strategy;

        // Leading character excludes '.' so the strategy can never produce
        // the `.` / `..` that `is_valid_component` rejects, while still
        // covering dots elsewhere in the component.
        "[^ /.][^ /]{0,7}".prop_map(EnvironmentName).boxed()
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
        if !is_valid_component(s) {
            Err(RemoteEnvironmentRefError::InvalidName(s.to_string()))?
        }

        Ok(EnvironmentName(s.to_string()))
    }
}

#[cfg(any(test, feature = "tests"))]
impl proptest::arbitrary::Arbitrary for EnvironmentOwner {
    type Parameters = ();
    type Strategy = proptest::strategy::BoxedStrategy<Self>;

    fn arbitrary_with(_: Self::Parameters) -> Self::Strategy {
        use proptest::prelude::Strategy;

        // See the note on `EnvironmentName`'s strategy.
        "[^ /.][^ /]{0,7}".prop_map(EnvironmentOwner).boxed()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, SerializeDisplay, DeserializeFromStr)]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub struct RemoteEnvironmentRef {
    owner: EnvironmentOwner,
    name: EnvironmentName,
}

impl RemoteEnvironmentRef {
    pub fn from_parts(owner: EnvironmentOwner, name: EnvironmentName) -> Self {
        Self { owner, name }
    }
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
        "Name '{0}' is invalid.\nEnvironment names cannot be empty, '.' or '..', and cannot contain spaces or '/'."
    )]
    InvalidName(String),

    #[error(
        "Owner '{0}' is invalid.\nEnvironment owners cannot be empty, '.' or '..', and cannot contain spaces or '/'."
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A reference is interpolated into a filesystem path, so a component that
    /// traverses out of that path must not parse in the first place.
    #[test]
    fn rejects_path_traversing_references() {
        for reference in ["../scratch", "owner/..", "./x", "x/.", "/name", "owner/"] {
            assert!(
                RemoteEnvironmentRef::from_str(reference).is_err(),
                "'{reference}' should not parse as an environment reference"
            );
        }
    }

    #[test]
    fn accepts_ordinary_references() {
        assert_eq!(
            RemoteEnvironmentRef::from_str("owner/name").unwrap(),
            RemoteEnvironmentRef::new("owner", "name").unwrap()
        );
        // A leading dot is only a problem when the component is exactly `.`
        // or `..`, so hidden-looking names stay valid.
        assert_eq!(
            RemoteEnvironmentRef::from_str(".owner/..name").unwrap(),
            RemoteEnvironmentRef::new(".owner", "..name").unwrap()
        );
    }
}
