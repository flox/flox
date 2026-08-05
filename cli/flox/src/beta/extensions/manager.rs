//! Extension manager — install / remove / list operations.
//!
//! `install_local`, `remove`, and `list` sit on top of the
//! [`super::layout`] paths and the [`super::manifest`] types. A small
//! [`LockGuard`] RAII wrapper around `fslock::LockFile` serializes
//! mutating operations against the same managed directory; `list` is
//! deliberately lock-free.
//!
//! # Lock discipline
//!
//! - `install_local` and `remove` each acquire the extensions lock with
//!   [`LockGuard::acquire`].
//! - `list` and `dispatch::find` are lock-free on purpose — they are
//!   pure reads.
//! - `state.toml` is written via `render_installed_state` and `fs::write`
//!   inside a staging dir; the atomic rename ([`atomic_install`]) is the
//!   only visible transition from "missing" to "installed".

use std::path::{Path, PathBuf};
use std::{fs, io};

use flox_rust_sdk::flox::Flox;
use fslock::LockFile;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tracing::debug;
use uuid::Uuid;

use super::extension::Extension;
use super::layout;
use super::manifest::{
    AuthorManifest,
    InstalledState,
    ManifestError,
    parse_author_manifest,
    parse_installed_state,
    render_installed_state,
};
use super::reserved::RESERVED_COMMAND_NAMES;

