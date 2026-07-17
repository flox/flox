use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::{self, Display};
use std::fs;
use std::path::{Path, PathBuf};

use indoc::formatdoc;
use rnix::ast;
use rnix::ast::HasEntry;
use rowan::ast::AstNode;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument, warn};

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

/// A scan failure that must stop locking.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScanError {
    /// A catalog root is referenced by a file whose top-level lambda does not
    /// declare it as a parameter. NEF supplies only declared arguments
    /// (callPackage semantics), so every reference through the root is
    /// guaranteed to fail evaluation as an undefined variable.
    #[error("{}", undeclared_root_message(root, file.as_deref(), *position))]
    UndeclaredRoot {
        root: String,
        file: Option<PathBuf>,
        /// 1-based `(line, column)` of the root's first use, when recorded.
        position: Option<(usize, usize)>,
    },
}

/// Render [ScanError::UndeclaredRoot] for the user; location parts are
/// best-effort (unit scans have no file, forwarded-only uses may lack a
/// position).
fn undeclared_root_message(
    root: &str,
    file: Option<&Path>,
    position: Option<(usize, usize)>,
) -> String {
    let location = match (file, position) {
        (Some(file), Some((line, column))) => format!(" at {}:{line}:{column}", file.display()),
        (Some(file), None) => format!(" in {}", file.display()),
        (None, Some((line, column))) => format!(" at {line}:{column}"),
        (None, None) => String::new(),
    };
    formatdoc! {"
        '{root}' is referenced{location} but is not declared in the function arguments.
        Add '{root}' to the function arguments, e.g. '{{ {root}, ... }}:'."}
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
/// Fails when a scanned file references a catalog root it does not declare in
/// its function arguments (see [ScanError::UndeclaredRoot]).
///
/// Uses the default `catalogs` root; see [scan_package_with_roots] to override.
pub fn scan_package(
    base_dir: impl AsRef<Path>,
    rel_file: impl AsRef<Path>,
) -> Result<BTreeSet<CatalogRef>, ScanError> {
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
) -> Result<BTreeSet<CatalogRef>, ScanError> {
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
                analyze_file_at(&content, roots, dir, &mut visited, Some(path))?,
            );
        }
        db
    };
    let references: BTreeSet<CatalogRef> = collect_transitive(db, base_dir.as_ref(), roots)?
        .into_iter()
        .map(CatalogRef)
        .collect();
    debug!(references = references.len(), "scanned catalog references");
    Ok(references)
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

    /// Warn that an import with a dynamic path forwards a catalog namespace
    /// the scanner cannot follow.
    fn warn_dynamic_import(&self, offset: usize) {
        let (line, column) = line_col(self.content, offset);
        match self.path {
            Some(path) => warn!(
                file = %path.display(),
                line,
                column,
                "import path is not statically known; the imported file is not scanned for catalog references",
            ),
            None => warn!(
                line,
                column,
                "import path is not statically known; the imported file is not scanned for catalog references",
            ),
        }
    }

    /// Warn that an import argument names a catalog root without forwarding
    /// it, so the imported file is not scanned through that name.
    fn warn_unfollowed_import(&self, offset: usize, name: &str) {
        let (line, column) = line_col(self.content, offset);
        match self.path {
            Some(path) => warn!(
                name,
                file = %path.display(),
                line,
                column,
                "import argument is not the catalog namespace; the imported file is not scanned through it",
            ),
            None => warn!(
                name,
                line,
                column,
                "import argument is not the catalog namespace; the imported file is not scanned through it",
            ),
        }
    }

    /// Warn that a catalog namespace escapes static analysis at `offset`,
    /// widening to `reference`.
    fn warn_escape(&self, offset: usize, reference: &str) {
        let (line, column) = line_col(self.content, offset);
        match self.path {
            Some(path) => warn!(
                reference,
                file = %path.display(),
                line,
                column,
                "catalog namespace escapes static analysis; locking the whole subtree",
            ),
            None => warn!(
                reference,
                line,
                column,
                "catalog namespace escapes static analysis; locking the whole subtree",
            ),
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
) -> Result<FileInfo, ScanError> {
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

    // The parameter names the top-level lambda declares. Only declared
    // arguments are supplied when NEF calls the file (callPackage semantics),
    // so a root referenced without being declared can never resolve; that is
    // reported as [ScanError::UndeclaredRoot] after the walk. `None` fails
    // open: a file whose top level is not a plain lambda (e.g. a let-wrapped
    // lambda) hides its parameters from the scanner, and the lenient seeding
    // below is kept without the declaration check.
    let declared_params: Option<HashSet<String>> = match root.expr() {
        Some(ast::Expr::Lambda(lambda)) => match lambda.param() {
            Some(ast::Param::IdentParam(param)) => Some(
                param
                    .ident()
                    .as_ref()
                    .and_then(ident_name)
                    .into_iter()
                    .collect(),
            ),
            Some(ast::Param::Pattern(pat)) => Some(
                pat.pat_entries()
                    .filter_map(|entry| entry.ident().as_ref().and_then(ident_name))
                    .collect(),
            ),
            None => None,
        },
        _ => None,
    };

    // Each dependency argument resolves as a whole sibling package (single
    // component). Members selected on an argument add longer attr-paths, e.g.
    // `python3Packages.isdr-zk-client`, resolved as sibling attribute sets.
    let mut deps: Vec<Vec<String>> = dep_names.iter().map(|name| vec![name.clone()]).collect();
    collect_dep_member_paths(root.syntax(), &dep_names, &mut deps);

    let ctx = ScanCtx { path, content };
    let mut walker = Walker {
        roots,
        declared_params: declared_params.as_ref(),
        ctx,
        refs: BTreeSet::new(),
        pending_imports: Vec::new(),
        first_root_use: HashMap::new(),
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
        first_root_use,
        ..
    } = walker;

    // Imports are IO: the walker only records facts, the drain here reads and
    // recurses. Relative paths resolve against the importing file's directory;
    // `visited` guards against import cycles.
    if let Some(dir) = file_dir {
        for pending in pending_imports {
            let target = dir.join(&pending.path);
            let target = fs::canonicalize(&target).unwrap_or(target);
            // `import ./dir` means `./dir/default.nix`.
            let target = if target.is_dir() {
                target.join("default.nix")
            } else {
                target
            };
            if !visited.insert(target.clone()) {
                continue;
            }
            let Ok(content) = fs::read_to_string(&target) else {
                warn!(
                    file = %target.display(),
                    "cannot read imported file; its catalog references are not scanned",
                );
                continue;
            };
            // The child is scanned with its own parameter names as roots;
            // its refs are rewritten back into the parent's namespace.
            let rewrites: HashMap<String, String> = match pending.arg {
                ImportArg::Set(forwards) => forwards,
                ImportArg::Root(parent_root) => match top_ident_param(&content) {
                    Some(param) => HashMap::from([(param, parent_root)]),
                    None => {
                        // A pattern parameter destructures the namespace into
                        // names the scanner cannot statically bind, so
                        // anything under the root may be referenced: it
                        // escapes whole rather than being dropped.
                        warn!(
                            file = %target.display(),
                            root = %parent_root,
                            "imported file does not take the namespace as a plain parameter; locking the whole root",
                        );
                        refs.insert(format!("{parent_root}.*"));
                        continue;
                    },
                },
            };
            let child_roots: HashSet<String> = rewrites.keys().cloned().collect();
            let import_dir = target.parent().map(Path::to_path_buf);
            let imported = analyze_file_at(
                &content,
                &child_roots,
                import_dir.as_deref(),
                visited,
                Some(&target),
            )?;
            refs.extend(
                imported
                    .refs
                    .into_iter()
                    .map(|reference| rewrite_root(reference, &rewrites)),
            );
        }
    }

    // With the refs complete (imports included), reject any that resolve
    // through a root the top-level lambda does not declare — they could never
    // evaluate. Roots are checked in sorted order for a deterministic error.
    if let Some(declared) = &declared_params {
        let mut undeclared: Vec<&String> = roots
            .iter()
            .filter(|root| !declared.contains(root.as_str()))
            .collect();
        undeclared.sort();
        for root in undeclared {
            if refs
                .iter()
                .any(|reference| reference_root(reference) == root)
            {
                return Err(ScanError::UndeclaredRoot {
                    root: root.clone(),
                    file: path.map(Path::to_path_buf),
                    position: first_root_use
                        .get(root)
                        .map(|&offset| line_col(content, offset)),
                });
            }
        }
    }

    Ok(FileInfo { refs, deps })
}

/// The root component of a dotted reference (`catalogs.a.b` → `catalogs`).
fn reference_root(reference: &str) -> &str {
    reference.split('.').next().unwrap_or(reference)
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
) -> Result<BTreeSet<String>, ScanError> {
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
            && let Some(info) = resolve_dep(dir, &path, roots)?
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

    Ok(result)
}

/// A name's meaning in the lexical environment threaded through the walk.
#[derive(Clone, Debug, PartialEq)]
enum Binding {
    /// The name resolves to a catalog attr-path: a root itself
    /// (`catalogs` = `Path(["catalogs"])`) or an alias of one. A path may end
    /// in `"*"` when a dynamic component collapsed it.
    Path(Vec<String>),
    /// The name resolves to one of several attr-paths (a conditional alias);
    /// every path is locked so the expression works whichever branch is
    /// taken.
    Paths(BTreeSet<Vec<String>>),
    /// The name is bound to an attrset literal; selecting a member continues
    /// resolution with that member's binding. Members the scanner cannot
    /// model are absent.
    Set(HashMap<String, Binding>),
    /// The name is bound to a lambda defined in this file, carrying its
    /// parameter names. Applying it to a namespace whose name the lambda
    /// binds is not an escape — the body was walked with that binding.
    Lambda(HashSet<String>),
    /// The name is bound to an unapplied `import <static-path>`; applying it
    /// follows the import like a direct `import <path> <arg>`.
    Import(String),
    /// The name is bound to something the scanner cannot model; selects on it
    /// produce no refs.
    Opaque,
}

/// Lexical environment: what each in-scope name is known to be.
type Env = HashMap<String, Binding>;

/// An `import <path> …` application forwarding catalog namespaces, recorded
/// by the walker and resolved by the IO drain loop in [analyze_file_at].
#[derive(Debug)]
struct PendingImport {
    /// Import path as written (relative paths resolve against the importing
    /// file's directory).
    path: String,
    /// How the imported file receives the catalog namespaces.
    arg: ImportArg,
}

/// The catalog-forwarding shape of an import's argument.
#[derive(Debug)]
enum ImportArg {
    /// An attrset argument: child parameter name → parent root it is bound
    /// to (`{ inherit catalogs; }`, `{ cats = catalogs; }`).
    Set(HashMap<String, String>),
    /// The whole argument is a root namespace (`import ./h.nix catalogs`);
    /// the child's lambda parameter binds this parent root.
    Root(String),
}

/// Where an `inherit` appears, which decides what a from-less `inherit x;`
/// means (see [Walker::walk_inherit]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InheritPosition {
    /// Inside a binding scope (`let`, `rec { }`, consumed set literal).
    Binding,
    /// Inside an attrset literal used as a value.
    Value,
}

