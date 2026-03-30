// Private tree module for building package hierarchies from locked sources
// This module remains internal to nef-lock-catalog and is not exposed in the public API

use std::collections::BTreeMap;

use anyhow::Result;
use flox_catalog::BuildType;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use crate::lock::NixFlakeref;

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
        /// A validated nix source ref
        /// Note: validation (parsing) is done during construction using [NixFlakeref::from_value].
        /// The [NixFlakeref] type is then unwrapped to a [Value] for serialization as
        /// [NixFlakeref] does not implement [serde::Serialize].
        source: Value,
    },
}

/// Builds a package tree from locked source items
pub struct PackageTreeBuilder {
    root: PackageTreeNode,
}

impl PackageTreeBuilder {
    pub fn new() -> Self {
        Self {
            root: PackageTreeNode::PackageSet {
                entries: BTreeMap::new(),
            },
        }
    }

    pub fn into_root(self) -> PackageTreeNode {
        self.root
    }

    /// Add a package to the tree using optimized split_last() approach
    ///
    /// # Parameters
    /// - `attr_path`: Path components (e.g., ["pkgs", "hello"])
    /// - `build_type`: Type of build
    /// - `source`: Validated source reference
    pub fn add_package(
        &mut self,
        attr_path: Vec<String>,
        build_type: BuildType,
        source: NixFlakeref,
    ) -> Result<()> {
        let Some((final_attribute, parent_attributes)) = attr_path.split_last() else {
            anyhow::bail!("Empty attribute path");
        };

        // Build the path step by step
        let mut current_node = &mut self.root;

        // Process intermediate components (all guaranteed to be package sets)
        for (index, attribute) in parent_attributes.iter().enumerate() {
            match current_node {
                PackageTreeNode::PackageSet { entries } => {
                    // Ensure package set exists and handle conflict resolution
                    let entry = entries.entry(attribute.clone()).or_insert_with(|| {
                        PackageTreeNode::PackageSet {
                            entries: BTreeMap::new(),
                        }
                    });

                    // Navigate to child package set
                    current_node = entry
                },
                PackageTreeNode::Package { .. } => {
                    // If the entry is a package, replace it with a package set
                    //
                    // TODO: allow user driven handling of conflicts, e.g. via excludes
                    warn!(
                        "Conflict: replacing package with package set at path: {}",
                        attr_path[..=index].join(".")
                    );
                    *current_node = PackageTreeNode::PackageSet {
                        entries: BTreeMap::new(),
                    };
                },
            }
        }

        // Insert final package using final component as key
        match current_node {
            PackageTreeNode::PackageSet { entries } => {
                // Check if there's already a package set at this location
                if let Some(PackageTreeNode::PackageSet { .. }) = entries.get(final_attribute) {
                    // TODO: allow user driven handling of conflicts, e.g. via excludes
                    warn!(
                        "Conflict: package set already exists at path: {}",
                        attr_path.join(".")
                    );
                    // Package set wins - don't add the package
                    return Ok(());
                }

                // Use the parsed flake reference data directly
                let source_value = source.as_parsed().clone();

                entries.insert(final_attribute.clone(), PackageTreeNode::Package {
                    build_type,
                    source: source_value,
                });
            },
            PackageTreeNode::Package { .. } => {
                // If the entry is a package, replace it with a package set
                //
                // TODO: allow user driven handling of conflicts, e.g. via excludes
                warn!(
                    "Conflict: replacing package with package set at path: {}",
                    attr_path.join(".")
                );
                *current_node = PackageTreeNode::PackageSet {
                    entries: BTreeMap::new(),
                };
            },
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use flox_catalog::BuildType;
    use serde_json::json;

    use super::*;

    /// Helper function to create a package node JSON value from a NixFlakeref
    fn make_package_json(flakeref: &NixFlakeref) -> Value {
        serde_json::json!({
            "type": "package",
            "build_type": "manifest",
            "source": flakeref.as_parsed()
        })
    }

    /// Helper function to create test NixFlakeref
    fn create_test_flakeref() -> NixFlakeref {
        NixFlakeref::try_from(serde_json::json!({
            "type": "git",
            "url": "https://example.com",
            "rev": "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2",
            "dir": "foo/bar/.flox"
        }))
        .unwrap()
    }

    #[test]
    fn build_simple_tree() {
        let flakeref = create_test_flakeref();

        let mut builder = PackageTreeBuilder::new();
        builder
            .add_package(
                vec!["pkgs".to_string(), "hello".to_string()],
                BuildType::Manifest,
                flakeref.clone(),
            )
            .unwrap();
        builder
            .add_package(
                vec!["pkgs".to_string(), "grep".to_string()],
                BuildType::Manifest,
                flakeref.clone(),
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
                        "hello": make_package_json(&flakeref),
                        "grep": make_package_json(&flakeref)
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
        let flakeref = create_test_flakeref();

        builder
            .add_package(
                vec!["standalone".to_string()],
                BuildType::Manifest,
                flakeref.clone(),
            )
            .unwrap();

        let tree = builder.into_root();

        // Build expected tree using serde_json::json! macro
        let expected_tree: PackageTreeNode = serde_json::from_value(json!({
            "type": "package_set",
            "entries": {
                "standalone": make_package_json(&flakeref)
            }
        }))
        .unwrap();

        assert_eq!(tree, expected_tree);
    }

    #[test]
    fn package_set_overrides_package_on_conflict() {
        // Add "conflict" as a leaf package, then add
        // "conflict.child" which forces "conflict" to become
        // a package set.
        let mut builder = PackageTreeBuilder::new();
        let flakeref = create_test_flakeref();

        builder
            .add_package(
                vec!["conflict".to_string()],
                BuildType::Manifest,
                flakeref.clone(),
            )
            .unwrap();

        // Then create package set that should replace it
        builder
            .add_package(
                vec!["conflict".to_string(), "child".to_string()],
                BuildType::Manifest,
                flakeref.clone(),
            )
            .unwrap();

        let tree = builder.into_root();

        // Build expected tree using serde_json::json! macro
        // Note: When package set replaces package, the original package is lost
        let expected_tree: PackageTreeNode = serde_json::from_value(json!({
            "type": "package_set",
            "entries": {
                "conflict": {
                    "type": "package_set",
                    "entries": {
                        "child": make_package_json(&flakeref)
                    }
                }
            }
        }))
        .unwrap();

        assert_eq!(tree, expected_tree);
    }

    #[test]
    fn package_ignored_on_conflict() {
        // Create package set first
        let mut builder = PackageTreeBuilder::new();
        let flakeref = create_test_flakeref();

        builder
            .add_package(
                vec!["conflict".to_string(), "child".to_string()],
                BuildType::Manifest,
                flakeref.clone(),
            )
            .unwrap();

        // Then create package that should be replaced
        builder
            .add_package(
                vec!["conflict".to_string()],
                BuildType::Manifest,
                flakeref.clone(),
            )
            .unwrap();

        let tree = builder.into_root();

        // Build expected tree using serde_json::json! macro
        let expected_tree: PackageTreeNode = serde_json::from_value(json!({
            "type": "package_set",
            "entries": {
                // On conflict, the package is replaced by the packageset
                "conflict": {
                    "type": "package_set",
                    "entries": {
                        "child": make_package_json(&flakeref)
                    }
                }
            }
        }))
        .unwrap();

        assert_eq!(tree, expected_tree);
    }

    #[test]
    fn empty_path_components() {
        let mut builder = PackageTreeBuilder::new();
        let flakeref = create_test_flakeref();

        // Try to add package with empty path
        let result = builder.add_package(vec![], BuildType::Manifest, flakeref);
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
        let flakeref = create_test_flakeref();

        builder
            .add_package(
                vec![
                    "a".to_string(),
                    "b".to_string(),
                    "c".to_string(),
                    "leaf".to_string(),
                ],
                BuildType::Manifest,
                flakeref.clone(),
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
                                        "leaf": make_package_json(&flakeref)
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
    fn serialization_preserved() {
        let mut builder = PackageTreeBuilder::new();
        let flakeref = create_test_flakeref();

        builder
            .add_package(
                vec!["test".to_string()],
                BuildType::Manifest,
                flakeref.clone(),
            )
            .unwrap();

        let tree = builder.into_root();

        // Build expected tree using serde_json::json! macro
        let expected_tree: PackageTreeNode = serde_json::from_value(json!({
            "type": "package_set",
            "entries": {
                "test": make_package_json(&flakeref)
            }
        }))
        .unwrap();

        assert_eq!(tree, expected_tree);

        // Verify the source is preserved correctly by checking the expected tree
        if let PackageTreeNode::PackageSet { entries: children } = &expected_tree {
            if let Some(PackageTreeNode::Package { source, .. }) = children.get("test") {
                // Verify the source is a proper Value
                assert!(source.is_object());

                // Verify it contains expected fields
                let source_obj = source.as_object().unwrap();
                assert!(source_obj.contains_key("type"));
                assert!(source_obj.contains_key("url"));
                assert!(source_obj.contains_key("rev"));
                assert_eq!(source_obj.get("url").unwrap(), "https://example.com");
                assert_eq!(
                    source_obj.get("rev").unwrap(),
                    "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2"
                );
            } else {
                panic!("Expected package node");
            }
        } else {
            panic!("Expected package set node");
        }
    }

    #[test]
    fn multiple_root_packages() {
        let mut builder = PackageTreeBuilder::new();
        let flakeref = create_test_flakeref();

        builder
            .add_package(
                vec!["pkg1".to_string()],
                BuildType::Manifest,
                flakeref.clone(),
            )
            .unwrap();
        builder
            .add_package(
                vec!["pkg2".to_string()],
                BuildType::Manifest,
                flakeref.clone(),
            )
            .unwrap();
        builder
            .add_package(
                vec!["pkg3".to_string()],
                BuildType::Manifest,
                flakeref.clone(),
            )
            .unwrap();

        let tree = builder.into_root();

        // Build expected tree using serde_json::json! macro
        let expected_tree: PackageTreeNode = serde_json::from_value(json!({
            "type": "package_set",
            "entries": {
                "pkg1": make_package_json(&flakeref),
                "pkg2": make_package_json(&flakeref),
                "pkg3": make_package_json(&flakeref)
            }
        }))
        .unwrap();

        assert_eq!(tree, expected_tree);
    }

    #[test]
    fn mixed_nesting_levels() {
        let mut builder = PackageTreeBuilder::new();
        let flakeref = create_test_flakeref();

        builder
            .add_package(
                vec!["a".to_string(), "b".to_string(), "c".to_string()],
                BuildType::Manifest,
                flakeref.clone(),
            )
            .unwrap();
        builder
            .add_package(
                vec!["a".to_string(), "d".to_string()],
                BuildType::Manifest,
                flakeref.clone(),
            )
            .unwrap();
        builder
            .add_package(vec!["e".to_string()], BuildType::Manifest, flakeref.clone())
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
                                "c": make_package_json(&flakeref)
                            }
                        },
                        "d": make_package_json(&flakeref)
                    }
                },
                "e": make_package_json(&flakeref)
            }
        }))
        .unwrap();

        assert_eq!(tree, expected_tree);
    }
}
