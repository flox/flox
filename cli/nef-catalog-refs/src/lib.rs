use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use rnix::ast;
use rnix::ast::HasEntry as _;
use rowan::ast::AstNode;

#[derive(Debug)]
pub struct FileInfo {
    pub refs: BTreeSet<String>,
    pub dep_args: Vec<String>,
}

pub fn parse_file(path: &Path, roots: &HashSet<String>) -> HashMap<String, FileInfo> {
    let mut db = HashMap::new();
    if let Ok(content) = fs::read_to_string(path) {
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let dir = path.parent();
        let mut visited = HashSet::new();
        db.insert(stem, analyze_file_at(&content, roots, dir, &mut visited));
    }
    db
}

pub fn parse_dir(dir: &Path, roots: &HashSet<String>) -> HashMap<String, FileInfo> {
    let mut db = HashMap::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return db;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "nix") {
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if let Ok(content) = fs::read_to_string(&path) {
                let mut visited = HashSet::new();
                db.insert(
                    stem,
                    analyze_file_at(&content, roots, path.parent(), &mut visited),
                );
            }
        }
    }
    db
}

pub fn analyze_file(content: &str, roots: &HashSet<String>) -> FileInfo {
    analyze_file_at(content, roots, None, &mut HashSet::new())
}

pub fn analyze_file_at(
    content: &str,
    roots: &HashSet<String>,
    file_dir: Option<&Path>,
    visited: &mut HashSet<PathBuf>,
) -> FileInfo {
    let parse = rnix::Root::parse(content);
    let root = parse.tree();

    let mut refs = BTreeSet::new();
    let mut dep_args = Vec::new();

    if let Some(rnix::ast::Expr::Lambda(lambda)) = root.expr()
        && let Some(rnix::ast::Param::Pattern(pat)) = lambda.param()
    {
        for entry in pat.pat_entries() {
            if let Some(ident) = entry.ident()
                && let Some(name) = ident.ident_token().map(|t| t.text().to_string())
                && !roots.contains(&name)
            {
                dep_args.push(name);
            }
        }
    }

    let aliases = collect_aliases(root.syntax(), roots);
    collect_refs(root.syntax(), &mut refs, roots, &aliases);

    if let Some(dir) = file_dir {
        follow_imports(root.syntax(), roots, &aliases, dir, visited, &mut refs);
    }

    FileInfo { refs, dep_args }
}

pub fn collect_transitive(
    mut db: HashMap<String, FileInfo>,
    dir: &Path,
    roots: &HashSet<String>,
) -> BTreeSet<String> {
    let mut visited: HashSet<String> = HashSet::new();
    let mut result: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<String> = db.keys().cloned().collect();

    while let Some(name) = queue.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        if !db.contains_key(&name)
            && let Some(info) = load_dep(dir, &name, roots)
        {
            db.insert(name.clone(), info);
        }
        let Some(info) = db.get(&name) else { continue };
        result.extend(info.refs.iter().cloned());
        let dep_args: Vec<String> = info.dep_args.clone();
        for dep in dep_args {
            if !visited.contains(&dep) {
                queue.push(dep);
            }
        }
    }

    result
}

fn collect_aliases(root: &rnix::SyntaxNode, roots: &HashSet<String>) -> HashMap<String, String> {
    let mut aliases: HashMap<String, String> = HashMap::new();
    let mut changed = true;
    while changed {
        changed = false;
        gather_let_aliases(root, roots, &mut aliases, &mut changed);
    }
    aliases
}

