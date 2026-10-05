use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use flox_core::{Version, WriteError, write_atomically};
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use crate::CatalogRef;
use crate::project::UPDATE_CATALOGS_COMMAND;

// Must match the configured base catalog's name on the catalog server and the
// pinned instance exposed as `catalogs.nixpkgs` by the NEF builder.
pub(crate) const BASE_CATALOG_NAME: &str = "nixpkgs";

/// Locked source information for a catalog: a package attribute hierarchy with
/// a locked source per package at its leaves, as returned by the catalog
/// `/build-inputs/lookup` endpoint.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub(crate) enum CatalogLock {
    #[serde(rename = "floxhub")]
    FloxHub {
        /// Tree structure of locked packages from FloxHub
        packages: super::tree::PackageTreeNode,
    },
}

/// A locked git source, as persisted in the on-disk lock.
///
/// Separate from the API type so regenerating the client cannot change the
/// persisted format. The API boundary preserves unknown git attributes
/// before deserialization would discard them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GitSource {
    pub dir: String,
    #[serde(rename = "ref")]
    pub ref_: String,
    pub rev: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub url: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl From<floxhub_client::LockedGitSource> for GitSource {
    fn from(value: floxhub_client::LockedGitSource) -> Self {
        Self {
            dir: value.dir,
            ref_: value.ref_,
            rev: value.rev,
            type_: value.type_,
            url: value.url,
            extra: value.extra,
        }
    }
}

impl From<&GitSource> for floxhub_client::LockedGitSource {
    fn from(value: &GitSource) -> Self {
        Self {
            dir: value.dir.clone(),
            ref_: value.ref_.clone(),
            rev: value.rev.clone(),
            type_: value.type_.clone(),
            url: value.url.clone(),
            extra: value.extra.clone(),
        }
    }
}

/// Persist a catalog input independently of the generated API client.
/// Regenerating the client must not change the on-disk lock format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildType {
    Manifest,
    Nef,
}

impl From<floxhub_client::BuildType> for BuildType {
    fn from(value: floxhub_client::BuildType) -> Self {
        match value {
            floxhub_client::BuildType::Manifest => Self::Manifest,
            floxhub_client::BuildType::Nef => Self::Nef,
        }
    }
}

impl From<BuildType> for floxhub_client::BuildType {
    fn from(value: BuildType) -> Self {
        match value {
            BuildType::Manifest => Self::Manifest,
            BuildType::Nef => Self::Nef,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockedInput {
    pub attr_path: Vec<String>,
    pub build_type: BuildType,
    pub catalog: String,
    /// `None` means the server left dependencies unstated, not that this is
    /// a leaf. Preserve it and refuse only a projection that reaches it.
    pub inputs: Option<Vec<String>>,
    pub locked_inputs_hash: String,
    /// Informational fields supplied by lookup. Keep nulls on disk and
    /// carry values through to check and publish without using them locally.
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub build: Option<String>,
    pub source: GitSource,
}

impl From<floxhub_client::LockedInputEntry> for LockedInput {
    fn from(entry: floxhub_client::LockedInputEntry) -> Self {
        Self {
            attr_path: entry.attr_path,
            build_type: entry.build_type.into(),
            catalog: entry.catalog,
            inputs: entry.inputs,
            locked_inputs_hash: entry.locked_inputs_hash,
            version: entry.version,
            build: entry.build,
            source: entry.source.into(),
        }
    }
}

impl From<&LockedInput> for floxhub_client::LockedInputEntry {
    fn from(value: &LockedInput) -> Self {
        Self {
            attr_path: value.attr_path.clone(),
            build_type: value.build_type.into(),
            catalog: value.catalog.clone(),
            inputs: value.inputs.clone(),
            locked_inputs_hash: value.locked_inputs_hash.clone(),
            version: value.version.clone(),
            build: value.build.clone(),
            source: (&value.source).into(),
            deep_overrides: None,
        }
    }
}

/// Lock format v2 stores the full catalog lookup alongside project-wide
/// direct input keys. Package publication selects roots from those keys and
/// follows dependencies through the full map.
///
/// Field order is part of the required empty-lock representation:
/// `{"version":2,"locked_inputs":{},"direct_inputs":[]}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildLock {
    pub(crate) version: Version<2>,
    pub(crate) locked_inputs: BTreeMap<String, LockedInput>,
    pub(crate) direct_inputs: BTreeSet<String>,
}

impl BuildLock {
    pub fn locked_inputs(&self) -> &BTreeMap<String, LockedInput> {
        &self.locked_inputs
    }

    pub fn direct_inputs(&self) -> &BTreeSet<String> {
        &self.direct_inputs
    }
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
    #[error(
        "the catalog lock at '{path}' is in an old format that this version \
         of Flox can no longer read.\n\nRun '{UPDATE_CATALOGS_COMMAND}', \
         then commit the new '.flox/catalog.lock' and retry.\n\nIf a relock \
         is not possible right now, remove '{path}' from the working tree \
         to build without a committed lock. The build will resolve catalog \
         inputs again.",
        path = path.display(),
    )]
    LegacyVersion { path: PathBuf },
    #[error(
        "the catalog lock at '{path}' was written by a newer version of \
         Flox (lock version {found}); upgrade Flox and retry.",
        path = path.display(),
    )]
    UnsupportedVersion { path: PathBuf, found: u64 },
    /// Valid JSON without a format version.
    #[error(
        "the catalog lock at '{path}' has no 'version' field. Run '{UPDATE_CATALOGS_COMMAND}' to relock."
    )]
    MissingVersion { path: PathBuf },
    #[error(
        "the catalog lock at '{path}' has an invalid version value: {found}. Run '{UPDATE_CATALOGS_COMMAND}' to relock."
    )]
    InvalidVersion {
        path: PathBuf,
        found: serde_json::Value,
    },
}

