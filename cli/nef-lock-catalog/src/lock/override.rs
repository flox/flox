//! Per-invocation replacement of locked input sources.
//!
//! The lock equivalent of `nix build --override-input`: after a lock is
//! read or resolved and before it is handed to the NEF, the source of a
//! named input is swapped for one the user supplied — typically a local
//! checkout — without touching the committed `.flox/catalog.lock`. The NEF
//! fetches whatever flakeref attribute set a package leaf carries, so no
//! change is needed downstream; the catalog, however, only ever accepts
//! locked git sources back, so a lock with overrides must never be
//! published.

use std::fmt::Display;
use std::str::FromStr;

use super::build_lock::{BuildLock, CatalogLock};
use super::flakeref::RawNixFlakerefAttrs;
use super::tree::PackageTreeNode;
use crate::CatalogId;

/// Names a locked input by the lock's canonical `<catalog>/<attr-path>`
/// key, the form `direct_catalog_inputs` is keyed by (e.g.
/// `myorg/python3Packages.boolex`). Any package in the lock can be named,
/// including a transitive one that has no direct entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct InputKey {
    catalog: String,
    attr_path: Vec<String>,
}

impl InputKey {
    pub fn catalog(&self) -> &str {
        &self.catalog
    }

    pub fn attr_path(&self) -> &[String] {
        &self.attr_path
    }
}

impl FromStr for InputKey {
    type Err = InputKeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let Some((catalog, attr_path)) = s.split_once('/') else {
            return Err(InputKeyError(s.to_string()));
        };
        if catalog.is_empty() || attr_path.is_empty() {
            return Err(InputKeyError(s.to_string()));
        }
        let attr_path: Vec<String> = attr_path.split('.').map(str::to_string).collect();
        if attr_path.iter().any(String::is_empty) {
            return Err(InputKeyError(s.to_string()));
        }
        Ok(InputKey {
            catalog: catalog.to_string(),
            attr_path,
        })
    }
}

impl Display for InputKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.catalog, self.attr_path.join("."))
    }
}

#[derive(Debug, thiserror::Error)]
#[error(
    "'{0}' is not a catalog input key; expected the form '<catalog>/<package>' as found in '.flox/catalog.lock'."
)]
pub struct InputKeyError(String);

/// The replacement of one locked input's source.
#[derive(Debug, Clone, PartialEq)]
pub struct InputOverride {
    pub key: InputKey,
    /// The replacement flakeref attribute set. A missing `dir` inherits the
    /// locked source's, since the NEF locates the package expressions
    /// beneath it.
    pub source: RawNixFlakerefAttrs,
}

#[derive(Debug, thiserror::Error)]
pub enum OverrideError {
    #[error(
        "The catalog lock has no input '{key}'.\nInputs in the lock: {}",
        .available.join(", ")
    )]
    UnknownInput {
        key: InputKey,
        available: Vec<String>,
    },
}

impl BuildLock {
    /// Replace the source of each named input, in the `catalogs` tree the
    /// NEF fetches from and in `direct_catalog_inputs` when the input has
    /// a direct entry. Every override must name an input the lock
    /// contains; the first that does not fails the whole call, listing the
    /// inputs the lock does have.
    ///
    /// A direct entry's `locked_inputs_hash` is left as found: it names
    /// the recorded build the *original* source pinned, and nothing
    /// consumes it from a lock that carries overrides — a publish refuses
    /// such a lock before reading it.
    pub fn override_inputs(
        &mut self,
        overrides: impl IntoIterator<Item = InputOverride>,
    ) -> Result<(), OverrideError> {
        for InputOverride { key, source } in overrides {
            let leaf = self
                .catalogs
                .get_mut(&CatalogId(key.catalog.clone()))
                .and_then(|CatalogLock::FloxHub { packages }| {
                    packages.get_package_mut(&key.attr_path)
                });
            let Some(PackageTreeNode::Package { source: locked, .. }) = leaf else {
                return Err(OverrideError::UnknownInput {
                    key,
                    available: self.input_keys().iter().map(ToString::to_string).collect(),
                });
            };
            let source = source.inheriting_dir_from(locked);
            *locked = source.clone();

            if let Some(direct) = self
                .direct_catalog_inputs
                .values_mut()
                .find(|direct| direct.catalog == key.catalog && direct.attr_path == key.attr_path)
            {
                direct.source = source;
            }
        }
        Ok(())
    }

