use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::{self, Display};
use std::fs;
use std::path::{Path, PathBuf};

use rnix::ast;
use rnix::ast::HasEntry as _;
use rowan::ast::AstNode;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

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

/// Catalog references and dependency attr-paths extracted from one file.
#[derive(Debug)]
struct FileInfo {
    /// Fully-qualified catalog attr-paths referenced by the file
    /// (e.g. `catalogs.myorg.toolkit.readVersion`).
    refs: BTreeSet<String>,
    /// Attr-paths of the packages this file depends on, resolved by
    /// [collect_transitive]. The first component is the dependency argument;
    /// any further components are members selected on it (a sibling attribute
    /// set), e.g. `["python3Packages", "isdr-zk-client"]` for
    /// `python3Packages.isdr-zk-client`. A bare argument is a single component.
    deps: Vec<Vec<String>>,
}

/// Catalog root parameter names assumed by [scan_package].
///
/// A NEF package receives the catalog namespace as the `catalogs` lambda
/// parameter; attribute paths reached through it (`catalogs.<org>.<pkg>…`) are
/// the references that must be locked. Use [scan_package_with_roots] to scan
/// against a different set of roots.
const DEFAULT_ROOTS: &[&str] = &["catalogs"];

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
/// Uses the default `catalogs` root; see [scan_package_with_roots] to override.
pub fn scan_package(
    base_dir: impl AsRef<Path>,
    rel_file: impl AsRef<Path>,
) -> BTreeSet<CatalogRef> {
    scan_package_with_roots(base_dir, rel_file, DEFAULT_ROOTS.iter().copied())
}

/// [scan_package] generalized over the set of catalog root parameter names.
///
/// `roots` are the lambda-parameter names treated as catalog namespaces; every
/// other parameter is a dependency argument followed to a sibling package.
/// Any iterable of names is accepted; duplicates are harmless.
#[instrument(
    skip(roots),
    fields(
        base_dir = %base_dir.as_ref().display(),
        rel_file = %rel_file.as_ref().display(),
    )
)]
pub fn scan_package_with_roots(
    base_dir: impl AsRef<Path>,
    rel_file: impl AsRef<Path>,
    roots: impl IntoIterator<Item = impl Into<String>>,
) -> BTreeSet<CatalogRef> {
    let roots: HashSet<String> = roots.into_iter().map(Into::into).collect();
    let roots = &roots;

    // Imports in the target resolve relative to its own directory; dependency
    // arguments resolve as siblings under base_dir.
    let db = {
        let path: &Path = &base_dir.as_ref().join(rel_file.as_ref());
        let mut db = HashMap::new();
        if let Ok(content) = fs::read_to_string(path) {
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let dir = path.parent();
            let mut visited = HashSet::new();
            db.insert(
                stem,
                analyze_file_at(&content, roots, dir, &mut visited, Some(path)),
            );
        }
        db
    };
    let references: BTreeSet<CatalogRef> = collect_transitive(db, base_dir.as_ref(), roots)
        .into_iter()
        .map(CatalogRef)
        .collect();
    debug!(references = references.len(), "scanned catalog references");
    references
}

/// Source context for verbose reference reporting: the file a reference was
/// found in (when known) plus its text, used to turn a byte offset into a
/// 1-based `line:column`.
#[derive(Clone, Debug)]
struct ScanCtx<'a> {
    path: Option<&'a Path>,
    content: &'a str,
}

impl ScanCtx<'_> {
    /// Emit a `debug` event locating one discovered reference at `offset` (a
    /// byte offset into the file). Surfaced by `lock --verbose`.
    fn report(&self, offset: usize, reference: &str) {
        let (line, column) = line_col(self.content, offset);
        match self.path {
            Some(path) => {
                debug!(reference, file = %path.display(), line, column, "catalog reference")
            },
            None => debug!(reference, line, column, "catalog reference"),
        }
    }
}

