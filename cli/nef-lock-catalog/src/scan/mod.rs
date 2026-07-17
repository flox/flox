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

    let ctx = ScanCtx { path, content };
    let mut walker = Walker {
        roots,
        ctx,
        refs: BTreeSet::new(),
        pending_imports: Vec::new(),
    };

    // Seed the environment with the catalog roots. Lambda parameters —
    // including the top-level package function's — keep root-named names
    // rooted and shadow everything else (see [Walker::walk_lambda]).
    let env: Env = roots
        .iter()
        .map(|root| (root.clone(), Binding::Path(vec![root.clone()])))
        .collect();
    if let Some(expr) = root.expr() {
        walker.walk(expr.syntax(), &env);
    }

    let Walker {
        mut refs,
        pending_imports,
        ..
    } = walker;

    // Imports are IO: the walker only records facts, the drain here reads and
    // recurses. Relative paths resolve against the importing file's directory;
    // `visited` guards against import cycles.
    if let Some(dir) = file_dir {
        for pending in pending_imports {
            let target = dir.join(&pending.path);
            let target = fs::canonicalize(&target).unwrap_or(target);
            if !visited.insert(target.clone()) {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&target) {
                let import_dir = target.parent().map(Path::to_path_buf);
                let imported = analyze_file_at(
                    &content,
                    &pending.roots,
                    import_dir.as_deref(),
                    visited,
                    Some(&target),
                );
                refs.extend(imported.refs);
            }
        }
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

/// A name's meaning in the lexical environment threaded through the walk.
#[derive(Clone, Debug)]
enum Binding {
    /// The name resolves to a catalog attr-path: a root itself
    /// (`catalogs` = `Path(["catalogs"])`) or an alias of one. A path may end
    /// in `"*"` when a dynamic component collapsed it.
    Path(Vec<String>),
    /// The name is bound to something the scanner cannot model; selects on it
    /// produce no refs.
    Opaque,
}

/// Lexical environment: what each in-scope name is known to be.
type Env = HashMap<String, Binding>;

/// An `import <path> { … }` application forwarding catalog roots, recorded by
/// the walker and resolved by the IO drain loop in [analyze_file_at].
#[derive(Debug)]
struct PendingImport {
    /// Import path as written (relative paths resolve against the importing
    /// file's directory).
    path: String,
    /// Root names the imported file receives.
    roots: HashSet<String>,
}

/// Syntax-tree walker threading the lexical environment.
///
/// The walker emits refs for use sites resolved through the environment
/// (selects, `inherit (…)`, `with`, `getAttr`) and records `import`
/// applications as [PendingImport] facts; it performs no IO itself.
struct Walker<'a> {
    roots: &'a HashSet<String>,
    ctx: ScanCtx<'a>,
    refs: BTreeSet<String>,
    pending_imports: Vec<PendingImport>,
}

