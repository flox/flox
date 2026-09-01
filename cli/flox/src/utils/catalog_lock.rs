//! The catalog lock a build consumes, and its lifetime.
//!
//! Lock *resolution* belongs to `nef-lock-catalog`; what lives here is the
//! CLI-level decision of which lock a given invocation builds against, and
//! the ownership of an ephemeral lock's file. Without a committed
//! `.flox/catalog.lock` the project builds locklessly: the CLI resolves a
//! fresh lock into a temp file that lives exactly as long as the build, and
//! nothing is ever written into the project tree.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use flox_rust_sdk::providers::build::nix_expression_dir_in;
use floxhub_client::CatalogClientTrait;
use nef_lock_catalog::{
    BuildLock,
    CATALOG_LOCKFILE_NAME,
    catalog_lockfile_path,
    read_lock,
    resolve_lock,
    scan_references,
    write_lock,
};
use tracing::debug;

/// The lock a build consumes, created before the package builder is
/// invoked and handed to it as `CATALOG_LOCKFILE`. Owns an ephemeral lock's
/// file: dropping the guard removes it.
#[derive(Debug)]
pub struct BuildLockGuard {
    path: PathBuf,
    lock: BuildLock,
    /// Keeps an ephemeral lock's temp file alive for as long as this value;
    /// `None` when the lock is the committed file.
    _ephemeral: Option<tempfile::TempPath>,
}

impl BuildLockGuard {
    /// The committed `.flox/catalog.lock` exactly as found when one exists;
    /// otherwise a fresh ephemeral lock resolving the union of the
    /// references of the expressions named by `rel_file_paths` (relative to
    /// the project's expression directory), written to a randomly named
    /// temp file that is removed when the returned value is dropped.
    pub async fn new_existing_or_ephemeral(
        client: &impl CatalogClientTrait,
        dot_flox_path: impl AsRef<Path>,
        rel_file_paths: impl IntoIterator<Item = impl AsRef<Path>>,
    ) -> Result<BuildLockGuard> {
        let dot_flox_path = dot_flox_path.as_ref();
        let committed = catalog_lockfile_path(dot_flox_path);
        if committed.exists() {
            let lock = read_lock(&committed)?;
            // The path handed to make is *relative to the project
            // directory* make is started in (`--directory`), composed of
            // two constant components — so a project path containing
            // whitespace (or any other character make's word-splitting
            // positions would mangle) never reaches the makefile.
            let dot_flox_dir_name = dot_flox_path
                .file_name()
                .expect("the .flox path has a final component");
            debug!(path = %committed.display(), "build consumes the committed catalog lock");
            return Ok(BuildLockGuard {
                path: Path::new(dot_flox_dir_name).join(CATALOG_LOCKFILE_NAME),
                lock,
                _ephemeral: None,
            });
        }

        let references = scan_references(nix_expression_dir_in(dot_flox_path), rel_file_paths)?;
        let lock = resolve_lock(client, references).await?;
        // The system temp dir, not flox's own temp dir: flox's derives from
        // `$HOME`, which the user may have placed at a path containing
        // whitespace, and the ephemeral path reaches make's word-splitting
        // positions. The system temp dir shares the whitespace-free
        // assumption the makefile's own PROJECT_TMPDIR (`$(TMPDIR)/<hash>`)
        // already makes. This deliberately sits outside flox's centralized
        // per-process temp cleanup; the guard's drop removes the file
        // instead.
        let temp_path = tempfile::Builder::new()
            .prefix("flox-catalog.lock.")
            .tempfile()
            .context("Could not create a temporary file for the catalog lock.")?
            .into_temp_path();
        write_lock(&lock, &temp_path)?;
        debug!(path = %temp_path.display(), "build consumes a fresh ephemeral catalog lock");
        Ok(BuildLockGuard {
            path: temp_path.to_path_buf(),
            lock,
            _ephemeral: Some(temp_path),
        })
    }

    /// The path to hand to the package builder as `CATALOG_LOCKFILE`:
    /// relative to the project directory (make's `--directory`) for the
    /// committed lock, absolute and whitespace-free for an ephemeral one.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The lock itself, to project the subset a publish submits out of.
    pub fn build_lock(&self) -> &BuildLock {
        &self.lock
    }

    /// Whether this is the committed `.flox/catalog.lock` rather than an
    /// ephemeral lock, e.g. to select stale-lock messaging.
    pub fn is_existing(&self) -> bool {
        self._ephemeral.is_none()
    }
}

#[cfg(test)]
pub mod test_helpers {
    use super::*;