/// Resolve a byte `offset` into `content` to a 1-based `(line, column)`.
fn line_col(content: &str, offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for (idx, ch) in content.char_indices() {
        if idx >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

/// Analyze one file's content, collecting catalog refs and the dependency
/// arguments it pulls in.
///
/// `roots` are the lambda parameters treated as catalog roots (e.g. `catalogs`).
/// When `file_dir` is `Some`, `import` calls forwarding a root are followed into
/// the imported file; `visited` guards against import cycles. `path` is the
/// file's location, used only for verbose reference reporting.
fn analyze_file_at(
    content: &str,
    roots: &HashSet<String>,
    file_dir: Option<&Path>,
    visited: &mut HashSet<PathBuf>,
    path: Option<&Path>,
) -> FileInfo {
    if let Some(path) = path {
        debug!(file = %path.display(), "reading NEF expression");
    }

    let parse = rnix::Root::parse(content);
    let root = parse.tree();

    let mut refs = BTreeSet::new();

    let mut dep_names = HashSet::new();
    if let Some(rnix::ast::Expr::Lambda(lambda)) = root.expr()
        && let Some(rnix::ast::Param::Pattern(pat)) = lambda.param()
    {
        for entry in pat.pat_entries() {
            if let Some(ident) = entry.ident()
                && let Some(name) = ident.ident_token().map(|t| t.text().to_string())
                && !roots.contains(name.as_str())
            {
                dep_names.insert(name);
            }
        }
    }

    // Each dependency argument resolves as a whole sibling package (single
    // component). Members selected on an argument add longer attr-paths, e.g.
    // `python3Packages.isdr-zk-client`, resolved as sibling attribute sets.
    let mut deps: Vec<Vec<String>> = dep_names.iter().map(|name| vec![name.clone()]).collect();
    collect_dep_member_paths(root.syntax(), &dep_names, &mut deps);

    let aliases = collect_aliases(root.syntax(), roots);
    let ctx = ScanCtx { path, content };
    collect_refs(root.syntax(), &mut refs, roots, &aliases, &ctx);

    if let Some(dir) = file_dir {
        follow_imports(root.syntax(), roots, &aliases, dir, visited, &mut refs);
    }

    FileInfo { refs, deps }
}

/// Collect the static attr-paths selected on dependency arguments.
///
/// For every `select` whose base identifier is a dependency argument in
/// `dep_names`, record `[arg, member…]` up to the first dynamic component.
/// These become sibling-attribute-set lookups in [resolve_dep].
fn collect_dep_member_paths(
    node: &rnix::SyntaxNode,
    dep_names: &HashSet<String>,
    out: &mut Vec<Vec<String>>,
) {
    if let Some(select) = ast::Select::cast(node.clone())
        && let Some(ast::Expr::Ident(base)) = select.expr()
        && let Some(base_name) = base.ident_token().map(|t| t.text().to_string())
        && dep_names.contains(&base_name)
    {
        let mut path = vec![base_name];
        if let Some(attrpath) = select.attrpath() {
            for attr in attrpath.attrs() {
                let Some(name) = attr_static_name(&attr) else {
                    break;
                };
                path.push(name);
            }
        }
        if path.len() > 1 {
            out.push(path);
        }
    }

    for child in node.children() {
        collect_dep_member_paths(&child, dep_names, out);
    }
}

/// Resolve the transitive closure of catalog refs across a set of files.
///
/// Starting from the files in `db`, follow each file's `deps` to sibling
/// packages (loaded on demand from `dir` via [resolve_dep]) and union their
/// refs. A dep is an attr-path: a bare argument resolves as a sibling file, a
/// longer path as a member of a sibling attribute set. Cycles are handled by
/// tracking visited attr-paths.
fn collect_transitive(
    mut db: HashMap<String, FileInfo>,
    dir: &Path,
    roots: &HashSet<String>,
) -> BTreeSet<String> {
    let mut visited: HashSet<String> = HashSet::new();
    let mut result: BTreeSet<String> = BTreeSet::new();
    let mut queue: Vec<Vec<String>> = db
        .keys()
        .map(|key| key.split('/').map(str::to_string).collect())
        .collect();

    while let Some(path) = queue.pop() {
        let key = path.join("/");
        if !visited.insert(key.clone()) {
            continue;
        }
        if !db.contains_key(&key)
            && let Some(info) = resolve_dep(dir, &path, roots)
        {
            db.insert(key.clone(), info);
        }
        let Some(info) = db.get(&key) else { continue };
        result.extend(info.refs.iter().cloned());
        let deps: Vec<Vec<String>> = info.deps.clone();
        for dep in deps {
            if !visited.contains(&dep.join("/")) {
                queue.push(dep);
            }
        }
    }

    result
}

/// Build the map of `let` bindings that alias a catalog path.
///
/// Repeats passes until no new alias is found, so bindings may reference
/// earlier aliases regardless of source order.
fn collect_aliases(root: &rnix::SyntaxNode, roots: &HashSet<String>) -> HashMap<String, String> {
    let mut aliases: HashMap<String, String> = HashMap::new();
    let mut changed = true;
    while changed {
        changed = false;
        gather_let_aliases(root, roots, &mut aliases, &mut changed);
    }
    aliases
}

/// Single pass over `let` bindings, recording any that resolve to a catalog
/// path and setting `changed` when a new alias is added.
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
            let Some(name) = attr_static_name(&attrs[0]) else {
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

/// Walk the tree for `import ./file { <root> = …; }` calls and merge the refs
/// found in each imported file into `refs`.
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
                let imported = analyze_file_at(
                    &content,
                    &import_roots,
                    import_dir.as_deref(),
                    visited,
                    Some(&target),
                );
                refs.extend(imported.refs);
            }
        }
    }
    for child in node.children() {
        follow_imports(&child, roots, aliases, file_dir, visited, refs);
    }
}

