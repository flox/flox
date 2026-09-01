use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use flox_core::{Version, WriteError, write_atomically};
use floxhub_client::LockedInputEntry;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use super::tree::PackageTreeNode;
use crate::{CatalogId, CatalogRef};

/// Locked source information for a catalog: a package attribute hierarchy with
/// a locked source per package at its leaves, as returned by the catalog
/// `/build-inputs/lookup` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum CatalogLock {
    #[serde(rename = "floxhub")]
    FloxHub {
        /// Tree structure of locked packages from FloxHub
        packages: PackageTreeNode,
    },
}

/// A `BuildLock` is a collection of locked sources for each catalog.
/// It is used to ensure reproducibility of builds by locking the
/// sources of declared dependencies.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildLock {
    #[serde(rename = "version")]
    pub(crate) _version: Version<1>,
    /// The direct (first-order) catalog inputs, keyed by the server's
    /// canonical `<catalog>/<attr-path>` form.
    ///
    /// Note: `LockedInputEntry` is generated from the catalog OpenAPI spec.
    /// Serializing it verbatim into this on-disk lock couples the persisted
    /// format to the generated schema, so regenerating the client can change
    /// the lock format. A later increment should insulate this with a
    /// hand-owned domain type (cf. the `BaseCatalogInfo` newtype and `From`
    /// impls in `floxhub-client`).
    pub(crate) direct_catalog_inputs: BTreeMap<String, LockedInputEntry>,
    pub(crate) catalogs: BTreeMap<CatalogId, CatalogLock>,
}

/// References a lock was asked to cover but does not contain: the lock is
/// stale relative to the expressions that were scanned.
#[derive(Debug, thiserror::Error)]
#[error(
    "The catalog lock does not cover: {}",
    .missing.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
)]
pub struct StaleLockError {
    /// The scanned references with no entry in the lock, in scan order.
    pub missing: Vec<CatalogRef>,
}

/// Failure reading, parsing, serializing or writing a lock file. Typed so
/// consumers can distinguish an unreadable lock from a corrupt one and offer
/// a real next step.
#[derive(Debug, thiserror::Error)]
pub enum LockfileError {
    #[error("failed to read {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse {path}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize the lock")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to write {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: WriteError,
    },
}

impl BuildLock {
    /// The direct-input entries covering exactly `references`; see
    /// [subset_direct_inputs].
    pub fn subset_direct(
        &self,
        references: &BTreeSet<CatalogRef>,
    ) -> Result<BTreeMap<String, LockedInputEntry>, StaleLockError> {
        subset_direct_inputs(&self.direct_catalog_inputs, references)
    }

    /// The direct (first-order) catalog inputs the lock pins.
    pub fn direct_catalog_inputs(&self) -> &BTreeMap<String, LockedInputEntry> {
        &self.direct_catalog_inputs
    }
}

/// The entries of `direct_catalog_inputs` covering exactly `references`.
///
/// This is the publish-time projection of a lock: the subset a single
/// package's scanned references select, submitted with the build instead of
/// everything the lock resolved — the map of a [BuildLock] the CLI read
/// from disk or resolved itself. Fails with the uncovered references when
/// the inputs are stale.
///
/// References are matched against the entries themselves — an entry covers a
/// reference when its `catalog` matches and its `attr_path` is a prefix of
/// the reference's path under the catalog (a reference may select a member
/// of the package it resolved to; the most specific entry wins). The map's
/// keys are the server's canonical `<catalog>/<attr-path>` form, a namespace
/// distinct from the dot-rendered references, so keys are carried through
/// verbatim and never reconstructed. A wildcard reference selects every
/// entry under its prefix.
pub fn subset_direct_inputs(
    direct_catalog_inputs: &BTreeMap<String, LockedInputEntry>,
    references: &BTreeSet<CatalogRef>,
) -> Result<BTreeMap<String, LockedInputEntry>, StaleLockError> {
    let mut subset = BTreeMap::new();
    let mut missing = Vec::new();
    for reference in references {
        // A reference names `<root>.<catalog>.<path...>`; its invariant
        // guarantees the catalog component is present.
        let names = reference.path().attribute_names();
        let (catalog, path) = (names[1], &names[2..]);
        let wildcard = reference.path().is_wildcard();

        let mut matched: Vec<(&String, &LockedInputEntry)> = direct_catalog_inputs
            .iter()
            .filter(|(_, entry)| {
                entry.catalog == catalog && {
                    let entry_path: Vec<&str> =
                        entry.attr_path.iter().map(String::as_str).collect();
                    path.starts_with(&entry_path[..]) || (wildcard && entry_path.starts_with(path))
                }
            })
            .collect();

        if !wildcard {
            // The reference resolved to exactly one package: the entry whose
            // attr_path names it most specifically.
            matched = matched
                .into_iter()
                .max_by_key(|(_, entry)| entry.attr_path.len())
                .into_iter()
                .collect();
        }

        if matched.is_empty() {
            missing.push(reference.clone());
        } else {
            for (key, entry) in matched {
                subset.insert(key.clone(), entry.clone());
            }
        }
    }
    if !missing.is_empty() {
        return Err(StaleLockError { missing });
    }
    Ok(subset)
}

