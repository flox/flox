//! `ExtensionManager` — install / remove / list operations.
//!
//! P02 wires up `install_local`, `remove`, and `list` on top of the
//! [`super::layout`] paths and the [`super::manifest`] types. A small
//! [`LockGuard`] RAII wrapper around `fslock::LockFile` serializes
//! mutating operations against the same managed directory; `list` is
//! deliberately lock-free.
//!
//! P03 reuses [`atomic_install`] for the GitHub-source install path and
//! the upgrade flow, so the helper lives here from the start.
//!
//! # Lock discipline (P05-T05 audit)
//!
//! - `install_local`, `install_github`, `upgrade` (all kinds, including
//!   `binary`), `upgrade_all` (per-item via `upgrade`), and `remove` each
//!   acquire the extensions lock with [`LockGuard::acquire`].
//! - `list`, `upgrade_dry_run`, `upgrade_all_dry_run`, and
//!   `dispatch::find` are lock-free on purpose — they are pure reads.
//! - Every write to `state.toml` goes through `render_installed_state`
//!   and `fs::write` inside a staging dir; the atomic rename
//!   ([`atomic_install`]) is the only visible transition from "missing"
//!   to "installed".
//! - `install_github_binary` does not acquire the lock itself because
//!   fslock is non-reentrant; its callers (`install_github` and
//!   `upgrade`, via `upgrade_binary`) hold the lock for its duration.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{fs, io};

use flox_rust_sdk::flox::Flox;
use fslock::LockFile;
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tracing::debug;
use uuid::Uuid;

