use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use thiserror::Error;

use super::flake_ref::{FlakeRefError, IndirectFlake, ToFlakeRef};

#[derive(Error, Debug)]
pub enum RegistryError {
    #[error(transparent)]
    FlakeRef(#[from] FlakeRefError),
}

#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
pub struct Registry {
    version: Version,
    /// Uses BTree implmentation to guarantee stable outputs
    /// [BTreeSet] unlike [std::collections::HashSet] guarantees
    /// that reading the set from a file and writing it back unchanged
    /// won't change the order of the elements.
    /// Hash Sets employ stochastic methods, that may change this order
    /// at the benefit of O(1) access (rather than O(log n) with BTree)
    flakes: BTreeSet<RegistryEntry>,
}

impl Registry {
    pub fn set(&mut self, name: impl ToString, to: ToFlakeRef) {
        let entry = RegistryEntry {
            from: FromFlakeRef::Indirect(IndirectFlake {
                id: name.to_string(),
            }),
            to,
            exact: None,
        };
        self.flakes.replace(entry);
    }

    #[allow(unused)]
    /// Todo: more functions such as remove, get, etc
    pub fn remove(&mut self, _name: impl ToString) {}
}

/// TODO: use https://github.com/dtolnay/serde-repr?
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct Version(u8);
impl Default for Version {
    fn default() -> Self {
        Self(2)
    }
}

#[skip_serializing_none]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct RegistryEntry {
    from: FromFlakeRef, // TODO merge into single flakeRef type @notgne2?
    to: ToFlakeRef,
    exact: Option<bool>,
}

impl Ord for RegistryEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.from.cmp(&other.from)
    }
}

impl PartialOrd for RegistryEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.from.partial_cmp(&other.from)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "type")]
#[serde(rename_all = "lowercase")]
enum FromFlakeRef {
    Indirect(IndirectFlake),
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use super::*;
    #[test]
    fn parses_nix_registry() {
        serde_json::from_reader::<_, Registry>(File::open("./test/registry.test.json").unwrap())
            .expect("should parse");
    }
}
