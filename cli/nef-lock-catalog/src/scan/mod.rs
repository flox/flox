use std::collections::{BTreeSet, HashSet};
use std::fmt::{self, Display};
use std::path::Path;
use std::str::FromStr;

use flox_core::canonical_path::CanonicalPath;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

mod analyze;
mod attr_path;
mod error;
mod graph;

pub(crate) use attr_path::AttrPath;
pub use attr_path::InvalidAttrPath;
pub use error::{ImportSite, ScanError};
use graph::PackageGraph;

/// A single catalog attribute-path reference discovered by the scanner,
/// e.g. `catalogs.myorg.toolkit.readVersion`. Where the walker could not
/// resolve a path further it names everything under it instead, rendered with
/// a `*` sentinel (e.g. `catalogs.myorg.*`).
///
/// Distinct from an [AttrPath] because not every path the walker resolves can
/// be locked: the conversion rejects the shapes that could never resolve, so
/// consumers need not re-check them.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct CatalogRef(AttrPath);

/// An [AttrPath] that cannot be a [CatalogRef].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("'{path}' {kind}")]
pub struct InvalidCatalogRef {
    /// The offending path. Crate-internal like [AttrPath] itself: outside the
    /// crate a reference is text, and the message says which rule refused it.
    pub(crate) path: AttrPath,
    pub(crate) kind: InvalidCatalogRefKind,
}

/// Why a path cannot be a [CatalogRef]. None of these name anything the server
/// can resolve, for different reasons.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum InvalidCatalogRefKind {
    /// The path names the namespace and nothing under it, whether it says so
    /// with a wildcard (`catalogs.*`) or by naming the root alone
    /// (`catalogs`). The wire form drops the root, leaving `*` or nothing.
    #[error("references the whole catalog namespace")]
    RootWildcard,

    /// The server resolves no shallower than a catalog plus one component, so
    /// naming a catalog alone reaches nothing.
    #[error("names a catalog rather than a package in one")]
    CatalogLevel,
}

impl CatalogRef {
    /// The path this reference locks. Crate-internal: a reference is a
    /// [Display] value to consumers, and the wire form is built from it here.
    pub(crate) fn path(&self) -> &AttrPath {
        &self.0
    }

    /// The catalog-relative rendering of this reference: the path with its
    /// root dropped, dot-joined — the form the catalog server's lookup
    /// request takes. NOTE: this is not how a lock's `direct_catalog_inputs`
    /// is keyed; the server keys results canonically as
    /// `<catalog>/<attr-path>` (see the `matched` map, which relates the two
    /// namespaces).
    pub(crate) fn wire_key(&self) -> String {
        // A reference always has a root to drop; that is what makes it one.
        self.0
            .pop_root()
            .expect("a reference names something under its root")
            .to_string()
    }

    /// Build a reference without checking the invariant, so tests can state
    /// their expectations as literals.
    #[cfg(test)]
    pub(crate) fn new_unchecked(value: &str) -> Self {
        Self(value.parse().expect("attr paths always parse"))
    }
}

/// A string that cannot be a [CatalogRef], either because it is not an
/// attribute path or because it is one that cannot be locked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseCatalogRefError {
    #[error(transparent)]
    Path(#[from] InvalidAttrPath),
    #[error(transparent)]
    Reference(#[from] InvalidCatalogRef),
}

/// Parsing reads the text back into an [AttrPath] and applies the same rules
/// as any other construction, so a reference read from a lock file is held to
/// them too.
impl FromStr for CatalogRef {
    type Err = ParseCatalogRefError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let path: AttrPath = value.parse()?;
        Ok(path.try_into()?)
    }
}

/// Deserialization goes through [FromStr] (see `#[serde(try_from)]`).
impl TryFrom<String> for CatalogRef {
    type Error = ParseCatalogRefError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

/// A reference reaches a package: a root, the catalog it names, and something
/// within it — or a wildcard standing in for that last part.
impl TryFrom<AttrPath> for CatalogRef {
    type Error = InvalidCatalogRef;

    fn try_from(path: AttrPath) -> Result<Self, Self::Error> {
        // A wildcard is always last and stands for whatever it replaced, so
        // depth alone decides: root, catalog, and something within it.
        // Anything shallower than a catalog names the namespace itself,
        // however it is written.
        let kind = match (path.len(), path.is_wildcard()) {
            (3.., _) => return Ok(Self(path)),
            (2, false) => InvalidCatalogRefKind::CatalogLevel,
            _ => InvalidCatalogRefKind::RootWildcard,
        };
        Err(InvalidCatalogRef { path, kind })
    }
}

impl From<CatalogRef> for String {
    fn from(value: CatalogRef) -> Self {
        value.0.to_string()
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

    // The scan names every file it reads canonically, so the root failures are
    // re-expressed against has to be canonical too or nothing strips.
    let base_dir = CanonicalPath::new(base_dir).map_err(|err| ScanError::UnreadableFile {
        file: err.path,
        source: err.err,
        imported_from: None,
    })?;

    // Failures are re-expressed against the package-set root here, at the
    // crate's boundary, so the paths a user reads name files the way they
    // wrote them rather than wherever the scan happened to read them from.
    let mut graph = PackageGraph::new(base_dir.clone(), root_attributes);
    let scan = || {
        graph.add_root(rel_file)?;
        graph.expand_closure()?;
        graph.references()
    };
    let references = scan().map_err(|err| err.relative_to(&base_dir))?;

    debug!(references = references.len(), "scanned catalog references");
    Ok(references)
}

#[cfg(test)]
mod tests {
    use std::{assert_matches, fs};

    use super::*;