#[derive(Debug, Error)]
pub enum LockError {
    #[error("failed to open lock file at {path}: {source}")]
    Open {
        path: PathBuf,
        #[source]
        source: fslock::Error,
    },
    #[error("failed to acquire lock at {path}: {source}")]
    Acquire {
        path: PathBuf,
        #[source]
        source: fslock::Error,
    },
}

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("source path '{0}' does not exist")]
    SourceMissing(PathBuf),
    #[error("source path '{0}' is not a directory")]
    SourceNotDirectory(PathBuf),
    #[error(
        "could not derive an extension name from source path '{0}' (expected directory name like 'flox-<name>')"
    )]
    NameUnderivable(PathBuf),
    #[error(
        "manifest extension name '{manifest}' does not match source directory name 'flox-{dirname}' \
        — fix one or the other"
    )]
    NameMismatch { manifest: String, dirname: String },
    #[error("{}", fmt_executable_missing(.name, .path))]
    ExecutableMissing { name: String, path: PathBuf },
    #[error(
        "derived extension name '{0}' is not valid: must match '^[a-z0-9][a-z0-9_-]*$' \
        (rename the source directory, or set '[extension] name' in flox-extension.toml)"
    )]
    InvalidName(String),
    #[error("flox-{0} is already installed (run with --force to overwrite)")]
    AlreadyInstalled(String),
    #[error("name '{0}' conflicts with a built-in flox command")]
    ReservedName(String),
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("filesystem error during install: {0}")]
    Io(#[from] io::Error),
    #[error("failed to format installation timestamp: {0}")]
    Time(#[from] time::error::Format),
}

#[derive(Debug, Error)]
pub enum RemoveError {
    #[error("extension 'flox-{0}' is not installed")]
    NotFound(String),
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error("filesystem error during remove: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Error)]
pub enum ListError {
    #[error("filesystem error during list: {0}")]
    Io(#[from] io::Error),
}

/// RAII guard around an `fslock::LockFile`. Drops the lock when dropped.
///
/// The full type-state guard pattern in
/// [`flox_rust_sdk::providers::upgrade_checks`] is overkill for our use case —
/// extensions only ever mutate while holding the lock, and we don't
/// need separate read/write capabilities at the type level.
#[derive(Debug)]
pub struct LockGuard {
    _lock: LockFile,
    path: PathBuf,
}

impl LockGuard {
    /// Acquire the lock at `path`, blocking until available. Creates the
    /// lock file (and parent directories) if missing.
    pub fn acquire(path: &Path) -> Result<Self, LockError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| LockError::Open {
                path: path.to_path_buf(),
                source: fslock::Error::from(source),
            })?;
        }
        let mut lock = LockFile::open(path).map_err(|source| LockError::Open {
            path: path.to_path_buf(),
            source,
        })?;
        lock.lock().map_err(|source| LockError::Acquire {
            path: path.to_path_buf(),
            source,
        })?;
        debug!(?path, "acquired extensions lock");
        Ok(Self {
            _lock: lock,
            path: path.to_path_buf(),
        })
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        debug!(path = ?self.path, "released extensions lock");
    }
}

/// Atomically promote `staging` to `final_dir` via `rename`. Both must
/// be on the same filesystem (the caller arranges this by placing the
/// staging dir under the same `extensions_root`).
pub fn atomic_install(staging: &Path, final_dir: &Path) -> io::Result<()> {
    fs::rename(staging, final_dir)
}

/// Install an extension from a local directory.
///
/// `source` must be a directory containing an executable `flox-<name>`.
/// An optional `flox-extension.toml` at the source root supplies metadata;
/// if it sets `[extension] name`, that name wins (and must match the
/// `flox-<name>` directory naming if both exist).
pub fn install_local(flox: &Flox, source: &Path, force: bool) -> Result<Extension, InstallError> {
    let source = match source.canonicalize() {
        Ok(p) => p,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(InstallError::SourceMissing(source.to_path_buf()));
        },
        Err(e) => return Err(InstallError::Io(e)),
    };
    if !source.is_dir() {
        return Err(InstallError::SourceNotDirectory(source));
    }

    let manifest = read_author_manifest(&source)?;
    let name = derive_name(&source, manifest.as_ref())?;
    validate_local_name(&name)?;
    let exe_name = format!("flox-{name}");

    let exe_path = source.join(&exe_name);
    if !is_executable(&exe_path) {
        return Err(InstallError::ExecutableMissing {
            name: name.clone(),
            path: exe_path,
        });
    }

    let extensions_root = layout::extensions_root(flox);
    fs::create_dir_all(&extensions_root)?;

    let _guard = LockGuard::acquire(&layout::lock_path(flox))?;

    let install_dir = layout::install_dir(flox, &name);
    if install_dir.exists() && !force {
        return Err(InstallError::AlreadyInstalled(name));
    }

    let staging = extensions_root.join(format!(".staging-{}", Uuid::new_v4()));
    fs::create_dir(&staging)?;

    // Everything that can fail while the staging dir exists runs inside this
    // closure so a single cleanup covers every error path. Staging names are
    // UUIDs that `list` ignores, so a leaked one is invisible and accumulates
    // silently — an arm that forgets to clean up is not self-correcting.
    let staged = (|| -> Result<InstalledState, InstallError> {
        let state =
            populate_local_staging(&staging, &source, &name, &exe_name, &exe_path, &install_dir)?;
        if install_dir.exists() {
            fs::remove_dir_all(&install_dir)?;
        }
        atomic_install(&staging, &install_dir)?;
        Ok(state)
    })();

    let state = match staged {
        Ok(state) => state,
        Err(err) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(err);
        },
    };

    Ok(Extension {
        name,
        install_dir,
        state,
    })
}

fn populate_local_staging(
    staging: &Path,
    source: &Path,
    name: &str,
    exe_name: &str,
    exe_path: &Path,
    install_dir: &Path,
) -> Result<InstalledState, InstallError> {
    let staged_exe = staging.join(exe_name);
    fs::copy(exe_path, &staged_exe)?;
    let manifest_src = source.join("flox-extension.toml");
    if manifest_src.exists() {
        fs::copy(&manifest_src, staging.join("flox-extension.toml"))?;
    }

    let now = OffsetDateTime::now_utc().format(&Rfc3339)?;

    let state = InstalledState {
        schema: "1".to_string(),
        name: name.to_string(),
        source: source.display().to_string(),
        installed_at: now,
        path: install_dir.display().to_string(),
    };
    let state_str = render_installed_state(&state)?;
    fs::write(staging.join("state.toml"), state_str)?;
    Ok(state)
}