fn gather_let_aliases(
    node: &rnix::SyntaxNode,
    roots: &HashSet<String>,
    aliases: &mut HashMap<String, String>,
    changed: &mut bool,
) {
    if let Some(let_in) = ast::LetIn::cast(node.clone()) {
        for entry in let_in.attrpath_values() {
            let Some(lhs) = entry.attrpath() else {
                continue;
            };
            let attrs: Vec<ast::Attr> = lhs.attrs().collect();
            if attrs.len() != 1 {
                continue;
            }
            let ast::Attr::Ident(ref id) = attrs[0] else {
                continue;
            };
            let Some(name) = id.ident_token().map(|t| t.text().to_string()) else {
                continue;
            };
            if aliases.contains_key(&name) {
                continue;
            }
            let Some(ast::Expr::Select(select)) = entry.value() else {
                continue;
            };
            if let Some(path) = extract_ref_path(&select, roots, aliases) {
                aliases.insert(name, path);
                *changed = true;
            }
        }
    }
    for child in node.children() {
        gather_let_aliases(&child, roots, aliases, changed);
    }
}

fn follow_imports(
    node: &rnix::SyntaxNode,
    roots: &HashSet<String>,
    aliases: &HashMap<String, String>,
    file_dir: &Path,
    visited: &mut HashSet<PathBuf>,
    refs: &mut BTreeSet<String>,
) {
    if let Some(apply) = ast::Apply::cast(node.clone())
        && let Some((path_str, import_roots)) = try_extract_import(&apply, roots, aliases)
    {
        let target = file_dir.join(&path_str);
        let target = fs::canonicalize(&target).unwrap_or(target);
        if !visited.contains(&target) {
            visited.insert(target.clone());
            if let Ok(content) = fs::read_to_string(&target) {
                let import_dir = target.parent().map(Path::to_path_buf);
                let imported =
                    analyze_file_at(&content, &import_roots, import_dir.as_deref(), visited);
                refs.extend(imported.refs);
            }
        }
    }
    for child in node.children() {
        follow_imports(&child, roots, aliases, file_dir, visited, refs);
    }
}

fn try_extract_import(
    apply: &ast::Apply,
    roots: &HashSet<String>,
    aliases: &HashMap<String, String>,
) -> Option<(String, HashSet<String>)> {
    let inner = match apply.lambda()? {
        ast::Expr::Apply(a) => a,
        _ => return None,
    };
    let ast::Expr::Ident(import_fn) = inner.lambda()? else {
        return None;
    };
    if import_fn.ident_token()?.text() != "import" {
        return None;
    }
    let path_str = static_path_str(&inner.argument()?)?;
    let ast::Expr::AttrSet(attrset) = apply.argument()? else {
        return None;
    };
    let passed = roots_passed_to_import(&attrset, roots, aliases);
    if passed.is_empty() {
        return None;
    }
    Some((path_str, passed))
}

fn static_path_str(expr: &ast::Expr) -> Option<String> {
    match expr {
        ast::Expr::PathRel(p) => Some(p.syntax().text().to_string()),
        ast::Expr::PathAbs(p) => Some(p.syntax().text().to_string()),
        ast::Expr::Str(_) => static_str(expr),
        _ => None,
    }
}

fn roots_passed_to_import(
    attrset: &ast::AttrSet,
    roots: &HashSet<String>,
    _aliases: &HashMap<String, String>,
) -> HashSet<String> {
    let mut passed = HashSet::new();
    for inherit in attrset.inherits() {
        if inherit.from().is_some() {
            continue;
        }
        for attr in inherit.attrs() {
            if let ast::Attr::Ident(id) = attr
                && let Some(name) = id.ident_token().map(|t| t.text().to_string())
                && roots.contains(&name)
            {
                passed.insert(name);
            }
        }
    }
    for apv in attrset.attrpath_values() {
        let Some(lhs) = apv.attrpath() else { continue };
        let attrs: Vec<ast::Attr> = lhs.attrs().collect();
        if attrs.len() != 1 {
            continue;
        }
        if let ast::Attr::Ident(id) = &attrs[0]
            && let Some(name) = id.ident_token().map(|t| t.text().to_string())
            && roots.contains(&name)
        {
            passed.insert(name);
        }
    }
    passed
}