/// Read a `BuildLock` from the specified file, as written by [write_lock].
#[instrument(fields(path = %path.as_ref().display()))]
pub fn read_lock(path: impl AsRef<Path>) -> Result<BuildLock, LockfileError> {
    let path = path.as_ref();
    let json = fs::read_to_string(path).map_err(|source| LockfileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_str(&json).map_err(|source| LockfileError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Serialize a `BuildLock` to the pretty-printed JSON format consumed by the
/// NEF. Shared by [write_lock] and callers that stream the lock elsewhere
/// (e.g. stdout).
pub fn render_lock(lock: &BuildLock) -> Result<String, LockfileError> {
    serde_json::to_string_pretty(lock).map_err(LockfileError::Serialize)
}

/// Write a `BuildLock` to the specified file.
/// The file is written in a pretty-printed JSON format
/// and consumed by the NEF.
/// The write is atomic — rendered to a temp file in the target's directory
/// and renamed into place — so a crash mid-write can never leave a
/// truncated lock for a later build to trust. The temp file gets a fresh
/// random name on every call (`flox_core::write_atomically`, the same
/// helper `flox-core` itself uses for its own state files) rather than one
/// derived from `path`: concurrent CLI invocations of one project share
/// the committed `.flox/catalog.lock` path and may share a temp directory
/// for their `catalog.lock.`-prefixed ephemeral files, and a temp name
/// derived from the target path would let those writers truncate or
/// overwrite each other's write before either renamed into place.
#[instrument(skip(lock), fields(path = %path.as_ref().display()))]
pub fn write_lock(lock: &BuildLock, path: impl AsRef<Path>) -> Result<(), LockfileError> {
    let path = path.as_ref();
    let json = format!("{}\n", render_lock(lock)?);
    write_atomically(path, &json).map_err(|source| LockfileError::Write {
        path: path.to_path_buf(),
        source,
    })?;
    debug!(bytes = json.len(), "wrote build lock");
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};

    use floxhub_client::{BuildType, LockedGitSource, LockedInputEntry};

    use super::*;
    use crate::lock::transform::build_lock_from_locked_inputs;

    fn entry(catalog: &str, attr_path: &[&str]) -> LockedInputEntry {
        LockedInputEntry {
            attr_path: attr_path.iter().map(|s| s.to_string()).collect(),
            build_type: BuildType::Nef,
            catalog: catalog.to_string(),
            inputs: None,
            locked_inputs_hash: "sha256-test".to_string(),
            source: LockedGitSource {
                dir: ".".to_string(),
                ref_: "refs/heads/main".to_string(),
                rev: "abc".to_string(),
                type_: "git".to_string(),
                url: "https://example.com/repo".to_string(),
            },
        }
    }

    /// A lock whose direct inputs are exactly the given canonical keys
    /// (`<catalog>/<attr-path>`, the server's keying — see
    /// test_data/build_inputs_lookup/success.json), assembled the same way
    /// the lookup response transform assembles a real lock.
    fn lock_with(keys: &[&str]) -> BuildLock {
        let locked: HashMap<String, LockedInputEntry> = keys
            .iter()
            .map(|key| {
                let (catalog, rest) = key.split_once('/').expect("test keys are catalog/attr");
                let attr_path: Vec<&str> = rest.split('.').collect();
                ((*key).to_string(), entry(catalog, &attr_path))
            })
            .collect();
        let direct_keys: Vec<String> = keys.iter().map(|key| (*key).to_string()).collect();
        build_lock_from_locked_inputs(locked, direct_keys.iter()).expect("transform succeeds")
    }

    fn references(refs: &[&str]) -> BTreeSet<CatalogRef> {
        refs.iter().map(|r| CatalogRef::new_unchecked(r)).collect()
    }

    #[test]
    fn subset_direct_selects_only_the_requested_references() {
        let lock = lock_with(&["myorg/hello", "myorg/world", "other/tool"]);

        let subset = lock
            .subset_direct(&references(&["catalogs.myorg.hello"]))
            .expect("references are covered");

        assert_eq!(subset.keys().collect::<Vec<_>>(), vec![
            &"myorg/hello".to_string()
        ]);
        assert_eq!(
            subset["myorg/hello"],
            lock.direct_catalog_inputs["myorg/hello"]
        );
    }

    /// A reference may select a member of the package it resolved to
    /// (`catalogs.myorg.toolkit.readVersion` → entry `myorg/toolkit`); the
    /// most specific entry wins when entries nest.
    #[test]
    fn subset_direct_resolves_a_member_selection_to_its_package() {
        let lock = lock_with(&["myorg/toolkit", "myorg/toolkit.extras"]);

        let subset = lock
            .subset_direct(&references(&["catalogs.myorg.toolkit.readVersion"]))
            .expect("references are covered");
        assert_eq!(subset.keys().collect::<Vec<_>>(), vec![
            &"myorg/toolkit".to_string()
        ]);

        let subset = lock
            .subset_direct(&references(&["catalogs.myorg.toolkit.extras.render"]))
            .expect("references are covered");
        assert_eq!(subset.keys().collect::<Vec<_>>(), vec![
            &"myorg/toolkit.extras".to_string()
        ]);
    }

    #[test]
    fn subset_direct_names_every_uncovered_reference() {
        let lock = lock_with(&["myorg/hello"]);

        let err = lock
            .subset_direct(&references(&[
                "catalogs.myorg.hello",
                "catalogs.myorg.missing",
                "catalogs.other.gone",
            ]))
            .expect_err("uncovered references fail the subset");

        assert_eq!(
            err.missing,
            references(&["catalogs.myorg.missing", "catalogs.other.gone"])
                .into_iter()
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn rendered_lock_reads_back_and_subsets() {
        let lock = lock_with(&["myorg/hello", "other/tool"]);
        let rendered = render_lock(&lock).expect("lock renders");

        let read: BuildLock = serde_json::from_str(&rendered).expect("rendered lock parses");
        let subset = read
            .subset_direct(&references(&["catalogs.other.tool"]))
            .expect("references are covered");

        assert_eq!(
            subset,
            lock.subset_direct(&references(&["catalogs.other.tool"]))
                .unwrap()
        );
    }
}
