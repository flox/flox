//! Analyze a single NEF expression. `analyze_file_at` parses a file,
//! discovers the catalog references it makes, and follows `import` forwards
//! into the files they name. The syntax-tree walk that finds the references
//! (`Walker`) records facts only; `analyze_file_at` performs the import IO
//! and is the module's entry point.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use rnix::ast;
use rnix::ast::HasEntry;
use rowan::ast::AstNode;
use tracing::{debug, warn};

use super::{CatalogRef, ScanError};

/// Catalog references and dependency attr-paths extracted from one file.
#[derive(Debug)]
pub(super) struct FileInfo {
    /// Fully-qualified catalog attr-paths referenced by the file
    /// (e.g. `catalogs.myorg.toolkit.readVersion`). Tracked as [CatalogRef]
    /// from the scan result onward; the walker builds them as strings.
    pub(super) refs: BTreeSet<CatalogRef>,
    /// Attr-paths of the packages this file depends on, resolved by the
    /// package graph's closure expansion. The first component is the
    /// dependency argument; any further components are members selected on it
    /// (a sibling attribute set), e.g. `["python3Packages", "isdr-zk-client"]`
    /// for `python3Packages.isdr-zk-client`. A bare argument is a single
    /// component.
    pub(super) dependency_args: Vec<Vec<String>>,
}

/// Source context for verbose reference reporting: the file a reference was
/// found in plus its text, used to turn a byte offset into a
/// 1-based `line:column`.
#[derive(Clone, Debug)]
struct ScanCtx<'a> {
    path: &'a Path,
    content: &'a str,
}

impl ScanCtx<'_> {
    /// Emit a `debug` event locating one discovered reference at `offset` (a
    /// byte offset into the file). Surfaced by `lock --verbose`.
    fn report(&self, offset: usize, reference: &str) {
        let (line, column) = line_col(self.content, offset);
        debug!(reference, file = %self.path.display(), line, column, "catalog reference");
    }

    /// Warn that an import with a dynamic path forwards a catalog namespace
    /// the scanner cannot follow.
    fn warn_dynamic_import(&self, offset: usize) {
        let (line, column) = line_col(self.content, offset);
        warn!(
            file = %self.path.display(),
            line,
            column,
            "import path is not statically known; the imported file is not scanned for catalog references",
        );
    }

    /// Warn that an import argument names a catalog root without forwarding
    /// it, so the imported file is not scanned through that name.
    fn warn_unfollowed_import(&self, offset: usize, name: &str) {
        let (line, column) = line_col(self.content, offset);
        warn!(
            name,
            file = %self.path.display(),
            line,
            column,
            "import argument is not the catalog namespace; the imported file is not scanned through it",
        );
    }

    /// Warn that a catalog namespace escapes static analysis at `offset`,
    /// widening to `reference`.
    fn warn_escape(&self, offset: usize, reference: &str) {
        let (line, column) = line_col(self.content, offset);
        warn!(
            reference,
            file = %self.path.display(),
            line,
            column,
            "catalog namespace escapes static analysis; locking the whole subtree",
        );
    }
}

