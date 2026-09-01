//! The project-level catalog lock.
//!
//! A project has exactly one catalog lock — `.flox/catalog.lock` — pinning
//! the source of every `catalogs.*` reference made by the project's Nix
//! expressions. A single lock means a single revision always evaluates
//! against one consistent set of inputs, leaving no room for diamond
//! dependency conflicts between packages of the same project.
//!
//! This module resolves and writes that lock; the CLI owns its lifecycle
//! and hands the package builder the file to pass through to the NEF evals.
//! The committed lock is created explicitly by [lock_project_catalog],
//! locking the union of every expression's references, and is consumed by
//! builds exactly as found — deliberately including one that no longer
//! covers the expressions' references, in which case the NEF eval fails and
//! the user recreates the lock explicitly.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use floxhub_client::CatalogClientTrait;
use thiserror::Error;
use tracing::debug;

use crate::{
    BuildLock,
    CatalogRef,
    LockError,
    LockfileError,
    ScanError,
    StaleLockError,
    lock_references,
    render_unresolvable,
    scan_package,
    write_lock,
};

/// File name of the project catalog lock, relative to the `.flox` directory.
pub const CATALOG_LOCKFILE_NAME: &str = "catalog.lock";

/// The location of the project catalog lock within `dot_flox_path`.
pub fn catalog_lockfile_path(dot_flox_path: impl AsRef<Path>) -> PathBuf {
    dot_flox_path.as_ref().join(CATALOG_LOCKFILE_NAME)
}

#[derive(Debug, Error)]
pub enum CatalogLockError {
    #[error(transparent)]
    Scan(#[from] ScanError),

    #[error(transparent)]
    Lock(#[from] LockError),

    /// Unresolvable references, pre-rendered with their dependency chains
    /// and the remediation footer (see [render_unresolvable]) so every CLI
    /// entry point that locks — build, publish, update-catalogs — reports
    /// the reference names and a next step rather than a bare count.
    #[error("{0}")]
    Unresolvable(String),

    #[error(transparent)]
    StaleLock(#[from] StaleLockError),

    #[error(transparent)]
    Lockfile(#[from] LockfileError),
}

/// Resolve `references` through the catalog, or produce an empty lock
/// without any request when there are none — the common case for projects
/// whose expressions make no catalog references.
pub async fn resolve_lock(
    client: &impl CatalogClientTrait,
    references: BTreeSet<CatalogRef>,
) -> Result<BuildLock, CatalogLockError> {
    if references.is_empty() {
        return Ok(BuildLock::default());
    }
    match lock_references(client, references).await {
        Ok(lock) => Ok(lock),
        Err(LockError::Unresolvable(entries)) => Err(CatalogLockError::Unresolvable(
            render_unresolvable(&entries),
        )),
        Err(err) => Err(err.into()),
    }
}

/// Scan every expression named by `rel_file_paths` (relative to
/// `expressions_dir`) and return the union of their catalog references.
pub fn scan_references(
    expressions_dir: impl AsRef<Path>,
    rel_file_paths: impl IntoIterator<Item = impl AsRef<Path>>,
) -> Result<BTreeSet<CatalogRef>, CatalogLockError> {
    let expressions_dir = expressions_dir.as_ref();
    let mut references = BTreeSet::new();
    for rel_file_path in rel_file_paths {
        references.extend(scan_package(expressions_dir, rel_file_path)?);
    }
    Ok(references)
}

/// Create (or re-create) the project catalog lock at `lockfile_path`.
///
/// Scans every expression named by `rel_file_paths` (relative to
/// `expressions_dir`) and locks the union of their catalog references in a
/// single request, so the resulting lock is internally consistent by
/// construction. Returns the scanned references.
pub async fn lock_project_catalog(
    client: &impl CatalogClientTrait,
    expressions_dir: impl AsRef<Path>,
    rel_file_paths: impl IntoIterator<Item = impl AsRef<Path>>,
    lockfile_path: impl AsRef<Path>,
) -> Result<BTreeSet<CatalogRef>, CatalogLockError> {
    let references = scan_references(expressions_dir, rel_file_paths)?;
    let lock = resolve_lock(client, references.clone()).await?;
    write_lock(&lock, &lockfile_path)?;
    debug!(
        path = %lockfile_path.as_ref().display(),
        references = references.len(),
        "wrote project catalog lock"
    );
    Ok(references)
}

#[cfg(test)]
mod tests {
    use floxhub_client::client::test_helpers::new_noop;
    use tempfile::tempdir;

    use super::*;

    /// A project directory whose single expression is `expression`.
    fn project_with_expression(expression: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let project = tempdir().unwrap();
        let dot_flox = project.path().join(".flox");
        let pkgs_dir = dot_flox.join("pkgs");
        std::fs::create_dir_all(&pkgs_dir).unwrap();
        std::fs::write(pkgs_dir.join("hello.nix"), expression).unwrap();
        (project, dot_flox, pkgs_dir)
    }

    #[test]
    fn no_references_scan_empty() {
        let (_project, _dot_flox, pkgs_dir) =
            project_with_expression("{ runCommand }: runCommand \"hello\" { } \"\"");

        let references = scan_references(&pkgs_dir, ["hello.nix"]).unwrap();
        assert_eq!(references, BTreeSet::new());
    }

    /// `lock_project_catalog` writes the committed lock file; with no
    /// references it does so without any catalog request: the no-op client
    /// fails every request it is asked to make, so reaching the network at
    /// all fails this test.
    #[tokio::test]
    async fn lock_project_catalog_writes_an_empty_lock_without_a_catalog_request() {
        let (_project, dot_flox, pkgs_dir) =
            project_with_expression("{ runCommand }: runCommand \"hello\" { } \"\"");
        let lockfile_path = catalog_lockfile_path(&dot_flox);

        let references =
            lock_project_catalog(&new_noop(), &pkgs_dir, ["hello.nix"], &lockfile_path)
                .await
                .unwrap();

        assert_eq!(references, BTreeSet::new());
        assert_eq!(
            std::fs::read_to_string(&lockfile_path).unwrap(),
            "{\n  \"version\": 1,\n  \"direct_catalog_inputs\": {},\n  \"catalogs\": {}\n}\n"
        );
    }
}