fn collect_refs(
    node: &rnix::SyntaxNode,
    refs: &mut BTreeSet<String>,
    roots: &HashSet<String>,
    aliases: &HashMap<String, String>,
) {
    if let Some(inherit) = ast::Inherit::cast(node.clone())
        && try_handle_inherit(&inherit, refs, roots, aliases)
    {
        return;
    }

    if let Some(with_expr) = ast::With::cast(node.clone())
        && let Some(ns) = with_expr.namespace()
        && let Some(path) = namespace_path(&ns, roots, aliases)
    {
        refs.insert(format!("{}.*", path));
        if let Some(body) = with_expr.body() {
            collect_refs(body.syntax(), refs, roots, aliases);
        }
        return;
    }

    if let Some(apply) = ast::Apply::cast(node.clone())
        && let Some(path) = try_handle_get_attr(&apply, roots, aliases)
    {
        refs.insert(path);
        return;
    }

    if let Some(select) = ast::Select::cast(node.clone())
        && let Some(path) = extract_ref_path(&select, roots, aliases)
    {
        refs.insert(path);
        return;
    }

    for child in node.children() {
        collect_refs(&child, refs, roots, aliases);
    }
}

fn namespace_path(
    expr: &ast::Expr,
    roots: &HashSet<String>,
    aliases: &HashMap<String, String>,
) -> Option<String> {
    match expr {
        ast::Expr::Select(select) => extract_ref_path(select, roots, aliases),
        ast::Expr::Ident(ident) => {
            let name = ident.ident_token()?.text().to_string();
            if roots.contains(&name) {
                Some(name)
            } else {
                aliases.get(&name).cloned()
            }
        },
        _ => None,
    }
}

fn try_handle_get_attr(
    apply: &ast::Apply,
    roots: &HashSet<String>,
    aliases: &HashMap<String, String>,
) -> Option<String> {
    let inner = match apply.lambda()? {
        ast::Expr::Apply(a) => a,
        _ => return None,
    };
    if !is_get_attr_fn(&inner.lambda()?) {
        return None;
    }
    let base_path = namespace_path(&apply.argument()?, roots, aliases)?;
    match static_str(&inner.argument()?) {
        Some(key) => Some(format!("{}.{}", base_path, key)),
        None => Some(format!("{}.*", base_path)),
    }
}

fn is_get_attr_fn(expr: &ast::Expr) -> bool {
    match expr {
        ast::Expr::Select(sel) => {
            let Some(ast::Expr::Ident(base)) = sel.expr() else {
                return false;
            };
            if base.ident_token().is_none_or(|t| t.text() != "builtins") {
                return false;
            }
            let Some(path) = sel.attrpath() else {
                return false;
            };
            let attrs: Vec<ast::Attr> = path.attrs().collect();
            attrs.len() == 1
                && matches!(&attrs[0], ast::Attr::Ident(id)
                    if id.ident_token().is_some_and(|t| t.text() == "getAttr"))
        },
        ast::Expr::Ident(id) => id.ident_token().is_some_and(|t| t.text() == "getAttr"),
        _ => false,
    }
}

fn static_str(expr: &ast::Expr) -> Option<String> {
    let ast::Expr::Str(s) = expr else { return None };
    if s.syntax().children().next().is_some() {
        return None;
    }
    s.syntax().children_with_tokens().find_map(|n| {
        if let rowan::NodeOrToken::Token(t) = n
            && t.kind() == rnix::SyntaxKind::TOKEN_STRING_CONTENT
        {
            return Some(t.text().to_string());
        }
        None
    })
}

fn try_handle_inherit(
    inherit: &ast::Inherit,
    refs: &mut BTreeSet<String>,
    roots: &HashSet<String>,
    aliases: &HashMap<String, String>,
) -> bool {
    let Some(from) = inherit.from() else {
        return false;
    };
    let Some(from_expr) = from.expr() else {
        return false;
    };
    let ast::Expr::Select(select) = from_expr else {
        return false;
    };
    let Some(base_path) = extract_ref_path(&select, roots, aliases) else {
        return false;
    };

    for attr in inherit.attrs() {
        if let ast::Attr::Ident(id) = attr
            && let Some(token) = id.ident_token()
        {
            refs.insert(format!("{}.{}", base_path, token.text()));
        }
    }
    true
}