/// Only the top-level `version` controls lock format dispatch. Entry versions
/// and the lookup envelope's wire version have separate meanings.
const LOCK_FORMAT_VERSION_KEY: &str = "version";

/// Failure projecting a package's roots and closure out of a [BuildLock].
#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    #[error(transparent)]
    Stale(#[from] StaleLockError),
    /// A direct key is missing from the full map. Projection validates even
    /// hand-edited or foreign locks that bypassed lookup conversion.
    #[error("direct input '{key}' does not appear in the lock's locked_inputs")]
    MissingRoot { key: String },
    /// An entry's `inputs` names a child key absent from `locked_inputs`.
    #[error("'{parent}' names dependency '{key}', which does not appear in the lock")]
    MissingChild { parent: String, key: String },
    /// A selected or reachable entry's `inputs` is `null` — the lookup
    /// response left its children unstated, which a resolved entry should
    /// never do (see [LockedInput::inputs]).
    #[error("'{key}' does not state its dependencies and cannot be published")]
    UnstatedInputs { key: String },
    /// A dependency cycle reachable from a selected root.
    #[error("dependency cycle: {}", .path.join(" -> "))]
    Cycle { path: Vec<String> },
}

/// The selected roots and reachable inputs for one published package.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PackageClosure {
    /// Empty for a leaf or manifest build with no catalog roots.
    pub direct_inputs: Vec<String>,
    /// Reachable entries, keyed canonically; shared children appear once.
    pub locked_inputs: BTreeMap<String, floxhub_client::LockedInputEntry>,
}

impl BuildLock {
    /// Project a package's direct roots and dependency closure for publication.
    /// A non-wildcard reference selects its most specific direct input;
    /// a wildcard selects every direct input under its prefix.
    /// References in the base catalog select no roots because the builder
    /// supplies them from its pinned nixpkgs instance.
    ///
    /// Walk only entries reachable from the selected roots. A lock can hold
    /// unrelated packages, and build/develop should not validate their graphs.
    /// The explicit stack tolerates graphs near the server entry cap and
    /// detects cycles instead of treating them as completed dependencies.
    pub fn project_package(
        &self,
        references: &BTreeSet<CatalogRef>,
    ) -> Result<PackageClosure, ProjectionError> {
        // Validate every direct key, including unselected ones, so a foreign
        // or hand-edited lock cannot silently discard a dangling root.
        let mut candidates: BTreeMap<&String, &LockedInput> = BTreeMap::new();
        for key in &self.direct_inputs {
            let entry = self
                .locked_inputs
                .get(key)
                .ok_or_else(|| ProjectionError::MissingRoot { key: key.clone() })?;
            candidates.insert(key, entry);
        }
        let roots = select_roots(&candidates, references)?;

        let mut closure: BTreeMap<String, floxhub_client::LockedInputEntry> = BTreeMap::new();
        let mut done: BTreeSet<String> = BTreeSet::new();

        for root in &roots {
            if done.contains(root) {
                continue;
            }

            let root_entry = self
                .locked_inputs
                .get(root)
                .expect("selected roots were already validated against locked_inputs");
            let root_children = root_entry
                .inputs
                .clone()
                .ok_or_else(|| ProjectionError::UnstatedInputs { key: root.clone() })?;

            let mut on_stack: BTreeSet<String> = BTreeSet::from([root.clone()]);
            let mut stack: Vec<(String, std::vec::IntoIter<String>)> =
                vec![(root.clone(), root_children.into_iter())];

            while let Some((key, mut children)) = stack.pop() {
                match children.next() {
                    Some(child) => {
                        stack.push((key.clone(), children));

                        if done.contains(&child) {
                            continue;
                        }
                        if on_stack.contains(&child) {
                            let mut path: Vec<String> =
                                stack.iter().map(|(k, _)| k.clone()).collect();
                            path.push(child);
                            return Err(ProjectionError::Cycle { path });
                        }

                        let child_entry = self.locked_inputs.get(&child).ok_or_else(|| {
                            ProjectionError::MissingChild {
                                parent: key.clone(),
                                key: child.clone(),
                            }
                        })?;
                        let grandchildren = child_entry.inputs.clone().ok_or_else(|| {
                            ProjectionError::UnstatedInputs { key: child.clone() }
                        })?;

                        on_stack.insert(child.clone());
                        stack.push((child, grandchildren.into_iter()));
                    },
                    None => {
                        on_stack.remove(&key);
                        done.insert(key.clone());
                        let entry = self
                            .locked_inputs
                            .get(&key)
                            .expect("looked up when the frame was pushed");
                        closure.insert(key, entry.into());
                    },
                }
            }
        }

        Ok(PackageClosure {
            direct_inputs: roots.into_iter().collect(),
            locked_inputs: closure,
        })
    }
}