use super::extension::Extension;
use super::github::{GitHubError, GitHubSource, SearchQuery, SearchResponse};
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
    #[error("another extensions operation is already in progress (lock at {path} is held)")]
    WouldBlock { path: PathBuf },
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
    #[error(
        "repo name '{0}' is not a valid extension repo: must be 'flox-<name>' \
        where <name> matches '^[a-z0-9][a-z0-9_-]*$'"
    )]
    InvalidRepoName(String),
    #[error("name '{0}' conflicts with a built-in flox command")]
    ReservedName(String),
    #[error(
        "spec '{0}' is not a valid GitHub source — use 'owner/repo' \
        (or '--from-path PATH' for local sources)"
    )]
    InvalidSpec(String),
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    GitHub(#[from] super::github::GitHubError),
    #[error(transparent)]
    Git(#[from] flox_rust_sdk::providers::git::GitRemoteCommandError),
    #[error("filesystem error during install: {0}")]
    Io(#[from] io::Error),
    #[error("failed to format installation timestamp: {0}")]
    Time(#[from] time::error::Format),
    #[error("no release asset matches '{platform}' for {owner}/{repo}")]
    NoMatchingAsset {
        owner: String,
        repo: String,
        platform: String,
    },
    #[error("archive extraction failed: {0}")]
    Archive(#[from] super::archive::ArchiveError),
    #[error(
        "binary asset integrity check failed — flox-extension.toml declared \
        sha256={expected} but the downloaded asset hashed to {actual}"
    )]
    Sha256Mismatch { expected: String, actual: String },
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

#[derive(Debug, Error)]
pub enum SearchError {
    #[error(transparent)]
    Github(#[from] GitHubError),
    #[error(transparent)]
    List(#[from] ListError),
}

/// One row in a search result, decorated with whether the repo is already
/// installed locally. `full_name` is `<owner>/<repo>` (matches GitHub
/// terminology and what we key installed state by).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchRow {
    pub full_name: String,
    pub stars: u64,
    pub description: Option<String>,
    pub installed: bool,
}

/// Outcome of a successful `upgrade` call (the `Err` arm of the result
/// type covers true failures).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpgradeStatus {
    /// Re-fetch produced a new commit; `state.toml` was rewritten.
    Upgraded { from: String, to: String },
    /// Remote HEAD already matched the recorded commit; no write.
    AlreadyCurrent,
    /// `state.pinned` is true and `--force` was not passed; no fetch.
    Pinned,
}

/// Outcome of a read-only dry-run upgrade resolve. Distinct from
/// `UpgradeStatus` because the dry-run path never mutates disk:
/// `WouldUpgrade` is the *planned* action, not a performed one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DryRunStatus {
    WouldUpgrade { from: String, to: String },
    AlreadyCurrent,
    Pinned,
}

/// Per-extension outcome inside an `upgrade_all_dry_run` run.
/// Mirrors `UpgradeResult` for ergonomic parity across the dry-run and
/// upgrade paths.
#[derive(Debug)]
pub struct DryRunResult {
    pub name: String,
    pub outcome: Result<DryRunStatus, UpgradeError>,
}

/// Per-extension outcome inside an `upgrade_all` / `upgrade --dry-run` run.
/// Wraps either `UpgradeStatus` (success, including no-op skip) or a
/// per-extension error surfaced without aborting the overall loop.
#[derive(Debug)]
pub struct UpgradeResult {
    pub name: String,
    pub outcome: Result<UpgradeStatus, UpgradeError>,
}

#[derive(Debug, Error)]
pub enum UpgradeError {
    #[error("extension 'flox-{0}' is not installed")]
    NotInstalled(String),
    #[error(
        "local extensions cannot be upgraded; reinstall with \
        'flox extension install --from-path <dir>'"
    )]
    LocalNotSupported,
    #[error(
        "extension 'flox-{0}' has no recorded branch — upgrade requires a tracked branch \
        (reinstall to refresh)"
    )]
    NoBranch(String),
    #[error("'git ls-remote origin {branch}' failed in {dir} (exit {code})")]
    LsRemoteFailed {
        dir: PathBuf,
        branch: String,
        code: i32,
    },
    #[error("git reset --hard FETCH_HEAD failed in {dir}: exit {code}")]
    ResetFailed { dir: PathBuf, code: i32 },
    #[error("git rev-parse HEAD failed in {dir}: exit {code}")]
    RevParseFailed { dir: PathBuf, code: i32 },
    #[error("repo {owner}/{repo} has no releases; cannot upgrade binary extension")]
    NoRelease { owner: String, repo: String },
    #[error(transparent)]
    Lock(#[from] LockError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error(transparent)]
    Git(#[from] flox_rust_sdk::providers::git::GitRemoteCommandError),
    #[error("filesystem error during upgrade: {0}")]
    Io(#[from] io::Error),
    #[error("failed to format upgrade timestamp: {0}")]
    Time(#[from] time::error::Format),
    #[error(transparent)]
    Install(#[from] Box<InstallError>),
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

    /// Try to acquire the lock at `path`; return `WouldBlock` immediately
    /// if another holder has it.
    #[allow(dead_code)] // P05 will use this for upgrade --all skip-on-busy
    pub fn try_acquire(path: &Path) -> Result<Self, LockError> {
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
        let acquired = lock.try_lock().map_err(|source| LockError::Acquire {
            path: path.to_path_buf(),
            source,
        })?;
        if !acquired {
            return Err(LockError::WouldBlock {
                path: path.to_path_buf(),
            });
        }
        debug!(?path, "acquired extensions lock (try)");
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
/// staging dir under the same `extensions_root`). P05 may reuse this
/// helper for the upgrade path.
pub fn atomic_install(staging: &Path, final_dir: &Path) -> io::Result<()> {
    fs::rename(staging, final_dir)
}

/// Install an extension from a local directory.
///
/// `source` must be a directory containing an executable `flox-<name>`.
/// An optional `flox-extension.toml` at the source root supplies metadata;
/// if it sets `[extension] name`, that name wins (and must match the
/// `flox-<name>` directory naming if both exist). If the source is a
/// git checkout, `commit` is populated from `git rev-parse HEAD` —
/// best-effort, empty string on failure.
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

    let state = match populate_local_staging(
        &staging,
        &source,
        &name,
        &exe_name,
        &exe_path,
        &install_dir,
    ) {
        Ok(s) => s,
        Err(err) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(err);
        },
    };

    if install_dir.exists() {
        fs::remove_dir_all(&install_dir)?;
    }
    if let Err(err) = atomic_install(&staging, &install_dir) {
        let _ = fs::remove_dir_all(&staging);
        return Err(err.into());
    }

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

    let commit = git_head_commit(source).unwrap_or_default();
    let now = OffsetDateTime::now_utc().format(&Rfc3339)?;

    let state = InstalledState {
        schema: "1".to_string(),
        name: name.to_string(),
        kind: "local".to_string(),
        source: source.display().to_string(),
        owner: String::new(),
        repo: String::new(),
        host: String::new(),
        tag: String::new(),
        branch: String::new(),
        commit,
        pinned: false,
        asset_sha256: String::new(),
        installed_at: now,
        path: install_dir.display().to_string(),
    };
    let state_str = render_installed_state(&state)?;
    fs::write(staging.join("state.toml"), state_str)?;
    Ok(state)
}

/// Enforce the same name rules as GitHub-source installs so the two paths
/// don't diverge: `[a-z0-9][a-z0-9_-]*` with no reserved subcommand names.
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

/// Install an extension from a GitHub source spec.
///
/// `spec` is `owner/repo` (e.g. `floxhub/flox-hello`). The repo segment
/// must be `flox-<name>` per [`extract_extension_name`]; `<name>` must
/// not collide with a built-in subcommand per [`check_not_reserved`].
///
/// When `pin` is `Some`, the install is recorded as `pinned = true`
/// (upgrade will skip it without `--force`). When `force` is true and an
/// install dir already exists for the same name, it is removed before the
/// atomic swap.
pub async fn install_github(
    flox: &Flox,
    spec: &str,
    pin: Option<&str>,
    force: bool,
) -> Result<Extension, InstallError> {
    let (owner, repo) = parse_owner_repo(spec)?;
    let name = extract_extension_name(&repo)
        .ok_or_else(|| InstallError::InvalidRepoName(repo.clone()))?
        .to_string();
    check_not_reserved(&name)?;

    let extensions_root = layout::extensions_root(flox);
    fs::create_dir_all(&extensions_root)?;

    let _guard = LockGuard::acquire(&layout::lock_path(flox))?;

    let install_dir = layout::install_dir(flox, &name);
    if install_dir.exists() && !force {
        return Err(InstallError::AlreadyInstalled(name));
    }

    let source = super::github::GitHubSource::from_env();
    let resolved = match pin {
        Some(p) => source.resolve_pin(&owner, &repo, p).await?,
        None => source.resolve_latest(&owner, &repo).await?,
    };

    // Binary install: if the release has assets and one matches the host
    // platform, download + extract instead of cloning.
    if let Some(tag) = resolved.tag.as_deref() {
        let assets = match source.list_release_assets(&owner, &repo, tag).await {
            Ok(a) => a,
            Err(super::github::GitHubError::NotFound(_)) => Vec::new(),
            Err(err) => return Err(err.into()),
        };
        if !assets.is_empty() {
            // Fetch the author manifest *before* asset resolution so the
            // `[extension.binary].asset` template can influence selection —
            // otherwise the template is dead code and selection is
            // substring-only.
            let manifest_ref = resolved.tag.as_deref().unwrap_or(&resolved.commit);
            let manifest = source
                .fetch_author_manifest(&owner, &repo, manifest_ref)
                .await?;
            match super::github::resolve_asset(&assets, manifest.as_ref(), &name) {
                Ok(asset) => {
                    let asset = asset.clone();
                    return install_github_binary(
                        flox,
                        &owner,
                        &repo,
                        &name,
                        &resolved,
                        &asset,
                        pin.is_some(),
                        &source,
                        manifest.as_ref(),
                    )
                    .await;
                },
                Err(e) => {
                    // A release exists but no asset matches this host. If the
                    // manifest declares a binary distribution, the author
                    // intends a binary install, so surface a clear
                    // NoMatchingAsset rather than falling through to a source
                    // clone that would later fail with ExecutableMissing.
                    if manifest
                        .as_ref()
                        .is_some_and(|m| m.extension.binary.is_some())
                    {
                        return Err(InstallError::NoMatchingAsset {
                            owner: owner.clone(),
                            repo: repo.clone(),
                            platform: e.platform,
                        });
                    }
                },
            }
        }
    }

    let staging = extensions_root.join(format!(".staging-{}", Uuid::new_v4()));
    // git clone --branch only accepts named refs (branch or tag), so SHA
    // pins still need a branch to clone by — `resolve_pin` returns the
    // default branch in that case. Post-clone, we reset to the resolved
    // commit so the working tree pins the exact SHA the caller asked for
    // (a no-op for tag/branch installs where the clone already lands
    // there).
    let clone_ref = resolved
        .tag
        .clone()
        .or_else(|| resolved.branch.clone())
        .unwrap_or_else(|| resolved.commit.clone());
    if let Err(err) = source.clone_repo(&owner, &repo, &clone_ref, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(err.into());
    }
    if !resolved.commit.is_empty() {
        let reset_status = Command::new("git")
            .arg("-C")
            .arg(&staging)
            .args(["reset", "--hard", &resolved.commit])
            .status();
        match reset_status {
            Ok(s) if s.success() => {},
            Ok(s) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(InstallError::Io(io::Error::other(format!(
                    "git reset --hard {} failed in staging: exit {}",
                    resolved.commit,
                    s.code().unwrap_or(-1)
                ))));
            },
            Err(err) => {
                let _ = fs::remove_dir_all(&staging);
                return Err(err.into());
            },
        }
    }

    let exe_name = format!("flox-{name}");
    let exe_path = staging.join(&exe_name);
    if !is_executable(&exe_path) {
        let _ = fs::remove_dir_all(&staging);
        return Err(InstallError::ExecutableMissing {
            name: name.clone(),
            path: exe_path,
        });
    }

    let manifest_path = staging.join("flox-extension.toml");
    let manifest = if manifest_path.exists() {
        Some(parse_author_manifest(&fs::read_to_string(&manifest_path)?)?)
    } else {
        None
    };

    // At this point no matching release asset was found (or there were no
    // assets at all), so the extension is either a pure script or a git
    // checkout. The `[extension.binary]` section is advisory here: it
    // *declares* a binary distribution but we couldn't find one, so the
    // install is treated as a source clone with kind=git/script.
    let kind = match manifest.as_ref().and_then(|m| m.extension.binary.as_ref()) {
        Some(_) => "git",
        None => "script",
    };

    let now = OffsetDateTime::now_utc().format(&Rfc3339)?;
    let state = InstalledState {
        schema: "1".to_string(),
        name: name.clone(),
        kind: kind.to_string(),
        source: format!("https://github.com/{owner}/{repo}"),
        owner: owner.clone(),
        repo: repo.clone(),
        host: "github.com".to_string(),
        tag: resolved.tag.clone().unwrap_or_default(),
        branch: resolved.branch.clone().unwrap_or_default(),
        commit: resolved.commit.clone(),
        pinned: pin.is_some(),
        asset_sha256: String::new(),
        installed_at: now,
        path: install_dir.display().to_string(),
    };
    let state_str = render_installed_state(&state)?;
    fs::write(staging.join("state.toml"), state_str)?;

    if install_dir.exists() {
        // --force path: remove the prior install before the atomic rename.
        fs::remove_dir_all(&install_dir)?;
    }
    if let Err(err) = atomic_install(&staging, &install_dir) {
        let _ = fs::remove_dir_all(&staging);
        return Err(err.into());
    }

    Ok(Extension {
        name,
        install_dir,
        state,
    })
}

/// Install a precompiled binary extension: download the release asset,
/// extract it (or copy it as-is for bare binaries), locate the
/// `flox-<name>` executable, and write `state.toml` with
/// `kind="binary"`. The caller has already acquired the extensions lock.
#[allow(clippy::too_many_arguments)]
async fn install_github_binary(
    flox: &Flox,
    owner: &str,
    repo: &str,
    name: &str,
    resolved: &super::github::ResolvedRef,
    asset: &super::github::ReleaseAsset,
    pinned: bool,
    source: &super::github::GitHubSource,
    manifest: Option<&super::manifest::AuthorManifest>,
) -> Result<Extension, InstallError> {
    // The two callers (`install_github` and `upgrade_binary`) hold the
    // extensions lock and have already decided whether to proceed past an
    // existing install_dir, so re-checking `install_dir.exists() && !force`
    // here is redundant and would race against the lock anyway.
    let extensions_root = layout::extensions_root(flox);
    let install_dir = layout::install_dir(flox, name);

    let staging = extensions_root.join(format!(".staging-{}", Uuid::new_v4()));
    fs::create_dir(&staging)?;

    let result = populate_binary_staging(
        source,
        asset,
        &staging,
        name,
        owner,
        repo,
        resolved,
        pinned,
        &install_dir,
        manifest,
    )
    .await;
    let state = match result {
        Ok(s) => s,
        Err(err) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(err);
        },
    };

    if install_dir.exists() {
        fs::remove_dir_all(&install_dir)?;
    }
    if let Err(err) = atomic_install(&staging, &install_dir) {
        let _ = fs::remove_dir_all(&staging);
        return Err(err.into());
    }

    Ok(Extension {
        name: name.to_string(),
        install_dir,
        state,
    })
}

#[allow(clippy::too_many_arguments)]
async fn populate_binary_staging(
    source: &super::github::GitHubSource,
    asset: &super::github::ReleaseAsset,
    staging: &Path,
    name: &str,
    owner: &str,
    repo: &str,
    resolved: &super::github::ResolvedRef,
    pinned: bool,
    install_dir: &Path,
    manifest: Option<&super::manifest::AuthorManifest>,
) -> Result<InstalledState, InstallError> {
    let asset_path = staging.join(".tmp-asset");
    let sha = source.download_asset(asset, &asset_path).await?;

    if let Some(expected) = manifest
        .and_then(|m| m.extension.binary.as_ref())
        .and_then(|b| b.sha256.as_deref())
    {
        let expected_lc = expected.to_ascii_lowercase();
        let actual_lc = sha.to_ascii_lowercase();
        if expected_lc != actual_lc {
            return Err(InstallError::Sha256Mismatch {
                expected: expected_lc,
                actual: actual_lc,
            });
        }
    }

    extract_asset(&asset_path, staging, &asset.name, name)?;
    // Drop the raw asset once extraction has finished.
    let _ = fs::remove_file(&asset_path);

    let staged_exe = staging.join(format!("flox-{name}"));
    if !is_executable(&staged_exe) {
        return Err(InstallError::ExecutableMissing {
            name: name.to_string(),
            path: staged_exe,
        });
    }
    // A zero-byte asset (e.g. an empty raw download) is +x but not a usable
    // executable; reject it here rather than "installing" a binary that
    // fails at exec.
    if fs::metadata(&staged_exe).map(|m| m.len()).unwrap_or(0) == 0 {
        return Err(InstallError::ExecutableMissing {
            name: name.to_string(),
            path: staged_exe,
        });
    }

    let now = OffsetDateTime::now_utc().format(&Rfc3339)?;
    let state = InstalledState {
        schema: "1".to_string(),
        name: name.to_string(),
        kind: "binary".to_string(),
        source: format!("https://github.com/{owner}/{repo}"),
        owner: owner.to_string(),
        repo: repo.to_string(),
        host: "github.com".to_string(),
        tag: resolved.tag.clone().unwrap_or_default(),
        branch: resolved.branch.clone().unwrap_or_default(),
        commit: resolved.commit.clone(),
        pinned,
        asset_sha256: sha,
        installed_at: now,
        path: install_dir.display().to_string(),
    };
    let state_str = render_installed_state(&state)?;
    fs::write(staging.join("state.toml"), state_str)?;

    // Persist the fetched author manifest so dispatch can honor the
    // extension's [environment] / [on_active] policy. A binary extension's
    // archive usually contains only the executable, so without this the
    // manifest would be lost and dispatch would default to Inherit.
    if let Some(manifest) = manifest {
        let manifest_str = super::manifest::render_author_manifest(manifest)?;
        fs::write(staging.join("flox-extension.toml"), manifest_str)?;
    }

    Ok(state)
}

fn extract_asset(
    asset_path: &Path,
    staging: &Path,
    asset_name: &str,
    extension_name: &str,
) -> Result<(), InstallError> {
    let lower = asset_name.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        super::archive::extract_tar_gz(asset_path, staging)?;
        super::archive::locate_executable(staging, extension_name)?;
        return Ok(());
    }
    if lower.ends_with(".zip") {
        super::archive::extract_zip(asset_path, staging)?;
        super::archive::locate_executable(staging, extension_name)?;
        return Ok(());
    }
    // Raw single-file asset: copy into place as `flox-<name>`.
    super::archive::install_raw(asset_path, staging, extension_name)?;
    Ok(())
}

/// Re-fetch the recorded branch and reset the install dir to FETCH_HEAD,
/// updating `state.toml` if the commit changed.
///
/// Local-kind extensions cannot be upgraded (they were copied without a
/// remote). Pinned installs are skipped unless `force` is true. The fetch
/// uses [`flox_rust_sdk::providers::git::GitCommandProvider::fetch_branch`]; the
/// reset shells out to `git -C <dir> reset --hard FETCH_HEAD` because no
/// trait wrapper exists for `reset` (P02-D2 precedent).
pub async fn upgrade(flox: &Flox, name: &str, force: bool) -> Result<UpgradeStatus, UpgradeError> {
    // Reject path-traversal / invalid names before composing any path
    // (upgrade builds `flox-<name>` for the install dir and state path).
    if !is_valid_extension_name(name) {
        return Err(UpgradeError::NotInstalled(name.to_string()));
    }
    // Acquire the lock BEFORE reading state.toml so a concurrent `install`
    // / `remove` / `upgrade` can't swap the file between the read here and
    // any subsequent write. The `local` / pinned early-returns stay, but
    // execute under the lock — no mutation happens on those paths, so
    // holding the lock is cost-free.
    let _guard = LockGuard::acquire(&layout::lock_path(flox))?;

    let install_dir = layout::install_dir(flox, name);
    let state_path = layout::state_path(flox, name);
    let state_str = fs::read_to_string(&state_path).map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            UpgradeError::NotInstalled(name.to_string())
        } else {
            UpgradeError::Io(err)
        }
    })?;
    let state = parse_installed_state(&state_str)?;

    if state.kind == "local" {
        return Err(UpgradeError::LocalNotSupported);
    }
    if state.pinned && !force {
        return Ok(UpgradeStatus::Pinned);
    }

    if state.kind == "binary" {
        return upgrade_binary(flox, name, &state).await;
    }

    if state.branch.is_empty() {
        // No tracked branch. A source extension installed from a release
        // tag records a tag but no branch; upgrade it by re-resolving and
        // re-cloning at the newest tag (mirrors the binary path). Only a
        // genuinely branchless *and* tagless install is unupgradeable.
        if state.tag.is_empty() {
            return Err(UpgradeError::NoBranch(name.to_string()));
        }
        // Release the lock before delegating: `install_github` re-acquires
        // it, and `LockGuard` is non-reentrant. `install_github` is
        // self-contained (re-resolves and force-installs atomically under
        // its own lock), so nothing here needs protecting across the gap.
        drop(_guard);
        return upgrade_source_tag(flox, &state).await;
    }

    let git = flox_rust_sdk::providers::git::GitCommandProvider::open(&install_dir)
        .map_err(|err| UpgradeError::Io(io::Error::other(err.to_string())))?;
    // Fetch the source ref without updating any local branch (no colon in
    // the refspec). Updating `refs/heads/<branch>` directly would be
    // refused by git because that branch is the current checkout.
    // FETCH_HEAD is always written and is what `reset --hard` consumes.
    git.fetch_ref("origin", &state.branch)?;

    let reset_status = Command::new("git")
        .arg("-C")
        .arg(&install_dir)
        .args(["reset", "--hard", "FETCH_HEAD"])
        .status()?;
    if !reset_status.success() {
        return Err(UpgradeError::ResetFailed {
            dir: install_dir.clone(),
            code: reset_status.code().unwrap_or(-1),
        });
    }

    let head_out = Command::new("git")
        .arg("-C")
        .arg(&install_dir)
        .args(["rev-parse", "HEAD"])
        .output()?;
    if !head_out.status.success() {
        return Err(UpgradeError::RevParseFailed {
            dir: install_dir.clone(),
            code: head_out.status.code().unwrap_or(-1),
        });
    }
    let new_commit = String::from_utf8_lossy(&head_out.stdout).trim().to_string();

    if new_commit == state.commit {
        return Ok(UpgradeStatus::AlreadyCurrent);
    }

    let from = state.commit.clone();
    let now = OffsetDateTime::now_utc().format(&Rfc3339)?;
    let new_state = InstalledState {
        commit: new_commit.clone(),
        installed_at: now,
        ..state
    };
    let new_state_str = render_installed_state(&new_state)?;
    atomic_write_state(&state_path, &new_state_str)?;

    Ok(UpgradeStatus::Upgraded {
        from,
        to: new_commit,
    })
}

