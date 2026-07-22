use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::analyze::{FileInfo, analyze_file_at, identity_origins};
use super::{CatalogRef, ScanError};

/// The NEF package files analyzed during closure resolution, keyed by package
/// key.
///
/// A package key is a dependency attr-path joined with `/`
/// (`python3Packages/isdr-zk-client`) — the same shape
/// [try_resolve_dependency_argument] uses to locate the file under `base_dir`.
/// Entry packages are added with [Self::add_root]; [Self::expand_closure] then
/// resolves each reachable dependency argument once and caches it in `scans`,
/// so a package shared by several dependents is analyzed a single time.
pub(super) struct PackageGraph {
    base_dir: PathBuf,
    /// Catalog root parameter names every package is scanned against.
    root_attributes: HashSet<String>,
    scans: HashMap<String, FileInfo>,
}

impl PackageGraph {
    /// An empty graph resolving packages under `base_dir` and scanning each
    /// against `root_attributes` (the catalog root parameter names).
    pub(super) fn new(base_dir: impl AsRef<Path>, root_attributes: HashSet<String>) -> Self {
        Self {
            base_dir: base_dir.as_ref().to_path_buf(),
            root_attributes,
            scans: HashMap::new(),
        }
    }

    /// Add an entry package by its path relative to `base_dir`, reading and
    /// analyzing it. Imports resolve against the entry's own directory. An
    /// unreadable entry is a no-op. Callable more than once.
    pub(super) fn add_root(&mut self, rel_file: impl AsRef<Path>) -> Result<(), ScanError> {
        let path = self.base_dir.join(rel_file.as_ref());
        let Ok(content) = fs::read_to_string(&path) else {
            return Ok(());
        };
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let scan = analyze_file_at(
            &content,
            &self.root_attributes,
            path.parent(),
            &mut HashMap::new(),
            &path,
            &identity_origins(&self.root_attributes),
        )?;
        self.scans.insert(stem, scan);
        Ok(())
    }

    /// Grow the graph to the transitive closure of the dependency arguments
    /// reachable from the roots. Each argument is resolved once via
    /// [try_resolve_dependency_argument] and cached; an argument that names
    /// nothing on disk is skipped. A dependency argument is an attr-path: a
    /// bare argument resolves as a sibling file, a longer path as a member of
    /// a sibling attribute set. Cycles are handled by tracking visited
    /// attr-paths.
    pub(super) fn expand_closure(&mut self) -> Result<(), ScanError> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: Vec<Vec<String>> = self
            .scans
            .keys()
            .map(|key| key.split('/').map(str::to_string).collect())
            .collect();

        while let Some(path) = queue.pop() {
            let key = path.join("/");
            if !visited.insert(key.clone()) {
                continue;
            }
            if !self.scans.contains_key(&key)
                && let Some(scan) =
                    try_resolve_dependency_argument(&self.base_dir, &path, &self.root_attributes)?
            {
                self.scans.insert(key.clone(), scan);
            }
            let Some(scan) = self.scans.get(&key) else {
                continue;
            };
            for dep in scan.dependency_args.clone() {
                if !visited.contains(&dep.join("/")) {
                    queue.push(dep);
                }
            }
        }

        Ok(())
    }

    /// Every catalog reference contributed by a package in the graph. Valid
    /// once [Self::expand_closure] has run: the graph then holds exactly the
    /// reachable packages, since only reachable arguments are ever resolved
    /// into it.
    pub(super) fn references(&self) -> BTreeSet<CatalogRef> {
        self.scans
            .values()
            .flat_map(|scan| scan.refs.iter().cloned())
            .collect()
    }
}

/// Resolve a dependency attr-path to the package file it names and analyze it.
///
/// `components` is the dependency's attr-path: the first element is the
/// dependency argument, the rest are members selected on it. Following the
/// `dirToAttrs` convention, each component is resolved against `dir` in turn:
/// a regular `<comp>.nix` is a package file (and shadows a same-named
/// directory); a `<comp>/default.nix` is a package directory; a directory with
/// no `default.nix` is an attribute set that is descended into. Components past
/// the package file are attributes within it and are ignored.
fn try_resolve_dependency_argument(
    dir: &Path,
    components: &[String],
    root_attributes: &HashSet<String>,
) -> Result<Option<FileInfo>, ScanError> {
    let mut cur = dir.to_path_buf();
    for comp in components {
        let file = cur.join(format!("{comp}.nix"));
        if file.is_file() {
            return read_and_analyze(&file, root_attributes);
        }
        let sub = cur.join(comp);
        let default = sub.join("default.nix");
        if default.is_file() {
            return read_and_analyze(&default, root_attributes);
        }
        if sub.is_dir() {
            cur = sub;
            continue;
        }
        return Ok(None);
    }
    Ok(None)
}

/// Read and analyze a resolved package file.
///
/// Relative imports in the file resolve against its own directory, so the
/// file's parent is passed as the import base. An unreadable file resolves to
/// `Ok(None)`; only scan failures are errors.
fn read_and_analyze(
    path: &Path,
    root_attributes: &HashSet<String>,
) -> Result<Option<FileInfo>, ScanError> {
    let Ok(content) = fs::read_to_string(path) else {
        return Ok(None);
    };
    analyze_file_at(
        &content,
        root_attributes,
        path.parent(),
        &mut HashMap::new(),
        path,
        &identity_origins(root_attributes),
    )
    .map(Some)
}