/// Select the direct keys covering each reference, or report uncovered ones.
///
/// References are matched against the entries themselves — an entry covers a
/// reference when its `catalog` matches and its `attr_path` is a prefix of
/// the reference's path under the catalog (a reference may select a member
/// of the package it resolved to; the most specific entry wins). The map's
/// keys are the server's canonical `<catalog>/<attr-path>` form, a namespace
/// distinct from the dot-rendered references, so keys are carried through
/// verbatim and never reconstructed. A wildcard reference selects every
/// entry under its prefix.
fn select_roots(
    candidates: &BTreeMap<&String, &LockedInput>,
    references: &BTreeSet<CatalogRef>,
) -> Result<BTreeSet<String>, StaleLockError> {
    let mut selected = BTreeSet::new();
    let mut missing = Vec::new();
    for reference in references {
        // A reference names `<root>.<catalog>.<path...>`; its invariant
        // guarantees the catalog component is present.
        let names = reference.path().attribute_names();
        let (catalog, path) = (names[1], &names[2..]);
        // Base references are evaluated from pinned nixpkgs; they have no
        // lock root even when a wildcard names many attributes.
        if catalog == BASE_CATALOG_NAME {
            continue;
        }
        let wildcard = reference.path().is_wildcard();

        let mut matched: Vec<&String> = candidates
            .iter()
            .filter(|(_, entry)| {
                entry.catalog == catalog && {
                    let entry_path: Vec<&str> =
                        entry.attr_path.iter().map(String::as_str).collect();
                    path.starts_with(&entry_path[..]) || (wildcard && entry_path.starts_with(path))
                }
            })
            .map(|(key, _)| *key)
            .collect();

        if !wildcard {
            // The reference resolved to exactly one package: the entry whose
            // attr_path names it most specifically.
            matched = matched
                .into_iter()
                .max_by_key(|key| candidates[key].attr_path.len())
                .into_iter()
                .collect();
        }

        if matched.is_empty() {
            missing.push(reference.clone());
        } else {
            selected.extend(matched.into_iter().cloned());
        }
    }
    if !missing.is_empty() {
        return Err(StaleLockError { missing });
    }
    Ok(selected)
}