/// Resolve a byte `offset` into `content` to a 1-based `(line, column)`.
pub(super) fn line_col(content: &str, offset: usize) -> (usize, usize) {
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
/// `root_attributes` are the lambda parameters treated as catalog root_attributes (e.g. `catalogs`).
/// When `file_dir` is `Some`, `import` calls forwarding a root are followed into
/// the imported file; `visited` maps each drained target file to the top-level
/// forwardings already scanned for it (see the drain below). `path` is the
/// file's location, recorded in scan errors and used for verbose reference
/// reporting. `root_origins` maps each root to the top-level root it stands
/// for — the identity map at the entry file, and the composition of the
/// forwarding chain in imported files.
pub(super) fn analyze_file_at(
    content: &str,
    root_attributes: &HashSet<String>,
    file_dir: Option<&Path>,
    visited: &mut HashMap<PathBuf, HashSet<BTreeMap<String, String>>>,
    path: &Path,
    root_origins: &BTreeMap<String, String>,
) -> Result<FileInfo, ScanError> {
    debug!(file = %path.display(), "reading NEF expression");

    let parse = rnix::Root::parse(content);
    let root = parse.tree();

    // Dependency arguments come from the eventual package function: wrappers
    // around it (`let … in`, `with …;`, parentheses) do not change which
    // lambda NEF calls, so they are looked through.
    let mut dependency_arg_names = HashSet::new();
    if let Some(lambda) = top_level_lambda(&root)
        && let Some(rnix::ast::Param::Pattern(pat)) = lambda.param()
    {
        for entry in pat.pat_entries() {
            if let Some(ident) = entry.ident()
                && let Some(name) = ident.ident_token().map(|t| t.text().to_string())
                && !root_attributes.contains(name.as_str())
            {
                dependency_arg_names.insert(name);
            }
        }
    }

    // The parameter names the top-level lambda declares. Only declared
    // arguments are supplied when NEF calls the file (callPackage semantics),
    // so a root referenced without being declared can never resolve; that is
    // reported as [ScanError::UndeclaredRoot] after the walk. `None` fails
    // open: the check applies only to a plain top-level lambda, because a
    // wrapper's own bindings could satisfy names the check would call
    // undeclared, and the lenient seeding below is kept without it.
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
    let mut dependency_args: Vec<Vec<String>> = dependency_arg_names
        .iter()
        .map(|name| vec![name.clone()])
        .collect();
    collect_dependency_selections(root.syntax(), &dependency_arg_names, &mut dependency_args);

    let ctx = ScanCtx { path, content };
    let mut walker = Walker::new(root_attributes, declared_params.as_ref(), ctx);
    walker.walk_root(&root);
    let (mut refs, pending_imports, first_root_use) = walker.finish();

    // Imports are IO: the walker only records facts, the drain here reads and
    // recurses. Relative paths resolve against the importing file's directory.
    if let Some(dir) = file_dir {
        for pending in pending_imports {
            let target = dir.join(&pending.path);
            // The fallback for an uncanonicalizable (missing) target still
            // normalizes `.` components so errors show a clean path.
            let target =
                fs::canonicalize(&target).unwrap_or_else(|_| target.components().collect());
            // `import ./dir` means `./dir/default.nix`.
            let target = if target.is_dir() {
                target.join("default.nix")
            } else {
                target
            };
            let Ok(imported_content) = fs::read_to_string(&target) else {
                // The refs the imported file would contribute cannot be
                // discovered, so fail rather than silently under-lock.
                return Err(ScanError::UnreadableImport {
                    target,
                    file: path.to_path_buf(),
                    position: line_col(content, pending.offset),
                });
            };
            // The child is scanned with its own parameter names as root_attributes;
            // its refs are rewritten back into the parent's namespace.
            let rewrites: HashMap<String, String> = match pending.arg {
                ImportArg::Set(forwards) => forwards,
                ImportArg::Root(parent_root) => match top_ident_param(&imported_content) {
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
            // The same file imported under a different forwarding contributes
            // different refs and is scanned again; only a repeat with the
            // same forwarding — including a true import cycle — is skipped.
            // The forwarding is composed down to the top-level root_attributes before
            // being stored: two chains whose immediate maps look identical can
            // still stand for different top-level root_attributes, and only an identical
            // composition contributes identical refs.
            let child_origins: BTreeMap<String, String> = rewrites
                .iter()
                .map(|(child, parent)| {
                    let origin = root_origins.get(parent).unwrap_or(parent);
                    (child.clone(), origin.clone())
                })
                .collect();
            if !visited
                .entry(target.clone())
                .or_default()
                .insert(child_origins.clone())
            {
                continue;
            }
            let child_root_attributes: HashSet<String> = rewrites.keys().cloned().collect();
            let import_dir = target.parent().map(Path::to_path_buf);
            let imported = analyze_file_at(
                &imported_content,
                &child_root_attributes,
                import_dir.as_deref(),
                visited,
                &target,
                &child_origins,
            )?;
            refs.extend(
                imported
                    .refs
                    .into_iter()
                    .map(|reference| rewrite_root(reference.into(), &rewrites)),
            );
        }
    }

    // With the refs complete (imports included), reject any that resolve
    // through a root the top-level lambda does not declare — they could never
    // evaluate. Roots are checked in sorted order for a deterministic error.
    if let Some(declared) = &declared_params {
        let mut undeclared: Vec<&String> = root_attributes
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
                    file: path.to_path_buf(),
                    position: first_root_use
                        .get(root)
                        .map(|&offset| line_col(content, offset)),
                });
            }
        }
    }

    Ok(FileInfo {
        refs: refs.into_iter().map(CatalogRef).collect(),
        dependency_args,
    })
}