/// Enforce the name rules for installs: `[a-z0-9][a-z0-9_-]*` with no
/// reserved subcommand names.
/// Without this, a source dir like `flox-` (empty name) or
/// `flox-foo bar` (space) installs silently and is then undispatchable.
fn validate_local_name(name: &str) -> Result<(), InstallError> {
    let repo = format!("flox-{name}");
    match extract_extension_name(&repo) {
        Some(n) if n == name => {},
        _ => return Err(InstallError::InvalidName(name.to_string())),
    }
    check_not_reserved(name)?;
    Ok(())
}

/// Remove an installed extension by name.
pub fn remove(flox: &Flox, name: &str) -> Result<(), RemoveError> {
    // Reject path-traversal / invalid names before composing any path.
    // Without this, `remove "x/../../../dir"` would build
    // `<root>/flox-x/../../../dir` and `remove_dir_all` a directory outside
    // the extensions root.
    if !is_valid_extension_name(name) {
        return Err(RemoveError::NotFound(name.to_string()));
    }
    let install_dir = layout::install_dir(flox, name);
    if !install_dir.exists() {
        return Err(RemoveError::NotFound(name.to_string()));
    }

    let _guard = LockGuard::acquire(&layout::lock_path(flox))?;

    if !install_dir.exists() {
        return Err(RemoveError::NotFound(name.to_string()));
    }

    fs::remove_dir_all(&install_dir)?;
    Ok(())
}

