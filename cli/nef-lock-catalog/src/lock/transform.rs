//! Transform the flat locked-input map returned by the catalog
//! `/build-inputs/lookup` endpoint into the hierarchical [BuildLock] consumed
//! by the NEF.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use floxhub_client::LockedInputEntry;
use serde_json::Value;

use crate::CatalogId;
use crate::lock::build_lock::{BuildLock, CatalogLock};
use crate::lock::tree::PackageTreeBuilder;

/// Build a hierarchical [BuildLock] from the flat locked-input map (the merged
/// `lock` maps of one or more `/build-inputs/lookup` groups).
///
/// Entries are grouped by their [`LockedInputEntry::catalog`]; each catalog's
/// packages are assembled into a [`crate::lock::tree::PackageTreeNode`] via
/// [`crate::lock::tree::PackageTreeBuilder`], keyed by
/// [`LockedInputEntry::attr_path`]. The wire `source` is stored **verbatim** —
/// no nix invocation.
pub(crate) fn build_lock_from_locked_inputs(
    locked: HashMap<String, LockedInputEntry>,
) -> Result<BuildLock> {
    let mut builders: BTreeMap<CatalogId, PackageTreeBuilder> = BTreeMap::new();

    for entry in locked.into_values() {
        let LockedInputEntry {
            attr_path,
            build_type,
            catalog,
            inputs: _,
            source,
        } = entry;

        builders
            .entry(CatalogId(catalog))
            .or_insert_with(PackageTreeBuilder::new)
            .add_package_source(attr_path, build_type, Value::Object(source))?;
    }

    let catalogs = builders
        .into_iter()
        .map(|(id, builder)| {
            (id, CatalogLock::FloxHub {
                packages: builder.into_root(),
            })
        })
        .collect();

    Ok(BuildLock {
        catalogs,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use floxhub_client::BuildType;
    use serde_json::json;

    use super::*;

    fn entry(
        catalog: &str,
        attr_path: &[&str],
        build_type: BuildType,
        source: serde_json::Value,
    ) -> LockedInputEntry {
        LockedInputEntry {
            attr_path: attr_path.iter().map(|s| s.to_string()).collect(),
            build_type,
            catalog: catalog.to_string(),
            inputs: None,
            source: source
                .as_object()
                .expect("test source must be a JSON object")
                .clone(),
        }
    }

    #[test]
    fn single_package_single_catalog() {
        let source = json!({ "type": "git", "url": "https://example.com/repo", "rev": "abc" });
        let locked = HashMap::from([(
            "myorg.hello".to_string(),
            entry("myorg", &["hello"], BuildType::Nef, source.clone()),
        )]);

        let lock = build_lock_from_locked_inputs(locked).expect("transform succeeds");

        assert_eq!(
            serde_json::to_value(&lock).unwrap(),
            json!({
                "version": 1,
                "catalogs": {
                    "myorg": {
                        "type": "floxhub",
                        "packages": {
                            "type": "package_set",
                            "entries": {
                                "hello": {
                                    "type": "package",
                                    "build_type": "nef",
                                    "source": source,
                                }
                            }
                        }
                    }
                }
            })
        );
    }

    #[test]
    fn nested_attr_path_builds_package_set() {
        let source = json!({ "type": "git", "url": "https://example.com/repo" });
        let locked = HashMap::from([(
            "myorg.python3Packages.boolex".to_string(),
            entry(
                "myorg",
                &["python3Packages", "boolex"],
                BuildType::Manifest,
                source.clone(),
            ),
        )]);

        let lock = build_lock_from_locked_inputs(locked).expect("transform succeeds");

        assert_eq!(
            serde_json::to_value(&lock).unwrap(),
            json!({
                "version": 1,
                "catalogs": {
                    "myorg": {
                        "type": "floxhub",
                        "packages": {
                            "type": "package_set",
                            "entries": {
                                "python3Packages": {
                                    "type": "package_set",
                                    "entries": {
                                        "boolex": {
                                            "type": "package",
                                            "build_type": "manifest",
                                            "source": source,
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            })
        );
    }

    #[test]
    fn groups_by_catalog_and_preserves_source_verbatim() {
        let src_a = json!({
            "type": "git",
            "url": "https://a.example/x",
            "rev": "deadbeef",
            "extra": { "nested": [1, 2, 3] },
        });
        let src_b = json!({ "type": "path", "path": "/store/b" });
        let locked = HashMap::from([
            (
                "a.foo".to_string(),
                entry("alpha", &["foo"], BuildType::Nef, src_a.clone()),
            ),
            (
                "b.bar".to_string(),
                entry("beta", &["bar"], BuildType::Nef, src_b.clone()),
            ),
        ]);

        let value = serde_json::to_value(
            build_lock_from_locked_inputs(locked).expect("transform succeeds"),
        )
        .unwrap();

        // Each catalog gets its own tree, and the locked source is stored
        // byte-for-byte (no nix normalization).
        assert_eq!(
            value["catalogs"]["alpha"]["packages"]["entries"]["foo"]["source"],
            src_a
        );
        assert_eq!(
            value["catalogs"]["beta"]["packages"]["entries"]["bar"]["source"],
            src_b
        );
    }
}
