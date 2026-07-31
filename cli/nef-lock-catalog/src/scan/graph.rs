use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use tracing::warn;

use super::analyze::{FileInfo, RefSource, analyze_file_at, identity_origins};
use super::{AttrPath, CatalogRef, ScanError};

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
    /// analyzing it. Imports resolve against the entry's own directory.
    /// Callable more than once.
    pub(super) fn add_root(&mut self, rel_file: impl AsRef<Path>) -> Result<(), ScanError> {
        let path = self.base_dir.join(rel_file.as_ref());
        let key = package_key(rel_file.as_ref());
        let scan = read_and_analyze(&path, &self.root_attributes)?;
        self.scans.insert(key, scan);
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
    ///
    /// This is where paths become references. Whether one can be locked turns
    /// on its depth below the top-level root, which a file scanned through a
    /// forwarded namespace cannot know — only here has every path been
    /// rewritten out of the namespace it was written in. Paths are visited in
    /// order and the first that cannot be locked is reported against the
    /// source it was found at.
    ///
    /// A reference that survives but ends in a wildcard is an over-lock: the
    /// scanner could not resolve that far and locked the subtree instead. That
    /// too is a property of the finished reference, so it is warned about
    /// here rather than where the widening happened.
    pub(super) fn references(&self) -> Result<BTreeSet<CatalogRef>, ScanError> {
        let mut paths: BTreeMap<&AttrPath, &RefSource> = BTreeMap::new();
        for scan in self.scans.values() {
            for (path, source) in &scan.refs {
                paths.entry(path).or_insert(source);
            }
        }
        paths
            .into_iter()
            .map(|(path, source)| {
                let reference = CatalogRef::try_from(path.clone()).map_err(|reason| {
                    ScanError::UnlockableReference {
                        file: source.file.clone(),
                        position: Some(source.position),
                        reason: reason.to_string(),
                    }
                })?;
                if reference.path().is_wildcard() {
                    warn!(
                        reference = %reference,
                        file = %source.file.display(),
                        line = source.position.0,
                        column = source.position.1,
                        "catalog namespace escapes static analysis; locking the whole subtree",
                    );
                }
                Ok(reference)
            })
            .collect()
    }
}

/// The package key naming `rel_file`, a path relative to `base_dir`.
///
/// Keys are attr-paths, so an entry package is keyed by its own path with the
/// extension dropped: `a/foo.nix` keys as `a/foo`, the attr-path
/// [try_resolve_dependency_argument] resolves back to that same file. Keying by
/// the bare file name instead would let two entries under different directories
/// overwrite each other, and would let an entry shadow the different file a
/// same-named dependency argument resolves to.
///
/// A `default.nix` names its directory, so its trailing component is dropped:
/// `foo/default.nix` keys as `foo`, the key the dependency argument `foo`
/// resolves to. A `default.nix` directly under `base_dir` has no directory to
/// name and keeps its own name.
fn package_key(rel_file: &Path) -> String {
    let key = match (rel_file.file_name(), rel_file.parent()) {
        (Some(name), Some(parent)) if name == "default.nix" && !parent.as_os_str().is_empty() => {
            parent
        },
        _ => &rel_file.with_extension(""),
    };
    key.to_string_lossy().into_owned()
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
///
/// `Ok(None)` means the argument names nothing on disk, which is how an
/// argument NEF satisfies from nixpkgs looks. A file that does resolve but
/// cannot be read is an error instead: it is a package of this set.
fn try_resolve_dependency_argument(
    dir: &Path,
    components: &[String],
    root_attributes: &HashSet<String>,
) -> Result<Option<FileInfo>, ScanError> {
    let mut cur = dir.to_path_buf();
    for comp in components {
        let file = cur.join(format!("{comp}.nix"));
        if file.is_file() {
            return read_and_analyze(&file, root_attributes).map(Some);
        }
        let sub = cur.join(comp);
        let default = sub.join("default.nix");
        if default.is_file() {
            return read_and_analyze(&default, root_attributes).map(Some);
        }
        if sub.is_dir() {
            cur = sub;
            continue;
        }
        return Ok(None);
    }
    Ok(None)
}

/// Read and analyze one resolved package file.
///
/// Relative imports in the file resolve against its own directory, so the
/// file's parent is passed as the import base. Shared by [PackageGraph::add_root]
/// (entry packages) and [try_resolve_dependency_argument] (dependencies).
///
/// Error, if the file at `path` cannot be read.
/// Skipping the analysis here would otherwise lead to incomplete closures,
/// and likely evaluation errors.
fn read_and_analyze(path: &Path, root_attributes: &HashSet<String>) -> Result<FileInfo, ScanError> {
    let content = fs::read_to_string(path).map_err(|source| ScanError::UnreadableFile {
        file: path.to_path_buf(),
        source,
        imported_from: None,
    })?;
    analyze_file_at(
        &content,
        root_attributes,
        path.parent(),
        &mut HashMap::new(),
        path,
        &identity_origins(root_attributes),
    )
}