/// Read a `BuildLock` from the specified file, as written by [write_lock].
///
/// Distinguish legacy, unknown, and corrupt locks so each gets an actionable
/// error. Parsing through [serde_json::Value] loses payload line and column.
#[instrument(fields(path = %path.as_ref().display()))]
pub fn read_lock(path: impl AsRef<Path>) -> Result<BuildLock, LockfileError> {
    let path = path.as_ref();
    let json = fs::read_to_string(path).map_err(|source| LockfileError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value: serde_json::Value =
        serde_json::from_str(&json).map_err(|source| LockfileError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
    match value.get(LOCK_FORMAT_VERSION_KEY) {
        Some(found) if found.as_u64() == Some(2) => {
            serde_json::from_value(value).map_err(|source| LockfileError::Parse {
                path: path.to_path_buf(),
                source,
            })
        },
        Some(found) if found.as_u64() == Some(1) => Err(LockfileError::LegacyVersion {
            path: path.to_path_buf(),
        }),
        Some(found) if found.as_u64() == Some(0) => Err(LockfileError::InvalidVersion {
            path: path.to_path_buf(),
            found: found.clone(),
        }),
        Some(found) if found.as_u64().is_some() => Err(LockfileError::UnsupportedVersion {
            path: path.to_path_buf(),
            found: found.as_u64().unwrap(),
        }),
        Some(found) => Err(LockfileError::InvalidVersion {
            path: path.to_path_buf(),
            found: found.clone(),
        }),
        None => Err(LockfileError::MissingVersion {
            path: path.to_path_buf(),
        }),
    }
}

/// Serialize the persisted project lock, also used by stdout callers.
pub fn render_lock(lock: &BuildLock) -> Result<String, LockfileError> {
    serde_json::to_string_pretty(lock).map_err(LockfileError::Serialize)
}

/// Write a `BuildLock` to the specified file.
/// The file is written in a pretty-printed JSON format
/// and consumed by the CLI. The NEF receives a separate materialized file.
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
    use std::collections::HashMap;

    use floxhub_client::{BuildType, LockedGitSource, LockedInputEntry};

    use super::*;
    use crate::lock::transform::build_lock_from_locked_inputs;

    fn entry(catalog: &str, attr_path: &[&str]) -> LockedInputEntry {
        LockedInputEntry {
            attr_path: attr_path.iter().map(|s| s.to_string()).collect(),
            build_type: BuildType::Nef,
            catalog: catalog.to_string(),
            inputs: Some(vec![]),
            locked_inputs_hash: "sha256-test".to_string(),
            version: None,
            build: None,
            source: LockedGitSource {
                dir: ".".to_string(),
                ref_: "refs/heads/main".to_string(),
                rev: "abc".to_string(),
                type_: "git".to_string(),
                url: "https://example.com/repo".to_string(),
                extra: BTreeMap::new(),
            },
            deep_overrides: None,
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
    fn project_package_selects_only_the_requested_references() {
        let lock = lock_with(&["myorg/hello", "myorg/world", "other/tool"]);

        let closure = lock
            .project_package(&references(&["catalogs.myorg.hello"]))
            .expect("references are covered");

        assert_eq!(closure.direct_inputs, vec!["myorg/hello".to_string()]);
        assert_eq!(
            closure.locked_inputs["myorg/hello"],
            floxhub_client::LockedInputEntry::from(&lock.locked_inputs["myorg/hello"])
        );
    }

    /// A reference may select a member of the package it resolved to
    /// (`catalogs.myorg.toolkit.readVersion` → entry `myorg/toolkit`); the
    /// most specific entry wins when entries nest.
    #[test]
    fn project_package_resolves_a_member_selection_to_its_package() {
        let lock = lock_with(&["myorg/toolkit", "myorg/toolkit.extras"]);

        let closure = lock
            .project_package(&references(&["catalogs.myorg.toolkit.readVersion"]))
            .expect("references are covered");
        assert_eq!(closure.direct_inputs, vec!["myorg/toolkit".to_string()]);

        let closure = lock
            .project_package(&references(&["catalogs.myorg.toolkit.extras.render"]))
            .expect("references are covered");
        assert_eq!(closure.direct_inputs, vec![
            "myorg/toolkit.extras".to_string()
        ]);
    }

    #[test]
    fn project_package_names_every_uncovered_reference() {
        let lock = lock_with(&["myorg/hello"]);

        let err = lock
            .project_package(&references(&[
                "catalogs.myorg.hello",
                "catalogs.myorg.missing",
                "catalogs.other.gone",
            ]))
            .expect_err("uncovered references fail the projection");

        match err {
            ProjectionError::Stale(StaleLockError { missing }) => {
                assert_eq!(
                    missing,
                    references(&["catalogs.myorg.missing", "catalogs.other.gone"])
                        .into_iter()
                        .collect::<Vec<_>>()
                );
            },
            other => panic!("expected ProjectionError::Stale, got {other:?}"),
        }
    }

    #[test]
    fn rendered_lock_reads_back_and_projects() {
        let lock = lock_with(&["myorg/hello", "other/tool"]);
        let rendered = render_lock(&lock).expect("lock renders");

        let read: BuildLock = serde_json::from_str(&rendered).expect("rendered lock parses");
        let closure = read
            .project_package(&references(&["catalogs.other.tool"]))
            .expect("references are covered");

        assert_eq!(
            closure,
            lock.project_package(&references(&["catalogs.other.tool"]))
                .unwrap()
        );
    }
}