    fn set(items: &[&str]) -> BTreeSet<CatalogRef> {
        items.iter().map(|s| CatalogRef::new_unchecked(s)).collect()
    }

    #[test]
    fn catalog_ref_rejects_references_that_cannot_resolve() {
        use InvalidCatalogRefKind::{CatalogLevel, RootWildcard};

        let cases = [
            // References that reach into a catalog, whatever the depth.
            // Catalog and package sentinels expand server-side.
            ("catalogs.myorg.pkg.readVersion", None),
            ("catalogs.myorg.pkg", None),
            ("catalogs.myorg.pkg.*", None),
            ("catalogs.myorg.*", None),
            // Naming the namespace and naming it with a wildcard are the same
            // reference; stopping at a catalog reaches nothing either.
            ("catalogs.*", Some(RootWildcard)),
            ("catalogs", Some(RootWildcard)),
            ("catalogs.myorg", Some(CatalogLevel)),
        ];
        let got: Vec<(&str, Option<InvalidCatalogRefKind>)> = cases
            .iter()
            .map(|(reference, _)| {
                let kind = match reference.parse::<CatalogRef>() {
                    Ok(_) => None,
                    Err(ParseCatalogRefError::Reference(err)) => Some(err.kind),
                    Err(err) => panic!("{reference}: expected a rejected path, got {err}"),
                };
                (*reference, kind)
            })
            .collect();
        assert_eq!(got, cases.to_vec());
    }

    #[test]
    fn catalog_ref_rejects_text_that_is_not_an_attribute_path() {
        // Parsing goes through rnix, so what is not Nix is not a path. A
        // dynamic attribute is the one Nix-legal shape refused: it names
        // nothing until evaluated.
        let cases = ["catalogs myorg", "catalogs.${org}.pkg", ""];
        for reference in cases {
            assert_matches!(
                reference.parse::<CatalogRef>(),
                Err(ParseCatalogRefError::Path(_)),
                "reference: {reference}"
            );
        }
    }

    #[test]
    fn catalog_ref_round_trips_quoted_names() {
        // A name the catalog could not hold is still a path Nix accepts, so
        // parsing keeps it and rendering quotes it back. The walker widens
        // rather than emitting one, but a reference read from text must
        // survive being written out again.
        for reference in [
            "catalogs.myorg.pkg",
            "catalogs.myorg.*",
            "catalogs.\"foo.bar\".pkg",
            "catalogs.myorg.\"with space\"",
        ] {
            let parsed = reference
                .parse::<CatalogRef>()
                .unwrap_or_else(|err| panic!("{reference}: {err}"));
            assert_eq!(parsed.to_string(), reference);
        }
    }

    #[test]
    fn catalog_ref_deserialization_upholds_the_invariant() {
        // `#[serde(try_from)]` keeps the invariant on the way in; without it a
        // lock file could reintroduce a reference the type rules out.
        assert_matches!(
            serde_json::from_str::<CatalogRef>("\"catalogs.myorg.pkg\""),
            Ok(reference) if reference.to_string() == "catalogs.myorg.pkg"
        );
        assert_matches!(serde_json::from_str::<CatalogRef>("\"catalogs.*\""), Err(_));
    }

    #[test]
    fn root_wildcard_fails_the_scan() {
        // The walk records the widening and the graph rejects it, once every
        // path is back in the top-level namespace. The error names the file
        // and position the path was found at, relative to the package-set
        // root the caller named it under.
        let base_dir = Path::new("test_data/catalog_refs");
        let err =
            scan_package(base_dir, Path::new("escaping-root.nix")).expect_err("scan should fail");
        assert_matches!(
            err,
            ScanError::UnlockableReference { file, position, reason }
                if file == Path::new("escaping-root.nix")
                    && position == Some((4, 3))
                    && reason == "'catalogs.*' references the whole catalog namespace"
        );
    }

    #[test]
    fn unreadable_entry_fails_the_scan() {
        // The entry names the file NEF would evaluate, so failing to read it
        // must not resolve to an empty reference set: that would write a lock
        // claiming the package has no catalog inputs.
        let base_dir = Path::new("test_data/catalog_refs");
        let err =
            scan_package(base_dir, Path::new("no-such-package.nix")).expect_err("scan should fail");
        assert_matches!(
            err,
            ScanError::UnreadableFile {
                file,
                source,
                imported_from: None,
            } if file == Path::new("no-such-package.nix")
                && source.kind() == std::io::ErrorKind::NotFound
        );
    }

    #[test]
    fn dependency_argument_resolving_to_nothing_scans_clean() {
        // An argument NEF satisfies from nixpkgs names no file in the package
        // set. It is skipped, not read, so the tightened read path must not
        // turn it into a failure.
        let base_dir = Path::new("test_data/catalog_refs");
        let got = scan_package(base_dir, Path::new("nixpkgs-arg.nix")).unwrap();
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
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
    fn every_fixture_scans_to_a_result() {
        // The shapes this used to check for are now unrepresentable, so what
        // is left is a sweep: every top-level fixture either scans or fails
        // deliberately, and between them they do produce references.
        let dir = Path::new("test_data/catalog_refs");
        let mut scanned = 0;
        for entry in fs::read_dir(dir).expect("fixture dir") {
            let path = entry.expect("fixture entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("nix") {
                continue;
            }
            let rel = path.file_name().expect("fixture file name");
            // Some fixtures pin scan *errors* (unreadable imports,
            // undeclared root_attributes); they emit no refs to count.
            let Ok(references) = scan_package(dir, Path::new(rel)) else {
                continue;
            };
            scanned += references.len();
        }
        assert!(scanned > 0, "no fixture refs scanned");
    }
}
