// Private tree module for building package hierarchies from locked sources
// This module remains internal to nef-lock-catalog and is not exposed in the public API

use std::collections::BTreeMap;

use anyhow::Result;
use floxhub_client::BuildType;
use serde::{Deserialize, Serialize};

use super::flakeref::RawNixFlakerefAttrs;

/// Represents a node in the package tree - either a package set or an individual package
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PackageTreeNode {
    /// A collection/group of packages
    PackageSet {
        entries: BTreeMap<String, PackageTreeNode>,
    },
    /// An individual package (leaf node)
    Package {
        build_type: BuildType,
        /// The package's locked source ref, stored verbatim. See
        /// [RawNixFlakerefAttrs] for the invariant it carries.
        source: RawNixFlakerefAttrs,
    },
}

/// Builds a package tree from locked source items
pub struct PackageTreeBuilder {
    root: BTreeMap<String, PackageTreeNode>,
}

impl PackageTreeBuilder {
    pub fn new() -> Self {
        Self {
            root: BTreeMap::new(),
        }
    }

    pub fn into_root(self) -> PackageTreeNode {
        PackageTreeNode::PackageSet { entries: self.root }
    }

    /// Add a package to the tree from a raw, already-locked source value.
    ///
    /// Unlike [Self::add_package], this stores `source` verbatim and performs
    /// no nix invocation — used for catalog `/build-inputs/lookup` results,
    /// whose `source` is already locked server-side.
    pub fn add_package_source(
        &mut self,
        attr_path: Vec<String>,
        build_type: BuildType,
        source: RawNixFlakerefAttrs,
    ) -> Result<()> {
        let Some((final_attribute, parent_attributes)) = attr_path.split_last() else {
            anyhow::bail!("Empty attribute path");
        };

        // Build the path step by step
        let mut entries = &mut self.root;

        // Process intermediate components (all guaranteed to be package sets)
        for (index, attribute) in parent_attributes.iter().enumerate() {
            let node =
                entries
                    .entry(attribute.clone())
                    .or_insert_with(|| PackageTreeNode::PackageSet {
                        entries: BTreeMap::new(),
                    });
            entries = match node {
                PackageTreeNode::PackageSet { entries } => entries,
                PackageTreeNode::Package { .. } => anyhow::bail!(
                    "package '{}' collides with package '{}'",
                    attr_path[..=index].join("."),
                    attr_path.join(".")
                ),
            };
        }

        // Insert final package using final component as key. The source is
        // stored verbatim — it is already locked server-side.
        let package = PackageTreeNode::Package { build_type, source };
        if let Some(PackageTreeNode::PackageSet { .. }) = entries.get(final_attribute) {
            let child = entries[final_attribute]
                .first_package_path()
                .expect("a package set created by insertion has a package");
            anyhow::bail!(
                "package '{}' collides with package '{}.{}'",
                attr_path.join("."),
                attr_path.join("."),
                child.join(".")
            );
        }
        if entries.contains_key(final_attribute) {
            anyhow::bail!("duplicate package path '{}'", attr_path.join("."));
        }
        entries.insert(final_attribute.clone(), package);

        Ok(())
    }
}

impl PackageTreeNode {
    fn first_package_path(&self) -> Option<Vec<String>> {
        match self {
            Self::Package { .. } => Some(Vec::new()),
            Self::PackageSet { entries } => entries.iter().find_map(|(name, node)| {
                node.first_package_path().map(|mut path| {
                    path.insert(0, name.clone());
                    path
                })
            }),
        }
    }
}

#[cfg(test)]
mod tests {

    use floxhub_client::BuildType;
    use serde_json::{Value, json};

    use super::*;

    /// A locked source value used across the tree tests.
    fn test_source() -> Value {
        json!({
            "type": "git",
            "url": "https://example.com",
            "rev": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
            "dir": "foo/bar/.flox"
        })
    }

    /// The expected serialized package node for a given source value.
    fn make_package_json(source: &Value) -> Value {
        json!({
            "type": "package",
            "build_type": "manifest",
            "source": source,
        })
    }