    /// The key of every package the lock pins, direct or transitive, in
    /// catalog then attr-path order.
    pub fn input_keys(&self) -> Vec<InputKey> {
        self.catalogs
            .iter()
            .flat_map(|(catalog, CatalogLock::FloxHub { packages })| {
                packages
                    .package_paths()
                    .into_iter()
                    .map(|attr_path| InputKey {
                        catalog: catalog.to_string(),
                        attr_path,
                    })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use floxhub_client::{BuildType, LockedGitSource, LockedInputEntry};
    use serde_json::{Value, json};

    use super::*;
    use crate::lock::transform::build_lock_from_locked_inputs;

    fn git_entry(catalog: &str, attr_path: &[&str]) -> LockedInputEntry {
        LockedInputEntry {
            attr_path: attr_path.iter().map(|s| s.to_string()).collect(),
            build_type: BuildType::Nef,
            catalog: catalog.to_string(),
            inputs: Some(vec![]),
            locked_inputs_hash: "sha256-test".to_string(),
            source: LockedGitSource {
                dir: ".flox".to_string(),
                ref_: "refs/heads/main".to_string(),
                rev: "abc".to_string(),
                type_: "git".to_string(),
                url: "https://example.com/repo".to_string(),
            },
        }
    }

    /// A lock pinning `locked` (canonical keys), of which `direct` have a
    /// direct entry; the rest are transitive.
    fn lock_with(locked: &[&str], direct: &[&str]) -> BuildLock {
        let locked: HashMap<String, LockedInputEntry> = locked
            .iter()
            .map(|key| {
                let (catalog, rest) = key.split_once('/').unwrap();
                let attr_path: Vec<&str> = rest.split('.').collect();
                ((*key).to_string(), git_entry(catalog, &attr_path))
            })
            .collect();
        let direct: Vec<String> = direct.iter().map(|key| (*key).to_string()).collect();
        build_lock_from_locked_inputs(locked, direct.iter()).unwrap()
    }

    fn tree_source(lock: &BuildLock, catalog: &str, attr_path: &[&str]) -> Value {
        let value = serde_json::to_value(lock).unwrap();
        let mut node = &value["catalogs"][catalog]["packages"];
        for attr in attr_path {
            node = &node["entries"][*attr];
        }
        node["source"].clone()
    }

    fn override_of(key: &str, source: Value) -> InputOverride {
        InputOverride {
            key: key.parse().unwrap(),
            source: RawNixFlakerefAttrs::new_unchecked(source),
        }
    }

    #[test]
    fn key_parses_and_renders_canonically() {
        let key: InputKey = "myorg/python3Packages.boolex".parse().unwrap();
        assert_eq!(key, InputKey {
            catalog: "myorg".to_string(),
            attr_path: vec!["python3Packages".to_string(), "boolex".to_string()],
        });
        assert_eq!(key.to_string(), "myorg/python3Packages.boolex");

        for invalid in ["hello", "/hello", "myorg/", "myorg/a..b", ""] {
            assert!(
                invalid.parse::<InputKey>().is_err(),
                "'{invalid}' must not parse"
            );
        }
    }

    /// An override rewrites both copies of the source: the tree leaf the
    /// NEF fetches from and the direct entry a publish would submit.
    #[test]
    fn override_rewrites_tree_leaf_and_direct_entry() {
        let mut lock = lock_with(&["myorg/hello", "myorg/world"], &["myorg/hello"]);
        let replacement = json!({ "type": "path", "path": "/src/hello", "dir": "." });

        lock.override_inputs([override_of("myorg/hello", replacement.clone())])
            .unwrap();

        assert_eq!(tree_source(&lock, "myorg", &["hello"]), replacement);
        assert_eq!(
            lock.direct_catalog_inputs["myorg/hello"].source.as_value(),
            &replacement
        );
        // Untouched inputs keep their locked source.
        assert_eq!(
            tree_source(&lock, "myorg", &["world"])["type"],
            json!("git")
        );
    }

    #[test]
    fn override_inherits_dir_when_the_replacement_has_none() {
        let mut lock = lock_with(&["myorg/hello"], &["myorg/hello"]);

        lock.override_inputs([override_of(
            "myorg/hello",
            json!({ "type": "git", "url": "file:///src/repo" }),
        )])
        .unwrap();

        assert_eq!(
            tree_source(&lock, "myorg", &["hello"]),
            json!({ "type": "git", "url": "file:///src/repo", "dir": ".flox" })
        );
    }

    /// A transitive input has a tree leaf but no direct entry; it can be
    /// overridden all the same, and the direct map is left alone.
    #[test]
    fn override_of_a_transitive_input_rewrites_only_the_tree() {
        let mut lock = lock_with(&["myorg/hello", "myorg/python3Packages.boolex"], &[
            "myorg/hello",
        ]);
        let replacement = json!({ "type": "path", "path": "/src/boolex", "dir": "." });

        lock.override_inputs([override_of(
            "myorg/python3Packages.boolex",
            replacement.clone(),
        )])
        .unwrap();

        assert_eq!(
            tree_source(&lock, "myorg", &["python3Packages", "boolex"]),
            replacement
        );
        assert_eq!(lock.direct_catalog_inputs.keys().collect::<Vec<_>>(), vec![
            "myorg/hello"
        ]);
    }

    #[test]
    fn unknown_input_fails_naming_the_inputs_the_lock_has() {
        let mut lock = lock_with(
            &["myorg/hello", "myorg/python3Packages.boolex", "other/tool"],
            &["myorg/hello"],
        );

        let err = lock
            .override_inputs([override_of(
                "myorg/missing",
                json!({ "type": "path", "path": "/x" }),
            )])
            .expect_err("an input the lock does not pin cannot be overridden");

        let OverrideError::UnknownInput { key, available } = err;
        assert_eq!(key.to_string(), "myorg/missing");
        assert_eq!(available, vec![
            "myorg/hello",
            "myorg/python3Packages.boolex",
            "other/tool"
        ]);
    }
}