impl Walker<'_> {
    fn emit(&mut self, offset: usize, reference: String) {
        self.ctx.report(offset, &reference);
        self.refs.insert(reference);
    }

    fn walk(&mut self, node: &rnix::SyntaxNode, env: &Env) {
        if let Some(attrpath) = ast::Attrpath::cast(node.clone()) {
            // Reached generically from attrset keys and `?` operands: only the
            // dynamic components are expressions, static names are not uses.
            return self.walk_attrpath_dynamics(&attrpath, env);
        }
        if let Some(inherit) = ast::Inherit::cast(node.clone()) {
            return self.walk_inherit(&inherit, env);
        }
        let Some(expr) = ast::Expr::cast(node.clone()) else {
            return self.walk_children(node, env);
        };
        match expr {
            ast::Expr::Lambda(lambda) => self.walk_lambda(&lambda, env),
            ast::Expr::LetIn(let_in) => self.walk_let_in(&let_in, env),
            ast::Expr::With(with_expr) => self.walk_with(&with_expr, env),
            ast::Expr::Apply(apply) => self.walk_apply(&apply, env),
            ast::Expr::Select(select) => self.walk_select(&select, env),
            ast::Expr::Ident(ident) => self.walk_ident(&ident, env),
            _ => self.walk_children(node, env),
        }
    }

    fn walk_children(&mut self, node: &rnix::SyntaxNode, env: &Env) {
        for child in node.children() {
            self.walk(&child, env);
        }
    }

    /// A bare ident in a value position: using an alias uses the package it
    /// names. Uses at catalog depth or shallower are escape hatches handled
    /// separately, not exact refs.
    fn walk_ident(&mut self, ident: &ast::Ident, env: &Env) {
        let Some(name) = ident_name(ident) else {
            return;
        };
        let Some(Binding::Path(path)) = env.get(&name) else {
            return;
        };
        if path.len() >= 3 || path.last().map(String::as_str) == Some("*") {
            self.emit(offset_of(ident.syntax()), join_path(path));
        }
    }

    /// Lambda parameters shadow the outer scope. A parameter named like a
    /// catalog root keeps its root meaning: such lambdas are (almost always)
    /// applied to the namespace itself, and assuming so can only produce
    /// spurious refs that fail loudly, while assuming the opposite drops
    /// real refs silently. Every other parameter is opaque — the scanner
    /// cannot know what it will be applied to.
    fn walk_lambda(&mut self, lambda: &ast::Lambda, env: &Env) {
        let mut inner = env.clone();
        match lambda.param() {
            Some(ast::Param::IdentParam(param)) => {
                if let Some(name) = param.ident().as_ref().and_then(ident_name) {
                    let binding = self.param_binding(&name);
                    inner.insert(name, binding);
                }
            },
            Some(ast::Param::Pattern(pat)) => {
                for entry in pat.pat_entries() {
                    if let Some(name) = entry.ident().as_ref().and_then(ident_name) {
                        let binding = self.param_binding(&name);
                        inner.insert(name, binding);
                    }
                }
                if let Some(name) = pat
                    .pat_bind()
                    .and_then(|bind| bind.ident())
                    .as_ref()
                    .and_then(ident_name)
                {
                    inner.insert(name, Binding::Opaque);
                }
                for entry in pat.pat_entries() {
                    if let Some(default) = entry.default() {
                        self.walk(default.syntax(), &inner);
                    }
                }
            },
            None => {},
        }
        if let Some(body) = lambda.body() {
            self.walk(body.syntax(), &inner);
        }
    }

    /// What a lambda parameter of this name means (see [Self::walk_lambda]).
    fn param_binding(&self, name: &str) -> Binding {
        if self.roots.contains(name) {
            Binding::Path(vec![name.to_string()])
        } else {
            Binding::Opaque
        }
    }

    fn walk_let_in(&mut self, let_in: &ast::LetIn, env: &Env) {
        let inner = let_scope_env(let_in, env);
        for entry in let_in.attrpath_values() {
            let attrs: Vec<ast::Attr> = entry
                .attrpath()
                .map(|attrpath| attrpath.attrs().collect())
                .unwrap_or_default();
            if let Some(attrpath) = entry.attrpath() {
                self.walk_attrpath_dynamics(&attrpath, &inner);
            }
            let Some(value) = entry.value() else { continue };
            // A binding consumed as an alias walks in binding mode (see
            // [Self::walk_binding_rhs]); everything else is a value position.
            let consumed = attrs.len() == 1
                && attrs
                    .first()
                    .and_then(attr_static_name)
                    .is_some_and(|name| matches!(inner.get(&name), Some(Binding::Path(_))));
            if consumed {
                self.walk_binding_rhs(&value, &inner);
            } else {
                self.walk(value.syntax(), &inner);
            }
        }
        for inherit in let_in.inherits() {
            self.walk_inherit(&inherit, &inner);
        }
        if let Some(body) = let_in.body() {
            self.walk(body.syntax(), &inner);
        }
    }

    /// `inherit (<source>) a b c;` — when the source resolves to a catalog
    /// path, each name is one member ref. A name the catalog cannot contain
    /// (dotted quoted attr) collapses to a sentinel at the source.
    fn walk_inherit(&mut self, inherit: &ast::Inherit, env: &Env) {
        let Some(from_expr) = inherit.from().and_then(|from| from.expr()) else {
            return;
        };
        let Some(base) = resolve_expr_path(&from_expr, env) else {
            self.walk(from_expr.syntax(), env);
            return;
        };
        for attr in inherit.attrs() {
            let path = match attr_static_name(&attr) {
                Some(name) => append_component(base.clone(), name),
                None => append_star(base.clone()),
            };
            let path = widen_if_catalog_level(path);
            self.emit(offset_of(attr.syntax()), join_path(&path));
        }
    }

    fn walk_with(&mut self, with_expr: &ast::With, env: &Env) {
        if let Some(ns) = with_expr.namespace()
            && let Some(path) = resolve_expr_path(&ns, env)
        {
            self.emit(offset_of(with_expr.syntax()), join_path(&append_star(path)));
            if let Some(body) = with_expr.body() {
                self.walk(body.syntax(), env);
            }
            return;
        }
        for child in with_expr.syntax().children() {
            self.walk(&child, env);
        }
    }

    fn walk_apply(&mut self, apply: &ast::Apply, env: &Env) {
        // `getAttr <key> <target>`: the resolved target is consumed, the key
        // subexpression is still scanned for refs of its own.
        if let Some(inner) = inner_apply(apply)
            && inner.lambda().is_some_and(|f| is_get_attr_fn(&f))
            && let Some(target) = apply.argument()
            && let Some(target_path) = resolve_expr_path(&target, env)
        {
            let key = inner.argument();
            let path = match key.as_ref().and_then(static_str) {
                Some(key) => append_component(target_path, key),
                None => append_star(target_path),
            };
            self.emit(
                offset_of(apply.syntax()),
                join_path(&widen_if_catalog_level(path)),
            );
            if let Some(key) = key {
                self.walk(key.syntax(), env);
            }
            return;
        }
        if let Some((path, arg)) = extract_import(apply) {
            let forwarded = roots_forwarded(&arg, self.roots);
            if !forwarded.is_empty() {
                self.pending_imports.push(PendingImport {
                    path,
                    roots: forwarded,
                });
            }
        }
        for child in apply.syntax().children() {
            self.walk(&child, env);
        }
    }

    fn walk_select(&mut self, select: &ast::Select, env: &Env) {
        match resolve_select_path(select, env) {
            Some(path) => self.emit(
                offset_of(select.syntax()),
                join_path(&widen_if_catalog_level(path)),
            ),
            None => {
                if let Some(base) = select.expr() {
                    self.walk(base.syntax(), env);
                }
            },
        }
        self.walk_select_parts(select, env);
    }

    /// Scan the non-consumed subtrees of a select: dynamic attr components and
    /// the `or` default.
    fn walk_select_parts(&mut self, select: &ast::Select, env: &Env) {
        if let Some(attrpath) = select.attrpath() {
            self.walk_attrpath_dynamics(&attrpath, env);
        }
        if let Some(default) = select.default_expr() {
            self.walk(default.syntax(), env);
        }
    }

    /// Walk the right-hand side of a binding consumed as a catalog alias.
    ///
    /// A deep RHS (two or more components past the root) is a package ref of
    /// its own and still emits; a catalog-level RHS is suppressed entirely —
    /// it only defines the alias, and the alias's use sites drive the refs.
    fn walk_binding_rhs(&mut self, value: &ast::Expr, env: &Env) {
        match value {
            ast::Expr::Select(select) => {
                match resolve_select_path(select, env) {
                    Some(path) if path.len() >= 3 => {
                        self.emit(offset_of(select.syntax()), join_path(&path));
                    },
                    Some(_) => {},
                    None => {
                        if let Some(base) = select.expr() {
                            self.walk(base.syntax(), env);
                        }
                    },
                }
                self.walk_select_parts(select, env);
            },
            ast::Expr::Ident(_) => {},
            ast::Expr::Paren(paren) => {
                if let Some(inner) = paren.expr() {
                    self.walk_binding_rhs(&inner, env);
                }
            },
            _ => self.walk(value.syntax(), env),
        }
    }

    /// Scan the expression components of an attr path: dynamic attrs and
    /// string interpolations. Static names are not value uses.
    fn walk_attrpath_dynamics(&mut self, attrpath: &ast::Attrpath, env: &Env) {
        for attr in attrpath.attrs() {
            match attr {
                ast::Attr::Dynamic(dynamic) => {
                    if let Some(expr) = dynamic.expr() {
                        self.walk(expr.syntax(), env);
                    }
                },
                ast::Attr::Str(s) => self.walk(s.syntax(), env),
                ast::Attr::Ident(_) => {},
            }
        }
    }
}

