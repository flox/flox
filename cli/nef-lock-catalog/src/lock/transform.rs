//! Transform the flat locked-input map returned by the catalog
//! `/build-inputs/lookup` endpoint into a persisted [BuildLock], then
//! materialize the derived hierarchy for the NEF when it is needed.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::{Context, Result};
use floxhub_client::LockedInputEntry;
use tracing::instrument;

use crate::CatalogId;
use crate::lock::build_lock::{BuildLock, CatalogLock, LockedInput};
use crate::lock::tree::PackageTreeBuilder;

/// Build the on-disk lock from resolved entries and the lookup's direct keys.
/// A direct key without a resolved entry makes the response invalid; reject
/// it before it can become a committed lock.
#[instrument(skip(locked, direct_keys), fields(packages = locked.len()))]
pub fn build_lock_from_locked_inputs<'d>(
    locked: HashMap<String, LockedInputEntry>,
    direct_keys: impl IntoIterator<Item = &'d String>,
) -> Result<BuildLock> {
    let mut locked_inputs: BTreeMap<String, LockedInput> = BTreeMap::new();

    for (key, entry) in locked {
        locked_inputs.insert(key, LockedInput::from(entry));
    }

    let direct_inputs: BTreeSet<String> = direct_keys
        .into_iter()
        .map(|key| {
            locked_inputs
                .contains_key(key)
                .then(|| key.clone())
                .with_context(|| format!("Direct dependency '{key}' does not appear to be locked"))
        })
        .collect::<Result<BTreeSet<String>>>()?;

    Ok(BuildLock {
        locked_inputs,
        direct_inputs,
        ..Default::default()
    })
}

/// Derive the NEF package trees from the persisted, server-provided inputs.
/// The same tree builder used by the original combined transform is retained.
pub fn materialize_catalogs(lock: &BuildLock) -> Result<serde_json::Value> {
    materialize_entries(lock.locked_inputs.iter())
}

fn materialize_entries<'a>(
    entries: impl IntoIterator<Item = (&'a String, &'a LockedInput)>,
) -> Result<serde_json::Value> {
    let mut builders: BTreeMap<CatalogId, PackageTreeBuilder> = BTreeMap::new();
    for (key, entry) in entries {
        builders
            .entry(CatalogId(entry.catalog.clone()))
            .or_insert_with(PackageTreeBuilder::new)
            .add_package_source(
                entry.attr_path.clone(),
                entry.build_type.into(),
                (&entry.source).into(),
                Vec::new(),
            )
            .with_context(|| {
                format!(
                    "Could not materialize catalog input '{key}' in catalog '{}'",
                    entry.catalog
                )
            })?;
    }
    let catalogs: BTreeMap<_, _> = builders
        .into_iter()
        .map(|(id, builder)| {
            (id, CatalogLock::FloxHub {
                packages: builder.into_root(),
            })
        })
        .collect();
    Ok(serde_json::to_value(catalogs)?)
}

