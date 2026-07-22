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
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;

    use super::analyze::{FileInfo, analyze_file_at, identity_origins, line_col};
    use super::*;

    fn root_attributes(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn analyze_file(content: &str, root_attributes: &HashSet<String>) -> FileInfo {
        analyze_file_at(
            content,
            root_attributes,
            None,
            &mut HashMap::new(),
            Path::new("test.nix"),
            &identity_origins(root_attributes),
        )
        .expect("scan should succeed")
    }

    fn refs(content: &str, root_attributes: &HashSet<String>) -> BTreeSet<CatalogRef> {
        analyze_file(content, root_attributes).refs
    }

    fn scan_err(content: &str, root_attributes: &HashSet<String>) -> ScanError {
        analyze_file_at(
            content,
            root_attributes,
            None,
            &mut HashMap::new(),
            Path::new("test.nix"),
            &identity_origins(root_attributes),
        )
        .expect_err("scan should fail")
    }

    fn refs_at(path: &str, root_attributes: &HashSet<String>) -> BTreeSet<CatalogRef> {
        let path = Path::new(path);
        let content = fs::read_to_string(path).expect("test fixture missing");
        let dir = path.parent();
        let mut visited = HashMap::new();
        analyze_file_at(
            &content,
            root_attributes,
            dir,
            &mut visited,
            path,
            &identity_origins(root_attributes),
        )
        .expect("scan should succeed")
        .refs
    }

    fn scan_err_at(path: &str, root_attributes: &HashSet<String>) -> ScanError {
        let path = Path::new(path);
        let content = fs::read_to_string(path).expect("test fixture missing");
        let dir = path.parent();
        let mut visited = HashMap::new();
        analyze_file_at(
            &content,
            root_attributes,
            dir,
            &mut visited,
            path,
            &identity_origins(root_attributes),
        )
        .expect_err("scan should fail")
    }

    fn set(items: &[&str]) -> BTreeSet<CatalogRef> {
        items.iter().map(|s| CatalogRef::from(*s)).collect()
    }

    #[test]
    fn line_col_maps_byte_offsets_to_1_based_positions() {
        // indices: a0 b1 c2 \n3 d4 e5 \n6 f7
        let content = "abc\nde\nf";
        assert_eq!(line_col(content, 0), (1, 1));
        assert_eq!(line_col(content, 2), (1, 3));
        assert_eq!(line_col(content, 4), (2, 1));
        assert_eq!(line_col(content, 7), (3, 1));
    }

    #[test]
    fn no_catalog_refs_fetchpypi() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/no-catalog-refs.nix"),
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn no_catalog_refs_rust_package() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/rust-no-catalog.nix"),
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn non_catalog_inherit_not_collected() {
        let content = include_str!("../../test_data/catalog_refs/non-catalog-inherit.nix");
        assert_eq!(
            refs(content, &root_attributes(&["catalogs"])),
            BTreeSet::new()
        );
        assert_eq!(
            refs(content, &root_attributes(&["inputs"])),
            BTreeSet::new()
        );
    }

    #[test]
    fn single_inherit_helper() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/single-inherit-helper.nix"),
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
    }

    #[test]
    fn two_inherits_toolkit_and_python_pkg() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/two-inherits.nix"),
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit.readVersion",
                "catalogs.myorg.python3Packages.beta-client",
            ])
        );
    }

    #[test]
    fn multi_attr_inherit_expands_all_names() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/multi-attr-inherit.nix"),
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit.readVersion",
                "catalogs.myorg.python3Packages.alpha-lib",
                "catalogs.myorg.python3Packages.delta-util",
                "catalogs.myorg.python3Packages.epsilon-core",
                "catalogs.myorg.python3Packages.eta-parser",
                "catalogs.myorg.python3Packages.theta-worker",
            ])
        );
    }

    #[test]
    fn multi_attr_inherit_no_bare_intermediate_path() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/multi-attr-inherit.nix"),
            &root_attributes(&["catalogs"]),
        );
        assert!(!got.contains(&CatalogRef::from("catalogs.myorg.python3Packages")));
        assert!(!got.contains(&CatalogRef::from("catalogs.myorg.toolkit")));
    }

    #[test]
    fn direct_select_native_packages() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/direct-select-native.nix"),
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit.readMakeVersion",
                "catalogs.myorg.python3Packages.epsilon-core",
                "catalogs.myorg.proxy-wrap",
                "catalogs.myorg.queue-bin",
            ])
        );
    }

    #[test]
    fn inherit_whole_subattrset() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/inherit-subattrset.nix"),
            &root_attributes(&["catalogs"]),
        );
        // `toolkit` is inherited (an exact depth-2 ref) and then used as an
        // alias in the body (`toolkit.buildGoModule`); the deeper use-site ref
        // canonicalizes to the same package server-side.
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit",
                "catalogs.myorg.toolkit.buildGoModule"
            ])
        );
    }

    #[test]
    fn nested_inline_package_does_not_hide_outer_refs() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/nested-inline-package.nix"),
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit.readVersion",
                "catalogs.myorg.python3Packages.alpha-lib",
                "catalogs.myorg.python3Packages.gamma-service",
                "catalogs.myorg.python3Packages.theta-worker",
            ])
        );
    }

    #[test]
    fn passthru_src_access_via_alias() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/passthru-src-access.nix"),
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit.readVersion",
                "catalogs.myorg.python3Packages.gamma-service",
                "catalogs.myorg.python3Packages.zeta-api",
                "catalogs.myorg.queue-bin",
                "catalogs.myorg.queue-bin.src",
            ])
        );
    }

    #[test]
    fn inputs_only_with_input_roots() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/inputs-only.nix"),
            &root_attributes(&["inputs"]),
        );
        // `inputs.self` is used at catalog depth in value positions
        // (`src = inputs.self;`, `"${inputs.self}/VERSION"`), so it widens to
        // a sentinel rather than an unresolvable exact ref.
        assert_eq!(
            got,
            set(&[
                "inputs.nixpkgs.lib",
                "inputs.nixpkgs.lib.fileContents",
                "inputs.devtools-flake.packages.default",
                "inputs.self.*",
            ])
        );
    }

    #[test]
    fn inputs_only_with_catalog_roots_returns_nothing() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/inputs-only.nix"),
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn mixed_roots_catalog_only() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/mixed-roots.nix"),
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit.readVersion",
                "catalogs.myorg.python3Packages.alpha-lib",
            ])
        );
    }

    #[test]
    fn mixed_roots_inputs_only() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/mixed-roots.nix"),
            &root_attributes(&["inputs"]),
        );
        assert_eq!(
            got,
            set(&[
                "inputs.nixpkgs.lib",
                "inputs.nixpkgs.lib.fakeStr",
                "inputs.devtools-flake.packages.default",
            ])
        );
    }

    #[test]
    fn mixed_roots_both() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/mixed-roots.nix"),
            &root_attributes(&["catalogs", "inputs"]),
        );
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit.readVersion",
                "catalogs.myorg.python3Packages.alpha-lib",
                "inputs.nixpkgs.lib",
                "inputs.nixpkgs.lib.fakeStr",
                "inputs.devtools-flake.packages.default",
            ])
        );
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
    fn dep_arguments_collected_through_wrapped_lambda() {
        // The dependency arguments come from the eventual package function,
        // regardless of the wrappers around it.
        let cases: &[(&str, &[&[&str]])] = &[
            ("{ catalogs, somedep }: catalogs.myorg.pkg", &[&["somedep"]]),
            (
                "let x = 1; in { catalogs, somedep }: catalogs.myorg.pkg",
                &[&["somedep"]],
            ),
            (
                "with builtins; { catalogs, somedep }: catalogs.myorg.pkg",
                &[&["somedep"]],
            ),
            ("({ catalogs, somedep }: catalogs.myorg.pkg)", &[&[
                "somedep",
            ]]),
        ];
        for (content, expected) in cases {
            let expected: Vec<Vec<String>> = expected
                .iter()
                .map(|dep| dep.iter().map(|s| s.to_string()).collect())
                .collect();
            assert_eq!(
                analyze_file(content, &root_attributes(&["catalogs"])).dependency_args,
                expected,
                "content: {content}"
            );
        }
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

    #[test]
    fn with_direct_namespace_emits_sentinel() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/with-namespace.nix"),
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.*"]));
    }

    #[test]
    fn with_namespace_does_not_emit_bare_path() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/with-namespace.nix"),
            &root_attributes(&["catalogs"]),
        );
        assert!(!got.contains(&CatalogRef::from("catalogs.myorg")));
    }

    #[test]
    fn with_alias_namespace_emits_sentinel() {
        let got = refs(
            "{ catalogs }: let org = catalogs.myorg; in with org; toolkit",
            &root_attributes(&["catalogs"]),
        );
        assert!(
            got.contains(&CatalogRef::from("catalogs.myorg.*")),
            "got: {got:?}"
        );
    }

    #[test]
    fn with_non_rooted_namespace_falls_through() {
        let got = refs(
            "{ catalogs }: with { x = 1; }; catalogs.myorg.pkg",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.pkg"]));
    }

    #[test]
    fn with_body_direct_refs_still_collected() {
        let got = refs(
            "{ catalogs }: with catalogs.myorg; catalogs.other.pkg",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.*", "catalogs.other.pkg"]));
    }

    #[test]
    fn aliased_select_single_level() {
        // The alias RHS names a whole catalog, which can never resolve as an
        // exact ref; only the use site drives the ref.
        let got = refs(
            "{ catalogs }: let org = catalogs.myorg; in org.toolkit",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit"]));
    }

    #[test]
    fn aliased_select_chained() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/aliased-select.nix"),
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit",
                "catalogs.myorg.toolkit.readVersion",
            ])
        );
    }

    #[test]
    fn aliased_select_order_independent() {
        let got = refs(
            "{ catalogs }: let b = a.hello; a = catalogs.myorg; in b",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.hello"]));
    }

    #[test]
    fn alias_rebound_as_lambda_param_not_emitted() {
        // `org` inside `g` is the lambda's own parameter, not the outer alias,
        // and the outer alias is never used in a value position.
        let got = refs(
            "{ catalogs, x }: let org = catalogs.myorg; g = org: org.other; in g x",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn rec_attrset_members_alias_each_other() {
        // `rec { }` scopes like `let`: `org` is visible to `pkg`. The set is
        // the file's value, so the catalog-level member also escapes.
        let got = refs(
            "{ catalogs }: rec { org = catalogs.myorg; pkg = org.toolkit; }",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.*", "catalogs.myorg.toolkit"]));
    }

    #[test]
    fn recursive_scope_resolves_forward_references() {
        let cases: &[(&str, &[&str])] = &[
            // A conditional branch referencing a later binding still unions
            // that branch.
            (
                "{ catalogs, x }: let org = if x then catalogs.a else other; other = catalogs.b; in org.pkg",
                &["catalogs.a.pkg", "catalogs.b.pkg"],
            ),
            // A set member referencing a later binding resolves through it.
            (
                "{ catalogs }: let s = { helper = other; }; other = catalogs.othercat; in s.helper.pkg",
                &["catalogs.othercat.pkg"],
            ),
            // An inherit whose source is a modeled set binds the member as an
            // alias; the use site drives the ref.
            (
                "{ catalogs }: let s = { org = catalogs.myorg; }; inherit (s) org; in org.pkg",
                &["catalogs.myorg.pkg"],
            ),
            // The same stepping works for a set literal's own inherits.
            (
                "{ catalogs }: let s = { org = catalogs.myorg; }; t = { inherit (s) org; }; in t.org.pkg",
                &["catalogs.myorg.pkg"],
            ),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn self_referential_alias_terminates() {
        // `a` can only evaluate through the else branch (forcing the then
        // branch recurses forever in Nix too). The property under test is
        // that resolution terminates with the base ref locked; the exact
        // tail refs past it depend on the resolution pass cap.
        let got = refs(
            "{ catalogs, x }: let a = if x then a.sub else catalogs.b.pkg; in a",
            &root_attributes(&["catalogs"]),
        );
        assert!(
            got.contains(&CatalogRef::from("catalogs.b.pkg")),
            "got: {got:?}"
        );
    }

    #[test]
    fn attrset_member_alias_resolves_through_select() {
        let got = refs(
            "{ catalogs }: let s = { org = catalogs.myorg; }; in s.org.toolkit",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit"]));
    }

    #[test]
    fn conditional_alias_unions_resolvable_branches() {
        let cases: &[(&str, &[&str])] = &[
            // The alias may be either catalog at runtime; lock both.
            (
                "{ catalogs, x }: let org = if x then catalogs.a else catalogs.b; in org.pkg",
                &["catalogs.a.pkg", "catalogs.b.pkg"],
            ),
            // A branch the scanner cannot model contributes nothing.
            (
                "{ catalogs, x, y }: let org = if x then catalogs.a else y; in org.pkg",
                &["catalogs.a.pkg"],
            ),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn escaping_namespaces_emit_sentinels() {
        let cases: &[(&str, &[&str])] = &[
            // The whole root escapes into an opaque function: anything under
            // it may be accessed.
            ("{ catalogs, f }: f catalogs", &["catalogs.*"]),
            // A whole catalog escapes through an alias.
            ("{ catalogs, f }: let org = catalogs.myorg; in f org", &[
                "catalogs.myorg.*",
            ]),
            // Escape positions other than function arguments.
            ("{ catalogs }: [ catalogs ]", &["catalogs.*"]),
            ("{ catalogs, f }: f { inherit catalogs; }", &["catalogs.*"]),
            // Helper-lambda indirection: the lambda parameter is opaque, so
            // the refs inside it are invisible; the escaping root covers them.
            (
                "{ catalogs }: let f = c: c.myorg.toolkit; in f catalogs",
                &["catalogs.*"],
            ),
            // A modeled set escapes through every path reachable from its
            // members.
            (
                "{ catalogs, f }: let s = { org = catalogs.myorg; }; in f s",
                &["catalogs.myorg.*"],
            ),
            (
                "{ catalogs, f }: let s = { org = catalogs.myorg; }; in f { inherit s; }",
                &["catalogs.myorg.*"],
            ),
            // A `rec { }` literal in value position escapes like a plain set;
            // its members still resolve against each other.
            ("{ catalogs, f }: f rec { org = catalogs.myorg; }", &[
                "catalogs.myorg.*",
            ]),
            (
                "{ catalogs, f }: f rec { org = catalogs.myorg; pkg = org.tool; }",
                &["catalogs.myorg.*", "catalogs.myorg.tool"],
            ),
            ("{ catalogs }: rec { org = catalogs.myorg; }", &[
                "catalogs.myorg.*",
            ]),
            // The @-name carries the root, so passing it whole escapes the
            // root like `f catalogs` does.
            ("args@{ catalogs, f, ... }: f args", &["catalogs.*"]),
            // A select reaching a nested modeled set escapes its members.
            (
                "{ catalogs, f }: let t = { sub = { org = catalogs.myorg; }; }; in f t.sub",
                &["catalogs.myorg.*"],
            ),
            // `with` over a modeled set: the body's names are not statically
            // bound, but the escaping members cover them.
            (
                "{ catalogs }: let s = { org = catalogs.myorg; }; in with s; org.pkg",
                &["catalogs.myorg.*"],
            ),
            // A dynamic member of a modeled set may be any member.
            (
                "{ catalogs, x }: let s = { org = catalogs.myorg; }; in s.${x}.pkg",
                &["catalogs.myorg.*"],
            ),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn unresolved_member_select_on_modeled_set_is_consumed() {
        let cases: &[(&str, &[&str])] = &[
            // Selecting an unknown member cannot leak the other members.
            (
                "{ catalogs }: let s = { org = catalogs.myorg; }; in s.unknown",
                &[],
            ),
            // The common `args.pname`-style access with an @-pattern.
            ("args@{ catalogs, ... }: args.pname", &[]),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn at_pattern_passed_to_known_lambda_escapes_only_uncovered_roots() {
        let cases: &[(&str, &[&str])] = &[
            // The lambda binds the root under the same name, so the body was
            // walked with it and the whole-set argument is consumed.
            (
                "args@{ catalogs, ... }: let mkPkg = { catalogs }: catalogs.myorg.inner; in mkPkg args",
                &["catalogs.myorg.inner"],
            ),
            // A lambda that binds the namespace under a different name
            // cannot be matched to the set's root member (and its body is
            // walked with an opaque parameter): the root escapes.
            (
                "args@{ catalogs, ... }: let mkPkg = { cats }: cats.myorg.inner; in mkPkg args",
                &["catalogs.*"],
            ),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn known_lambda_inherit_from_wrapper_is_consumed() {
        // `inherit (wrapper) catalogs;` binds the root under the lambda's own
        // `catalogs` parameter, so the body was walked with it: the argument
        // is consumed, not an escape.
        let got = refs(
            "{ catalogs }: let wrapper = { inherit catalogs; }; mkPkg = { catalogs }: catalogs.myorg.viaWrapper; in mkPkg { inherit (wrapper) catalogs; }",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.viaWrapper"]));
    }

    #[test]
    fn catalog_level_value_use_emits_sentinel() {
        // Passing a whole catalog to a function makes every member reachable;
        // an exact `catalogs.myorg` ref would be unresolvable.
        let got = refs(
            "{ catalogs }: builtins.attrValues catalogs.myorg",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.*"]));
    }

    #[test]
    fn select_or_default_scans_both_arms() {
        let got = refs(
            "{ catalogs }: catalogs.a.b or catalogs.c.d",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.a.b", "catalogs.c.d"]));
    }

    #[test]
    fn get_attr_key_subexpression_scanned() {
        let got = refs(
            "{ catalogs, f }: builtins.getAttr (f catalogs.a.key) catalogs.myorg",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.a.key", "catalogs.myorg.*"]));
    }

    #[test]
    fn dynamic_attr_interpolation_scanned() {
        let got = refs(
            "{ catalogs }: catalogs.myorg.${catalogs.a.name}",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.a.name", "catalogs.myorg.*"]));
    }

    #[test]
    fn ident_aliases_resolve_through_scope() {
        let cases: &[(&str, &[&str])] = &[
            // A whole-root alias: uses through it are ordinary refs.
            ("{ catalogs }: let c = catalogs; in c.myorg.pkg", &[
                "catalogs.myorg.pkg",
            ]),
            // An alias of an alias resolves transitively.
            ("{ catalogs }: let c = catalogs; d = c; in d.myorg.pkg", &[
                "catalogs.myorg.pkg",
            ]),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn inherit_from_root_and_alias_ident() {
        let cases: &[(&str, &[&str])] = &[
            // Inheriting a member of the bare root names a whole catalog,
            // which can never resolve as an exact ref — widen to a sentinel.
            ("{ catalogs }: { inherit (catalogs) myorg; }", &[
                "catalogs.myorg.*",
            ]),
            // The inherit source may be an alias ident, not just a select.
            (
                "{ catalogs }: let org = catalogs.myorg; in { inherit (org) toolkit; }",
                &["catalogs.myorg.toolkit"],
            ),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn binding_inherit_of_catalog_member_acts_as_alias() {
        let cases: &[(&str, &[&str])] = &[
            // A catalog-level member inherited in a binding scope only
            // defines an alias, like `let myorg = catalogs.myorg;` — the
            // alias's use sites drive the refs.
            (
                "{ catalogs }: let inherit (catalogs) myorg; in myorg.pkg",
                &["catalogs.myorg.pkg"],
            ),
            // An unused inherit binding is never evaluated and locks
            // nothing, like an unused alias binding.
            ("{ catalogs }: let inherit (catalogs) myorg; in null", &[]),
            // A quoted name the catalog cannot contain still collapses to a
            // sentinel at the source.
            (
                "{ catalogs }: let inherit (catalogs.myorg) \"a.b\"; in null",
                &["catalogs.myorg.*"],
            ),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn inherit_bound_name_acts_as_alias() {
        let got = refs(
            "{ catalogs }: let inherit (catalogs.myorg) toolkit; in toolkit.readVersion",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit",
                "catalogs.myorg.toolkit.readVersion"
            ])
        );
    }

    #[test]
    fn shadowed_root_names_do_not_emit() {
        // A let binding shadows a root for its whole scope.
        let got = refs(
            "{ catalogs }: let catalogs = { other.pkg = null; }; in catalogs.other.pkg",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn root_named_lambda_params_stay_rooted() {
        let cases: &[(&str, &[&str])] = &[
            // A helper lambda taking the namespace under the root's name and
            // applied to it: its body refs are real and must be locked.
            (
                "{ catalogs }: let mkPkg = { catalogs }: catalogs.myorg.helper-used; in mkPkg { inherit catalogs; }",
                &["catalogs.myorg.helper-used"],
            ),
            // A `rec { }` argument classifies its entries like a plain set:
            // the forwarded root is consumed, not an escape.
            (
                "{ catalogs }: let mkPkg = { catalogs }: catalogs.myorg.helper-used; in mkPkg rec { inherit catalogs; version = \"1\"; }",
                &["catalogs.myorg.helper-used"],
            ),
            // Even when the application passes something else, assuming the
            // parameter is the namespace can only add refs that fail loudly;
            // assuming the opposite would under-lock silently.
            (
                "{ catalogs }: let f = catalogs: catalogs.other.pkg; in f {}",
                &["catalogs.other.pkg"],
            ),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn at_pattern_bind_carries_the_pattern_entries() {
        let cases: &[(&str, &[&str])] = &[
            // The @-name is the whole argument set; the root member keeps its
            // root meaning through it.
            ("args@{ catalogs, ... }: args.catalogs.myorg.pkg", &[
                "catalogs.myorg.pkg",
            ]),
            // The bind may appear on either side of the pattern.
            ("{ catalogs, ... }@args: args.catalogs.myorg.pkg", &[
                "catalogs.myorg.pkg",
            ]),
            // A non-root member stays opaque.
            ("args@{ catalogs, pkgs, ... }: args.pkgs.foo", &[]),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn conditional_branch_refs_both_collected() {
        let got = refs(
            "{ catalogs, x }: if x then catalogs.a.p else catalogs.b.q",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.a.p", "catalogs.b.q"]));
    }

    #[test]
    fn nested_lambda_body_refs_collected() {
        let got = refs(
            "{ catalogs }: x: catalogs.myorg.pkg",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.pkg"]));
    }

    #[test]
    fn consumed_source_dynamic_components_scanned() {
        let cases: &[(&str, &[&str])] = &[
            // The inherit source is consumed, but the dynamic component
            // inside it holds a ref of its own.
            (
                "{ catalogs }: { inherit (catalogs.myorg.${catalogs.a.name}) x; }",
                &["catalogs.a.name", "catalogs.myorg.*"],
            ),
            (
                "{ catalogs }: with catalogs.${catalogs.a.name}; toolkit",
                &["catalogs.*", "catalogs.a.name"],
            ),
            (
                "{ catalogs }: builtins.getAttr \"k\" catalogs.myorg.${catalogs.a.name}",
                &["catalogs.a.name", "catalogs.myorg.*"],
            ),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn quoted_attrs_static_when_valid_names() {
        let cases: &[(&str, &[&str])] = &[
            // A non-interpolated string attr that is a valid catalog component
            // name is an ordinary static component.
            (r#"{ catalogs }: catalogs.myorg."with-dash".x"#, &[
                "catalogs.myorg.with-dash.x",
            ]),
            // A quoted attr containing `.` cannot exist as a catalog component
            // (server rejects dotted names), so it collapses to a sentinel.
            (r#"{ catalogs }: catalogs.myorg."foo.bar""#, &[
                "catalogs.myorg.*",
            ]),
            // Quoted inherit names are components too, not silently dropped.
            (
                r#"{ catalogs }: { inherit (catalogs.myorg) "with-dash" baz; }"#,
                &["catalogs.myorg.baz", "catalogs.myorg.with-dash"],
            ),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn dynamic_attr_emits_sentinel() {
        let got = refs(
            "{ catalogs, name }: catalogs.myorg.${name}",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.*"]));
    }

    #[test]
    fn dynamic_attr_at_first_component_emits_root_sentinel() {
        let got = refs(
            "{ catalogs, org }: catalogs.${org}.pkg",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.*"]));
    }

    #[test]
    fn dynamic_attr_stops_at_first_dynamic_component() {
        let got = refs(
            "{ catalogs, name }: catalogs.myorg.${name}.subpkg",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.*"]));
    }

    #[test]
    fn dynamic_attr_at_set_depth_emits_set_sentinel() {
        // The sentinel keeps the full static prefix: a dynamic member of a
        // package set widens to the set (`<catalog>.<set>.*`), not the whole
        // catalog.
        let got = refs(
            "{ catalogs, name }: catalogs.myorg.pythonPackages.${name}",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.pythonPackages.*"]));
    }

    #[test]
    fn get_attr_static_key_qualified() {
        let got = refs(
            "{ catalogs }: builtins.getAttr \"hello\" catalogs.myorg",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.hello"]));
    }

    #[test]
    fn get_attr_static_key_bare() {
        let got = refs(
            "{ catalogs }: with builtins; getAttr \"hello\" catalogs.myorg",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.hello"]));
    }

    #[test]
    fn get_attr_dynamic_key_emits_sentinel() {
        let got = refs(
            "{ catalogs, name }: builtins.getAttr name catalogs.myorg",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.*"]));
    }

    #[test]
    fn get_attr_with_alias_target() {
        let got = refs(
            "{ catalogs, name }: let org = catalogs.myorg; in builtins.getAttr \"hello\" org",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.hello"]));
    }

    #[test]
    fn get_attr_non_rooted_target_ignored() {
        let got = refs(
            "{ catalogs, someOtherAttrset }: builtins.getAttr \"hello\" someOtherAttrset",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn import_inherit_catalogs_follows_into_helper() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry.nix",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
    }

    #[test]
    fn import_explicit_catalogs_arg_followed() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-explicit.nix",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
    }

    #[test]
    fn import_without_catalogs_not_followed() {
        let got = refs(
            "{ catalogs }: let x = import ./import-helper.nix { foo = 1; }; in catalogs.myorg.pkg",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.pkg"]));
    }

    #[test]
    fn import_renamed_root_followed_and_rewritten() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-renamed.nix",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
    }

    #[test]
    fn import_arg_namespace_not_forwarded_escapes() {
        let cases: &[(&str, &[&str])] = &[
            // A catalog-level alias as an import argument is not forwarded
            // (only whole root_attributes are), so the child uses the namespace where
            // the scanner cannot see it: it escapes.
            (
                "{ catalogs }: let org = catalogs.myorg; in import ./x.nix { dep = org; }",
                &["catalogs.myorg.*"],
            ),
            (
                "{ catalogs }: let org = catalogs.myorg; in import ./x.nix { inherit org; }",
                &["catalogs.myorg.*"],
            ),
            // A modeled set argument escapes through its members.
            (
                "{ catalogs }: let s = { org = catalogs.myorg; }; in import ./x.nix { arg = s; }",
                &["catalogs.myorg.*"],
            ),
            // A package-deep value is an exact ref, not an escape.
            (
                "{ catalogs }: import ./x.nix { dep = catalogs.myorg.pkg; }",
                &["catalogs.myorg.pkg"],
            ),
            (
                "{ catalogs }: let pkg = catalogs.myorg.pkg; in import ./x.nix { inherit pkg; }",
                &["catalogs.myorg.pkg"],
            ),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn import_arg_forwarded_namespaces_are_consumed() {
        let cases: &[(&str, &[&str])] = &[
            ("{ catalogs }: import ./x.nix { inherit catalogs; }", &[]),
            ("{ catalogs }: import ./x.nix { cats = catalogs; }", &[]),
            // Forwarding a root that lives in a member of a modeled set.
            (
                "{ catalogs }: let wrapper = { inherit catalogs; }; in import ./x.nix { inherit (wrapper) catalogs; }",
                &[],
            ),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn import_whole_root_argument_followed() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-whole.nix",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.whole-pkg"]));
    }

    #[test]
    fn import_whole_root_to_pattern_param_escapes() {
        // The helper destructures the namespace with a pattern parameter;
        // its entries are namespace members, not root_attributes, so they cannot be
        // bound statically and the whole root escapes rather than being
        // dropped.
        let got = refs_at(
            "test_data/catalog_refs/import-entry-pattern.nix",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.*"]));
    }

    #[test]
    fn import_root_name_bound_to_other_value_not_followed() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-shadowed.nix",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.direct-pkg"]));
    }

    #[test]
    fn import_directory_target_resolves_default_nix() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-dir.nix",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.dir-pkg"]));
    }

    #[test]
    fn import_dynamic_path_forwarding_root_is_conservative() {
        // The import target cannot be read, so the forwarded namespace
        // escapes analysis (a warning points at the dynamic path).
        let got = refs(
            "{ catalogs, p }: import p { inherit catalogs; }",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.*"]));
    }

    #[test]
    fn import_same_file_under_different_forwards_scans_both() {
        let got = refs_at(
            "test_data/catalog_refs/diamond-import/entry.nix",
            &root_attributes(&["catalogs", "inputs"]),
        );
        assert_eq!(
            got,
            set(&["catalogs.somepkg.someattr", "inputs.somepkg.someattr"])
        );
    }

    #[test]
    fn import_same_file_under_composed_forwards_scans_both() {
        // The two chains reach common.nix with textually identical immediate
        // forwardings (`{ ns = cats; }`) that compose to different top-level
        // root_attributes; both compositions must be scanned.
        let got = refs_at(
            "test_data/catalog_refs/deep-diamond/entry.nix",
            &root_attributes(&["catalogs", "inputs"]),
        );
        assert_eq!(
            got,
            set(&["catalogs.somepkg.someattr", "inputs.somepkg.someattr"])
        );
    }

    #[test]
    fn import_cycle_terminates_and_collects_refs() {
        let got = refs_at(
            "test_data/catalog_refs/import-cycle/entry.nix",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(
            got,
            set(&["catalogs.myorg.cycle-pkg", "catalogs.myorg.entry-pkg"])
        );
    }

    #[test]
    fn import_unreadable_target_fails_scan() {
        // The import target cannot be read, so the refs it would contribute
        // through the forwarded namespaces cannot be discovered; the scan
        // fails rather than silently under-locking — for both forwarding
        // shapes.
        let cases = [
            ("test_data/catalog_refs/import-entry-unreadable.nix", (4, 1)),
            (
                "test_data/catalog_refs/import-entry-unreadable-whole.nix",
                (5, 1),
            ),
        ];
        for (path, position) in cases {
            let err = scan_err_at(path, &root_attributes(&["catalogs"]));
            assert_eq!(
                err,
                ScanError::UnreadableImport {
                    target: PathBuf::from("test_data/catalog_refs/no-such-helper.nix"),
                    file: PathBuf::from(path),
                    position,
                },
                "fixture: {path}"
            );
        }
    }

    #[test]
    fn unreadable_import_error_message_points_at_the_import() {
        let err = ScanError::UnreadableImport {
            target: PathBuf::from("pkgs/helper.nix"),
            file: PathBuf::from("pkgs/foo.nix"),
            position: (4, 1),
        };
        assert_eq!(err.to_string(), indoc::indoc! {"
                'pkgs/helper.nix' is imported at pkgs/foo.nix:4:1 but cannot be read.
                Check that the imported file exists and is readable."});
    }

    #[test]
    fn import_let_bound_function_followed() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-letbound.nix",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
    }

    #[test]
    fn import_direct_refs_in_entry_still_collected() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-with-direct-ref.nix",
            &root_attributes(&["catalogs"]),
        );
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.extra-pkg",
                "catalogs.myorg.toolkit.readVersion",
            ])
        );
    }

    #[test]
    fn undeclared_root_reference_is_an_error() {
        // Each case references `catalogs` through a different emit path while
        // the top-level lambda does not declare it; the expected position is
        // the first use.
        let cases = [
            (
                "direct select",
                "{ mkDerivation }:\nmkDerivation {\n  version = catalogs.myorg.toolkit.readVersion;\n}\n",
                (3, 13),
            ),
            (
                "namespace escaping as a value",
                "{ mkDerivation }:\nmkDerivation {\n  passthru = catalogs;\n}\n",
                (3, 14),
            ),
            (
                "inherit from the namespace",
                "{ mkDerivation }:\nlet\n  inherit (catalogs.myorg.toolkit) readVersion;\nin\nmkDerivation { version = readVersion; }\n",
                (3, 36),
            ),
            (
                "with over the namespace",
                "{ mkDerivation }:\nwith catalogs;\nmkDerivation { }\n",
                (2, 1),
            ),
        ];
        for (label, content, position) in cases {
            assert_eq!(
                scan_err(content, &root_attributes(&["catalogs"])),
                ScanError::UndeclaredRoot {
                    root: "catalogs".to_string(),
                    file: PathBuf::from("test.nix"),
                    position: Some(position),
                },
                "{label}",
            );
        }
    }

    #[test]
    fn root_named_inner_param_only_rooted_when_file_can_receive_root() {
        let cases: &[(&str, &[&str])] = &[
            // The file's top level cannot receive `catalogs`, so an inner
            // parameter of that name cannot be the namespace; its uses are
            // not refs and must not trip the undeclared-root rejection.
            (
                "{ mkDerivation }: let f = catalogs: catalogs.version; in mkDerivation { v = f { version = \"1\"; }; }",
                &[],
            ),
            // An unrecognized top-level shape fails open: the parameter
            // keeps its root meaning.
            ("let f = catalogs: catalogs.other.pkg; in f {}", &[
                "catalogs.other.pkg",
            ]),
        ];
        for (content, expected) in cases {
            assert_eq!(
                refs(content, &root_attributes(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn undeclared_root_without_references_is_not_an_error() {
        let content = "{ mkDerivation }: mkDerivation { pname = \"tool\"; }";
        assert_eq!(
            refs(content, &root_attributes(&["catalogs"])),
            BTreeSet::new()
        );
    }

    #[test]
    fn let_bound_root_name_shadows_without_error() {
        let content = "{ config }:\nlet catalogs = config;\nin catalogs.myorg.toolkit.readVersion";
        assert_eq!(
            refs(content, &root_attributes(&["catalogs"])),
            BTreeSet::new()
        );
    }

    #[test]
    fn unrecognized_top_level_shape_scans_leniently() {
        // A let-wrapped file still evaluates to a function, but the scanner
        // cannot see its parameters; the declaration check fails open and the
        // refs are kept.
        let content = "let version = \"1.0\";\nin { mkDerivation }:\nmkDerivation { v = catalogs.myorg.pkg.readVersion; }";
        assert_eq!(
            refs(content, &root_attributes(&["catalogs"])),
            set(&["catalogs.myorg.pkg.readVersion"])
        );
    }

    #[test]
    fn undeclared_root_forwarded_to_import_errors_at_forward_site() {
        let path = Path::new("test_data/catalog_refs/undeclared-forward/entry.nix");
        let content = fs::read_to_string(path).expect("test fixture missing");
        let err = analyze_file_at(
            &content,
            &root_attributes(&["catalogs"]),
            path.parent(),
            &mut HashMap::new(),
            path,
            &identity_origins(&root_attributes(&["catalogs"])),
        )
        .expect_err("scan should fail");
        assert_eq!(err, ScanError::UndeclaredRoot {
            root: "catalogs".to_string(),
            file: path.to_path_buf(),
            position: Some((6, 35)),
        });
    }

    #[test]
    fn undeclared_root_error_message_points_at_the_arguments() {
        let err = ScanError::UndeclaredRoot {
            root: "catalogs".to_string(),
            file: PathBuf::from("pkgs/foo.nix"),
            position: Some((4, 13)),
        };
        assert_eq!(err.to_string(), indoc::indoc! {"
                'catalogs' is referenced at pkgs/foo.nix:4:13 but is not declared in the function arguments.
                Add 'catalogs' to the function arguments, e.g. '{ catalogs, ... }:'."});
    }
}