/// Environment for a `let … in` scope.
///
/// Every bound name shadows the outer scope. `let` is recursive, so bindings
/// that alias catalog paths are resolved to a fixpoint: a pass may enable
/// another binding regardless of source order, and names that never resolve
/// stay opaque (which is what makes `let catalogs = …;` shadow a root).
fn let_scope_env(let_in: &ast::LetIn, outer: &Env) -> Env {
    let mut env = outer.clone();

    let mut values: Vec<(String, ast::Expr)> = Vec::new();
    for entry in let_in.attrpath_values() {
        let Some(attrpath) = entry.attrpath() else {
            continue;
        };
        let attrs: Vec<ast::Attr> = attrpath.attrs().collect();
        let Some(name) = attrs.first().and_then(attr_static_name) else {
            continue;
        };
        env.insert(name.clone(), Binding::Opaque);
        if attrs.len() == 1
            && let Some(value) = entry.value()
        {
            values.push((name, value));
        }
    }
    for inherit in let_in.inherits() {
        for attr in inherit.attrs() {
            let Some(name) = attr_static_name(&attr) else {
                continue;
            };
            // `inherit x;` (no source) rebinds the outer x under the same name.
            let binding = match inherit.from() {
                None => outer.get(&name).cloned().unwrap_or(Binding::Opaque),
                Some(_) => Binding::Opaque,
            };
            env.insert(name, binding);
        }
    }

    loop {
        let mut changed = false;
        for (name, value) in &values {
            if !matches!(env.get(name), Some(Binding::Opaque)) {
                continue;
            }
            if let Some(binding) = resolve_binding(value, &env) {
                env.insert(name.clone(), binding);
                changed = true;
            }
        }
        for inherit in let_in.inherits() {
            let Some(from_expr) = inherit.from().and_then(|from| from.expr()) else {
                continue;
            };
            let Some(base) = resolve_expr_path(&from_expr, &env) else {
                continue;
            };
            for attr in inherit.attrs() {
                let Some(name) = attr_static_name(&attr) else {
                    continue;
                };
                if matches!(env.get(&name), Some(Binding::Opaque)) {
                    let path = append_component(base.clone(), name.clone());
                    env.insert(name, Binding::Path(path));
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    env
}

/// Resolve a binding's right-hand side to what the bound name will mean.
fn resolve_binding(expr: &ast::Expr, env: &Env) -> Option<Binding> {
    resolve_expr_path(expr, env).map(Binding::Path)
}

/// Resolve an expression to the catalog attr-path it denotes, following the
/// environment for idents and aliases. `None` when the expression does not
/// reach a catalog root.
fn resolve_expr_path(expr: &ast::Expr, env: &Env) -> Option<Vec<String>> {
    match expr {
        ast::Expr::Ident(ident) => match env.get(&ident_name(ident)?)? {
            Binding::Path(path) => Some(path.clone()),
            Binding::Opaque => None,
        },
        ast::Expr::Select(select) => resolve_select_path(select, env),
        ast::Expr::Paren(paren) => resolve_expr_path(&paren.expr()?, env),
        _ => None,
    }
}

/// [resolve_expr_path] for a select: base path plus the static components of
/// the attr path, collapsing at the first dynamic component.
fn resolve_select_path(select: &ast::Select, env: &Env) -> Option<Vec<String>> {
    let mut path = resolve_expr_path(&select.expr()?, env)?;
    for attr in select.attrpath()?.attrs() {
        match attr_static_name(&attr) {
            Some(name) => path = append_component(path, name),
            None => {
                path = append_star(path);
                break;
            },
        }
    }
    Some(path)
}

/// Append one component; a path already collapsed to `*` absorbs it.
fn append_component(mut path: Vec<String>, name: String) -> Vec<String> {
    if path.last().map(String::as_str) != Some("*") {
        path.push(name);
    }
    path
}

/// Collapse the path's tail to a `*` sentinel (idempotent).
fn append_star(path: Vec<String>) -> Vec<String> {
    append_component(path, "*".to_string())
}

/// Widen a path that names a whole catalog or the root itself (fewer than two
/// components past the root) to a `.*` sentinel: the server's resolution floor
/// is catalog + one component, so such an exact ref can never resolve.
fn widen_if_catalog_level(path: Vec<String>) -> Vec<String> {
    if path.len() < 3 {
        append_star(path)
    } else {
        path
    }
}

fn join_path(path: &[String]) -> String {
    path.join(".")
}

fn ident_name(ident: &ast::Ident) -> Option<String> {
    Some(ident.ident_token()?.text().to_string())
}

/// The inner application of a two-argument call `f a b`, i.e. `f a`.
fn inner_apply(apply: &ast::Apply) -> Option<ast::Apply> {
    match apply.lambda()? {
        ast::Expr::Apply(inner) => Some(inner),
        _ => None,
    }
}

/// Recognize an `import <static-path> { … }` application, returning the
/// import path and its argument attrset.
fn extract_import(apply: &ast::Apply) -> Option<(String, ast::AttrSet)> {
    let inner = inner_apply(apply)?;
    let ast::Expr::Ident(import_fn) = inner.lambda()? else {
        return None;
    };
    if import_fn.ident_token()?.text() != "import" {
        return None;
    }
    let path = static_path_str(&inner.argument()?)?;
    let ast::Expr::AttrSet(arg) = apply.argument()? else {
        return None;
    };
    Some((path, arg))
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
fn roots_forwarded(attrset: &ast::AttrSet, roots: &HashSet<String>) -> HashSet<String> {
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

/// Byte offset of a syntax node's start, for [`ScanCtx::report`].
fn offset_of(node: &rnix::SyntaxNode) -> usize {
    u32::from(node.text_range().start()) as usize
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
                "inputs.nixpkgs.lib.fakeStr",
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
                "inputs.nixpkgs.lib.fakeStr",
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
        // The alias RHS names a whole catalog, which can never resolve as an
        // exact ref; only the use site drives the ref.
        let got = refs(
            "{ catalogs }: let org = catalogs.myorg; in org.toolkit",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit"]));
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
        assert_eq!(got, set(&["catalogs.myorg.hello"]));
    }

    #[test]
    fn alias_rebound_as_lambda_param_not_emitted() {
        // `org` inside `g` is the lambda's own parameter, not the outer alias,
        // and the outer alias is never used in a value position.
        let got = refs(
            "{ catalogs, x }: let org = catalogs.myorg; g = org: org.other; in g x",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, BTreeSet::new());
    }

    #[test]
    fn catalog_level_value_use_emits_sentinel() {
        // Passing a whole catalog to a function makes every member reachable;
        // an exact `catalogs.myorg` ref would be unresolvable.
        let got = refs(
            "{ catalogs }: builtins.attrValues catalogs.myorg",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.*"]));
    }

    #[test]
    fn select_or_default_scans_both_arms() {
        let got = refs(
            "{ catalogs }: catalogs.a.b or catalogs.c.d",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.a.b", "catalogs.c.d"]));
    }

    #[test]
    fn get_attr_key_subexpression_scanned() {
        let got = refs(
            "{ catalogs, f }: builtins.getAttr (f catalogs.a.key) catalogs.myorg",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.a.key", "catalogs.myorg.*"]));
    }

    #[test]
    fn dynamic_attr_interpolation_scanned() {
        let got = refs(
            "{ catalogs }: catalogs.myorg.${catalogs.a.name}",
            &roots(&["catalogs"]),
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
                refs(content, &roots(&["catalogs"])),
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
                refs(content, &roots(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn inherit_bound_name_acts_as_alias() {
        let got = refs(
            "{ catalogs }: let inherit (catalogs.myorg) toolkit; in toolkit.readVersion",
            &roots(&["catalogs"]),
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
            &roots(&["catalogs"]),
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
                refs(content, &roots(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn conditional_branch_refs_both_collected() {
        let got = refs(
            "{ catalogs, x }: if x then catalogs.a.p else catalogs.b.q",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.a.p", "catalogs.b.q"]));
    }

    #[test]
    fn nested_lambda_body_refs_collected() {
        let got = refs("{ catalogs }: x: catalogs.myorg.pkg", &roots(&["catalogs"]));
        assert_eq!(got, set(&["catalogs.myorg.pkg"]));
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
    fn dynamic_attr_at_set_depth_emits_set_sentinel() {
        // The sentinel keeps the full static prefix: a dynamic member of a
        // package set widens to the set (`<catalog>.<set>.*`), not the whole
        // catalog.
        let got = refs(
            "{ catalogs, name }: catalogs.myorg.pythonPackages.${name}",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.pythonPackages.*"]));
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
            "{ catalogs }: with builtins; getAttr \"hello\" catalogs.myorg",
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
        assert_eq!(got, set(&["catalogs.myorg.hello"]));
    }

    #[test]
    fn get_attr_non_rooted_target_ignored() {
        let got = refs(
            "{ catalogs, someOtherAttrset }: builtins.getAttr \"hello\" someOtherAttrset",
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