/// The identity `root_origins` map for an entry file, whose root_attributes are the
/// top-level root_attributes themselves (see [analyze_file_at]).
pub(super) fn identity_origins(root_attributes: &HashSet<String>) -> BTreeMap<String, String> {
    root_attributes
        .iter()
        .map(|root| (root.clone(), root.clone()))
        .collect()
}

/// The root component of a dotted reference (`catalogs.a.b` → `catalogs`).
fn reference_root(reference: &str) -> &str {
    reference.split('.').next().unwrap_or(reference)
}

/// Collect the static attr-paths selected on dependency arguments.
///
/// For every `select` whose base identifier is a dependency argument in
/// `dependency_arg_names`, record `[arg, member…]` up to the first dynamic component.
/// These become sibling-attribute-set lookups when the package graph resolves
/// the dependency argument.
fn collect_dependency_selections(
    node: &rnix::SyntaxNode,
    dependency_arg_names: &HashSet<String>,
    out: &mut Vec<Vec<String>>,
) {
    if let Some(select) = ast::Select::cast(node.clone())
        && let Some(ast::Expr::Ident(base)) = select.expr()
        && let Some(base_name) = base.ident_token().map(|t| t.text().to_string())
        && dependency_arg_names.contains(&base_name)
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
        collect_dependency_selections(&child, dependency_arg_names, out);
    }
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
    /// Byte offset of the import application, for locating an
    /// unreadable-target error.
    offset: usize,
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

