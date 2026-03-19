// Private tree module for building package hierarchies from locked sources
// This module remains internal to nef-lock-catalog and is not exposed in the public API

use std::collections::BTreeMap;

use anyhow::Result;
use flox_catalog::BuildType;
use serde_json::Value;
use tracing::warn;

use crate::lock::NixFlakeref;

/// Represents a node in the package tree - either a package set or an individual package
#[derive(Debug, Clone, PartialEq)]
enum PackageTreeNode {
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
struct PackageTreeBuilder {
    root: PackageTreeNode,
}

impl PackageTreeBuilder {
    fn new() -> Self {
        Self {
            root: PackageTreeNode::PackageSet {
                entries: BTreeMap::new(),
            },
        }
    }

    fn into_root(self) -> PackageTreeNode {
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
