use std::collections::{BTreeSet, HashSet};
use std::fmt::{self, Display};
use std::path::Path;

use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

mod analyze;
mod error;
mod graph;

pub use error::ScanError;
use graph::PackageGraph;

/// A single catalog attribute-path reference discovered by the scanner,
/// e.g. `catalogs.myorg.toolkit.readVersion`. A dynamic component collapses
/// the tail to a `*` sentinel (e.g. `catalogs.myorg.*`).
///
/// Distinct from a bare `String` so downstream lookup grouping consumes a
/// typed reference rather than an arbitrary string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CatalogRef(String);

impl CatalogRef {
    /// The reference as a dotted attr-path string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for CatalogRef {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for CatalogRef {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<CatalogRef> for String {
    fn from(value: CatalogRef) -> Self {
        value.0
    }
}

impl Display for CatalogRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Catalog root parameter names assumed by [scan_package].
///
/// A NEF package receives the catalog namespace as the `catalogs` lambda
/// parameter; attribute paths reached through it (`catalogs.<org>.<pkg>…`) are
/// the references that must be locked. Use [scan_package_with_roots] to scan
/// against a different set of root_attributes.
const DEFAULT_ROOT_ATTRIBUTES: &[&str] = &["catalogs"];

/// Resolve the catalog-reference closure of a single NEF package.
///
/// `base_dir` is the package-set root (e.g. `pkgs/`) and `rel_file` is the
/// target expression relative to it. The returned set contains every catalog
/// attr-path the target transitively depends on: references in the target
/// itself (including those reached through `import`), plus references reached
/// through its dependency arguments. A dependency argument is resolved as a
/// sibling package (`<name>.nix` or `<name>/default.nix`); a member selected on
/// it (`<name>.<member>`) is resolved as a member of a sibling attribute set,
/// descending namespace directories under `base_dir`.
///
/// Fails when a scanned file references a catalog root it does not declare in
/// its function arguments (see [ScanError::UndeclaredRoot]).
///
/// Uses the default `catalogs` root; see [scan_package_with_roots] to override.
pub fn scan_package(
    base_dir: impl AsRef<Path>,
    rel_file: impl AsRef<Path>,
) -> Result<BTreeSet<CatalogRef>, ScanError> {
    scan_package_with_roots(base_dir, rel_file, DEFAULT_ROOT_ATTRIBUTES.iter().copied())
}