    #[test]
    fn build_simple_tree() {
        let source = test_source();

        let mut builder = PackageTreeBuilder::new();
        builder
            .add_package_source(
                vec!["pkgs".to_string(), "hello".to_string()],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap();
        builder
            .add_package_source(
                vec!["pkgs".to_string(), "grep".to_string()],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap();

        let tree = builder.into_root();

        // Build expected tree using serde_json::json! macro
        let expected_tree: PackageTreeNode = serde_json::from_value(json!({
            "type": "package_set",
            "entries": {
                "pkgs": {
                    "type": "package_set",
                    "entries": {
                        "hello": make_package_json(&source),
                        "grep": make_package_json(&source)
                    }
                }
            }
        }))
        .unwrap();

        assert_eq!(tree, expected_tree);
    }

    #[test]
    fn single_package() {
        let mut builder = PackageTreeBuilder::new();
        let source = test_source();

        builder
            .add_package_source(
                vec!["standalone".to_string()],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap();

        let tree = builder.into_root();

        // Build expected tree using serde_json::json! macro
        let expected_tree: PackageTreeNode = serde_json::from_value(json!({
            "type": "package_set",
            "entries": {
                "standalone": make_package_json(&source)
            }
        }))
        .unwrap();

        assert_eq!(tree, expected_tree);
    }

    #[test]
    fn package_then_child_is_rejected() {
        // Add "conflict" as a leaf package, then add
        // "conflict.child" which forces "conflict" to become
        // a package set.
        let mut builder = PackageTreeBuilder::new();
        let source = test_source();

        builder
            .add_package_source(
                vec!["conflict".to_string()],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap();

        let err = builder
            .add_package_source(
                vec!["conflict".to_string(), "child".to_string()],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("'conflict' collides with package 'conflict.child'")
        );
    }

    #[test]
    fn child_then_package_is_rejected() {
        // Create package set first
        let mut builder = PackageTreeBuilder::new();
        let source = test_source();

        builder
            .add_package_source(
                vec!["conflict".to_string(), "child".to_string()],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap();

        let err = builder
            .add_package_source(
                vec!["conflict".to_string()],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("'conflict' collides with package 'conflict.child'")
        );
    }

    #[test]
    fn empty_path_components() {
        let mut builder = PackageTreeBuilder::new();
        let source = test_source();

        // Try to add package with empty path
        let result = builder.add_package_source(
            vec![],
            BuildType::Manifest,
            RawNixFlakerefAttrs::new_unchecked(source),
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Empty attribute path")
        );
    }

    #[test]
    fn deep_nesting() {
        let mut builder = PackageTreeBuilder::new();
        let source = test_source();

        builder
            .add_package_source(
                vec![
                    "a".to_string(),
                    "b".to_string(),
                    "c".to_string(),
                    "leaf".to_string(),
                ],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap();

        let tree = builder.into_root();

        // Build expected tree using serde_json::json! macro
        let expected_tree: PackageTreeNode = serde_json::from_value(json!({
            "type": "package_set",
            "entries": {
                "a": {
                    "type": "package_set",
                    "entries": {
                        "b": {
                            "type": "package_set",
                            "entries": {
                                "c": {
                                    "type": "package_set",
                                    "entries": {
                                        "leaf": make_package_json(&source)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }))
        .unwrap();

        assert_eq!(tree, expected_tree);
    }

    #[test]
    fn duplicate_exact_path_is_rejected() {
        let mut builder = PackageTreeBuilder::new();
        for first in [true, false] {
            let result = builder.add_package_source(
                vec!["a".into()],
                BuildType::Nef,
                RawNixFlakerefAttrs::new_unchecked(test_source()),
            );
            if first {
                result.unwrap();
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .to_string()
                        .contains("duplicate package path 'a'")
                );
            }
        }
    }

    #[test]
    fn serialization_preserved() {
        let mut builder = PackageTreeBuilder::new();
        let source = test_source();

        builder
            .add_package_source(
                vec!["test".to_string()],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap();

        let tree = builder.into_root();

        // Build expected tree using serde_json::json! macro
        let expected_tree: PackageTreeNode = serde_json::from_value(json!({
            "type": "package_set",
            "entries": {
                "test": make_package_json(&source)
            }
        }))
        .unwrap();

        assert_eq!(tree, expected_tree);

        // The source is stored verbatim as the locked flakeref attrs.
        let PackageTreeNode::PackageSet { entries: children } = &tree else {
            panic!("expected package set node");
        };
        let Some(PackageTreeNode::Package { source, .. }) = children.get("test") else {
            panic!("expected package node");
        };
        assert_eq!(
            source,
            &RawNixFlakerefAttrs::new_unchecked(json!({
                "type": "git",
                "url": "https://example.com",
                "rev": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
                "dir": "foo/bar/.flox"
            }))
        );
    }

    #[test]
    fn multiple_root_packages() {
        let mut builder = PackageTreeBuilder::new();
        let source = test_source();

        builder
            .add_package_source(
                vec!["pkg1".to_string()],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap();
        builder
            .add_package_source(
                vec!["pkg2".to_string()],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap();
        builder
            .add_package_source(
                vec!["pkg3".to_string()],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap();

        let tree = builder.into_root();

        // Build expected tree using serde_json::json! macro
        let expected_tree: PackageTreeNode = serde_json::from_value(json!({
            "type": "package_set",
            "entries": {
                "pkg1": make_package_json(&source),
                "pkg2": make_package_json(&source),
                "pkg3": make_package_json(&source)
            }
        }))
        .unwrap();

        assert_eq!(tree, expected_tree);
    }

    #[test]
    fn mixed_nesting_levels() {
        let mut builder = PackageTreeBuilder::new();
        let source = test_source();

        builder
            .add_package_source(
                vec!["a".to_string(), "b".to_string(), "c".to_string()],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap();
        builder
            .add_package_source(
                vec!["a".to_string(), "d".to_string()],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap();
        builder
            .add_package_source(
                vec!["e".to_string()],
                BuildType::Manifest,
                RawNixFlakerefAttrs::new_unchecked(source.clone()),
            )
            .unwrap();

        let tree = builder.into_root();

        // Build expected tree using serde_json::json! macro
        let expected_tree: PackageTreeNode = serde_json::from_value(json!({
            "type": "package_set",
            "entries": {
                "a": {
                    "type": "package_set",
                    "entries": {
                        "b": {
                            "type": "package_set",
                            "entries": {
                                "c": make_package_json(&source)
                            }
                        },
                        "d": make_package_json(&source)
                    }
                },
                "e": make_package_json(&source)
            }
        }))
        .unwrap();

        assert_eq!(tree, expected_tree);
    }
}