/// Syntax-tree walker threading the lexical environment.
///
/// The walker emits refs for use sites resolved through the environment
/// (selects, `inherit (…)`, `with`, `getAttr`) and records `import`
/// applications as [PendingImport] facts; it performs no IO itself.
struct Walker<'a> {
    roots: &'a HashSet<String>,
    /// The top-level lambda's parameter names; `None` when the file's shape
    /// hides them (see the declaration check in [analyze_file_at]).
    declared_params: Option<&'a HashSet<String>>,
    ctx: ScanCtx<'a>,
    refs: BTreeSet<String>,
    pending_imports: Vec<PendingImport>,
    /// Byte offset of the first use of each root, for locating an
    /// undeclared-root error after the walk.
    first_root_use: HashMap<String, usize>,
}

impl Walker<'_> {
    fn emit(&mut self, offset: usize, reference: String) {
        self.ctx.report(offset, &reference);
        self.note_root_use(offset, &reference);
        self.refs.insert(reference);
    }

    /// Record where a root was first used (`reference` may be the bare root
    /// name or a dotted path under it).
    fn note_root_use(&mut self, offset: usize, reference: &str) {
        let root = reference_root(reference);
        if !self.first_root_use.contains_key(root) {
            self.first_root_use.insert(root.to_string(), offset);
        }
    }

    fn walk(&mut self, node: &rnix::SyntaxNode, env: &Env) {
        if let Some(attrpath) = ast::Attrpath::cast(node.clone()) {
            // Reached generically from attrset keys and `?` operands: only the
            // dynamic components are expressions, static names are not uses.
            return self.walk_attrpath_dynamics(&attrpath, env);
        }
        if let Some(inherit) = ast::Inherit::cast(node.clone()) {
            return self.walk_inherit(&inherit, env, InheritPosition::Value);
        }
        let Some(expr) = ast::Expr::cast(node.clone()) else {
            return self.walk_children(node, env);
        };
        match expr {
            ast::Expr::Lambda(lambda) => self.walk_lambda(&lambda, env),
            ast::Expr::LetIn(let_in) => self.walk_let_in(&let_in, env),
            ast::Expr::With(with_expr) => self.walk_with(&with_expr, env),
            // `rec { }` scopes like `let`: members see each other.
            ast::Expr::AttrSet(attrset) if attrset.rec_token().is_some() => {
                let inner = recursive_scope_env(&attrset, env);
                self.walk_binding_entries(&attrset, &inner);
            },
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

    /// A bare ident in a value position: using an alias uses the package(s)
    /// it names.
    fn walk_ident(&mut self, ident: &ast::Ident, env: &Env) {
        let Some(name) = ident_name(ident) else {
            return;
        };
        let Some(binding) = env.get(&name) else {
            return;
        };
        self.emit_value_binding(offset_of(ident.syntax()), binding);
    }

    /// Emit the refs for a binding used as a plain value. An alias emits its
    /// paths; a modeled set escaping into unknown code emits every path
    /// reachable through its members.
    fn emit_value_binding(&mut self, offset: usize, binding: &Binding) {
        let paths = escaping_paths_of(binding);
        if !paths.is_empty() {
            self.emit_value_paths(offset, paths);
        }
    }

    /// Emit refs for paths used as plain values (bare idents, from-less
    /// inherits). A package-deep path is an exact ref; a whole catalog or the
    /// root escaping into unknown code widens to a sentinel with a warning,
    /// since anything under it may be accessed.
    fn emit_value_paths(&mut self, offset: usize, paths: BTreeSet<Vec<String>>) {
        for path in paths {
            if path.len() >= 3 || path.last().map(String::as_str) == Some("*") {
                self.emit(offset, join_path(&path));
            } else {
                let reference = join_path(&append_star(path));
                self.ctx.warn_escape(offset, &reference);
                self.note_root_use(offset, &reference);
                self.refs.insert(reference);
            }
        }
    }

    /// [Self::emit_value_paths] restricted to package-deep paths, for
    /// positions where shallow paths are consumed rather than escaping.
    fn emit_deep_paths(&mut self, offset: usize, paths: BTreeSet<Vec<String>>) {
        for path in paths {
            if path.len() >= 3 || path.last().map(String::as_str) == Some("*") {
                self.emit(offset, join_path(&path));
            }
        }
    }

    /// Lambda parameters shadow the outer scope. A parameter named like a
    /// receivable catalog root keeps its root meaning (see
    /// [Self::param_binding]): such lambdas are (almost always) applied to
    /// the namespace itself, and assuming so can only produce spurious refs
    /// that fail loudly, while assuming the opposite drops real refs
    /// silently. Every other parameter is opaque — the scanner cannot know
    /// what it will be applied to.
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
                // The @-name is the whole argument set: members named in the
                // pattern resolve like the parameters themselves, so
                // `args.catalogs.<org>…` stays rooted.
                if let Some(name) = pat
                    .pat_bind()
                    .and_then(|bind| bind.ident())
                    .as_ref()
                    .and_then(ident_name)
                {
                    let members: HashMap<String, Binding> = pat
                        .pat_entries()
                        .filter_map(|entry| entry.ident().as_ref().and_then(ident_name))
                        .map(|entry_name| {
                            let binding = self.param_binding(&entry_name);
                            (entry_name, binding)
                        })
                        .collect();
                    inner.insert(name, Binding::Set(members));
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
    ///
    /// A root-named parameter keeps its root meaning only when the file's
    /// top level can receive that root: a root the file never declares
    /// cannot flow to an inner lambda, so there the name is just a name.
    /// An unrecognized top-level shape fails open and keeps the heuristic.
    fn param_binding(&self, name: &str) -> Binding {
        let receivable = self
            .declared_params
            .is_none_or(|declared| declared.contains(name));
        if receivable && self.roots.contains(name) {
            Binding::Path(vec![name.to_string()])
        } else {
            Binding::Opaque
        }
    }

    fn walk_let_in(&mut self, let_in: &ast::LetIn, env: &Env) {
        let inner = recursive_scope_env(let_in, env);
        self.walk_binding_entries(let_in, &inner);
        if let Some(body) = let_in.body() {
            self.walk(body.syntax(), &inner);
        }
    }

    /// Walk the entries of a binding scope (`let`, `rec { }`, or an attrset
    /// literal consumed as a binding): an entry that models as a binding
    /// walks in binding mode (see [Self::walk_binding_rhs]), everything else
    /// is a value position.
    fn walk_binding_entries(&mut self, scope: &impl HasEntry, env: &Env) {
        for entry in scope.attrpath_values() {
            let attrs: Vec<ast::Attr> = entry
                .attrpath()
                .map(|attrpath| attrpath.attrs().collect())
                .unwrap_or_default();
            if let Some(attrpath) = entry.attrpath() {
                self.walk_attrpath_dynamics(&attrpath, env);
            }
            let Some(value) = entry.value() else { continue };
            let consumed = attrs.len() == 1
                && attrs.first().and_then(attr_static_name).is_some()
                && resolve_binding(&value, env).is_some();
            if consumed {
                self.walk_binding_rhs(&value, env);
            } else {
                self.walk(value.syntax(), env);
            }
        }
        for inherit in scope.inherits() {
            self.walk_inherit(&inherit, env, InheritPosition::Binding);
        }
    }

    /// `inherit (<source>) a b c;` — a name the catalog cannot contain
    /// (dotted quoted attr) collapses to a sentinel at the source. In a
    /// binding scope each name only becomes an alias (use sites drive the
    /// refs, like an alias binding's RHS: only package-deep members are refs
    /// of their own); in a value position each name is a value use of the
    /// member (a catalog-level member escapes and widens).
    ///
    /// A from-less `inherit x;` depends on position: in a binding scope it
    /// only rebinds the outer name; in a value position it puts the named
    /// value into an attrset, which is an ordinary use (and an escape when
    /// the name is a whole namespace).
    fn walk_inherit(&mut self, inherit: &ast::Inherit, env: &Env, position: InheritPosition) {
        let Some(from_expr) = inherit.from().and_then(|from| from.expr()) else {
            if position == InheritPosition::Binding {
                return;
            }
            for attr in inherit.attrs() {
                if let Some(name) = attr_static_name(&attr)
                    && let Some(binding) = env.get(&name)
                {
                    self.emit_value_binding(offset_of(attr.syntax()), binding);
                }
            }
            return;
        };
        let Some(source) = resolve_expr_binding(&from_expr, env) else {
            self.walk(from_expr.syntax(), env);
            return;
        };
        match paths_of(source.clone()) {
            Some(bases) => {
                self.walk_consumed_source(&from_expr, env);
                for attr in inherit.attrs() {
                    let name = attr_static_name(&attr);
                    let paths: BTreeSet<Vec<String>> = bases
                        .iter()
                        .map(|base| match &name {
                            Some(name) => append_component(base.clone(), name.clone()),
                            None => append_star(base.clone()),
                        })
                        .collect();
                    let offset = offset_of(attr.syntax());
                    match position {
                        InheritPosition::Binding => self.emit_deep_paths(offset, paths),
                        InheritPosition::Value => {
                            for path in paths {
                                self.emit(offset, join_path(&widen_if_catalog_level(path)));
                            }
                        },
                    }
                }
            },
            // A modeled-set source: each inherited name is that member's
            // value. In a binding scope the name becomes an alias (use sites
            // drive the refs, like an alias binding's RHS: only package-deep
            // members are refs of their own); in a value position it is an
            // ordinary value use.
            None => {
                let Binding::Set(members) = source else {
                    self.walk(from_expr.syntax(), env);
                    return;
                };
                self.walk_consumed_source(&from_expr, env);
                for attr in inherit.attrs() {
                    let paths = attr_static_name(&attr)
                        .and_then(|name| members.get(&name).cloned())
                        .and_then(paths_of);
                    let Some(paths) = paths else { continue };
                    let offset = offset_of(attr.syntax());
                    match position {
                        InheritPosition::Binding => self.emit_deep_paths(offset, paths),
                        InheritPosition::Value => self.emit_value_paths(offset, paths),
                    }
                }
            },
        }
    }

    fn walk_with(&mut self, with_expr: &ast::With, env: &Env) {
        if let Some(ns) = with_expr.namespace()
            && let Some(paths) = resolve_expr_paths(&ns, env)
        {
            for path in paths {
                self.emit(offset_of(with_expr.syntax()), join_path(&append_star(path)));
            }
            self.walk_consumed_source(&ns, env);
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
            && let Some(target_paths) = resolve_expr_paths(&target, env)
        {
            let key = inner.argument();
            for target_path in target_paths {
                let path = match key.as_ref().and_then(static_str) {
                    Some(key) => append_component(target_path, key),
                    None => append_star(target_path),
                };
                self.emit(
                    offset_of(apply.syntax()),
                    join_path(&widen_if_catalog_level(path)),
                );
            }
            self.walk_consumed_source(&target, env);
            if let Some(key) = key {
                self.walk(key.syntax(), env);
            }
            return;
        }
        match extract_import(apply) {
            Some((Some(path), arg)) => {
                if self.apply_import(path, &arg, env) {
                    return;
                }
            },
            // A dynamic import path cannot be followed; when the argument
            // would forward a namespace, say so rather than failing silently
            // (the argument then escapes analysis as a value below).
            Some((None, arg)) if self.forwards_any_root(&arg, env) => {
                self.ctx.warn_dynamic_import(offset_of(apply.syntax()));
            },
            Some((None, _)) | None => {},
        }
        if let Some(fn_expr) = apply.lambda() {
            match resolve_expr_binding(&fn_expr, env) {
                // Applying a let-bound `import ./file` follows the import
                // like a direct application.
                Some(Binding::Import(path)) => {
                    if let Some(argument) = apply.argument()
                        && self.apply_import(path, &argument, env)
                    {
                        self.walk_consumed_source(&fn_expr, env);
                        return;
                    }
                },
                // Applying a lambda defined in this file: an argument that
                // binds a root under the parameter name the lambda declares
                // for it was already accounted for when the body was walked,
                // so it is consumed rather than escaping.
                Some(Binding::Lambda(params)) => {
                    self.walk_consumed_source(&fn_expr, env);
                    if let Some(argument) = apply.argument() {
                        self.walk_known_lambda_arg(&argument, &params, env);
                    }
                    return;
                },
                _ => {},
            }
        }
        for child in apply.syntax().children() {
            self.walk(&child, env);
        }
    }

    /// Handle the application of `import <path>` to `arg` — direct or through
    /// a bound name. Records the pending import and consumes the argument
    /// when its forwarding shape is recognized; returns whether it was.
    fn apply_import(&mut self, path: String, arg: &ast::Expr, env: &Env) -> bool {
        match arg {
            ast::Expr::AttrSet(attrset) => {
                let forwards = self.import_forwards(attrset, env);
                if !forwards.is_empty() {
                    // Forwarding is a use of each parent root: refs surfacing
                    // from the import trace back to this argument.
                    let offset = offset_of(attrset.syntax());
                    for parent_root in forwards.values() {
                        self.note_root_use(offset, parent_root);
                    }
                    self.pending_imports.push(PendingImport {
                        path,
                        arg: ImportArg::Set(forwards),
                    });
                }
                self.walk_import_arg(attrset, env);
                true
            },
            other => {
                // `import ./h.nix catalogs` — the whole namespace is the
                // argument, consumed by following the import.
                if let Some(Binding::Path(root_path)) = resolve_expr_binding(other, env)
                    && let [root] = root_path.as_slice()
                {
                    self.note_root_use(offset_of(other.syntax()), root);
                    self.pending_imports.push(PendingImport {
                        path,
                        arg: ImportArg::Root(root.clone()),
                    });
                    return true;
                }
                false
            },
        }
    }

    /// Walk the argument of an application whose callee is a known lambda
    /// with parameter names `params`. A binding is covered — and therefore
    /// consumed — when the lambda body saw the same root under the same name;
    /// everything else is an ordinary value position.
    fn walk_known_lambda_arg(&mut self, argument: &ast::Expr, params: &HashSet<String>, env: &Env) {
        let covered = |name: &str, binding: Option<Binding>| {
            params.contains(name)
                && self.roots.contains(name)
                && matches!(binding, Some(Binding::Path(path)) if path == vec![name.to_string()])
        };
        match argument {
            // `mkPkg catalogs` where the lambda's plain parameter is the
            // root's own name.
            ast::Expr::Ident(ident) => {
                if let Some(name) = ident_name(ident) {
                    if covered(&name, env.get(&name).cloned()) {
                        return;
                    }
                    // A modeled set passed whole (`mkPkg args`): members the
                    // lambda binds under the same root name were walked in
                    // the body; only the rest escapes.
                    if let Some(Binding::Set(members)) = env.get(&name) {
                        let uncovered: BTreeSet<Vec<String>> = members
                            .iter()
                            .filter(|(member, binding)| !covered(member, Some((*binding).clone())))
                            .flat_map(|(_, binding)| escaping_paths_of(binding))
                            .collect();
                        if !uncovered.is_empty() {
                            self.emit_value_paths(offset_of(ident.syntax()), uncovered);
                        }
                        return;
                    }
                }
                self.walk(argument.syntax(), env);
            },
            ast::Expr::AttrSet(attrset) if attrset.rec_token().is_none() => {
                for entry in attrset.attrpath_values() {
                    if let Some(attrpath) = entry.attrpath() {
                        self.walk_attrpath_dynamics(&attrpath, env);
                    }
                    let Some(value) = entry.value() else { continue };
                    let attrs: Vec<ast::Attr> = entry
                        .attrpath()
                        .map(|attrpath| attrpath.attrs().collect())
                        .unwrap_or_default();
                    let name = (attrs.len() == 1)
                        .then(|| attrs.first().and_then(attr_static_name))
                        .flatten();
                    if let Some(name) = name
                        && covered(&name, resolve_expr_binding(&value, env))
                    {
                        continue;
                    }
                    self.walk(value.syntax(), env);
                }
                for inherit in attrset.inherits() {
                    if inherit.from().is_some() {
                        self.walk_inherit(&inherit, env, InheritPosition::Value);
                        continue;
                    }
                    for attr in inherit.attrs() {
                        let Some(name) = attr_static_name(&attr) else {
                            continue;
                        };
                        if covered(&name, env.get(&name).cloned()) {
                            continue;
                        }
                        if let Some(binding) = env.get(&name) {
                            self.emit_value_binding(offset_of(attr.syntax()), binding);
                        }
                    }
                }
            },
            other => self.walk(other.syntax(), env),
        }
    }

    /// Whether an import argument would forward a whole root namespace.
    fn forwards_any_root(&self, arg: &ast::Expr, env: &Env) -> bool {
        match arg {
            ast::Expr::AttrSet(attrset) => {
                attrset.attrpath_values().any(|entry| {
                    entry
                        .value()
                        .and_then(|value| resolve_expr_binding(&value, env))
                        .is_some_and(|binding| is_root_binding(&binding))
                }) || attrset.inherits().any(|inherit| {
                    inherit.from().is_none()
                        && inherit.attrs().any(|attr| {
                            attr_static_name(&attr)
                                .and_then(|name| env.get(&name))
                                .is_some_and(is_root_binding)
                        })
                })
            },
            other => {
                resolve_expr_binding(other, env).is_some_and(|binding| is_root_binding(&binding))
            },
        }
    }

    /// Resolve which names an import's argument attrset forwards a whole
    /// root under (`inherit catalogs;`, `catalogs = catalogs;`,
    /// `cats = catalogs;`). A binding that *names* a root but is bound to
    /// something else is not forwarded; [Self::walk_import_arg] warns about
    /// it — the imported file will not be scanned through it.
    fn import_forwards(&self, arg: &ast::AttrSet, env: &Env) -> HashMap<String, String> {
        let mut forwards = HashMap::new();
        for entry in arg.attrpath_values() {
            let Some(attrpath) = entry.attrpath() else {
                continue;
            };
            let attrs: Vec<ast::Attr> = attrpath.attrs().collect();
            if attrs.len() != 1 {
                continue;
            }
            let Some(name) = attrs.first().and_then(attr_static_name) else {
                continue;
            };
            let Some(value) = entry.value() else { continue };
            if let Some(Binding::Path(path)) = resolve_expr_binding(&value, env)
                && let [root] = path.as_slice()
            {
                forwards.insert(name, root.clone());
            }
        }
        for inherit in arg.inherits() {
            let from_binding = inherit
                .from()
                .and_then(|from| from.expr())
                .and_then(|from_expr| resolve_expr_binding(&from_expr, env));
            for attr in inherit.attrs() {
                let Some(name) = attr_static_name(&attr) else {
                    continue;
                };
                let binding = match (inherit.from().is_some(), &from_binding) {
                    (false, _) => env.get(&name).cloned(),
                    (true, Some(Binding::Set(members))) => members.get(&name).cloned(),
                    (true, _) => None,
                };
                if let Some(Binding::Path(path)) = binding
                    && let [root] = path.as_slice()
                {
                    forwards.insert(name, root.clone());
                }
            }
        }
        forwards
    }

    /// Walk an import's argument attrset. An entry whose name forwards a
    /// namespace (per [Self::import_forwards]) is consumed — the imported
    /// file is scanned through it — so it is not an escape; a root-named
    /// entry that does not forward is warned about, and every other entry is
    /// an ordinary value position.
    fn walk_import_arg(&mut self, attrset: &ast::AttrSet, env: &Env) {
        let forwards = self.import_forwards(attrset, env);
        for entry in attrset.attrpath_values() {
            if let Some(attrpath) = entry.attrpath() {
                self.walk_attrpath_dynamics(&attrpath, env);
            }
            let Some(value) = entry.value() else { continue };
            let attrs: Vec<ast::Attr> = entry
                .attrpath()
                .map(|attrpath| attrpath.attrs().collect())
                .unwrap_or_default();
            let name = (attrs.len() == 1)
                .then(|| attrs.first().and_then(attr_static_name))
                .flatten();
            if let Some(name) = &name {
                if forwards.contains_key(name) {
                    self.walk_consumed_source(&value, env);
                    continue;
                }
                if self.roots.contains(name) {
                    let offset = entry
                        .attrpath()
                        .map(|attrpath| offset_of(attrpath.syntax()))
                        .unwrap_or_else(|| offset_of(value.syntax()));
                    self.ctx.warn_unfollowed_import(offset, name);
                }
            }
            self.walk(value.syntax(), env);
        }
        for inherit in attrset.inherits() {
            let from_expr = inherit.from().and_then(|from| from.expr());
            let from_binding = from_expr
                .as_ref()
                .and_then(|from_expr| resolve_expr_binding(from_expr, env));
            if let Some(from_expr) = &from_expr {
                match &from_binding {
                    Some(_) => self.walk_consumed_source(from_expr, env),
                    None => self.walk(from_expr.syntax(), env),
                }
            }
            for attr in inherit.attrs() {
                let Some(name) = attr_static_name(&attr) else {
                    continue;
                };
                if forwards.contains_key(&name) {
                    continue;
                }
                if self.roots.contains(&name) {
                    self.ctx
                        .warn_unfollowed_import(offset_of(attr.syntax()), &name);
                }
                let binding = match (from_expr.is_some(), &from_binding) {
                    (false, _) => env.get(&name).cloned(),
                    (true, Some(source)) => member_binding(source, &name),
                    (true, None) => None,
                };
                if let Some(binding) = binding {
                    self.emit_value_binding(offset_of(attr.syntax()), &binding);
                }
            }
        }
    }

    fn walk_select(&mut self, select: &ast::Select, env: &Env) {
        match resolve_expr_binding(&ast::Expr::Select(select.clone()), env) {
            Some(binding) => {
                match paths_of(binding.clone()) {
                    Some(paths) => {
                        for path in paths {
                            self.emit(
                                offset_of(select.syntax()),
                                join_path(&widen_if_catalog_level(path)),
                            );
                        }
                    },
                    // The select reaches a modeled set (`t.sub`) used as a
                    // value: its members escape.
                    None => self.emit_value_binding(offset_of(select.syntax()), &binding),
                }
                if let Some(base) = select.expr() {
                    self.walk_consumed_source(&base, env);
                }
            },
            None => self.walk_unresolved_select(select, env),
        }
        self.walk_select_parts(select, env);
    }

    /// Walk a select whose path did not resolve. A base that is a modeled
    /// set is consumed when the path is static — selecting an unknown member
    /// cannot leak the other members — while a dynamic component may select
    /// any member, so the set's paths escape (collapsed). Any other base is
    /// an ordinary value position.
    fn walk_unresolved_select(&mut self, select: &ast::Select, env: &Env) {
        let Some(base) = select.expr() else { return };
        match resolve_expr_binding(&base, env) {
            Some(binding @ Binding::Set(_)) => {
                let dynamic = select.attrpath().is_some_and(|attrpath| {
                    attrpath
                        .attrs()
                        .any(|attr| attr_static_name(&attr).is_none())
                });
                if dynamic {
                    let paths: BTreeSet<Vec<String>> = escaping_paths_of(&binding)
                        .into_iter()
                        .map(append_star)
                        .collect();
                    if !paths.is_empty() {
                        self.emit_value_paths(offset_of(select.syntax()), paths);
                    }
                }
                self.walk_consumed_source(&base, env);
            },
            _ => self.walk(base.syntax(), env),
        }
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

    /// A resolved (consumed) source expression — an inherit source, `with`
    /// namespace, `getAttr` target, or select base — may still contain
    /// dynamic components with refs of their own
    /// (`inherit (catalogs.${catalogs.a.name}) x;`); scan those.
    fn walk_consumed_source(&mut self, expr: &ast::Expr, env: &Env) {
        match expr {
            ast::Expr::Select(select) => {
                if let Some(base) = select.expr() {
                    self.walk_consumed_source(&base, env);
                }
                self.walk_select_parts(select, env);
            },
            ast::Expr::Paren(paren) => {
                if let Some(inner) = paren.expr() {
                    self.walk_consumed_source(&inner, env);
                }
            },
            _ => {},
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
                match resolve_select_paths(select, env) {
                    Some(paths) => {
                        for path in paths {
                            if path.len() >= 3 {
                                self.emit(offset_of(select.syntax()), join_path(&path));
                            }
                        }
                        if let Some(base) = select.expr() {
                            self.walk_consumed_source(&base, env);
                        }
                    },
                    None => self.walk_unresolved_select(select, env),
                }
                self.walk_select_parts(select, env);
            },
            // Branches of a conditional alias are themselves binding
            // positions; the condition is an ordinary value.
            ast::Expr::IfElse(if_else) => {
                if let Some(condition) = if_else.condition() {
                    self.walk(condition.syntax(), env);
                }
                for branch in [if_else.body(), if_else.else_body()].into_iter().flatten() {
                    self.walk_binding_rhs(&branch, env);
                }
            },
            ast::Expr::Ident(_) => {},
            ast::Expr::Paren(paren) => {
                if let Some(inner) = paren.expr() {
                    self.walk_binding_rhs(&inner, env);
                }
            },
            // An attrset literal consumed as a binding: its members follow
            // the binding policy too (`s = { org = catalogs.myorg; }` only
            // defines aliases reachable through `s`).
            ast::Expr::AttrSet(attrset) => {
                let scope = if attrset.rec_token().is_some() {
                    recursive_scope_env(attrset, env)
                } else {
                    env.clone()
                };
                self.walk_binding_entries(attrset, &scope);
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

/// Environment for a recursive binding scope: `let … in` or `rec { }`.
///
/// Every bound name shadows the outer scope. Both constructs are recursive,
/// so bindings that alias catalog paths are resolved to a fixpoint: a pass
/// may enable another binding regardless of source order, and names that
/// never resolve stay opaque (which is what makes `let catalogs = …;` shadow
/// a root).
fn recursive_scope_env(scope: &impl HasEntry, outer: &Env) -> Env {
    let mut env = outer.clone();

    let mut values: Vec<(String, ast::Expr)> = Vec::new();
    for entry in scope.attrpath_values() {
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
    for inherit in scope.inherits() {
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

    // Bindings are re-resolved every pass, including ones already resolved:
    // a `Paths` or `Set` resolved early may reference names that only
    // resolve on a later pass. A self-referential binding
    // (`a = if c then a.sub else catalogs.x`) could otherwise grow its path
    // set forever, so passes are capped at the number of bindings — enough
    // for any acyclic reference chain to settle.
    let inherit_count: usize = scope
        .inherits()
        .map(|inherit| inherit.attrs().count())
        .sum();
    let max_passes = values.len() + inherit_count + 1;
    for _ in 0..max_passes {
        let mut changed = false;
        for (name, value) in &values {
            let Some(binding) = resolve_binding(value, &env) else {
                continue;
            };
            if env.get(name) != Some(&binding) {
                env.insert(name.clone(), binding);
                changed = true;
            }
        }
        for inherit in scope.inherits() {
            let Some(from_expr) = inherit.from().and_then(|from| from.expr()) else {
                continue;
            };
            let Some(source) = resolve_expr_binding(&from_expr, &env) else {
                continue;
            };
            for attr in inherit.attrs() {
                let Some(name) = attr_static_name(&attr) else {
                    continue;
                };
                let Some(binding) = member_binding(&source, &name) else {
                    continue;
                };
                if env.get(&name) != Some(&binding) {
                    env.insert(name, binding);
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
    match expr {
        ast::Expr::AttrSet(attrset) => Some(attr_set_binding(attrset, env)),
        ast::Expr::Lambda(lambda) => Some(Binding::Lambda(lambda_param_names(lambda))),
        _ => resolve_expr_binding(expr, env),
    }
}

/// The names a lambda's parameter binds (a plain parameter or the entries of
/// a pattern).
fn lambda_param_names(lambda: &ast::Lambda) -> HashSet<String> {
    let mut names = HashSet::new();
    match lambda.param() {
        Some(ast::Param::IdentParam(param)) => {
            names.extend(param.ident().as_ref().and_then(ident_name));
        },
        Some(ast::Param::Pattern(pat)) => {
            for entry in pat.pat_entries() {
                names.extend(entry.ident().as_ref().and_then(ident_name));
            }
        },
        None => {},
    }
    names
}

/// Model an attrset literal as a [Binding::Set] of its statically-known
/// members. A `rec { }` literal resolves its members in its own scope.
fn attr_set_binding(attrset: &ast::AttrSet, env: &Env) -> Binding {
    let scope = if attrset.rec_token().is_some() {
        recursive_scope_env(attrset, env)
    } else {
        env.clone()
    };
    let mut members: HashMap<String, Binding> = HashMap::new();
    for entry in attrset.attrpath_values() {
        let Some(attrpath) = entry.attrpath() else {
            continue;
        };
        let attrs: Vec<ast::Attr> = attrpath.attrs().collect();
        if attrs.len() != 1 {
            continue;
        }
        let Some(name) = attrs.first().and_then(attr_static_name) else {
            continue;
        };
        let Some(value) = entry.value() else { continue };
        if let Some(binding) = resolve_binding(&value, &scope) {
            members.insert(name, binding);
        }
    }
    for inherit in attrset.inherits() {
        match inherit.from().and_then(|from| from.expr()) {
            Some(from_expr) => {
                if let Some(source) = resolve_expr_binding(&from_expr, &scope) {
                    for attr in inherit.attrs() {
                        if let Some(name) = attr_static_name(&attr)
                            && let Some(binding) = member_binding(&source, &name)
                        {
                            members.insert(name, binding);
                        }
                    }
                }
            },
            None => {
                for attr in inherit.attrs() {
                    if let Some(name) = attr_static_name(&attr)
                        && let Some(binding) = scope.get(&name)
                    {
                        members.insert(name, binding.clone());
                    }
                }
            },
        }
    }
    Binding::Set(members)
}

/// Resolve an expression to its binding, following the environment for idents
/// and stepping through attrset members for selects. `None` when the
/// expression does not reach anything the scanner can model.
fn resolve_expr_binding(expr: &ast::Expr, env: &Env) -> Option<Binding> {
    match expr {
        ast::Expr::Ident(ident) => match env.get(&ident_name(ident)?)? {
            Binding::Opaque => None,
            binding => Some(binding.clone()),
        },
        ast::Expr::Select(select) => {
            let mut current = resolve_expr_binding(&select.expr()?, env)?;
            for attr in select.attrpath()?.attrs() {
                current = match (current, attr_static_name(&attr)) {
                    (Binding::Path(path), Some(name)) => {
                        Binding::Path(append_component(path, name))
                    },
                    // A dynamic component collapses the path and consumes
                    // whatever follows.
                    (Binding::Path(path), None) => return Some(Binding::Path(append_star(path))),
                    (Binding::Paths(paths), Some(name)) => Binding::Paths(
                        paths
                            .into_iter()
                            .map(|path| append_component(path, name.clone()))
                            .collect(),
                    ),
                    (Binding::Paths(paths), None) => {
                        return Some(Binding::Paths(paths.into_iter().map(append_star).collect()));
                    },
                    (Binding::Set(members), Some(name)) => members.get(&name)?.clone(),
                    (Binding::Set(_), None) => return None,
                    (Binding::Lambda(_) | Binding::Import(_), _) => return None,
                    (Binding::Opaque, _) => return None,
                };
            }
            match current {
                Binding::Opaque => None,
                binding => Some(binding),
            }
        },
        // An unapplied `import <static-path>` — applying the bound name later
        // follows the import.
        ast::Expr::Apply(apply) => {
            let ast::Expr::Ident(import_fn) = apply.lambda()? else {
                return None;
            };
            if import_fn.ident_token()?.text() != "import" {
                return None;
            }
            static_path_str(&apply.argument()?).map(Binding::Import)
        },
        // A conditional resolves to whichever branches the scanner can model;
        // all of them must be locked.
        ast::Expr::IfElse(if_else) => {
            let mut paths: BTreeSet<Vec<String>> = BTreeSet::new();
            for branch in [if_else.body(), if_else.else_body()].into_iter().flatten() {
                match resolve_expr_binding(&branch, env) {
                    Some(Binding::Path(path)) => {
                        paths.insert(path);
                    },
                    Some(Binding::Paths(more)) => paths.extend(more),
                    _ => {},
                }
            }
            (!paths.is_empty()).then_some(Binding::Paths(paths))
        },
        ast::Expr::Paren(paren) => resolve_expr_binding(&paren.expr()?, env),
        _ => None,
    }
}

/// The attr-paths a binding denotes: one for an alias, several for a
/// conditional alias, none for sets and opaque bindings.
fn paths_of(binding: Binding) -> Option<BTreeSet<Vec<String>>> {
    match binding {
        Binding::Path(path) => Some(BTreeSet::from([path])),
        Binding::Paths(paths) => Some(paths),
        _ => None,
    }
}

/// The attr-paths reachable through a binding when its value escapes into
/// code the scanner cannot follow: an alias's own paths, or every path
/// reachable through the members of a modeled set (recursively). Bindings
/// that carry no catalog paths contribute nothing.
fn escaping_paths_of(binding: &Binding) -> BTreeSet<Vec<String>> {
    match binding {
        Binding::Path(path) => BTreeSet::from([path.clone()]),
        Binding::Paths(paths) => paths.clone(),
        Binding::Set(members) => members.values().flat_map(escaping_paths_of).collect(),
        _ => BTreeSet::new(),
    }
}

/// Resolve an expression to the set of attr-paths it may denote.
fn resolve_expr_paths(expr: &ast::Expr, env: &Env) -> Option<BTreeSet<Vec<String>>> {
    paths_of(resolve_expr_binding(expr, env)?)
}

/// [resolve_expr_paths] for a select node.
fn resolve_select_paths(select: &ast::Select, env: &Env) -> Option<BTreeSet<Vec<String>>> {
    paths_of(resolve_expr_binding(
        &ast::Expr::Select(select.clone()),
        env,
    )?)
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

/// The binding `inherit (<source>) <name>;` produces for `name`, given the
/// source's binding: one more component on an attr-path, or a member of a
/// modeled set.
fn member_binding(source: &Binding, name: &str) -> Option<Binding> {
    match source {
        Binding::Path(base) => Some(Binding::Path(append_component(
            base.clone(),
            name.to_string(),
        ))),
        Binding::Paths(bases) => Some(Binding::Paths(
            bases
                .iter()
                .cloned()
                .map(|base| append_component(base, name.to_string()))
                .collect(),
        )),
        Binding::Set(members) => members.get(name).cloned(),
        _ => None,
    }
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

/// Recognize an `import <path> <arg>` application, returning the import path
/// (`None` when it is not statically known) and the argument expression.
fn extract_import(apply: &ast::Apply) -> Option<(Option<String>, ast::Expr)> {
    let inner = inner_apply(apply)?;
    let ast::Expr::Ident(import_fn) = inner.lambda()? else {
        return None;
    };
    if import_fn.ident_token()?.text() != "import" {
        return None;
    }
    let path = static_path_str(&inner.argument()?);
    Some((path, apply.argument()?))
}

/// Whether a binding is a whole catalog root.
fn is_root_binding(binding: &Binding) -> bool {
    matches!(binding, Binding::Path(path) if path.len() == 1)
}

/// The name of a file's top-level plain lambda parameter (`cats: …`), if any.
fn top_ident_param(content: &str) -> Option<String> {
    let root = rnix::Root::parse(content).tree();
    let Some(ast::Expr::Lambda(lambda)) = root.expr() else {
        return None;
    };
    let Some(ast::Param::IdentParam(param)) = lambda.param() else {
        return None;
    };
    param.ident().as_ref().and_then(ident_name)
}

/// Rewrite a child-rooted reference back to the parent's namespace using the
/// child-name → parent-root map of the import that forwarded it.
fn rewrite_root(reference: String, rewrites: &HashMap<String, String>) -> String {
    match reference.split_once('.') {
        Some((root, rest)) => match rewrites.get(root) {
            Some(parent) => format!("{parent}.{rest}"),
            None => reference,
        },
        None => reference,
    }
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
fn resolve_dep(
    dir: &Path,
    components: &[String],
    roots: &HashSet<String>,
) -> Result<Option<FileInfo>, ScanError> {
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
        return Ok(None);
    }
    Ok(None)
}

/// Read and analyze a resolved package file.
///
/// Relative imports in the file resolve against its own directory, so the
/// file's parent is passed as the import base. An unreadable file resolves to
/// `Ok(None)`; only scan failures are errors.
fn read_and_analyze(path: &Path, roots: &HashSet<String>) -> Result<Option<FileInfo>, ScanError> {
    let Ok(content) = fs::read_to_string(path) else {
        return Ok(None);
    };
    analyze_file_at(
        &content,
        roots,
        path.parent(),
        &mut HashSet::new(),
        Some(path),
    )
    .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn analyze_file(content: &str, roots: &HashSet<String>) -> FileInfo {
        analyze_file_at(content, roots, None, &mut HashSet::new(), None)
            .expect("scan should succeed")
    }

    fn refs(content: &str, roots: &HashSet<String>) -> BTreeSet<String> {
        analyze_file(content, roots).refs
    }

    fn scan_err(content: &str, roots: &HashSet<String>) -> ScanError {
        analyze_file_at(content, roots, None, &mut HashSet::new(), None)
            .expect_err("scan should fail")
    }

    fn refs_at(path: &str, roots: &HashSet<String>) -> BTreeSet<String> {
        let path = Path::new(path);
        let content = fs::read_to_string(path).expect("test fixture missing");
        let dir = path.parent();
        let mut visited = HashSet::new();
        analyze_file_at(&content, roots, dir, &mut visited, Some(path))
            .expect("scan should succeed")
            .refs
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

        let got = collect_transitive(db, Path::new("."), &r).unwrap();
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

        let got = collect_transitive(db, Path::new("."), &r).unwrap();
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

        let got = collect_transitive(db, Path::new("."), &r).unwrap();
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
        let got = scan_package(base_dir, Path::new("dep-entry.nix")).unwrap();
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
        let got = scan_package(base_dir, Path::new("entry.nix")).unwrap();
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
        let got = scan_package(base_dir, Path::new("isdr-zk-client.nix")).unwrap();
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
        let got = scan_package(base_dir, Path::new("foo/bar.nix")).unwrap();
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
        let got = scan_package(base_dir, Path::new("top.nix")).unwrap();
        assert_eq!(
            got,
            refset(&["catalogs.myorg.widget-src", "catalogs.myorg.helper-lib-src"]),
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
            for reference in scan_package(dir, Path::new(rel)).unwrap() {
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
    fn rec_attrset_members_alias_each_other() {
        // `rec { }` scopes like `let`: `org` is visible to `pkg`, and its
        // catalog-level RHS only defines the alias.
        let got = refs(
            "{ catalogs }: rec { org = catalogs.myorg; pkg = org.toolkit; }",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit"]));
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
                refs(content, &roots(&["catalogs"])),
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
            &roots(&["catalogs"]),
        );
        assert!(got.contains("catalogs.b.pkg"), "got: {got:?}");
    }

    #[test]
    fn attrset_member_alias_resolves_through_select() {
        let got = refs(
            "{ catalogs }: let s = { org = catalogs.myorg; }; in s.org.toolkit",
            &roots(&["catalogs"]),
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
                refs(content, &roots(&["catalogs"])),
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
                refs(content, &roots(&["catalogs"])),
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
                refs(content, &roots(&["catalogs"])),
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
                refs(content, &roots(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
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
                refs(content, &roots(&["catalogs"])),
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
    fn import_renamed_root_followed_and_rewritten() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-renamed.nix",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
    }

    #[test]
    fn import_arg_namespace_not_forwarded_escapes() {
        let cases: &[(&str, &[&str])] = &[
            // A catalog-level alias as an import argument is not forwarded
            // (only whole roots are), so the child uses the namespace where
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
                refs(content, &roots(&["catalogs"])),
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
                refs(content, &roots(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn import_whole_root_argument_followed() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-whole.nix",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.whole-pkg"]));
    }

    #[test]
    fn import_whole_root_to_pattern_param_escapes() {
        // The helper destructures the namespace with a pattern parameter;
        // its entries are namespace members, not roots, so they cannot be
        // bound statically and the whole root escapes rather than being
        // dropped.
        let got = refs_at(
            "test_data/catalog_refs/import-entry-pattern.nix",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.*"]));
    }

    #[test]
    fn import_root_name_bound_to_other_value_not_followed() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-shadowed.nix",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.direct-pkg"]));
    }

    #[test]
    fn import_directory_target_resolves_default_nix() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-dir.nix",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.dir-pkg"]));
    }

    #[test]
    fn import_dynamic_path_forwarding_root_is_conservative() {
        // The import target cannot be read, so the forwarded namespace
        // escapes analysis (a warning points at the dynamic path).
        let got = refs(
            "{ catalogs, p }: import p { inherit catalogs; }",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.*"]));
    }

    #[test]
    fn import_let_bound_function_followed() {
        let got = refs_at(
            "test_data/catalog_refs/import-entry-letbound.nix",
            &roots(&["catalogs"]),
        );
        assert_eq!(got, set(&["catalogs.myorg.toolkit.readVersion"]));
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
                scan_err(content, &roots(&["catalogs"])),
                ScanError::UndeclaredRoot {
                    root: "catalogs".to_string(),
                    file: None,
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
                refs(content, &roots(&["catalogs"])),
                set(expected),
                "content: {content}"
            );
        }
    }

    #[test]
    fn undeclared_root_without_references_is_not_an_error() {
        let content = "{ mkDerivation }: mkDerivation { pname = \"tool\"; }";
        assert_eq!(refs(content, &roots(&["catalogs"])), BTreeSet::new());
    }

    #[test]
    fn let_bound_root_name_shadows_without_error() {
        let content = "{ config }:\nlet catalogs = config;\nin catalogs.myorg.toolkit.readVersion";
        assert_eq!(refs(content, &roots(&["catalogs"])), BTreeSet::new());
    }

    #[test]
    fn unrecognized_top_level_shape_scans_leniently() {
        // A let-wrapped file still evaluates to a function, but the scanner
        // cannot see its parameters; the declaration check fails open and the
        // refs are kept.
        let content = "let version = \"1.0\";\nin { mkDerivation }:\nmkDerivation { v = catalogs.myorg.pkg.readVersion; }";
        assert_eq!(
            refs(content, &roots(&["catalogs"])),
            set(&["catalogs.myorg.pkg.readVersion"])
        );
    }

    #[test]
    fn undeclared_root_forwarded_to_import_errors_at_forward_site() {
        let path = Path::new("test_data/catalog_refs/undeclared-forward/entry.nix");
        let content = fs::read_to_string(path).expect("test fixture missing");
        let err = analyze_file_at(
            &content,
            &roots(&["catalogs"]),
            path.parent(),
            &mut HashSet::new(),
            Some(path),
        )
        .expect_err("scan should fail");
        assert_eq!(err, ScanError::UndeclaredRoot {
            root: "catalogs".to_string(),
            file: Some(path.to_path_buf()),
            position: Some((6, 35)),
        });
    }

    #[test]
    fn undeclared_root_error_message_points_at_the_arguments() {
        let err = ScanError::UndeclaredRoot {
            root: "catalogs".to_string(),
            file: Some(PathBuf::from("pkgs/foo.nix")),
            position: Some((4, 13)),
        };
        assert_eq!(err.to_string(), indoc::indoc! {"
                'catalogs' is referenced at pkgs/foo.nix:4:13 but is not declared in the function arguments.
                Add 'catalogs' to the function arguments, e.g. '{ catalogs, ... }:'."});
    }
}