/// Recognize an `import <path> { … }` application that forwards at least one
/// catalog root, returning the import path and the roots passed to it.
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

/// Extract a statically-known path or string literal as a string, or `None`
/// for dynamic expressions.
fn static_path_str(expr: &ast::Expr) -> Option<String> {
    match expr {
        ast::Expr::PathRel(p) => Some(p.syntax().text().to_string()),
        ast::Expr::PathAbs(p) => Some(p.syntax().text().to_string()),
        ast::Expr::Str(_) => static_str(expr),
        _ => None,
    }
}

/// Collect the catalog roots forwarded into an import's argument attrset,
/// via either `inherit` or `<root> = <root>;` bindings.
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
            if let Some(name) = attr_static_name(&attr)
                && roots.contains(name.as_str())
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
        if let Some(name) = attr_static_name(&attrs[0])
            && roots.contains(name.as_str())
        {
            passed.insert(name);
        }
    }
    passed
}

/// Recursively walk the syntax tree, inserting every catalog attr-path
/// reference into `refs`.
///
/// Handles `inherit (…)`, `with`, `builtins.getAttr`, and plain selects;
/// dynamic attrs collapse to a `<path>.*` sentinel.
fn collect_refs(
    node: &rnix::SyntaxNode,
    refs: &mut BTreeSet<String>,
    roots: &HashSet<String>,
    aliases: &HashMap<String, String>,
    ctx: &ScanCtx,
) {
    if let Some(inherit) = ast::Inherit::cast(node.clone())
        && try_handle_inherit(&inherit, refs, roots, aliases, ctx)
    {
        return;
    }

    if let Some(with_expr) = ast::With::cast(node.clone())
        && let Some(ns) = with_expr.namespace()
        && let Some(path) = namespace_path(&ns, roots, aliases)
    {
        let reference = format!("{}.*", path);
        ctx.report(offset_of(with_expr.syntax()), &reference);
        refs.insert(reference);
        if let Some(body) = with_expr.body() {
            collect_refs(body.syntax(), refs, roots, aliases, ctx);
        }
        return;
    }

    if let Some(apply) = ast::Apply::cast(node.clone())
        && let Some(path) = try_handle_get_attr(&apply, roots, aliases)
    {
        ctx.report(offset_of(apply.syntax()), &path);
        refs.insert(path);
        return;
    }

    if let Some(select) = ast::Select::cast(node.clone())
        && let Some(path) = extract_ref_path(&select, roots, aliases)
    {
        ctx.report(offset_of(select.syntax()), &path);
        refs.insert(path);
        return;
    }

    for child in node.children() {
        collect_refs(&child, refs, roots, aliases, ctx);
    }
}