/// What consumes an attrset appearing in value position, deciding which of
/// its entries are accounted for elsewhere rather than escaping as values
/// (see [Walker::walk_attrset]).
#[derive(Debug)]
enum AttrsetConsumer<'a> {
    /// Nothing: an ordinary value attrset.
    None,
    /// An `import` application: names forwarded into the imported file (per
    /// [Walker::import_forwards]) are scanned through it; a root-named entry
    /// that does not forward is warned about.
    Import(&'a HashMap<String, String>),
    /// A lambda defined in this file: an entry binding a root under a
    /// parameter of the same name was already walked in the body.
    Lambda(&'a HashSet<String>),
}

/// Syntax-tree walker threading the lexical environment.
///
/// The walker emits refs for use sites resolved through the environment
/// (selects, `inherit (…)`, `with`, `getAttr`) and records `import`
/// applications as [PendingImport] facts; it performs no IO itself.
struct Walker<'a> {
    root_attributes: &'a HashSet<String>,
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

impl<'a> Walker<'a> {
    /// Create a walker over `root_attributes` with empty results. [Self::walk_root]
    /// performs the walk and [Self::finish] extracts the collected facts.
    fn new(
        root_attributes: &'a HashSet<String>,
        declared_params: Option<&'a HashSet<String>>,
        ctx: ScanCtx<'a>,
    ) -> Self {
        Self {
            root_attributes,
            declared_params,
            ctx,
            refs: BTreeSet::new(),
            pending_imports: Vec::new(),
            first_root_use: HashMap::new(),
        }
    }

    /// Walk the file's top-level expression, seeding the environment with the
    /// catalog root_attributes. Lambda parameters — including the top-level package
    /// function's — keep root-named names rooted and shadow everything else
    /// (see [Self::walk_lambda]).
    fn walk_root(&mut self, root: &rnix::Root) {
        let env: Env = self
            .root_attributes
            .iter()
            .map(|root| (root.clone(), Binding::Path(vec![root.clone()])))
            .collect();
        if let Some(expr) = root.expr() {
            self.walk(expr.syntax(), &env);
        }
    }

    /// Consume the walker, returning its collected facts as
    /// `(refs, pending imports, first-use offsets)` for the IO drain in
    /// [analyze_file_at].
    fn finish(self) -> (BTreeSet<String>, Vec<PendingImport>, HashMap<String, usize>) {
        (self.refs, self.pending_imports, self.first_root_use)
    }

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
            return self.walk_value_inherit(&inherit, &AttrsetConsumer::None, env);
        }
        let Some(expr) = ast::Expr::cast(node.clone()) else {
            return self.walk_children(node, env);
        };
        match expr {
            ast::Expr::Lambda(lambda) => self.walk_lambda(&lambda, env),
            ast::Expr::LetIn(let_in) => self.walk_let_in(&let_in, env),
            ast::Expr::With(with_expr) => self.walk_with(&with_expr, env),
            // `rec { }` scopes like `let` — members see each other — but a
            // set reached here is in value position, so its entries escape
            // like a plain set's; alias suppression only applies where the
            // set is consumed as a binding (see [Self::walk_binding_rhs]).
            ast::Expr::AttrSet(attrset) if attrset.rec_token().is_some() => {
                let inner = recursive_scope_env(&attrset, env);
                self.walk_attrset(&attrset, &AttrsetConsumer::None, &inner);
            },
            ast::Expr::AttrSet(attrset) => self.walk_attrset(&attrset, &AttrsetConsumer::None, env),
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
        if receivable && self.root_attributes.contains(name) {
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
            self.walk_binding_inherit(&inherit, env);
        }
    }

    /// `inherit (<source>) a b c;` in a binding scope (`let`, `rec { }`,
    /// consumed set literal) — each name only becomes an alias (use sites
    /// drive the refs, like an alias binding's RHS: only package-deep
    /// members are refs of their own). A name the catalog cannot contain
    /// (dotted quoted attr) collapses to a sentinel at the source. A
    /// from-less `inherit x;` only rebinds the outer name.
    fn walk_binding_inherit(&mut self, inherit: &ast::Inherit, env: &Env) {
        let Some(from_expr) = inherit.from().and_then(|from| from.expr()) else {
            return;
        };
        let Some(source) = resolve_expr_binding(&from_expr, env) else {
            self.walk(from_expr.syntax(), env);
            return;
        };
        self.walk_consumed_source(&from_expr, env);
        match paths_of(source.clone()) {
            Some(bases) => {
                for attr in inherit.attrs() {
                    let name = attr_static_name(&attr);
                    let paths: BTreeSet<Vec<String>> = bases
                        .iter()
                        .map(|base| match &name {
                            Some(name) => append_component(base.clone(), name.clone()),
                            None => append_star(base.clone()),
                        })
                        .collect();
                    self.emit_deep_paths(offset_of(attr.syntax()), paths);
                }
            },
            None => {
                for attr in inherit.attrs() {
                    let paths = attr_static_name(&attr)
                        .and_then(|name| member_binding(&source, &name))
                        .and_then(paths_of);
                    if let Some(paths) = paths {
                        self.emit_deep_paths(offset_of(attr.syntax()), paths);
                    }
                }
            },
        }
    }

    /// Walk an attrset in value position. An entry the `consumer` accounts
    /// for elsewhere is consumed — it is not an escape — and every other
    /// entry is an ordinary value position.
    fn walk_attrset(&mut self, attrset: &ast::AttrSet, consumer: &AttrsetConsumer, env: &Env) {
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
                let binding = resolve_expr_binding(&value, env);
                if self.consumed_by(consumer, name, binding.as_ref()) {
                    self.walk_consumed_source(&value, env);
                    continue;
                }
                self.warn_unforwarded_root(consumer, name, || {
                    entry
                        .attrpath()
                        .map(|attrpath| offset_of(attrpath.syntax()))
                        .unwrap_or_else(|| offset_of(value.syntax()))
                });
            }
            self.walk(value.syntax(), env);
        }
        for inherit in attrset.inherits() {
            self.walk_value_inherit(&inherit, consumer, env);
        }
    }

    /// One `inherit` inside a value attrset: each name not consumed by the
    /// `consumer` is a value use of the inherited binding.
    fn walk_value_inherit(
        &mut self,
        inherit: &ast::Inherit,
        consumer: &AttrsetConsumer,
        env: &Env,
    ) {
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
            let offset = offset_of(attr.syntax());
            let Some(name) = attr_static_name(&attr) else {
                // A name the catalog cannot contain (dotted quoted attr)
                // collapses to a sentinel at the source.
                if let Some(source) = &from_binding {
                    let paths: BTreeSet<Vec<String>> = escaping_paths_of(source)
                        .into_iter()
                        .map(append_star)
                        .collect();
                    self.emit_value_paths(offset, paths);
                }
                continue;
            };
            let binding = match (from_expr.is_some(), &from_binding) {
                (false, _) => env.get(&name).cloned(),
                (true, Some(source)) => member_binding(source, &name),
                (true, None) => None,
            };
            if self.consumed_by(consumer, &name, binding.as_ref()) {
                continue;
            }
            self.warn_unforwarded_root(consumer, &name, || offset);
            if let Some(binding) = binding {
                self.emit_value_binding(offset, &binding);
            }
        }
    }

    /// Whether the `consumer` accounts for binding `name` to `binding`
    /// elsewhere, making the entry a consumed forward rather than a value.
    fn consumed_by(
        &self,
        consumer: &AttrsetConsumer,
        name: &str,
        binding: Option<&Binding>,
    ) -> bool {
        match consumer {
            AttrsetConsumer::None => false,
            AttrsetConsumer::Import(forwards) => forwards.contains_key(name),
            AttrsetConsumer::Lambda(params) => {
                params.contains(name)
                    && self.root_attributes.contains(name)
                    && matches!(binding, Some(Binding::Path(path)) if path.len() == 1 && path[0] == name)
            },
        }
    }

    /// Warn when an import argument names a root without forwarding it — the
    /// imported file is not scanned through that name.
    fn warn_unforwarded_root(
        &self,
        consumer: &AttrsetConsumer,
        name: &str,
        offset: impl FnOnce() -> usize,
    ) {
        if matches!(consumer, AttrsetConsumer::Import(_)) && self.root_attributes.contains(name) {
            self.ctx.warn_unfollowed_import(offset(), name);
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
                if self.apply_import(offset_of(apply.syntax()), path, &arg, env) {
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
                        && self.apply_import(offset_of(apply.syntax()), path, &argument, env)
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
    /// `offset` is the import application's byte offset, for locating a drain
    /// failure at the import site.
    fn apply_import(&mut self, offset: usize, path: String, arg: &ast::Expr, env: &Env) -> bool {
        match arg {
            ast::Expr::AttrSet(attrset) => {
                let forwards = self.import_forwards(attrset, env);
                self.walk_attrset(attrset, &AttrsetConsumer::Import(&forwards), env);
                if !forwards.is_empty() {
                    // Forwarding is a use of each parent root: refs surfacing
                    // from the import trace back to this argument.
                    let arg_offset = offset_of(attrset.syntax());
                    for parent_root in forwards.values() {
                        self.note_root_use(arg_offset, parent_root);
                    }
                    self.pending_imports.push(PendingImport {
                        path,
                        arg: ImportArg::Set(forwards),
                        offset,
                    });
                }
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
                        offset,
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
        let consumer = AttrsetConsumer::Lambda(params);
        match argument {
            // `mkPkg catalogs` where the lambda's plain parameter is the
            // root's own name.
            ast::Expr::Ident(ident) => {
                if let Some(name) = ident_name(ident) {
                    if self.consumed_by(&consumer, &name, env.get(&name)) {
                        return;
                    }
                    // A modeled set passed whole (`mkPkg args`): members the
                    // lambda binds under the same root name were walked in
                    // the body; only the rest escapes.
                    if let Some(Binding::Set(members)) = env.get(&name) {
                        let uncovered: BTreeSet<Vec<String>> = members
                            .iter()
                            .filter(|(member, binding)| {
                                !self.consumed_by(&consumer, member, Some(binding))
                            })
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
            ast::Expr::AttrSet(attrset) => {
                let scope = if attrset.rec_token().is_some() {
                    recursive_scope_env(attrset, env)
                } else {
                    env.clone()
                };
                self.walk_attrset(attrset, &consumer, &scope);
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

/// The file's package function: the top-level lambda, looked for through
/// wrappers (`let … in`, `with …;`, parentheses) that do not change which
/// function the file evaluates to.
fn top_level_lambda(root: &rnix::Root) -> Option<ast::Lambda> {
    let mut expr = root.expr()?;
    loop {
        match expr {
            ast::Expr::Lambda(lambda) => return Some(lambda),
            ast::Expr::LetIn(let_in) => expr = let_in.body()?,
            ast::Expr::With(with_expr) => expr = with_expr.body()?,
            ast::Expr::Paren(paren) => expr = paren.expr()?,
            _ => return None,
        }
    }
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
