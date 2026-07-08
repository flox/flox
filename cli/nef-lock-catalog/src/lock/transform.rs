//! Transform the flat locked-input map returned by the catalog
//! `/build-inputs/lookup` endpoint into the hierarchical [BuildLock] consumed
//! by the NEF.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use floxhub_client::LockedInputEntry;
use tracing::instrument;

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
#[instrument(skip(locked, direct_keys), fields(packages = locked.len()))]
pub(crate) fn build_lock_from_locked_inputs<'d>(
    locked: HashMap<String, LockedInputEntry>,
    direct_keys: impl IntoIterator<Item = &'d String>,
) -> Result<BuildLock> {
    let mut builders: BTreeMap<CatalogId, PackageTreeBuilder> = BTreeMap::new();

    let direct_locks = direct_keys
        .into_iter()
        .map(|key| {
            let entry = locked.get(key).cloned().with_context(|| {
                format!("Direct dependency '{key}' does not appear to be locked")
            })?;
            Ok((key.clone(), entry))
        })
        .collect::<Result<HashMap<String, LockedInputEntry>>>()?;

    for entry in locked.into_values() {
        let LockedInputEntry {
            attr_path,
            build_type,
            catalog,
            inputs: _,
            locked_inputs_hash: _,
            source,
        } = entry;

        builders
            .entry(CatalogId(catalog))
            .or_insert_with(PackageTreeBuilder::new)
            .add_package_source(attr_path, build_type, source.into())?;
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
        direct_catalog_inputs: direct_locks,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use floxhub_client::{BuildType, LockedGitSource};
    use serde_json::json;

    use super::*;

    /// A locked git source. Locked inputs are only ever tracked as git
    /// flakerefs, so the typed wire model carries exactly these fields.
    fn git_source(url: &str, rev: &str) -> LockedGitSource {
        LockedGitSource {
            dir: ".".to_string(),
            ref_: "refs/heads/main".to_string(),
            rev: rev.to_string(),
            type_: "git".to_string(),
            url: url.to_string(),
        }
    }

    fn entry(
        catalog: &str,
        attr_path: &[&str],
        build_type: BuildType,
        source: LockedGitSource,
    ) -> LockedInputEntry {
        LockedInputEntry {
            attr_path: attr_path.iter().map(|s| s.to_string()).collect(),
            build_type,
            catalog: catalog.to_string(),
            inputs: None,
            locked_inputs_hash: "sha256-test".to_string(),
            source,
        }
    }

    #[test]
    fn single_package_single_catalog() {
        let source = git_source("https://example.com/repo", "abc");
        let expected_source = serde_json::to_value(&source).unwrap();
        let locked = HashMap::from([(
            "myorg.hello".to_string(),
            entry("myorg", &["hello"], BuildType::Nef, source),
        )]);

        let lock = build_lock_from_locked_inputs(locked, [&"myorg.hello".to_string()])
            .expect("transform succeeds");

        assert_eq!(
            serde_json::to_value(&lock).unwrap(),
            json!({
                "version": 1,
                "direct_catalog_inputs": {
                    "myorg.hello": {
                        "attr_path": ["hello"],
                        "build_type": "nef",
                        "catalog": "myorg",
                        "locked_inputs_hash": "sha256-test",
                        "source": expected_source.clone(),
                    }
                },
                "catalogs": {
                    "myorg": {
                        "type": "floxhub",
                        "packages": {
                            "type": "package_set",
                            "entries": {
                                "hello": {
                                    "type": "package",
                                    "build_type": "nef",
                                    "source": expected_source,
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
        let source = git_source("https://example.com/repo", "abc");
        let expected_source = serde_json::to_value(&source).unwrap();
        let locked = HashMap::from([(
            "myorg.python3Packages.boolex".to_string(),
            entry(
                "myorg",
                &["python3Packages", "boolex"],
                BuildType::Manifest,
                source,
            ),
        )]);

        let lock =
            build_lock_from_locked_inputs(locked, [&"myorg.python3Packages.boolex".to_string()])
                .expect("transform succeeds");

        assert_eq!(
            serde_json::to_value(&lock).unwrap(),
            json!({
                "version": 1,
                "direct_catalog_inputs": {
                    "myorg.python3Packages.boolex": {
                        "attr_path": ["python3Packages", "boolex"],
                        "build_type": "manifest",
                        "catalog": "myorg",
                        "locked_inputs_hash": "sha256-test",
                        "source": expected_source.clone(),
                    }
                },
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
                                            "source": expected_source,
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
        let src_a = git_source("https://a.example/x", "deadbeef");
        let src_b = git_source("https://b.example/y", "cafebabe");
        let expected_a = serde_json::to_value(&src_a).unwrap();
        let expected_b = serde_json::to_value(&src_b).unwrap();
        let locked = HashMap::from([
            (
                "a.foo".to_string(),
                entry("alpha", &["foo"], BuildType::Nef, src_a),
            ),
            (
                "b.bar".to_string(),
                entry("beta", &["bar"], BuildType::Nef, src_b),
            ),
        ]);

        let value = serde_json::to_value(
            build_lock_from_locked_inputs(locked, [&"a.foo".to_string()])
                .expect("transform succeeds"),
        )
        .unwrap();

        // Each catalog gets its own tree, and the locked source is stored
        // verbatim (no nix normalization).
        assert_eq!(
            value["catalogs"]["alpha"]["packages"]["entries"]["foo"]["source"],
            expected_a
        );
        assert_eq!(
            value["catalogs"]["beta"]["packages"]["entries"]["bar"]["source"],
            expected_b
        );
    }
}