/// Write `state.toml` atomically via a sibling `.tmp` + `rename`. If the
/// caller process crashes mid-write, the next `upgrade` sees the prior
/// file unchanged rather than a truncated one.
fn atomic_write_state(state_path: &Path, contents: &str) -> io::Result<()> {
    let tmp = state_path.with_extension("toml.tmp");
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, state_path)?;
    Ok(())
}

/// Binary-kind upgrade: re-run `resolve_latest` for the recorded
/// owner/repo, compare tags, and re-install from the fresh release asset
/// if it has advanced. Does not acquire the extensions lock itself —
/// the caller (`upgrade`) holds it for the full mutator, because fslock
/// is non-reentrant and `install_github_binary` assumes the lock is
/// already held.
async fn upgrade_binary(
    flox: &Flox,
    name: &str,
    state: &InstalledState,
) -> Result<UpgradeStatus, UpgradeError> {
    let source = super::github::GitHubSource::from_env();
    let resolved = source
        .resolve_latest(&state.owner, &state.repo)
        .await
        .map_err(|err| UpgradeError::Install(Box::new(InstallError::GitHub(err))))?;

    let new_tag = resolved.tag.clone().unwrap_or_default();
    if !new_tag.is_empty() && new_tag == state.tag {
        return Ok(UpgradeStatus::AlreadyCurrent);
    }

    let tag_for_lookup = match resolved.tag.as_deref() {
        Some(t) if !t.is_empty() => t,
        _ => {
            return Err(UpgradeError::NoRelease {
                owner: state.owner.clone(),
                repo: state.repo.clone(),
            });
        },
    };
    let assets = source
        .list_release_assets(&state.owner, &state.repo, tag_for_lookup)
        .await
        .map_err(|err| UpgradeError::Install(Box::new(InstallError::GitHub(err))))?;

    // Fetch the manifest before resolving the asset so its `asset` template
    // can influence selection (see the matching reorder in `install_github`).
    let manifest_ref = resolved.tag.as_deref().unwrap_or(&resolved.commit);
    let manifest = source
        .fetch_author_manifest(&state.owner, &state.repo, manifest_ref)
        .await
        .map_err(|err| UpgradeError::Install(Box::new(InstallError::GitHub(err))))?;

    let asset = match super::github::resolve_asset(&assets, manifest.as_ref(), name) {
        Ok(a) => a.clone(),
        Err(err) => {
            return Err(UpgradeError::Install(Box::new(
                InstallError::NoMatchingAsset {
                    owner: state.owner.clone(),
                    repo: state.repo.clone(),
                    platform: err.platform,
                },
            )));
        },
    };

    let from = if !state.tag.is_empty() {
        state.tag.clone()
    } else {
        state.commit.clone()
    };

    install_github_binary(
        flox,
        &state.owner,
        &state.repo,
        name,
        &resolved,
        &asset,
        state.pinned,
        &source,
        manifest.as_ref(),
    )
    .await
    .map_err(|e| UpgradeError::Install(Box::new(e)))?;

    let to = if new_tag.is_empty() {
        resolved.commit.clone()
    } else {
        new_tag
    };
    Ok(UpgradeStatus::Upgraded { from, to })
}