    /// Construct a [BuildLockGuard] from parts, for tests that need to
    /// exercise consumers (e.g. publish's stale-lock messaging) without a
    /// scan or a catalog round-trip.
    pub fn build_lock_guard_from_parts(
        path: impl Into<PathBuf>,
        lock: BuildLock,
        committed: bool,
    ) -> BuildLockGuard {
        BuildLockGuard {
            path: path.into(),
            lock,
            _ephemeral: match committed {
                true => None,
                false => Some(
                    tempfile::NamedTempFile::new()
                        .expect("temp file for test lock")
                        .into_temp_path(),
                ),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use floxhub_client::client::test_helpers::new_noop;
    use nef_lock_catalog::scan_package;
    use tempfile::tempdir;

    use super::*;

    /// A committed lock with one canonical entry, plus an expression that
    /// references it.
    const COMMITTED_LOCK: &str = r#"{
  "version": 1,
  "direct_catalog_inputs": {
    "myorg/hello": {
      "attr_path": ["hello"],
      "build_type": "nef",
      "catalog": "myorg",
      "locked_inputs_hash": "sha256-test",
      "source": {
        "dir": ".",
        "ref": "refs/heads/main",
        "rev": "0000000000000000000000000000000000000000",
        "type": "git",
        "url": "https://example.com/repo"
      }
    }
  },
  "catalogs": {}
}
"#;

    fn project_with_expression(expression: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let project = tempdir().unwrap();
        let dot_flox = project.path().join(".flox");
        let pkgs_dir = nix_expression_dir_in(&dot_flox);
        std::fs::create_dir_all(&pkgs_dir).unwrap();
        std::fs::write(pkgs_dir.join("hello.nix"), expression).unwrap();
        (project, dot_flox, pkgs_dir)
    }

    /// A project whose expressions make no catalog references resolves an
    /// empty ephemeral lock without any catalog request: the no-op client
    /// fails every request it is asked to make, so reaching the network at
    /// all fails this test.
    #[tokio::test]
    async fn no_references_resolve_without_a_catalog_request() {
        let (_project, dot_flox, _pkgs_dir) =
            project_with_expression("{ runCommand }: runCommand \"hello\" { } \"\"");
        let lock = BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"])
            .await
            .unwrap();

        assert!(!lock.is_existing());
        assert!(
            !lock.path().to_string_lossy().contains(char::is_whitespace),
            "an ephemeral lock path must be whitespace-free: {}",
            lock.path().display()
        );
        assert_eq!(
            std::fs::read_to_string(lock.path()).unwrap(),
            "{\n  \"version\": 1,\n  \"direct_catalog_inputs\": {},\n  \"catalogs\": {}\n}\n"
        );
    }

    /// A committed lock is consumed exactly as found: no catalog request
    /// (no-op client), no rewrite (byte-identical file), and the subset
    /// selects the committed entry by the scanned reference.
    #[tokio::test]
    async fn committed_lock_is_consumed_as_found_without_a_catalog_request() {
        let (_project, dot_flox, pkgs_dir) =
            project_with_expression("{ catalogs }: catalogs.myorg.hello");
        std::fs::write(catalog_lockfile_path(&dot_flox), COMMITTED_LOCK).unwrap();
        let lock = BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"])
            .await
            .unwrap();

        assert!(lock.is_existing());
        assert_eq!(lock.path(), Path::new(".flox").join(CATALOG_LOCKFILE_NAME));
        assert_eq!(
            std::fs::read_to_string(catalog_lockfile_path(&dot_flox)).unwrap(),
            COMMITTED_LOCK,
            "the committed lock must not be rewritten"
        );

        let references = scan_package(&pkgs_dir, "hello.nix").unwrap();
        let subset = lock.build_lock().subset_direct(&references).unwrap();
        assert_eq!(subset.keys().collect::<Vec<_>>(), vec![
            &"myorg/hello".to_string()
        ]);
    }

    /// A committed lock that does not cover a scanned reference is still
    /// consumed as found; the staleness surfaces from the subset, naming
    /// the uncovered reference.
    #[tokio::test]
    async fn stale_committed_lock_names_the_uncovered_reference() {
        let (_project, dot_flox, pkgs_dir) =
            project_with_expression("{ catalogs }: catalogs.myorg.world");
        std::fs::write(catalog_lockfile_path(&dot_flox), COMMITTED_LOCK).unwrap();
        let lock = BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"])
            .await
            .unwrap();
        assert!(lock.is_existing());

        let references = scan_package(&pkgs_dir, "hello.nix").unwrap();
        let err = lock
            .build_lock()
            .subset_direct(&references)
            .expect_err("an uncovered reference must be stale");
        assert!(
            err.to_string().contains("myorg.world"),
            "the uncovered reference must be named, got: {err}"
        );
    }
}