/// List installed extensions. Lock-free — entries with unparseable
/// `state.toml` (e.g., a crashed install left behind) are skipped with a
/// debug log rather than failing the whole listing.
pub fn list(flox: &Flox) -> Result<Vec<Extension>, ListError> {
    let root = layout::extensions_root(flox);
    if !root.exists() {
        return Ok(vec![]);
    }

    let mut out = vec![];
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let dir_name = entry.file_name();
        let Some(dir_name_str) = dir_name.to_str() else {
            continue;
        };
        let Some(name) = dir_name_str.strip_prefix("flox-") else {
            continue;
        };
        let install_dir = entry.path();
        if !install_dir.is_dir() {
            continue;
        }
        let state_path = layout::state_path(flox, name);
        let state_str = match fs::read_to_string(&state_path) {
            Ok(s) => s,
            Err(err) => {
                debug!(
                    ?install_dir,
                    ?err,
                    "skipping entry with missing/unreadable state.toml"
                );
                continue;
            },
        };
        let state = match parse_installed_state(&state_str) {
            Ok(s) => s,
            Err(err) => {
                debug!(
                    ?install_dir,
                    ?err,
                    "skipping entry with unparseable state.toml"
                );
                continue;
            },
        };
        out.push(Extension {
            name: name.to_string(),
            install_dir,
            state,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn read_author_manifest(source: &Path) -> Result<Option<AuthorManifest>, InstallError> {
    let path = source.join("flox-extension.toml");
    if !path.exists() {
        return Ok(None);
    }
    let s = fs::read_to_string(&path)?;
    Ok(Some(parse_author_manifest(&s).map_err(InstallError::from)?))
}

fn derive_name(source: &Path, manifest: Option<&AuthorManifest>) -> Result<String, InstallError> {
    let dirname_name = source
        .file_name()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("flox-"))
        .map(str::to_string);

    let manifest_name = manifest.and_then(|m| {
        let n = m.extension.name.trim();
        if n.is_empty() {
            None
        } else {
            Some(n.to_string())
        }
    });

    match (manifest_name, dirname_name) {
        (Some(m), Some(d)) if m == d => Ok(m),
        (Some(m), Some(d)) => Err(InstallError::NameMismatch {
            manifest: m,
            dirname: d,
        }),
        (Some(m), None) => Ok(m),
        (None, Some(d)) => Ok(d),
        (None, None) => Err(InstallError::NameUnderivable(source.to_path_buf())),
    }
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(p)
        .map(|md| md.is_file() && md.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// Strip the `flox-` prefix from a directory or repo name and return the
/// bare extension `<name>`, or `None` if the prefix is missing or `<name>`
/// doesn't match `^[a-z0-9][a-z0-9_-]*$`.
pub fn extract_extension_name(repo: &str) -> Option<&str> {
    let name = repo.strip_prefix("flox-")?;
    if name.is_empty() {
        return None;
    }
    let mut chars = name.chars();
    let first = chars.next()?;
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return None;
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_') {
        return None;
    }
    Some(name)
}

/// Reject extension names that collide with a built-in `flox` subcommand
/// (see [`RESERVED_COMMAND_NAMES`]).
///
/// `try_dispatch_external` only fires when bpaf fails to parse the first
/// positional, so a reserved name would never dispatch to the extension
/// even if installed. Catching the conflict at install time gives a much
/// better error than a silent shadowing.
pub fn check_not_reserved(name: &str) -> Result<(), InstallError> {
    let lowered = name.to_ascii_lowercase();
    if RESERVED_COMMAND_NAMES.contains(&lowered.as_str()) {
        return Err(InstallError::ReservedName(name.to_string()));
    }
    Ok(())
}

/// Whether `name` is a syntactically valid extension name — the same
/// `[a-z0-9][a-z0-9_-]*` rule install enforces via [`extract_extension_name`].
///
/// This is the guard `remove` uses before composing a `flox-<name>`
/// path. A name containing `/`, a path separator, or `..` fails the
/// charset check, which is what prevents `remove` from building a path
/// that escapes the extensions root and recursively deleting it.
pub fn is_valid_extension_name(name: &str) -> bool {
    extract_extension_name(&format!("flox-{name}")) == Some(name)
}

fn fmt_executable_missing(name: &str, path: &Path) -> String {
    format!("extension '{name}' has no executable at {}", path.display())
}

#[cfg(test)]
#[cfg(feature = "beta-tests")]
mod tests {
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;

    #[cfg(unix)]
    fn write_exe(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(not(unix))]
    fn write_exe(path: &Path, body: &str) {
        fs::write(path, body).unwrap();
    }

    fn make_source(parent: &Path, name: &str) -> PathBuf {
        let dir = parent.join(format!("flox-{name}"));
        fs::create_dir(&dir).unwrap();
        write_exe(&dir.join(format!("flox-{name}")), "#!/bin/sh\necho hi\n");
        dir
    }

    #[test]
    fn atomic_install_renames() {
        let temp = TempDir::new().unwrap();
        let staging = temp.path().join(".staging-x");
        let final_dir = temp.path().join("flox-foo");
        fs::create_dir(&staging).unwrap();
        write_exe(&staging.join("flox-foo"), "#!/bin/sh\n");

        atomic_install(&staging, &final_dir).unwrap();

        assert!(!staging.exists());
        assert!(final_dir.join("flox-foo").exists());
    }

    #[test]
    fn install_local_happy_path() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = make_source(src_root.path(), "hello");

        let ext = install_local(&flox, &src, false).unwrap();
        assert_eq!(ext.name, "hello");
        assert_eq!(ext.install_dir, layout::install_dir(&flox, "hello"));
        assert!(ext.install_dir.join("flox-hello").exists());
        assert!(is_executable(&ext.install_dir.join("flox-hello")));

        let state_str = fs::read_to_string(ext.install_dir.join("state.toml")).unwrap();
        let state = parse_installed_state(&state_str).unwrap();
        assert_eq!(state.name, "hello");
        assert_eq!(state.source, src.display().to_string());
        assert_eq!(state.path, ext.install_dir.display().to_string());
    }

    #[test]
    fn install_local_rejects_when_no_executable() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = src_root.path().join("flox-hello");
        fs::create_dir(&src).unwrap();
        // intentionally no flox-hello executable

        let err = install_local(&flox, &src, false).unwrap_err();
        assert!(matches!(err, InstallError::ExecutableMissing { .. }));
    }

    #[test]
    fn install_local_rejects_when_already_installed() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = make_source(src_root.path(), "hello");

        install_local(&flox, &src, false).unwrap();
        let err = install_local(&flox, &src, false).unwrap_err();
        assert!(matches!(err, InstallError::AlreadyInstalled(ref n) if n == "hello"));
    }

    #[test]
    fn install_local_force_overwrites_existing_install() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = make_source(src_root.path(), "hello");

        install_local(&flox, &src, false).unwrap();
        // Second install with `force=true` must succeed where `force=false`
        // returns `AlreadyInstalled`. This keeps the CLI `--force` flag's
        // promise for the local install path.
        let ext = install_local(&flox, &src, true).unwrap();
        assert_eq!(ext.name, "hello");
    }

    #[test]
    fn install_local_rejects_empty_name() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = src_root.path().join("flox-");
        fs::create_dir(&src).unwrap();
        write_exe(&src.join("flox-"), "#!/bin/sh\n");

        let err = install_local(&flox, &src, false).unwrap_err();
        assert!(matches!(err, InstallError::InvalidName(ref n) if n.is_empty()));
    }

    #[test]
    fn install_local_rejects_invalid_name_chars() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = src_root.path().join("flox-foo bar");
        fs::create_dir(&src).unwrap();
        write_exe(&src.join("flox-foo bar"), "#!/bin/sh\n");

        let err = install_local(&flox, &src, false).unwrap_err();
        assert!(matches!(err, InstallError::InvalidName(ref n) if n == "foo bar"));
    }

    #[test]
    fn install_local_rejects_uppercase_name() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = src_root.path().join("flox-Hello");
        fs::create_dir(&src).unwrap();
        write_exe(&src.join("flox-Hello"), "#!/bin/sh\n");

        let err = install_local(&flox, &src, false).unwrap_err();
        assert!(matches!(err, InstallError::InvalidName(ref n) if n == "Hello"));
    }

    #[test]
    fn install_local_rejects_reserved_name() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = make_source(src_root.path(), "install");

        let err = install_local(&flox, &src, false).unwrap_err();
        assert!(matches!(err, InstallError::ReservedName(ref n) if n == "install"));
    }

    #[test]
    fn install_local_reports_not_found_when_source_missing() {
        let (flox, _tempdir) = flox_instance();
        let missing = PathBuf::from("/definitely/does/not/exist/flox-ghost");

        let err = install_local(&flox, &missing, false).unwrap_err();
        assert!(matches!(err, InstallError::SourceMissing(_)));
    }

    /// BUG-01 regression: the staging directory must not leak after a
    /// failure between `create_dir(&staging)` and `atomic_install`.
    ///
    /// The failure has to be reachable *after* staging is populated, which
    /// rules out the two guards that fire earlier (`AlreadyInstalled`, and
    /// `atomic_install` never sees `EEXIST` because `--force` removes the
    /// install dir first). A plain file at the install path is the reachable
    /// one: it passes `exists()`, so `--force` tries to `remove_dir_all` it
    /// and gets `ENOTDIR`.
    #[test]
    fn install_local_cleans_staging_on_failure_after_staging() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = make_source(src_root.path(), "hello");

        let install_dir = layout::install_dir(&flox, "hello");
        fs::create_dir_all(install_dir.parent().unwrap()).unwrap();
        fs::write(&install_dir, "not a directory").unwrap();

        let err = install_local(&flox, &src, true).unwrap_err();
        assert!(
            matches!(err, InstallError::Io(_)),
            "expected the pre-rename removal to fail, got {err:?}"
        );

        let leftover_staging: Vec<_> = fs::read_dir(layout::extensions_root(&flox))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with(".staging-"))
            })
            .collect();
        assert!(
            leftover_staging.is_empty(),
            "expected no .staging-* dirs after failed install, found {leftover_staging:?}"
        );
    }

    #[test]
    fn install_local_name_mismatch_is_error() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = src_root.path().join("flox-hello");
        fs::create_dir(&src).unwrap();
        write_exe(&src.join("flox-hello"), "#!/bin/sh\n");
        fs::write(
            src.join("flox-extension.toml"),
            "schema = \"1\"\n[extension]\nname = \"goodbye\"\n",
        )
        .unwrap();

        let err = install_local(&flox, &src, false).unwrap_err();
        assert!(matches!(err, InstallError::NameMismatch { .. }));
    }

    #[test]
    fn list_returns_installed_extensions_sorted() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let beta = make_source(src_root.path(), "beta");
        let alpha = make_source(src_root.path(), "alpha");

        install_local(&flox, &beta, false).unwrap();
        install_local(&flox, &alpha, false).unwrap();

        let listed = list(&flox).unwrap();
        let names: Vec<_> = listed.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[test]
    fn list_returns_empty_when_root_missing() {
        let (flox, _tempdir) = flox_instance();
        // extensions_root never created
        let listed = list(&flox).unwrap();
        assert_eq!(listed, vec![]);
    }

    #[test]
    fn list_skips_unparseable_state() {
        let (flox, _tempdir) = flox_instance();
        let root = layout::extensions_root(&flox);
        fs::create_dir_all(&root).unwrap();
        let bad = root.join("flox-broken");
        fs::create_dir(&bad).unwrap();
        fs::write(bad.join("state.toml"), "this is not toml = =").unwrap();

        let listed = list(&flox).unwrap();
        assert_eq!(listed, vec![]);
    }

    #[test]
    fn remove_deletes_install_dir() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = make_source(src_root.path(), "hello");

        install_local(&flox, &src, false).unwrap();
        assert!(layout::install_dir(&flox, "hello").exists());

        remove(&flox, "hello").unwrap();
        assert!(!layout::install_dir(&flox, "hello").exists());
    }

    #[test]
    fn remove_errors_when_not_installed() {
        let (flox, _tempdir) = flox_instance();
        let err = remove(&flox, "missing").unwrap_err();
        assert!(matches!(err, RemoveError::NotFound(ref n) if n == "missing"));
    }

    #[test]
    fn is_valid_extension_name_rejects_traversal_and_separators() {
        assert!(is_valid_extension_name("hello"));
        assert!(is_valid_extension_name("hello-world"));
        assert!(is_valid_extension_name("h"));
        assert!(!is_valid_extension_name(""));
        assert!(!is_valid_extension_name("a/b"));
        assert!(!is_valid_extension_name("hello/../../../etc"));
        assert!(!is_valid_extension_name("../victim"));
        assert!(!is_valid_extension_name("a b"));
        assert!(!is_valid_extension_name("Hello"));
    }

    #[test]
    fn remove_rejects_traversal_name_without_deleting_outside() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = make_source(src_root.path(), "hello");
        install_local(&flox, &src, false).unwrap();

        // A directory just outside the extensions root that a naive
        // `flox-{name}` join + remove_dir_all could otherwise reach.
        let victim = layout::extensions_root(&flox)
            .parent()
            .unwrap()
            .join("victim");
        fs::create_dir_all(&victim).unwrap();

        let err = remove(&flox, "hello/../../victim").unwrap_err();
        assert!(matches!(err, RemoveError::NotFound(_)));
        assert!(
            victim.exists(),
            "a traversal name must not delete a directory outside the extensions root"
        );
        assert!(layout::install_dir(&flox, "hello").exists());
    }

    #[test]
    fn extract_extension_name_strips_flox_prefix() {
        assert_eq!(extract_extension_name("flox-hello"), Some("hello"));
        assert_eq!(extract_extension_name("flox-foo-bar"), Some("foo-bar"));
        assert_eq!(extract_extension_name("flox-foo_bar"), Some("foo_bar"));
        assert_eq!(extract_extension_name("flox-h"), Some("h"));
        assert_eq!(extract_extension_name("flox-2nd"), Some("2nd"));
    }

    #[test]
    fn extract_extension_name_rejects_no_prefix() {
        assert_eq!(extract_extension_name("hello"), None);
        assert_eq!(extract_extension_name("FLOX-hello"), None);
        assert_eq!(extract_extension_name("my-flox-hello"), None);
    }

    #[test]
    fn extract_extension_name_rejects_uppercase() {
        assert_eq!(extract_extension_name("flox-Hello"), None);
        assert_eq!(extract_extension_name("flox-HELLO"), None);
        assert_eq!(extract_extension_name("flox-hElLo"), None);
    }

    #[test]
    fn extract_extension_name_rejects_leading_separator() {
        assert_eq!(extract_extension_name("flox--foo"), None);
        assert_eq!(extract_extension_name("flox-_foo"), None);
        assert_eq!(extract_extension_name("flox-"), None);
    }

    #[test]
    fn check_not_reserved_rejects_install() {
        let err = check_not_reserved("install").unwrap_err();
        assert!(matches!(err, InstallError::ReservedName(ref n) if n == "install"));
    }

    #[test]
    fn check_not_reserved_rejects_short_aliases() {
        // 'i' and 'l' are short aliases for install/list and must be reserved.
        assert!(check_not_reserved("i").is_err());
        assert!(check_not_reserved("l").is_err());
    }

    #[test]
    fn check_not_reserved_accepts_unique_name() {
        check_not_reserved("hello").unwrap();
        check_not_reserved("my-extension").unwrap();
    }

    /// BUG-12 regression: reserved-name check must be case-insensitive so
    /// that `Install` and `INSTALL` collide with the built-in `install`.
    #[test]
    fn check_not_reserved_is_case_insensitive() {
        for name in ["Install", "INSTALL", "InStAlL"] {
            let err = check_not_reserved(name).unwrap_err();
            assert!(
                matches!(err, InstallError::ReservedName(ref n) if n == name),
                "expected ReservedName for {name}, got {err:?}"
            );
        }
    }

    /// TS06: under the lock, a concurrent second install of the same
    /// name must not corrupt the install dir. Without the lock the two
    /// threads would race past the AlreadyInstalled check and both try
    /// to atomic_install into the same final dir; with the lock, they
    /// serialize and the loser sees AlreadyInstalled cleanly.
    #[test]
    fn concurrent_install_serializes_via_lock() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = make_source(src_root.path(), "twin");
        let flox_ref = &flox;
        let src_ref = &src;

        let (r1, r2) = std::thread::scope(|scope| {
            let h1 = scope.spawn(move || install_local(flox_ref, src_ref, false));
            let h2 = scope.spawn(move || install_local(flox_ref, src_ref, false));
            (h1.join().unwrap(), h2.join().unwrap())
        });

        let outcomes = [r1.is_ok(), r2.is_ok()];
        assert_eq!(
            outcomes.iter().filter(|x| **x).count(),
            1,
            "exactly one install must succeed"
        );

        // The install dir is intact and parseable.
        let install_dir = layout::install_dir(&flox, "twin");
        assert!(install_dir.join("flox-twin").exists());
        let state_str = fs::read_to_string(install_dir.join("state.toml")).unwrap();
        let _state = parse_installed_state(&state_str).unwrap();
    }

    #[test]
    fn install_error_display_matches_spec_strings() {
        let e = InstallError::ReservedName("activate".to_string());
        assert_eq!(
            e.to_string(),
            "name 'activate' conflicts with a built-in flox command"
        );

        let e = InstallError::AlreadyInstalled("deploy".to_string());
        assert_eq!(
            e.to_string(),
            "flox-deploy is already installed (run with --force to overwrite)"
        );

        let e = InstallError::ExecutableMissing {
            name: "deploy".to_string(),
            path: std::path::PathBuf::from("/tmp/flox-deploy/flox-deploy"),
        };
        assert_eq!(
            e.to_string(),
            "extension 'deploy' has no executable at /tmp/flox-deploy/flox-deploy"
        );
    }
}