/// Upgrade a source (`script`/`git`) extension that was installed from a
/// release tag and therefore has no tracked branch: re-resolve the latest
/// tag and, if it advanced, re-clone at it via the install flow.
async fn upgrade_source_tag(
    flox: &Flox,
    state: &InstalledState,
) -> Result<UpgradeStatus, UpgradeError> {
    let source = super::github::GitHubSource::from_env();
    let resolved = source
        .resolve_latest(&state.owner, &state.repo)
        .await
        .map_err(|err| UpgradeError::Install(Box::new(InstallError::GitHub(err))))?;
    let new_tag = resolved.tag.clone().unwrap_or_default();
    if new_tag.is_empty() || new_tag == state.tag {
        return Ok(UpgradeStatus::AlreadyCurrent);
    }

    // Re-clone at the newest tag. `install_github` re-resolves latest and
    // overwrites the existing install under `force`.
    let spec = format!("{}/{}", state.owner, state.repo);
    install_github(flox, &spec, None, true)
        .await
        .map_err(|err| UpgradeError::Install(Box::new(err)))?;

    Ok(UpgradeStatus::Upgraded {
        from: state.tag.clone(),
        to: new_tag,
    })
}

/// Upgrade every installed extension, collecting per-item results.
///
/// Design note: per D4 in the P05 plan, each call to the inner `upgrade()`
/// acquires and releases the extensions lock independently. `list` stays
/// lock-free; other operations may interleave between items. The
/// alternative (one lock across the whole loop) was rejected to keep
/// blast radius small and to preserve the existing per-mutator locking
/// pattern.
pub async fn upgrade_all(flox: &Flox, force: bool) -> Result<Vec<UpgradeResult>, ListError> {
    let extensions = list(flox)?;
    let mut out = Vec::with_capacity(extensions.len());
    for ext in extensions {
        let outcome = upgrade(flox, &ext.name, force).await;
        out.push(UpgradeResult {
            name: ext.name,
            outcome,
        });
    }
    Ok(out)
}

