mod flox_version;

use std::fmt::Display;

pub use flox_core::canonical_path::{CanonicalPath, CanonicalizeError};
pub use flox_version::FloxVersion;
pub type System = String;

/// Different representations of the same attribute path
#[derive(Debug, Clone)]
pub enum AttrPath {
    Parts(Vec<String>),
    Joined(String),
}

impl From<Vec<String>> for AttrPath {
    fn from(value: Vec<String>) -> Self {
        AttrPath::Parts(value)
    }
}

impl From<&str> for AttrPath {
    fn from(value: &str) -> Self {
        AttrPath::Joined(value.to_string())
    }
}

impl From<String> for AttrPath {
    fn from(value: String) -> Self {
        AttrPath::Joined(value)
    }
}

impl From<AttrPath> for String {
    fn from(value: AttrPath) -> Self {
        match value {
            AttrPath::Parts(parts) => parts.join("."),
            AttrPath::Joined(s) => s,
        }
    }
}

impl Display for AttrPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <AttrPath as Into<String>>::into(self.clone()).fmt(f)
    }
}

impl PartialEq for AttrPath {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Parts(parts_self), Self::Parts(parts_other)) => parts_self == parts_other,
            (Self::Joined(joined_self), Self::Joined(joined_other)) => joined_self == joined_other,
            (Self::Joined(joined), Self::Parts(parts)) => joined == &parts.join("."),
            (Self::Parts(parts), Self::Joined(joined)) => joined == &parts.join("."),
        }
    }
}