/// Byte offset of a syntax node's start, for [`ScanCtx::report`].
fn offset_of(node: &rnix::SyntaxNode) -> usize {
    u32::from(node.text_range().start()) as usize
}

/// Resolve an expression used as a namespace (in `with` or `getAttr`) to its
/// catalog path, following aliases.
fn namespace_path(
    expr: &ast::Expr,
    roots: &HashSet<String>,
    aliases: &HashMap<String, String>,
) -> Option<String> {
    match expr {
        ast::Expr::Select(select) => extract_ref_path(select, roots, aliases),
        ast::Expr::Ident(ident) => {
            let name = ident.ident_token()?.text().to_string();
            if roots.contains(name.as_str()) {
                Some(name)
            } else {
                aliases.get(&name).cloned()
            }
        },
        _ => None,
    }
}

/// Resolve a `getAttr "key" <root>` application to a `<path>.key` reference,
/// or a `<path>.*` sentinel when the key is dynamic.
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

/// Whether an expression is the `getAttr` builtin, named either bare
/// (`getAttr`) or qualified (`builtins.getAttr`).
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

/// Extract the contents of a string literal with no interpolation, or `None`.
fn static_str(expr: &ast::Expr) -> Option<String> {
    let ast::Expr::Str(s) = expr else { return None };
    static_str_content(s)
}

/// Extract a string node's contents when it has no interpolation, or `None`.
fn static_str_content(s: &ast::Str) -> Option<String> {
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

/// Resolve an attribute to its statically-known component name: a plain
/// identifier, or a non-interpolated string literal that is a valid catalog
/// component name. Returns `None` for dynamic attrs and for quoted names the
/// catalog cannot contain (`.`, `"`, `\` are rejected by the server's name
/// validators), which callers collapse to a `*` sentinel.
fn attr_static_name(attr: &ast::Attr) -> Option<String> {
    match attr {
        ast::Attr::Ident(id) => Some(id.ident_token()?.text().to_string()),
        ast::Attr::Str(s) => {
            let name = static_str_content(s)?;
            let valid = !name.is_empty() && !name.contains(['.', '"', '\\']);
            valid.then_some(name)
        },
        ast::Attr::Dynamic(_) => None,
    }
}

/// Handle `inherit (<root-path>) a b c;`, emitting one `<path>.<name>`
/// reference per inherited name. Returns whether the inherit was rooted.
fn try_handle_inherit(
    inherit: &ast::Inherit,
    refs: &mut BTreeSet<String>,
    roots: &HashSet<String>,
    aliases: &HashMap<String, String>,
    ctx: &ScanCtx,
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
        // A name the catalog cannot contain (dotted quoted attr) collapses to
        // a sentinel at the inherit base.
        let reference = match attr_static_name(&attr) {
            Some(name) => format!("{}.{}", base_path, name),
            None => format!("{}.*", base_path),
        };
        ctx.report(offset_of(attr.syntax()), &reference);
        refs.insert(reference);
    }
    true
}

/// Build the dotted catalog path for a `select` expression rooted at a catalog
/// root or alias. A dynamic component collapses the path to end in `*`.
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

    let base_path = if roots.contains(base_name.as_str()) {
        base_name
    } else if let Some(alias) = aliases.get(&base_name) {
        alias.clone()
    } else {
        return None;
    };

    let attrpath = select.attrpath()?;
    let mut parts = vec![base_path];
    for attr in attrpath.attrs() {
        match attr_static_name(&attr) {
            Some(name) => parts.push(name),
            None => {
                parts.push("*".to_string());
                break;
            },
        }
    }
    Some(parts.join("."))
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
fn resolve_dep(dir: &Path, components: &[String], roots: &HashSet<String>) -> Option<FileInfo> {
    let mut cur = dir.to_path_buf();
    for comp in components {
        let file = cur.join(format!("{comp}.nix"));
        if file.is_file() {
            return read_and_analyze(&file, roots);
        }
        let sub = cur.join(comp);
        let default = sub.join("default.nix");
        if default.is_file() {
            return read_and_analyze(&default, roots);
        }
        if sub.is_dir() {
            cur = sub;
            continue;
        }
        return None;
    }
    None
}