/// Render the temporary builder-facing lock, including the derived catalogs.
pub fn render_builder_lock(lock: &BuildLock) -> Result<String> {
    let mut value = serde_json::to_value(lock)?;
    value["catalogs"] = materialize_catalogs(lock)?;
    Ok(format!("{}\n", serde_json::to_string_pretty(&value)?))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

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
            extra: BTreeMap::new(),
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
            inputs: Some(vec![]),
            locked_inputs_hash: "sha256-test".to_string(),
            deep_overrides: None,
            version: None,
            build: None,
            source,
        }
    }

    #[test]
    fn single_package_single_catalog() {
        let source = git_source("https://example.com/repo", "abc");
        let expected_source = serde_json::to_value(&source).unwrap();
        let mut input = entry("myorg", &["hello"], BuildType::Nef, source);
        input.version = Some("1.2.3".to_string());
        input.build = Some("build-42".to_string());
        let locked = HashMap::from([("myorg/hello".to_string(), input)]);

        let lock = build_lock_from_locked_inputs(locked, [&"myorg/hello".to_string()])
            .expect("transform succeeds");

        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&render_builder_lock(&lock).unwrap())
                .unwrap(),
            json!({
                "version": 2,
                "locked_inputs": {
                    "myorg/hello": {
                        "attr_path": ["hello"],
                        "build_type": "nef",
                        "catalog": "myorg",
                        "inputs": [],
                        "locked_inputs_hash": "sha256-test",
                        "version": "1.2.3",
                        "build": "build-42",
                        "source": expected_source.clone(),
                    }
                },
                "direct_inputs": ["myorg/hello"],
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

        let closure = lock
            .project_package(&BTreeSet::from([crate::CatalogRef::new_unchecked(
                "catalogs.myorg.hello",
            )]))
            .unwrap();
        let wire_entry = serde_json::to_value(&closure.locked_inputs["myorg/hello"]).unwrap();
        assert_eq!(wire_entry["version"], json!("1.2.3"));
        assert_eq!(wire_entry["build"], json!("build-42"));
    }

    #[test]
    fn nested_attr_path_builds_package_set() {
        let source = git_source("https://example.com/repo", "abc");
        let expected_source = serde_json::to_value(&source).unwrap();
        let locked = HashMap::from([(
            "myorg/python3Packages.boolex".to_string(),
            entry(
                "myorg",
                &["python3Packages", "boolex"],
                BuildType::Manifest,
                source,
            ),
        )]);

        let lock =
            build_lock_from_locked_inputs(locked, [&"myorg/python3Packages.boolex".to_string()])
                .expect("transform succeeds");

        let value: serde_json::Value =
            serde_json::from_str(&render_builder_lock(&lock).unwrap()).unwrap();
        assert_eq!(
            value["catalogs"]["myorg"]["packages"],
            json!({
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
                "alpha/foo".to_string(),
                entry("alpha", &["foo"], BuildType::Nef, src_a),
            ),
            (
                "beta/bar".to_string(),
                entry("beta", &["bar"], BuildType::Nef, src_b),
            ),
        ]);

        let value: serde_json::Value = serde_json::from_str(
            &render_builder_lock(
                &build_lock_from_locked_inputs(locked, [&"alpha/foo".to_string()])
                    .expect("transform succeeds"),
            )
            .unwrap(),
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

    /// A populated `deep_overrides` on the wire entry reaches the built
    /// tree's package node, rather than being dropped in translation.
    #[test]
    fn deep_overrides_carried_into_package_tree() {
        let source = git_source("https://example.com/repo", "abc");
        let mut wire_entry = entry("myorg", &["hello"], BuildType::Nef, source);
        wire_entry.deep_overrides = Some(vec![vec!["openssl".try_into().unwrap()]]);
        let locked = HashMap::from([("myorg.hello".to_string(), wire_entry)]);

        let lock = build_lock_from_locked_inputs(locked, [&"myorg.hello".to_string()])
            .expect("transform succeeds");
        let value: serde_json::Value =
            serde_json::from_str(&render_builder_lock(&lock).unwrap()).unwrap();

        assert_eq!(
            value["catalogs"]["myorg"]["packages"]["entries"]["hello"]["deep_overrides"],
            json!([["openssl"]])
        );
    }

    /// A nested attribute path on the wire entry (a package-set member)
    /// reaches the tree as a multi-component list, not a dotted string.
    #[test]
    fn nested_deep_override_path_carried_into_package_tree() {
        let source = git_source("https://example.com/repo", "abc");
        let mut wire_entry = entry("myorg", &["hello"], BuildType::Nef, source);
        wire_entry.deep_overrides = Some(vec![vec![
            "setMakeScope".try_into().unwrap(),
            "makeScopeDependency".try_into().unwrap(),
        ]]);
        let locked = HashMap::from([("myorg.hello".to_string(), wire_entry)]);

        let lock = build_lock_from_locked_inputs(locked, [&"myorg.hello".to_string()])
            .expect("transform succeeds");
        let value: serde_json::Value =
            serde_json::from_str(&render_builder_lock(&lock).unwrap()).unwrap();

        assert_eq!(
            value["catalogs"]["myorg"]["packages"]["entries"]["hello"]["deep_overrides"],
            json!([["setMakeScope", "makeScopeDependency"]])
        );
    }
}
