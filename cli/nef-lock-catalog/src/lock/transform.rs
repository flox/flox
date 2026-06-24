//! Transform the flat locked-input map returned by the catalog
//! `/build-inputs/lookup` endpoint into the hierarchical [BuildLock] consumed
//! by the NEF.

use std::collections::{BTreeMap, HashMap};

use anyhow::Result;
use floxhub_client::LockedInputEntry;

use crate::CatalogId;
use crate::lock::build_lock::{BuildLock, CatalogLock};
use crate::lock::flakeref::RawNixFlakerefAttrs;
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
            // The closure-identity hash is the publish round-trip
            // disambiguator; the lock NEF consumes only needs the source.
            locked_inputs_hash: _,
            source,
        } = entry;

        builders
            .entry(CatalogId(catalog))
            .or_insert_with(PackageTreeBuilder::new)
            .add_package_source(
                attr_path,
                build_type,
                RawNixFlakerefAttrs::new_unchecked(serde_json::to_value(source)?),
            )?;
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
            build_lock_from_locked_inputs(locked).expect("transform succeeds"),
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