/// Read-only dry-run of `upgrade` for a single extension. Never writes
/// `state.toml`; never writes `FETCH_HEAD`; never downloads an asset.
///
/// - `kind = local` → `UpgradeError::LocalNotSupported` (same as `upgrade`).
/// - `state.pinned` → `DryRunStatus::Pinned` (no remote I/O).
/// - `kind = binary` → `GitHubSource::resolve_latest`; compare tag.
/// - `kind = script|git` → `git ls-remote <origin-url> <branch>`; compare sha.
pub async fn upgrade_dry_run(
    flox: &Flox,
    name: &str,
    force: bool,
) -> Result<DryRunStatus, UpgradeError> {
    let state_path = layout::state_path(flox, name);
    let state_str = fs::read_to_string(&state_path).map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            UpgradeError::NotInstalled(name.to_string())
        } else {
            UpgradeError::Io(err)
        }
    })?;
    let state = parse_installed_state(&state_str)?;

    if state.kind == "local" {
        return Err(UpgradeError::LocalNotSupported);
    }
    if state.pinned && !force {
        return Ok(DryRunStatus::Pinned);
    }

    if state.kind == "binary" {
        let source = super::github::GitHubSource::from_env();
        let resolved = source
            .resolve_latest(&state.owner, &state.repo)
            .await
            .map_err(|err| UpgradeError::Install(Box::new(InstallError::GitHub(err))))?;
        let new_tag = resolved.tag.clone().unwrap_or_default();
        if !new_tag.is_empty() && new_tag == state.tag {
            return Ok(DryRunStatus::AlreadyCurrent);
        }
        // Match `upgrade_binary`'s gate: if there's no published release on
        // the remote, report NoRelease rather than offering to "upgrade" to
        // whatever the default branch HEAD happens to be. A real `upgrade`
        // would bail with the same error.
        if new_tag.is_empty() {
            return Err(UpgradeError::NoRelease {
                owner: state.owner.clone(),
                repo: state.repo.clone(),
            });
        }
        let from = if !state.tag.is_empty() {
            state.tag.clone()
        } else {
            state.commit.clone()
        };
        return Ok(DryRunStatus::WouldUpgrade { from, to: new_tag });
    }

    if state.branch.is_empty() {
        return Err(UpgradeError::NoBranch(name.to_string()));
    }

    // script / git: `git ls-remote origin <branch>` — read-only.
    let install_dir = layout::install_dir(flox, name);
    let out = Command::new("git")
        .arg("-C")
        .arg(&install_dir)
        .args(["ls-remote", "origin", &state.branch])
        .output()?;
    if !out.status.success() {
        return Err(UpgradeError::LsRemoteFailed {
            dir: install_dir.clone(),
            branch: state.branch.clone(),
            code: out.status.code().unwrap_or(-1),
        });
    }
    let remote_sha = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().next())
        .unwrap_or("")
        .to_string();
    if remote_sha.is_empty() {
        return Err(UpgradeError::NoBranch(name.to_string()));
    }
    if remote_sha == state.commit {
        return Ok(DryRunStatus::AlreadyCurrent);
    }
    Ok(DryRunStatus::WouldUpgrade {
        from: state.commit.clone(),
        to: remote_sha,
    })
}

/// `upgrade_all` analogue for dry-run mode. Mirrors `upgrade_all`'s
/// `Result<Vec<...>, ListError>` shape so the outer `list` failure is
/// propagated cleanly rather than swallowed per-item.
pub async fn upgrade_all_dry_run(
    flox: &Flox,
    force: bool,
) -> Result<Vec<DryRunResult>, ListError> {
    let extensions = list(flox)?;
    let mut out = Vec::with_capacity(extensions.len());
    for ext in extensions {
        let outcome = upgrade_dry_run(flox, &ext.name, force).await;
        out.push(DryRunResult {
            name: ext.name,
            outcome,
        });
    }
    Ok(out)
}

/// Search GitHub for repositories tagged `flox-extension` and mark any
/// that are already installed locally. Returns the decorated rows plus
/// the Search API's `incomplete_results` flag so the caller can warn.
///
/// Wraps `GitHubSource::from_env().search_repos()` (so `GH_TOKEN` /
/// `GITHUB_TOKEN` and `FLOX_EXTENSIONS_GITHUB_BASE_URL` flow through)
/// and cross-references `list(flox)` for the ✓ column. Local-only
/// extensions (kind = "local") have no remote identity and are skipped.
pub async fn search(flox: &Flox, q: &SearchQuery) -> Result<(Vec<SearchRow>, bool), SearchError> {
    let source = GitHubSource::from_env();
    let SearchResponse {
        incomplete_results,
        items,
        ..
    } = source.search_repos(q).await?;

    let installed = list(flox)?;
    // GitHub owner/repo handles are case-insensitive but preserve the
    // registered casing in API payloads. Installed state preserves
    // whatever case the user typed at install time. Compare lowercased
    // on both sides so `Vercel/foo` locally matches `vercel/foo` from
    // the API.
    let installed_keys: HashSet<String> = installed
        .into_iter()
        .filter(|e| e.state.kind != "local")
        .map(|e| format!("{}/{}", e.state.owner, e.state.repo).to_ascii_lowercase())
        .collect();

    let rows = items
        .into_iter()
        .map(|item| {
            let installed = installed_keys.contains(&item.full_name.to_ascii_lowercase());
            SearchRow {
                full_name: item.full_name,
                stars: item.stargazers_count,
                description: item.description,
                installed,
            }
        })
        .collect();

    Ok((rows, incomplete_results))
}

/// Parse `owner/repo` strictly: exactly two non-empty segments, no extra
/// `/` or whitespace.
fn parse_owner_repo(spec: &str) -> Result<(String, String), InstallError> {
    let trimmed = spec.trim();
    let parts: Vec<&str> = trimmed.split('/').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        return Err(InstallError::InvalidSpec(spec.to_string()));
    }
    // Validate the owner charset so odd input (`owner /repo`, control
    // characters) is rejected cleanly here rather than flowing into a clone
    // URL / API path and producing a confusing downstream 404. The repo
    // segment is validated separately by `extract_extension_name`.
    if super::github::validate_owner(parts[0]).is_err() {
        return Err(InstallError::InvalidSpec(spec.to_string()));
    }
    Ok((parts[0].to_string(), parts[1].to_string()))
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

/// Strip the `flox-` prefix from a repo name and return the bare extension
/// `<name>`, or `None` if the prefix is missing or `<name>` doesn't match
/// `^[a-z0-9][a-z0-9_-]*$`.
///
/// Used by `install_github` to derive the on-disk extension name from a
/// `<owner>/<repo>` spec. Lives here next to the other manager validators
/// so it shares the InstallError surface.
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
/// This is the guard `remove` and `upgrade` use before composing a
/// `flox-<name>` path. A name containing `/`, a path separator, or `..`
/// fails the charset check, which is what prevents those operations from
/// building a path that escapes the extensions root and, in `remove`'s
/// case, recursively deleting it.
pub fn is_valid_extension_name(name: &str) -> bool {
    extract_extension_name(&format!("flox-{name}")) == Some(name)
}