/// [scan_package] generalized over the set of catalog root parameter names.
///
/// `root_attributes` are the lambda-parameter names treated as catalog namespaces; every
/// other parameter is a dependency argument followed to a sibling package.
/// Any iterable of names is accepted; duplicates are harmless.
#[instrument(
    skip(root_attributes),
    fields(
        base_dir = %base_dir.as_ref().display(),
        rel_file = %rel_file.as_ref().display(),
    )
)]
pub fn scan_package_with_roots(
    base_dir: impl AsRef<Path>,
    rel_file: impl AsRef<Path>,
    root_attributes: impl IntoIterator<Item = impl Into<String>>,
) -> Result<BTreeSet<CatalogRef>, ScanError> {
    let root_attributes: HashSet<String> = root_attributes.into_iter().map(Into::into).collect();

    let mut graph = PackageGraph::new(base_dir, root_attributes);
    graph.add_root(rel_file)?;
    graph.expand_closure()?;
    let references = graph.references();

    debug!(references = references.len(), "scanned catalog references");
    Ok(references)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn set(items: &[&str]) -> BTreeSet<CatalogRef> {
        items.iter().map(|s| CatalogRef::from(*s)).collect()
    }

    #[test]
    fn transitive_cycle_safe() {
        // A dependency-argument cycle (pkg-a <-> pkg-b) must terminate and
        // still union both packages' refs.
        let base_dir = Path::new("test_data/catalog_refs/dep-cycle");
        let got = scan_package(base_dir, Path::new("pkg-a.nix")).unwrap();
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit.readVersion",
                "catalogs.myorg.python3Packages.alpha-lib",
            ])
        );
    }

    #[test]
    fn transitive_inputs_root() {
        // Transitive closure under a non-default root: `main` pulls in the
        // `dep-pkg` sibling, whose `inputs.*` refs join the closure.
        let base_dir = Path::new("test_data/catalog_refs/inputs-transitive");
        let got = scan_package_with_roots(base_dir, Path::new("main.nix"), ["inputs"]).unwrap();
        assert_eq!(
            got,
            set(&[
                "inputs.nixpkgs.lib",
                "inputs.devtools-flake.packages.default",
            ])
        );
    }

    #[test]
    fn scan_package_follows_dep_of_wrapped_lambda() {
        let base_dir = Path::new("test_data/catalog_refs");
        // dep-entry-wrapped.nix wraps the package function in `let … in`; the
        // `dep-helper` dependency argument must still pull the sibling's refs
        // into the closure.
        let got = scan_package(base_dir, Path::new("dep-entry-wrapped.nix")).unwrap();
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit.readVersion",
                "catalogs.myorg.python3Packages.alpha-lib",
            ])
        );
    }

    #[test]
    fn scan_package_unions_target_and_sibling_dep_refs() {
        let base_dir = Path::new("test_data/catalog_refs");
        // dep-entry.nix references one catalog path and pulls in a `dep-helper`
        // dependency argument; dep-helper.nix (its sibling under base_dir)
        // references another. The closure is the union of both.
        let got = scan_package(base_dir, Path::new("dep-entry.nix")).unwrap();
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit.readVersion",
                "catalogs.myorg.python3Packages.alpha-lib",
            ])
        );
    }

    /// Relative imports inside a `<name>/default.nix` dependency.
    ///
    /// A dependency argument resolved as `foo/default.nix` may import a helper
    /// with a path relative to its own directory (`./helper.nix` ->
    /// `foo/helper.nix`). Following that import must resolve the path against
    /// `foo/`, not the package-set root, so the helper's refs are collected.
    #[test]
    fn scan_package_dep_subdir_default_follows_relative_import() {
        let base_dir = Path::new("test_data/catalog_refs/depdir-import");
        let got = scan_package(base_dir, Path::new("entry.nix")).unwrap();
        assert_eq!(
            got,
            set(&["catalogs.myorg.direct", "catalogs.myorg.helper-ref"]),
        );
    }

    /// Same-repo package-set aliases.
    ///
    /// A top-level package can be a thin alias that re-exports a member of an
    /// in-repo package set, written in the deep-overlay form
    /// `{ python3Packages }: python3Packages.isdr-zk-client`.
    /// The catalog inputs of that package live in the member file
    /// `python3Packages/isdr-zk-client/default.nix`, so the alias's closure must
    /// include the member's refs.
    #[test]
    fn scan_package_follows_alias_to_pkgset_member() {
        let base_dir = Path::new("test_data/catalog_refs/pkgset-member-alias");
        let got = scan_package(base_dir, Path::new("isdr-zk-client.nix")).unwrap();
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
    }

    /// A nested file as the scan target resolves dependency_args against the root.
    ///
    /// Scanning `foo/bar.nix` directly must resolve its dependency arguments
    /// against the package-set root, not `foo/`, so a root-level package like
    /// `top` is reachable and its refs join the closure.
    #[test]
    fn scan_package_nested_target_resolves_deps_at_root() {
        let base_dir = Path::new("test_data/catalog_refs/nested-target-access");
        let got = scan_package(base_dir, Path::new("foo/bar.nix")).unwrap();
        assert_eq!(
            got,
            set(&["catalogs.myorg.bar-own", "catalogs.myorg.top-src"]),
        );
    }

    /// A package-set member's own dependencies are followed transitively.
    ///
    /// `top.nix` selects the `widget` member of the `python3Packages` namespace;
    /// the member references a catalog input and depends on a sibling package
    /// `helper-lib` (resolved at the package-set root). The closure unions the
    /// member's ref and the sibling's ref.
    #[test]
    fn scan_package_follows_pkgset_member_transitive_deps() {
        let base_dir = Path::new("test_data/catalog_refs/pkgset-member-transitive");
        let got = scan_package(base_dir, Path::new("top.nix")).unwrap();
        assert_eq!(
            got,
            set(&["catalogs.myorg.widget-src", "catalogs.myorg.helper-lib-src"]),
        );
    }

    /// Invariant over every fixture: an emitted ref is either a sentinel
    /// (`….*`) or an exact ref with at least two components past the root —
    /// anything shallower can never resolve (the server's floor is catalog +
    /// one component) and would fail the whole lock.
    #[test]
    fn all_fixture_refs_are_lockable_shapes() {
        let dir = Path::new("test_data/catalog_refs");
        let mut scanned = 0;
        let mut violations: Vec<String> = Vec::new();
        for entry in fs::read_dir(dir).expect("fixture dir") {
            let path = entry.expect("fixture entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("nix") {
                continue;
            }
            let rel = path.file_name().expect("fixture file name");
            // Some fixtures pin scan *errors* (unreadable imports,
            // undeclared root_attributes); they emit no refs to check.
            let Ok(references) = scan_package(dir, Path::new(rel)) else {
                continue;
            };
            for reference in references {
                scanned += 1;
                let reference = reference.as_str();
                let post_root = reference.split('.').count() - 1;
                if !reference.ends_with(".*") && post_root < 2 {
                    violations.push(reference.to_string());
                }
            }
        }
        assert_eq!(violations, Vec::<String>::new());
        assert!(scanned > 0, "no fixture refs scanned");
    }
}
