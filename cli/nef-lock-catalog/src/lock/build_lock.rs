use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use flox_core::{Version, WriteError, write_atomically};
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

use super::tree::PackageTreeNode;
use crate::project::UPDATE_CATALOGS_COMMAND;
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockedInput {
    pub attr_path: Vec<String>,
    pub build_type: floxhub_client::BuildType,
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
            build_type: entry.build_type,
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
            build_type: value.build_type,
            catalog: value.catalog.clone(),
            inputs: value.inputs.clone(),
            locked_inputs_hash: value.locked_inputs_hash.clone(),
            version: value.version.clone(),
            build: value.build.clone(),
            source: (&value.source).into(),
        }
    }
}

/// Lock format v2 stores the full catalog lookup alongside project-wide
/// direct input keys. Package publication selects roots from those keys and
/// follows dependencies through the full map.
///
/// Field order is part of the required empty-lock representation:
/// `{"version":2,"locked_inputs":{},"direct_inputs":[],"catalogs":{}}`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildLock {
    pub version: Version<2>,
    pub locked_inputs: BTreeMap<String, LockedInput>,
    pub direct_inputs: BTreeSet<String>,
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
    /// Valid JSON lacking both a format version and the recognizable v1 shape.
    #[error("the catalog lock at '{path}' has no 'version' field.")]
    MissingVersion { path: PathBuf },
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
    match value
        .get(LOCK_FORMAT_VERSION_KEY)
        .and_then(serde_json::Value::as_u64)
    {
        Some(2) => serde_json::from_value(value).map_err(|source| LockfileError::Parse {
            path: path.to_path_buf(),
            source,
        }),
        Some(1) => Err(LockfileError::LegacyVersion {
            path: path.to_path_buf(),
        }),
        Some(found) => Err(LockfileError::UnsupportedVersion {
            path: path.to_path_buf(),
            found,
        }),
        None => Err(LockfileError::MissingVersion {
            path: path.to_path_buf(),
        }),
    }
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
    fn project_package_includes_a_shared_diamond_child_once() {
        let mut locked = HashMap::new();
        locked.insert("myorg/app".to_string(), {
            let mut e = entry("myorg", &["app"]);
            e.inputs = Some(vec!["myorg/left".to_string(), "myorg/right".to_string()]);
            e
        });
        locked.insert("myorg/left".to_string(), {
            let mut e = entry("myorg", &["left"]);
            e.inputs = Some(vec!["myorg/shared".to_string()]);
            e
        });
        locked.insert("myorg/right".to_string(), {
            let mut e = entry("myorg", &["right"]);
            e.inputs = Some(vec!["myorg/shared".to_string()]);
            e
        });
        locked.insert("myorg/shared".to_string(), entry("myorg", &["shared"]));

        let lock = build_lock_from_locked_inputs(locked, [&"myorg/app".to_string()])
            .expect("transform succeeds");

        let closure = lock
            .project_package(&references(&["catalogs.myorg.app"]))
            .expect("references are covered");

        assert_eq!(closure.direct_inputs, vec!["myorg/app".to_string()]);
        assert_eq!(closure.locked_inputs.len(), 4);
        assert!(closure.locked_inputs.contains_key("myorg/shared"));
    }

    #[test]
    fn project_package_reports_a_cycle() {
        let mut locked = HashMap::new();
        locked.insert("myorg/a".to_string(), {
            let mut e = entry("myorg", &["a"]);
            e.inputs = Some(vec!["myorg/b".to_string()]);
            e
        });
        locked.insert("myorg/b".to_string(), {
            let mut e = entry("myorg", &["b"]);
            e.inputs = Some(vec!["myorg/a".to_string()]);
            e
        });

        let lock = build_lock_from_locked_inputs(locked, [&"myorg/a".to_string()])
            .expect("transform succeeds");

        let err = lock
            .project_package(&references(&["catalogs.myorg.a"]))
            .expect_err("a cycle is refused");
        assert!(matches!(err, ProjectionError::Cycle { .. }), "{err:?}");
    }

    #[test]
    fn project_package_reports_unstated_inputs_only_when_reached() {
        let mut locked = HashMap::new();
        let mut unstated = entry("myorg", &["hello"]);
        unstated.inputs = None;
        locked.insert("myorg/hello".to_string(), unstated);
        locked.insert("myorg/world".to_string(), entry("myorg", &["world"]));

        let lock = build_lock_from_locked_inputs(locked, [
            &"myorg/hello".to_string(),
            &"myorg/world".to_string(),
        ])
        .expect("transform succeeds: the anomaly is only refused when walked");

        let closure = lock
            .project_package(&references(&["catalogs.myorg.world"]))
            .expect("unrelated entries are unaffected");
        assert_eq!(closure.direct_inputs, vec!["myorg/world".to_string()]);

        let err = lock
            .project_package(&references(&["catalogs.myorg.hello"]))
            .expect_err("unstated inputs are refused once selected");
        assert!(
            matches!(err, ProjectionError::UnstatedInputs { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn project_package_reports_a_missing_root() {
        let mut lock = lock_with(&["myorg/hello"]);
        lock.direct_inputs.insert("myorg/dangling".to_string());

        let err = lock
            .project_package(&references(&["catalogs.myorg.hello"]))
            .expect_err("an unselected dangling direct input is refused");
        assert!(
            matches!(err, ProjectionError::MissingRoot { .. }),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            "direct input 'myorg/dangling' does not appear in the lock's locked_inputs"
        );
    }

    #[test]
    fn leaf_and_manifest_publish_send_empty_closures() {
        let lock = lock_with(&["myorg/hello"]);

        let closure = lock
            .project_package(&BTreeSet::new())
            .expect("no references selects nothing");

        assert!(closure.direct_inputs.is_empty());
        assert!(closure.locked_inputs.is_empty());
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

    #[test]
    fn empty_lock_serializes_to_the_required_bytes() {
        assert_eq!(
            serde_json::to_value(BuildLock::default()).unwrap(),
            serde_json::json!({
                "version": 2,
                "locked_inputs": {},
                "direct_inputs": [],
                "catalogs": {},
            })
        );
    }

    #[test]
    fn source_extras_survive_into_the_projected_closure() {
        let mut with_extra = entry("myorg", &["hello"]);
        with_extra
            .source
            .extra
            .insert("narHash".to_string(), serde_json::json!("sha256-abc123"));
        let locked = HashMap::from([("myorg/hello".to_string(), with_extra)]);

        let lock = build_lock_from_locked_inputs(locked, [&"myorg/hello".to_string()])
            .expect("transform succeeds");
        let closure = lock
            .project_package(&references(&["catalogs.myorg.hello"]))
            .expect("references are covered");

        assert_eq!(
            closure.locked_inputs["myorg/hello"].source.extra["narHash"],
            serde_json::json!("sha256-abc123")
        );
    }

    #[test]
    fn deep_chain_near_the_server_cap_does_not_exhaust_the_stack() {
        const DEPTH: usize = 16_384;
        let mut locked = HashMap::new();
        for i in 0..DEPTH {
            let name = format!("pkg{i}");
            let mut e = entry("myorg", &[&name]);
            if i + 1 < DEPTH {
                e.inputs = Some(vec![format!("myorg/pkg{}", i + 1)]);
            }
            locked.insert(format!("myorg/pkg{i}"), e);
        }

        let lock = build_lock_from_locked_inputs(locked, [&"myorg/pkg0".to_string()])
            .expect("transform succeeds");
        let closure = lock
            .project_package(&references(&["catalogs.myorg.pkg0"]))
            .expect("the whole chain resolves");

        assert_eq!(closure.locked_inputs.len(), DEPTH);
    }
}