fn extract_ref_path(
    select: &ast::Select,
    roots: &HashSet<String>,
    aliases: &HashMap<String, String>,
) -> Option<String> {
    let expr = select.expr()?;
    let ast::Expr::Ident(base) = expr else {
        return None;
    };
    let base_name = base.ident_token()?.text().to_string();

    let base_path = if roots.contains(&base_name) {
        base_name
    } else if let Some(alias) = aliases.get(&base_name) {
        alias.clone()
    } else {
        return None;
    };

    let attrpath = select.attrpath()?;
    let mut parts = vec![base_path];
    for attr in attrpath.attrs() {
        match attr {
            ast::Attr::Ident(id) => {
                parts.push(id.ident_token()?.text().to_string());
            },
            _ => {
                parts.push("*".to_string());
                break;
            },
        }
    }
    Some(parts.join("."))
}

fn load_dep(dir: &Path, name: &str, roots: &HashSet<String>) -> Option<FileInfo> {
    let candidates = [
        dir.join(format!("{}.nix", name)),
        dir.join(name).join("default.nix"),
    ];
    for path in &candidates {
        if path.is_file()
            && let Ok(content) = fs::read_to_string(path)
        {
            return Some(analyze_file(&content, roots));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn catalog_roots() -> HashSet<String> {
        roots(&["catalogs"])
    }

    fn input_roots() -> HashSet<String> {
        roots(&["inputs"])
    }

    fn both_roots() -> HashSet<String> {
        roots(&["catalogs", "inputs"])
    }

    fn refs(content: &str, roots: &HashSet<String>) -> BTreeSet<String> {
        analyze_file(content, roots).refs
    }

    fn refs_at(path: &str, roots: &HashSet<String>) -> BTreeSet<String> {
        let path = Path::new(path);
        let content = fs::read_to_string(path).expect("test fixture missing");
        let dir = path.parent();
        let mut visited = HashSet::new();
        analyze_file_at(&content, roots, dir, &mut visited).refs
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_catalog_refs_fetchpypi() {
        let got = refs(
            include_str!("../test_data/catalog_refs/no-catalog-refs.nix"),
            &catalog_roots(),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn no_catalog_refs_rust_package() {
        let got = refs(
            include_str!("../test_data/catalog_refs/rust-no-catalog.nix"),
            &catalog_roots(),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn non_catalog_inherit_not_collected() {
        let content = include_str!("../test_data/catalog_refs/non-catalog-inherit.nix");
        assert_eq!(refs(content, &catalog_roots()), BTreeSet::new());
        assert_eq!(refs(content, &input_roots()), BTreeSet::new());
    }

    #[test]
    fn single_inherit_helper() {
        let got = refs(
            include_str!("../test_data/catalog_refs/single-inherit-helper.nix"),
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
    }

    #[test]
    fn two_inherits_toolkit_and_python_pkg() {
        let got = refs(
            include_str!("../test_data/catalog_refs/two-inherits.nix"),
            &catalog_roots(),
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
            include_str!("../test_data/catalog_refs/multi-attr-inherit.nix"),
            &catalog_roots(),
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
            include_str!("../test_data/catalog_refs/multi-attr-inherit.nix"),
            &catalog_roots(),
        );
        assert!(!got.contains("catalogs.myorg.python3Packages"));
        assert!(!got.contains("catalogs.myorg.toolkit"));
    }

    #[test]
    fn direct_select_native_packages() {
        let got = refs(
            include_str!("../test_data/catalog_refs/direct-select-native.nix"),
            &catalog_roots(),
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
            include_str!("../test_data/catalog_refs/inherit-subattrset.nix"),
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit"]));
    }

    #[test]
    fn nested_inline_package_does_not_hide_outer_refs() {
        let got = refs(
            include_str!("../test_data/catalog_refs/nested-inline-package.nix"),
            &catalog_roots(),
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
            include_str!("../test_data/catalog_refs/passthru-src-access.nix"),
            &catalog_roots(),
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
            include_str!("../test_data/catalog_refs/inputs-only.nix"),
            &input_roots(),
        );
        assert_eq!(
            got,
            set(&[
                "inputs.nixpkgs.lib",
                "inputs.devtools-flake.packages.default",
                "inputs.self",
            ])
        );
    }

    #[test]
    fn inputs_only_with_catalog_roots_returns_nothing() {
        let got = refs(
            include_str!("../test_data/catalog_refs/inputs-only.nix"),
            &catalog_roots(),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn mixed_roots_catalog_only() {
        let got = refs(
            include_str!("../test_data/catalog_refs/mixed-roots.nix"),
            &catalog_roots(),
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
            include_str!("../test_data/catalog_refs/mixed-roots.nix"),
            &input_roots(),
        );
        assert_eq!(
            got,
            set(&[
                "inputs.nixpkgs.lib",
                "inputs.devtools-flake.packages.default",
            ])
        );
    }

    #[test]
    fn mixed_roots_both() {
        let got = refs(
            include_str!("../test_data/catalog_refs/mixed-roots.nix"),
            &both_roots(),
        );
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit.readVersion",
                "catalogs.myorg.python3Packages.alpha-lib",
                "inputs.nixpkgs.lib",
                "inputs.devtools-flake.packages.default",
            ])
        );
    }

    #[test]
    fn transitive_follows_intra_dir_dep_args() {
        let r = catalog_roots();
        let file_a = "{ catalogs, beta-client }: catalogs.myorg.toolkit.readVersion";
        let file_b = "{ catalogs }: catalogs.myorg.python3Packages.gamma-service";

        let mut db = HashMap::new();
        db.insert("alpha-lib".to_string(), analyze_file(file_a, &r));
        db.insert("beta-client".to_string(), analyze_file(file_b, &r));

        let got = collect_transitive(db, Path::new("."), &r);
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.toolkit.readVersion",
                "catalogs.myorg.python3Packages.gamma-service",
            ])
        );
    }

    #[test]
    fn transitive_cycle_safe() {
        let r = catalog_roots();
        let file_a = "{ catalogs, pkg-b }: catalogs.myorg.x";
        let file_b = "{ catalogs, pkg-a }: catalogs.myorg.y";

        let mut db = HashMap::new();
        db.insert("pkg-a".to_string(), analyze_file(file_a, &r));
        db.insert("pkg-b".to_string(), analyze_file(file_b, &r));

        let got = collect_transitive(db, Path::new("."), &r);
        assert_eq!(got, set(&["catalogs.myorg.x", "catalogs.myorg.y"]));
    }

    #[test]
    fn transitive_inputs_root() {
        let r = input_roots();
        let file_a = "{ inputs, dep-pkg }: inputs.nixpkgs.lib";
        let file_b = "{ inputs }: inputs.devtools-flake.packages.default";

        let mut db = HashMap::new();
        db.insert("main-pkg".to_string(), analyze_file(file_a, &r));
        db.insert("dep-pkg".to_string(), analyze_file(file_b, &r));

        let got = collect_transitive(db, Path::new("."), &r);
        assert_eq!(
            got,
            set(&[
                "inputs.nixpkgs.lib",
                "inputs.devtools-flake.packages.default",
            ])
        );
    }

    #[test]
    fn with_direct_namespace_emits_sentinel() {
        let got = refs(
            include_str!("../test_data/catalog_refs/with-namespace.nix"),
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg.*"]));
    }

    #[test]
    fn with_namespace_does_not_emit_bare_path() {
        let got = refs(
            include_str!("../test_data/catalog_refs/with-namespace.nix"),
            &catalog_roots(),
        );
        assert!(!got.contains("catalogs.myorg"));
    }

    #[test]
    fn with_alias_namespace_emits_sentinel() {
        let got = refs(
            "{ catalogs }: let org = catalogs.myorg; in with org; toolkit",
            &catalog_roots(),
        );
        assert!(got.contains("catalogs.myorg.*"), "got: {:?}", got);
    }

    #[test]
    fn with_non_rooted_namespace_falls_through() {
        let got = refs(
            "{ catalogs }: with { x = 1; }; catalogs.myorg.pkg",
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg.pkg"]));
    }

    #[test]
    fn with_body_direct_refs_still_collected() {
        let got = refs(
            "{ catalogs }: with catalogs.myorg; catalogs.other.pkg",
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg.*", "catalogs.other.pkg"]));
    }

    #[test]
    fn aliased_select_single_level() {
        let got = refs(
            "{ catalogs }: let org = catalogs.myorg; in org.toolkit",
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg", "catalogs.myorg.toolkit"]));
    }

    #[test]
    fn aliased_select_chained() {
        let got = refs(
            include_str!("../test_data/catalog_refs/aliased-select.nix"),
            &catalog_roots(),
        );
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg",
                "catalogs.myorg.toolkit",
                "catalogs.myorg.toolkit.readVersion",
            ])
        );
    }

    #[test]
    fn aliased_select_order_independent() {
        let got = refs(
            "{ catalogs }: let b = a.hello; a = catalogs.myorg; in b",
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg", "catalogs.myorg.hello"]));
    }

    #[test]
    fn dynamic_attr_emits_sentinel() {
        let got = refs(
            "{ catalogs, name }: catalogs.myorg.${name}",
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg.*"]));
    }

    #[test]
    fn dynamic_attr_at_first_component_emits_root_sentinel() {
        let got = refs("{ catalogs, org }: catalogs.${org}.pkg", &catalog_roots());
        assert_eq!(got, set(&["catalogs.*"]));
    }

    #[test]
    fn dynamic_attr_stops_at_first_dynamic_component() {
        let got = refs(
            "{ catalogs, name }: catalogs.myorg.${name}.subpkg",
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg.*"]));
    }

    #[test]
    fn get_attr_static_key_qualified() {
        let got = refs(
            "{ catalogs }: builtins.getAttr \"hello\" catalogs.myorg",
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg.hello"]));
    }

    #[test]
    fn get_attr_static_key_bare() {
        let got = refs(
            "{ catalogs }: getAttr \"hello\" catalogs.myorg",
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg.hello"]));
    }

    #[test]
    fn get_attr_dynamic_key_emits_sentinel() {
        let got = refs(
            "{ catalogs, name }: builtins.getAttr name catalogs.myorg",
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg.*"]));
    }

    #[test]
    fn get_attr_with_alias_target() {
        let got = refs(
            "{ catalogs, name }: let org = catalogs.myorg; in builtins.getAttr \"hello\" org",
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg", "catalogs.myorg.hello"]));
    }

    #[test]
    fn get_attr_non_rooted_target_ignored() {
        let got = refs(
            "{ catalogs }: builtins.getAttr \"hello\" someOtherAttrset",
            &catalog_roots(),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn import_inherit_catalogs_follows_into_helper() {
        let got = refs_at("test_data/catalog_refs/import-entry.nix", &catalog_roots());
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
    }

    #[test]
    fn import_explicit_catalogs_arg_followed() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-explicit.nix",
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
    }

    #[test]
    fn import_without_catalogs_not_followed() {
        let got = refs(
            "{ catalogs }: let x = import ./import-helper.nix { foo = 1; }; in catalogs.myorg.pkg",
            &catalog_roots(),
        );
        assert_eq!(got, set(&["catalogs.myorg.pkg"]));
    }

    #[test]
    fn import_direct_refs_in_entry_still_collected() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-with-direct-ref.nix",
            &catalog_roots(),
        );
        assert_eq!(
            got,
            set(&[
                "catalogs.myorg.extra-pkg",
                "catalogs.myorg.toolkit.readVersion",
            ])
        );
    }
}