/// Read and analyze a resolved package file.
///
/// Relative imports in the file resolve against its own directory, so the
/// file's parent is passed as the import base.
fn read_and_analyze(path: &Path, roots: &HashSet<String>) -> Option<FileInfo> {
    let content = fs::read_to_string(path).ok()?;
    Some(analyze_file_at(
        &content,
        roots,
        path.parent(),
        &mut HashSet::new(),
        Some(path),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn analyze_file(content: &str, roots: &HashSet<String>) -> FileInfo {
        analyze_file_at(content, roots, None, &mut HashSet::new(), None)
    }

    fn refs(content: &str, roots: &HashSet<String>) -> BTreeSet<String> {
        analyze_file(content, roots).refs
    }

    fn refs_at(path: &str, roots: &HashSet<String>) -> BTreeSet<String> {
        let path = Path::new(path);
        let content = fs::read_to_string(path).expect("test fixture missing");
        let dir = path.parent();
        let mut visited = HashSet::new();
        analyze_file_at(&content, roots, dir, &mut visited, Some(path)).refs
    }

    fn set(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn refset(items: &[&str]) -> BTreeSet<CatalogRef> {
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
            &roots(&["catalogs"]),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn no_catalog_refs_rust_package() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/rust-no-catalog.nix"),
            &roots(&["catalogs"]),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn non_catalog_inherit_not_collected() {
        let content = include_str!("../../test_data/catalog_refs/non-catalog-inherit.nix");
        assert_eq!(refs(content, &roots(&["catalogs"])), BTreeSet::new());
        assert_eq!(refs(content, &roots(&["inputs"])), BTreeSet::new());
    }

    #[test]
    fn single_inherit_helper() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/single-inherit-helper.nix"),
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
    }

    #[test]
    fn two_inherits_toolkit_and_python_pkg() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/two-inherits.nix"),
            &roots(&["catalogs"]),
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
            &roots(&["catalogs"]),
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
            &roots(&["catalogs"]),
        );
        assert!(!got.contains("catalogs.myorg.python3Packages"));
        assert!(!got.contains("catalogs.myorg.toolkit"));
    }

    #[test]
    fn direct_select_native_packages() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/direct-select-native.nix"),
            &roots(&["catalogs"]),
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
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit"]));
    }

    #[test]
    fn nested_inline_package_does_not_hide_outer_refs() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/nested-inline-package.nix"),
            &roots(&["catalogs"]),
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
            &roots(&["catalogs"]),
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
            &roots(&["inputs"]),
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
            include_str!("../../test_data/catalog_refs/inputs-only.nix"),
            &roots(&["catalogs"]),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn mixed_roots_catalog_only() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/mixed-roots.nix"),
            &roots(&["catalogs"]),
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
            &roots(&["inputs"]),
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
            include_str!("../../test_data/catalog_refs/mixed-roots.nix"),
            &roots(&["catalogs", "inputs"]),
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
        let r = roots(&["catalogs"]);
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
        let r = roots(&["catalogs"]);
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
        let r = roots(&["inputs"]);
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
    fn scan_package_unions_target_and_sibling_dep_refs() {
        let base_dir = Path::new("test_data/catalog_refs");
        // dep-entry.nix references one catalog path and pulls in a `dep-helper`
        // dependency argument; dep-helper.nix (its sibling under base_dir)
        // references another. The closure is the union of both.
        let got = scan_package(base_dir, Path::new("dep-entry.nix"));
        assert_eq!(
            got,
            refset(&[
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
        let got = scan_package(base_dir, Path::new("entry.nix"));
        assert_eq!(
            got,
            refset(&["catalogs.myorg.direct", "catalogs.myorg.helper-ref"]),
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
        let got = scan_package(base_dir, Path::new("isdr-zk-client.nix"));
        assert_eq!(got, refset(&["catalogs.myorg.toolkit.readVersion"]));
    }

    /// A nested file as the scan target resolves deps against the root.
    ///
    /// Scanning `foo/bar.nix` directly must resolve its dependency arguments
    /// against the package-set root, not `foo/`, so a root-level package like
    /// `top` is reachable and its refs join the closure.
    #[test]
    fn scan_package_nested_target_resolves_deps_at_root() {
        let base_dir = Path::new("test_data/catalog_refs/nested-target-access");
        let got = scan_package(base_dir, Path::new("foo/bar.nix"));
        assert_eq!(
            got,
            refset(&["catalogs.myorg.bar-own", "catalogs.myorg.top-src"]),
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
        let got = scan_package(base_dir, Path::new("top.nix"));
        assert_eq!(
            got,
            refset(&["catalogs.myorg.widget-src", "catalogs.myorg.helper-lib-src"]),
        );
    }

    #[test]
    fn with_direct_namespace_emits_sentinel() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/with-namespace.nix"),
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.*"]));
    }

    #[test]
    fn with_namespace_does_not_emit_bare_path() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/with-namespace.nix"),
            &roots(&["catalogs"]),
        );
        assert!(!got.contains("catalogs.myorg"));
    }

    #[test]
    fn with_alias_namespace_emits_sentinel() {
        let got = refs(
            "{ catalogs }: let org = catalogs.myorg; in with org; toolkit",
            &roots(&["catalogs"]),
        );
        assert!(got.contains("catalogs.myorg.*"), "got: {:?}", got);
    }

    #[test]
    fn with_non_rooted_namespace_falls_through() {
        let got = refs(
            "{ catalogs }: with { x = 1; }; catalogs.myorg.pkg",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.pkg"]));
    }

    #[test]
    fn with_body_direct_refs_still_collected() {
        let got = refs(
            "{ catalogs }: with catalogs.myorg; catalogs.other.pkg",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.*", "catalogs.other.pkg"]));
    }

    #[test]
    fn aliased_select_single_level() {
        let got = refs(
            "{ catalogs }: let org = catalogs.myorg; in org.toolkit",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg", "catalogs.myorg.toolkit"]));
    }

    #[test]
    fn aliased_select_chained() {
        let got = refs(
            include_str!("../../test_data/catalog_refs/aliased-select.nix"),
            &roots(&["catalogs"]),
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
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg", "catalogs.myorg.hello"]));
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
                refs(content, &roots(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn dynamic_attr_emits_sentinel() {
        let got = refs(
            "{ catalogs, name }: catalogs.myorg.${name}",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.*"]));
    }

    #[test]
    fn dynamic_attr_at_first_component_emits_root_sentinel() {
        let got = refs(
            "{ catalogs, org }: catalogs.${org}.pkg",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.*"]));
    }

    #[test]
    fn dynamic_attr_stops_at_first_dynamic_component() {
        let got = refs(
            "{ catalogs, name }: catalogs.myorg.${name}.subpkg",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.*"]));
    }

    #[test]
    fn get_attr_static_key_qualified() {
        let got = refs(
            "{ catalogs }: builtins.getAttr \"hello\" catalogs.myorg",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.hello"]));
    }

    #[test]
    fn get_attr_static_key_bare() {
        let got = refs(
            "{ catalogs }: getAttr \"hello\" catalogs.myorg",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.hello"]));
    }

    #[test]
    fn get_attr_dynamic_key_emits_sentinel() {
        let got = refs(
            "{ catalogs, name }: builtins.getAttr name catalogs.myorg",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.*"]));
    }

    #[test]
    fn get_attr_with_alias_target() {
        let got = refs(
            "{ catalogs, name }: let org = catalogs.myorg; in builtins.getAttr \"hello\" org",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg", "catalogs.myorg.hello"]));
    }

    #[test]
    fn get_attr_non_rooted_target_ignored() {
        let got = refs(
            "{ catalogs }: builtins.getAttr \"hello\" someOtherAttrset",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn import_inherit_catalogs_follows_into_helper() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry.nix",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
    }

    #[test]
    fn import_explicit_catalogs_arg_followed() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-explicit.nix",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
    }

    #[test]
    fn import_without_catalogs_not_followed() {
        let got = refs(
            "{ catalogs }: let x = import ./import-helper.nix { foo = 1; }; in catalogs.myorg.pkg",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.pkg"]));
    }

    #[test]
    fn import_direct_refs_in_entry_still_collected() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-with-direct-ref.nix",
            &roots(&["catalogs"]),
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