fn git_head_commit(source: &Path) -> Option<String> {
    if !source.join(".git").exists() {
        return None;
    }
    let out = Command::new("git")
        .arg("-C")
        .arg(source)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        debug!(?source, status = ?out.status, "git rev-parse HEAD failed");
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn fmt_executable_missing(name: &str, path: &Path) -> String {
    format!("extension '{name}' has no executable at {}", path.display())
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

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
    fn lock_guard_blocks_second_try_acquire() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join(".lock");

        std::thread::scope(|scope| {
            let (held, hold_rx) = mpsc::sync_channel::<()>(0);
            let (release, release_rx) = mpsc::sync_channel::<()>(0);
            let path_a = path.clone();
            let path_b = path.clone();

            scope.spawn(move || {
                let _guard = LockGuard::acquire(&path_a).unwrap();
                held.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(2)).unwrap();
            });

            scope.spawn(move || {
                hold_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                let err = LockGuard::try_acquire(&path_b).unwrap_err();
                assert!(matches!(err, LockError::WouldBlock { .. }));
                release.send(()).unwrap();
            });
        });
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
        assert_eq!(state.kind, "local");
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
    /// failure between `create_dir(&staging)` and `atomic_install`. We
    /// trigger the failure by pre-creating the final install dir so
    /// `atomic_install` gets `EEXIST`/`ENOTEMPTY`. The same staging-cleanup
    /// path also covers any earlier failure inside `populate_local_staging`.
    #[test]
    fn install_local_cleans_staging_on_atomic_install_failure() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = make_source(src_root.path(), "hello");

        // Pre-populate install_dir so atomic_install cannot rename onto it.
        let install_dir = layout::install_dir(&flox, "hello");
        fs::create_dir_all(&install_dir).unwrap();
        fs::write(install_dir.join("marker"), "pre-existing").unwrap();

        let _ = install_local(&flox, &src, false).unwrap_err();

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
    fn parse_owner_repo_rejects_invalid_owner_and_shape() {
        assert!(parse_owner_repo("good-owner/flox-hi").is_ok());
        assert!(parse_owner_repo("owner /flox-hi").is_err());
        assert!(parse_owner_repo("ow ner/flox-hi").is_err());
        assert!(parse_owner_repo("a/b/c").is_err());
        assert!(parse_owner_repo("/flox-hi").is_err());
        assert!(parse_owner_repo("owner/").is_err());
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

    /// Build a bare git repo with a single commit containing an executable
    /// `flox-<name>` script, returning `(bare_repo_path, head_sha)`.
    fn build_bare_repo_with_extension(
        parent: &Path,
        name: &str,
        script_body: &str,
    ) -> (PathBuf, String) {
        let work = parent.join(format!("work-flox-{name}"));
        let bare = parent.join(format!("bare-flox-{name}.git"));
        fs::create_dir(&work).unwrap();
        Command::new("git")
            .arg("init")
            .arg("--bare")
            .arg(&bare)
            .status()
            .unwrap();
        Command::new("git")
            .arg("init")
            .arg("-b")
            .arg("main")
            .arg(&work)
            .status()
            .unwrap();
        write_exe(&work.join(format!("flox-{name}")), script_body);
        for (k, v) in [
            ("user.email", "t@e"),
            ("user.name", "t"),
            ("commit.gpgsign", "false"),
        ] {
            Command::new("git")
                .arg("-C")
                .arg(&work)
                .args(["config", k, v])
                .status()
                .unwrap();
        }
        Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(["add", "-A"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(["commit", "-q", "-m", "initial"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(["remote", "add", "origin"])
            .arg(&bare)
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(["push", "-q", "origin", "main"])
            .status()
            .unwrap();
        let head = Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap();
        let sha = String::from_utf8(head.stdout).unwrap().trim().to_string();
        (bare, sha)
    }

    /// Clone a bare repo as if `install_github` had done it: copy the worktree
    /// shape into `extensions_root/flox-<name>` and write a matching `state.toml`.
    fn install_from_bare_repo(
        flox: &Flox,
        name: &str,
        bare: &Path,
        sha: &str,
        branch: &str,
    ) -> PathBuf {
        let install_dir = layout::install_dir(flox, name);
        let parent = install_dir.parent().unwrap();
        fs::create_dir_all(parent).unwrap();
        let bare_url = format!("file://{}", bare.display());
        Command::new("git")
            .args([
                "clone",
                "--quiet",
                "--single-branch",
                "--no-tags",
                "--branch",
                branch,
                &bare_url,
            ])
            .arg(&install_dir)
            .status()
            .unwrap();

        let state = InstalledState {
            schema: "1".to_string(),
            name: name.to_string(),
            kind: "script".to_string(),
            source: bare_url,
            owner: "owner".to_string(),
            repo: format!("flox-{name}"),
            host: "github.com".to_string(),
            tag: String::new(),
            branch: branch.to_string(),
            commit: sha.to_string(),
            pinned: false,
            asset_sha256: String::new(),
            installed_at: "2026-04-17T00:00:00Z".to_string(),
            path: install_dir.display().to_string(),
        };
        let state_str = render_installed_state(&state).unwrap();
        fs::write(layout::state_path(flox, name), state_str).unwrap();
        install_dir
    }

    #[tokio::test]
    async fn upgrade_errors_when_not_installed() {
        let (flox, _tempdir) = flox_instance();
        let err = upgrade(&flox, "missing", false).await.unwrap_err();
        assert!(matches!(err, UpgradeError::NotInstalled(ref n) if n == "missing"));
    }

    #[tokio::test]
    async fn upgrade_errors_for_local_kind() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let src = make_source(src_root.path(), "hello");
        install_local(&flox, &src, false).unwrap();

        let err = upgrade(&flox, "hello", false).await.unwrap_err();
        assert!(matches!(err, UpgradeError::LocalNotSupported));
    }

    #[tokio::test]
    async fn upgrade_returns_pinned_when_pinned_and_no_force() {
        let (flox, _tempdir) = flox_instance();
        let parent = TempDir::new().unwrap();
        let (bare, sha) =
            build_bare_repo_with_extension(parent.path(), "hello", "#!/bin/sh\necho v1\n");
        let install_dir = install_from_bare_repo(&flox, "hello", &bare, &sha, "main");

        // Mark the install as pinned.
        let mut state =
            parse_installed_state(&fs::read_to_string(install_dir.join("state.toml")).unwrap())
                .unwrap();
        state.pinned = true;
        fs::write(
            install_dir.join("state.toml"),
            render_installed_state(&state).unwrap(),
        )
        .unwrap();

        let result = upgrade(&flox, "hello", false).await.unwrap();
        assert_eq!(result, UpgradeStatus::Pinned);
    }

    #[tokio::test]
    async fn upgrade_returns_already_current_when_remote_unchanged() {
        let (flox, _tempdir) = flox_instance();
        let parent = TempDir::new().unwrap();
        let (bare, sha) =
            build_bare_repo_with_extension(parent.path(), "hello", "#!/bin/sh\necho v1\n");
        install_from_bare_repo(&flox, "hello", &bare, &sha, "main");

        let result = upgrade(&flox, "hello", false).await.unwrap();
        assert_eq!(result, UpgradeStatus::AlreadyCurrent);
    }

    #[tokio::test]
    async fn upgrade_advances_commit_when_remote_changed() {
        let (flox, _tempdir) = flox_instance();
        let parent = TempDir::new().unwrap();
        let (bare, sha_v1) =
            build_bare_repo_with_extension(parent.path(), "hello", "#!/bin/sh\necho v1\n");
        let install_dir = install_from_bare_repo(&flox, "hello", &bare, &sha_v1, "main");

        // Push a new commit to the bare repo by re-using its working clone.
        let work = parent.path().join("work-flox-hello");
        write_exe(&work.join("flox-hello"), "#!/bin/sh\necho v2\n");
        Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(["add", "-A"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(["commit", "-q", "-m", "v2"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(["push", "-q", "origin", "main"])
            .status()
            .unwrap();
        let sha_v2 = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&work)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();
        assert_ne!(sha_v1, sha_v2);

        let result = upgrade(&flox, "hello", false).await.unwrap();
        match result {
            UpgradeStatus::Upgraded { from, to } => {
                assert_eq!(from, sha_v1);
                assert_eq!(to, sha_v2);
            },
            other => panic!("expected Upgraded, got {other:?}"),
        }

        // state.toml now records the new commit.
        let state =
            parse_installed_state(&fs::read_to_string(install_dir.join("state.toml")).unwrap())
                .unwrap();
        assert_eq!(state.commit, sha_v2);
    }

    /// TS05: binary upgrade short-circuits to `AlreadyCurrent` when the
    /// remote tag matches the recorded tag. Uses httpmock to simulate the
    /// GitHub API and `FLOX_EXTENSIONS_GITHUB_BASE_URL` to point the
    /// `GitHubSource::from_env()` client at it. No file-system writes are
    /// expected beyond the pre-seeded state.
    #[tokio::test]
    async fn upgrade_binary_returns_already_current_when_tag_unchanged() {
        use httpmock::Method::GET;
        use httpmock::MockServer;
        use serde_json::json;

        let server = MockServer::start_async().await;
        let _release_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/releases/latest");
            then.status(200).json_body(json!({
                "tag_name": "v1.0.0",
                "target_commitish": "main"
            }));
        });
        let _commit_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/commits/v1.0.0");
            then.status(200).json_body(json!({
                "sha": "cafef00d00000000000000000000000000000000"
            }));
        });

        let (flox, _tempdir) = flox_instance();

        // Seed install_dir and state.toml as if `install_github_binary`
        // had already run for v1.0.0.
        let install_dir = layout::install_dir(&flox, "hello");
        fs::create_dir_all(&install_dir).unwrap();
        write_exe(&install_dir.join("flox-hello"), "#!/bin/sh\necho v1\n");
        let seeded = InstalledState {
            schema: "1".to_string(),
            name: "hello".to_string(),
            kind: "binary".to_string(),
            source: "https://github.com/owner/flox-hello".to_string(),
            owner: "owner".to_string(),
            repo: "flox-hello".to_string(),
            host: "github.com".to_string(),
            tag: "v1.0.0".to_string(),
            branch: String::new(),
            commit: "cafef00d00000000000000000000000000000000".to_string(),
            pinned: false,
            asset_sha256: "deadbeef".to_string(),
            installed_at: "2026-04-17T00:00:00Z".to_string(),
            path: install_dir.display().to_string(),
        };
        fs::write(
            layout::state_path(&flox, "hello"),
            render_installed_state(&seeded).unwrap(),
        )
        .unwrap();

        let prior_mtime = fs::metadata(install_dir.join("flox-hello"))
            .unwrap()
            .modified()
            .ok();

        let status = temp_env::async_with_vars(
            [("FLOX_EXTENSIONS_GITHUB_BASE_URL", Some(server.base_url()))],
            upgrade(&flox, "hello", false),
        )
        .await
        .unwrap();
        assert_eq!(status, UpgradeStatus::AlreadyCurrent);

        // The executable file has not been rewritten.
        let post_mtime = fs::metadata(install_dir.join("flox-hello"))
            .unwrap()
            .modified()
            .ok();
        assert_eq!(prior_mtime, post_mtime);
    }

    /// BUG-05/08/16 regression: `atomic_write_state` must write via a
    /// sibling `.tmp` file and rename, so a crash mid-write leaves the
    /// previous `state.toml` intact and `upgrade` doesn't read a partially
    /// written file. We observe the tmp file is cleaned up on success.
    #[test]
    fn atomic_write_state_renames_tmp_onto_target() {
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let state_path = dir.path().join("state.toml");
        fs::write(&state_path, "schema = \"1\"\nold = true\n").unwrap();
        atomic_write_state(&state_path, "schema = \"1\"\nnew = true\n").unwrap();
        assert_eq!(
            fs::read_to_string(&state_path).unwrap(),
            "schema = \"1\"\nnew = true\n"
        );
        assert!(!state_path.with_extension("toml.tmp").exists());
    }

    /// BUG-13 regression: `upgrade` must distinguish "state.toml does not
    /// exist" (extension not installed) from other I/O failures — a
    /// blanket `|_| NotInstalled` hides permission/disk errors behind a
    /// confusing "not installed" message. Here we write a state.toml that
    /// is a directory (so `read_to_string` fails with IsADirectory /
    /// InvalidInput, not NotFound) and assert we get `Io`.
    #[tokio::test]
    async fn upgrade_surfaces_non_notfound_io_errors() {
        let (flox, _tempdir) = flox_instance();
        let state_path = layout::state_path(&flox, "hello");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        fs::create_dir(&state_path).unwrap();

        let err = upgrade(&flox, "hello", false).await.unwrap_err();
        assert!(
            matches!(err, UpgradeError::Io(_)),
            "expected Io, got {err:?}"
        );
    }

    /// BUG-13 regression (dry-run): `upgrade_dry_run` must mirror
    /// `upgrade`'s classifier — non-NotFound I/O errors surface as
    /// `Io` rather than masquerading as `NotInstalled`.
    #[tokio::test]
    async fn upgrade_dry_run_surfaces_non_notfound_io_errors() {
        let (flox, _tempdir) = flox_instance();
        let state_path = layout::state_path(&flox, "hello");
        fs::create_dir_all(state_path.parent().unwrap()).unwrap();
        fs::create_dir(&state_path).unwrap();

        let err = upgrade_dry_run(&flox, "hello", false).await.unwrap_err();
        assert!(
            matches!(err, UpgradeError::Io(_)),
            "expected Io, got {err:?}"
        );
    }

    /// BUG-04 regression: `populate_binary_staging` must compare the
    /// downloaded asset's SHA-256 against the digest declared in the author
    /// manifest's `[extension.binary]` block and surface a
    /// `Sha256Mismatch` when they disagree. A matching digest (and the
    /// no-digest case) must pass through silently.
    #[tokio::test]
    async fn populate_binary_staging_rejects_sha256_mismatch() {
        use httpmock::Method::GET;
        use httpmock::MockServer;
        use sha2::{Digest, Sha256};
        use tempfile::TempDir;

        use crate::extensions::github as github_mod;
        use crate::extensions::manifest::{BinaryMeta, ExtensionMeta};

        let payload: Vec<u8> = b"#!/bin/sh\necho hello\n".to_vec();
        let actual_sha = {
            let mut h = Sha256::new();
            h.update(&payload);
            format!("{:x}", h.finalize())
        };

        let server = MockServer::start_async().await;
        let _asset_mock = server.mock(|when, then| {
            when.method(GET).path("/asset/flox-hello");
            then.status(200)
                .header("Content-Type", "application/octet-stream")
                .body(&payload);
        });

        let asset = github_mod::ReleaseAsset {
            name: "flox-hello".to_string(),
            browser_download_url: format!("{}/asset/flox-hello", server.base_url()),
            size: payload.len() as u64,
            content_type: "application/octet-stream".to_string(),
        };
        let source = github_mod::GitHubSource::new(reqwest::Client::new(), server.base_url());
        let staging = TempDir::new().unwrap();
        let install_dir = staging.path().join("install");
        let resolved = github_mod::ResolvedRef {
            commit: "deadbeef".to_string(),
            tag: Some("v1.0.0".to_string()),
            branch: None,
        };

        let manifest = AuthorManifest {
            schema: "1".to_string(),
            extension: ExtensionMeta {
                name: "hello".to_string(),
                description: None,
                binary: Some(BinaryMeta {
                    source: "github-release".to_string(),
                    asset: "flox-hello".to_string(),
                    sha256: Some(
                        "0000000000000000000000000000000000000000000000000000000000000000"
                            .to_string(),
                    ),
                }),
            },
            environment: None,
            on_active: None,
        };

        let err = populate_binary_staging(
            &source,
            &asset,
            staging.path(),
            "hello",
            "owner",
            "flox-hello",
            &resolved,
            false,
            &install_dir,
            Some(&manifest),
        )
        .await
        .unwrap_err();
        match err {
            InstallError::Sha256Mismatch { expected, actual } => {
                assert_eq!(
                    expected,
                    "0000000000000000000000000000000000000000000000000000000000000000"
                );
                assert_eq!(actual, actual_sha);
            },
            other => panic!("expected Sha256Mismatch, got {other:?}"),
        }
    }

    #[test]
    fn upgrade_result_carries_name_and_outcome() {
        let ok = UpgradeResult {
            name: "hello".to_string(),
            outcome: Ok(UpgradeStatus::AlreadyCurrent),
        };
        let err = UpgradeResult {
            name: "ghost".to_string(),
            outcome: Err(UpgradeError::NotInstalled("ghost".to_string())),
        };
        assert_eq!(ok.name, "hello");
        assert!(matches!(ok.outcome, Ok(UpgradeStatus::AlreadyCurrent)));
        assert!(matches!(err.outcome, Err(UpgradeError::NotInstalled(_))));
    }

    #[tokio::test]
    async fn upgrade_all_returns_one_result_per_installed_extension() {
        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let a = make_source(src_root.path(), "alpha");
        let b = make_source(src_root.path(), "beta");
        install_local(&flox, &a, false).unwrap();
        install_local(&flox, &b, false).unwrap();

        let results = upgrade_all(&flox, false).await.unwrap();

        let names: Vec<_> = results.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "beta"]);
        for r in &results {
            assert!(matches!(r.outcome, Err(UpgradeError::LocalNotSupported)));
        }
    }

    #[tokio::test]
    async fn upgrade_all_on_empty_root_returns_empty_vec() {
        let (flox, _tempdir) = flox_instance();
        let results = upgrade_all(&flox, false).await.unwrap();
        assert!(results.is_empty());
    }

    /// P05-TS03: concurrent `upgrade_all` + `remove` must not corrupt
    /// state. Because each upgrade item acquires its own lock (D4), a
    /// concurrent `remove` either (a) runs before upgrade takes the lock
    /// and the upgrade of that item returns `NotInstalled`, or
    /// (b) runs after upgrade completes that item. The test asserts both
    /// threads terminate without panic and that `list` is parseable after.
    ///
    /// Note: `Flox` does not implement `Clone`, so this test shares the
    /// fixture across tasks via `Arc<Flox>` rather than cloning the
    /// spec-literal way. Same observable behaviour — both tasks hit the
    /// same on-disk extensions root.
    #[tokio::test]
    async fn upgrade_all_and_remove_serialize_cleanly() {
        use std::sync::Arc;

        let (flox, _tempdir) = flox_instance();
        let src_root = TempDir::new().unwrap();
        let a = make_source(src_root.path(), "alpha");
        let b = make_source(src_root.path(), "beta");
        install_local(&flox, &a, false).unwrap();
        install_local(&flox, &b, false).unwrap();

        let flox = Arc::new(flox);
        let flox_c = Arc::clone(&flox);
        let h1 = tokio::spawn(async move { upgrade_all(&flox_c, false).await });
        let flox_c = Arc::clone(&flox);
        let h2 = tokio::task::spawn_blocking(move || {
            // Small sleep so upgrade has a chance to hold the lock at least once.
            std::thread::sleep(std::time::Duration::from_millis(10));
            remove(&flox_c, "beta")
        });

        let _ = h1.await.unwrap();
        let _ = h2.await.unwrap();

        // Post-condition: state is parseable (no crash inside list).
        let _ = list(&flox).unwrap();
    }

    #[tokio::test]
    async fn upgrade_dry_run_reports_would_upgrade_for_advanced_remote() {
        let (flox, _tempdir) = flox_instance();
        let parent = TempDir::new().unwrap();
        let (bare, sha_v1) =
            build_bare_repo_with_extension(parent.path(), "hello", "#!/bin/sh\necho v1\n");
        let install_dir = install_from_bare_repo(&flox, "hello", &bare, &sha_v1, "main");

        // Advance the remote.
        let work = parent.path().join("work-flox-hello");
        write_exe(&work.join("flox-hello"), "#!/bin/sh\necho v2\n");
        Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(["add", "-A"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(["commit", "-q", "-m", "v2"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&work)
            .args(["push", "-q", "origin", "main"])
            .status()
            .unwrap();

        // Snapshot state.toml mtime BEFORE the dry-run.
        let marker = install_dir.join("state.toml");
        let before = fs::metadata(&marker).unwrap().modified().unwrap();

        let status = upgrade_dry_run(&flox, "hello", false).await.unwrap();
        match status {
            DryRunStatus::WouldUpgrade { from, to } => {
                assert_eq!(from, sha_v1);
                assert_ne!(to, sha_v1);
            },
            other => panic!("expected WouldUpgrade, got {other:?}"),
        }

        // Dry-run must not touch state.toml.
        let after = fs::metadata(&marker).unwrap().modified().unwrap();
        assert_eq!(before, after);
    }

    /// BUG-09 regression: `upgrade_dry_run`'s binary branch must report
    /// `NoRelease` when the remote has no published tag, matching
    /// `upgrade_binary`'s real-upgrade gate. Previously it fell through
    /// to `WouldUpgrade { to: <default-branch-commit> }`, which lied to
    /// the user — a subsequent non-dry-run `upgrade` would immediately
    /// fail with `NoRelease`.
    #[tokio::test]
    async fn upgrade_dry_run_binary_returns_no_release_when_remote_has_no_tag() {
        use httpmock::Method::GET;
        use httpmock::MockServer;
        use serde_json::json;

        let server = MockServer::start_async().await;
        // /releases/latest 404 triggers default-branch fallback.
        let _release_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/releases/latest");
            then.status(404);
        });
        let _repo_mock = server.mock(|when, then| {
            when.method(GET).path("/repos/owner/flox-hello");
            then.status(200)
                .json_body(json!({ "default_branch": "main" }));
        });
        let _commit_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/commits/main");
            then.status(200)
                .json_body(json!({ "sha": "cafef00d00000000000000000000000000000000" }));
        });

        let (flox, _tempdir) = flox_instance();
        let install_dir = layout::install_dir(&flox, "hello");
        fs::create_dir_all(&install_dir).unwrap();
        write_exe(&install_dir.join("flox-hello"), "#!/bin/sh\necho v1\n");
        let seeded = InstalledState {
            schema: "1".to_string(),
            name: "hello".to_string(),
            kind: "binary".to_string(),
            source: "https://github.com/owner/flox-hello".to_string(),
            owner: "owner".to_string(),
            repo: "flox-hello".to_string(),
            host: "github.com".to_string(),
            tag: "v1.0.0".to_string(),
            branch: String::new(),
            commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            pinned: false,
            asset_sha256: "deadbeef".to_string(),
            installed_at: "2026-04-17T00:00:00Z".to_string(),
            path: install_dir.display().to_string(),
        };
        fs::write(
            layout::state_path(&flox, "hello"),
            render_installed_state(&seeded).unwrap(),
        )
        .unwrap();

        let err = temp_env::async_with_vars(
            [("FLOX_EXTENSIONS_GITHUB_BASE_URL", Some(server.base_url()))],
            upgrade_dry_run(&flox, "hello", false),
        )
        .await
        .unwrap_err();
        match err {
            UpgradeError::NoRelease { owner, repo } => {
                assert_eq!(owner, "owner");
                assert_eq!(repo, "flox-hello");
            },
            other => panic!("expected NoRelease, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn upgrade_dry_run_reports_pinned_without_resolving() {
        let (flox, _tempdir) = flox_instance();
        let parent = TempDir::new().unwrap();
        let (bare, sha) =
            build_bare_repo_with_extension(parent.path(), "hello", "#!/bin/sh\necho v1\n");
        let install_dir = install_from_bare_repo(&flox, "hello", &bare, &sha, "main");
        let mut state =
            parse_installed_state(&fs::read_to_string(install_dir.join("state.toml")).unwrap())
                .unwrap();
        state.pinned = true;
        fs::write(
            install_dir.join("state.toml"),
            render_installed_state(&state).unwrap(),
        )
        .unwrap();

        let status = upgrade_dry_run(&flox, "hello", false).await.unwrap();
        assert_eq!(status, DryRunStatus::Pinned);
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

        let e = InstallError::NoMatchingAsset {
            owner: "flox-examples".to_string(),
            repo: "flox-deploy".to_string(),
            platform: "linux-x86_64".to_string(),
        };
        assert_eq!(
            e.to_string(),
            "no release asset matches 'linux-x86_64' for flox-examples/flox-deploy"
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
