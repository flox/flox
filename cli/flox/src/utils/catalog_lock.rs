//! The catalog lock a build consumes, and its lifetime.
//!
//! Lock *resolution* belongs to `nef-lock-catalog`; what lives here is the
//! CLI-level decision of which lock a given invocation builds against, and
//! the ownership of a temporary builder-facing file. Without a committed
//! `.flox/catalog.lock` the project builds locklessly: the CLI resolves a
//! fresh lock. Both paths materialize the derived catalog tree into a temp
//! file that lives exactly as long as the build.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use flox_rust_sdk::providers::build::nix_expression_dir_in;
use floxhub_client::CatalogClientTrait;
use nef_lock_catalog::{
    BuildLock,
    catalog_lockfile_path,
    read_lock,
    render_builder_lock,
    resolve_lock,
    scan_references,
};
use tracing::debug;

/// The lock a build consumes, created before the package builder is
/// invoked and handed to it as `CATALOG_LOCKFILE`. Owns the builder-facing
/// file, written by [Self::write_builder_file]: dropping the guard removes
/// it, unless diagnostic retention asked for it to be kept.
#[derive(Debug)]
pub struct BuildLockGuard {
    lock: BuildLock,
    /// The builder-facing file, once written.
    ephemeral: Option<tempfile::TempPath>,
    /// Leave the builder-facing file in place on drop, with its path logged.
    retain: bool,
    committed: bool,
}

impl BuildLockGuard {
    /// The lock for a build over the given expressions, or `None` when none
    /// are involved. Without expressions nothing consumes a catalog lock, so
    /// an existing file is neither read nor validated.
    pub async fn for_expression_builds(
        client: &impl CatalogClientTrait,
        dot_flox_path: impl AsRef<Path>,
        rel_file_paths: &[PathBuf],
        retain: bool,
    ) -> Result<Option<BuildLockGuard>> {
        if rel_file_paths.is_empty() {
            return Ok(None);
        }
        Self::new_existing_or_ephemeral(client, dot_flox_path, rel_file_paths, retain)
            .await
            .map(Some)
    }

    /// Read the committed lock if present; otherwise resolve the scanned
    /// references. The builder-facing file outlives the guard when `retain`
    /// is set.
    pub async fn new_existing_or_ephemeral(
        client: &impl CatalogClientTrait,
        dot_flox_path: impl AsRef<Path>,
        rel_file_paths: impl IntoIterator<Item = impl AsRef<Path>>,
        retain: bool,
    ) -> Result<BuildLockGuard> {
        let dot_flox_path = dot_flox_path.as_ref();
        let committed = catalog_lockfile_path(dot_flox_path);
        let is_existing = committed.exists();
        let lock = if is_existing {
            read_lock(&committed)?
        } else {
            let references = scan_references(nix_expression_dir_in(dot_flox_path), rel_file_paths)?;
            resolve_lock(client, references).await?
        };
        Ok(BuildLockGuard {
            lock,
            ephemeral: None,
            retain,
            committed: is_existing,
        })
    }

    /// Materialize the lock into the builder-facing file and return its
    /// path, to hand to the package builder as `CATALOG_LOCKFILE`: an
    /// absolute, whitespace-free temporary path. Writing again replaces the
    /// file's contents.
    pub fn write_builder_file(&mut self) -> Result<&Path> {
        let temp_path = match self.ephemeral.take() {
            Some(temp_path) => temp_path,
            // The system temp dir, not flox's own temp dir: flox's derives
            // from `$HOME`, which the user may have placed at a path
            // containing whitespace, and the ephemeral path reaches make's
            // word-splitting positions. The system temp dir shares the
            // whitespace-free assumption the makefile's own PROJECT_TMPDIR
            // (`$(TMPDIR)/<hash>`) already makes. This deliberately sits
            // outside flox's centralized per-process temp cleanup; the
            // guard's drop removes the file instead, or keeps it when
            // diagnostic retention is enabled.
            None => tempfile::Builder::new()
                .prefix("flox-catalog.lock.")
                .tempfile()
                .context("Could not create a temporary file for the catalog lock.")?
                .into_temp_path(),
        };
        let temp_path = self.ephemeral.insert(temp_path);
        std::fs::write(&*temp_path, render_builder_lock(&self.lock)?)
            .context("Could not write the temporary builder catalog lock")?;
        debug!(path = %temp_path.display(), committed = self.committed, "build consumes a materialized catalog lock");
        Ok(temp_path)
    }

    /// The lock itself, to project the subset a publish submits out of.
    pub fn build_lock(&self) -> &BuildLock {
        &self.lock
    }

    /// Whether the in-memory lock came from the committed file.
    pub fn is_existing(&self) -> bool {
        self.committed
    }

    /// The lock, to change before the builder-facing file is written, such
    /// as by `--override-input`. The committed lock is not changed.
    pub fn build_lock_mut(&mut self) -> &mut BuildLock {
        &mut self.lock
    }
}

impl Drop for BuildLockGuard {
    fn drop(&mut self) {
        if !self.retain {
            return;
        }
        let Some(ephemeral) = self.ephemeral.take() else {
            return;
        };
        match ephemeral.keep() {
            Ok(path) => {
                debug!(path = %path.display(), "leaving materialized catalog lock in place")
            },
            Err(err) => debug!(%err, "could not keep materialized catalog lock"),
        }
    }
}

#[cfg(test)]
pub mod test_helpers {
    use super::*;

    /// Construct a [BuildLockGuard] from parts, for tests that need to
    /// exercise consumers (e.g. publish's stale-lock messaging) without a
    /// scan or a catalog round-trip.
    pub fn build_lock_guard_from_parts(lock: BuildLock, committed: bool) -> BuildLockGuard {
        BuildLockGuard {
            lock,
            ephemeral: None,
            retain: false,
            committed,
        }
    }
}

#[cfg(test)]
mod tests {
    use floxhub_client::client::test_helpers::new_noop;
    use nef_lock_catalog::{UPDATE_CATALOGS_COMMAND, scan_package};
    use tempfile::tempdir;

    use super::*;

    /// A committed lock with one canonical entry, plus an expression that
    /// references it.
    const COMMITTED_LOCK: &str = r#"{
  "version": 2,
  "locked_inputs": {
    "myorg/hello": {
      "attr_path": ["hello"],
      "build_type": "nef",
      "catalog": "myorg",
      "inputs": [],
      "locked_inputs_hash": "sha256-test",
      "version": null,
      "build": null,
      "source": {
        "dir": ".",
        "ref": "refs/heads/main",
        "rev": "0000000000000000000000000000000000000000",
        "type": "git",
        "url": "https://example.com/repo"
      }
    }
  },
  "direct_inputs": ["myorg/hello"]
}
"#;

    const COMMITTED_V1_LOCK: &str = r#"{
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
        let lock =
            BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"], false)
                .await
                .unwrap();

        assert!(!lock.is_existing());
        assert!(lock.build_lock().locked_inputs().is_empty());
        assert!(lock.build_lock().direct_inputs().is_empty());
    }

    /// A committed lock needs no catalog request and remains byte-identical,
    /// and the subset selects the committed entry by the scanned reference.
    #[tokio::test]
    async fn committed_lock_is_consumed_as_found_without_a_catalog_request() {
        let (_project, dot_flox, pkgs_dir) =
            project_with_expression("{ catalogs }: catalogs.myorg.hello");
        std::fs::write(catalog_lockfile_path(&dot_flox), COMMITTED_LOCK).unwrap();
        let lock =
            BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"], false)
                .await
                .unwrap();

        assert!(lock.is_existing());
        assert_eq!(
            std::fs::read_to_string(catalog_lockfile_path(&dot_flox)).unwrap(),
            COMMITTED_LOCK,
            "the committed lock must not be rewritten"
        );

        let references = scan_package(&pkgs_dir, "hello.nix").unwrap();
        let closure = lock.build_lock().project_package(&references).unwrap();
        assert_eq!(closure.direct_inputs, vec!["myorg/hello".to_string()]);
    }

    #[tokio::test]
    async fn committed_lock_projects_base_references_without_a_base_root() {
        let (_project, dot_flox, pkgs_dir) =
            project_with_expression("{ catalogs }: catalogs.nixpkgs.writeText \"base\" \"hello\"");
        std::fs::write(catalog_lockfile_path(&dot_flox), COMMITTED_LOCK).unwrap();
        let lock =
            BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"], false)
                .await
                .unwrap();
        let references = scan_package(&pkgs_dir, "hello.nix").unwrap();
        let closure = lock.build_lock().project_package(&references).unwrap();

        // Both check-build and publish serialize these closure fields.
        assert!(closure.direct_inputs.is_empty());
        assert!(closure.locked_inputs.is_empty());

        std::fs::write(
            pkgs_dir.join("hello.nix"),
            "{ catalogs }: [ catalogs.nixpkgs.writeText catalogs.myorg.hello ]",
        )
        .unwrap();
        let references = scan_package(&pkgs_dir, "hello.nix").unwrap();
        let mixed = lock.build_lock().project_package(&references).unwrap();
        assert_eq!(mixed.direct_inputs, vec!["myorg/hello".to_string()]);
        assert_eq!(mixed.locked_inputs.len(), 1);
    }

    #[tokio::test]
    async fn committed_v1_lock_is_refused() {
        let (_project, dot_flox, _pkgs_dir) =
            project_with_expression("{ catalogs }: catalogs.myorg.hello");
        std::fs::write(catalog_lockfile_path(&dot_flox), COMMITTED_V1_LOCK).unwrap();

        let err =
            BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"], false)
                .await
                .expect_err("a v1 lock must be refused, not read as v2");
        let message = format!("{err:#}");
        assert!(
            message.contains(UPDATE_CATALOGS_COMMAND),
            "the refusal must name the relock command, got: {message}"
        );
    }

    /// A committed lock that does not cover a scanned reference is still
    /// consumed as found; the staleness surfaces from the subset, naming
    /// the uncovered reference.
    #[tokio::test]
    async fn stale_committed_lock_names_the_uncovered_reference() {
        let (_project, dot_flox, pkgs_dir) =
            project_with_expression("{ catalogs }: catalogs.myorg.world");
        std::fs::write(catalog_lockfile_path(&dot_flox), COMMITTED_LOCK).unwrap();
        let lock =
            BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"], false)
                .await
                .unwrap();
        assert!(lock.is_existing());

        let references = scan_package(&pkgs_dir, "hello.nix").unwrap();
        let err = lock
            .build_lock()
            .project_package(&references)
            .expect_err("an uncovered reference must be stale");
        assert!(
            err.to_string().contains("myorg.world"),
            "the uncovered reference must be named, got: {err}"
        );
    }

    /// A pure manifest build consumes no catalog lock, so a v1 or malformed
    /// committed file is neither read nor rewritten, and no catalog request
    /// is made: the no-op client fails every request.
    #[tokio::test]
    async fn no_expression_builds_leave_an_unused_lock_untouched() {
        let (_project, dot_flox, _pkgs_dir) =
            project_with_expression("{ catalogs }: catalogs.myorg.hello");
        let lock_path = catalog_lockfile_path(&dot_flox);
        for unused in [COMMITTED_V1_LOCK, "not json"] {
            std::fs::write(&lock_path, unused).unwrap();
            let lock = BuildLockGuard::for_expression_builds(&new_noop(), &dot_flox, &[], false)
                .await
                .unwrap();
            assert!(lock.is_none());
            assert_eq!(std::fs::read_to_string(&lock_path).unwrap(), unused);
        }
    }

    /// An expression build still reads the committed lock, and refuses a v1
    /// one with the relock command.
    #[tokio::test]
    async fn expression_builds_still_refuse_a_v1_lock() {
        let (_project, dot_flox, _pkgs_dir) =
            project_with_expression("{ catalogs }: catalogs.myorg.hello");
        std::fs::write(catalog_lockfile_path(&dot_flox), COMMITTED_V1_LOCK).unwrap();
        let err = BuildLockGuard::for_expression_builds(
            &new_noop(),
            &dot_flox,
            &[PathBuf::from("hello.nix")],
            false,
        )
        .await
        .expect_err("a v1 lock must be refused where expressions read it");
        assert!(format!("{err:#}").contains(UPDATE_CATALOGS_COMMAND));
    }

    /// The builder file holds the rendered lock at a whitespace-free path
    /// apart from the committed lock. With retention it survives the guard
    /// with its content intact, and the committed lock is untouched; without
    /// it the file goes.
    #[tokio::test]
    async fn builder_file_holds_the_rendered_lock_for_the_guards_life() {
        let (_project, dot_flox, _pkgs_dir) =
            project_with_expression("{ catalogs }: catalogs.myorg.hello");
        std::fs::write(catalog_lockfile_path(&dot_flox), COMMITTED_LOCK).unwrap();
        for retain in [false, true] {
            let mut lock = BuildLockGuard::new_existing_or_ephemeral(
                &new_noop(),
                &dot_flox,
                ["hello.nix"],
                retain,
            )
            .await
            .unwrap();
            let builder_path = lock.write_builder_file().unwrap().to_path_buf();
            assert_ne!(builder_path, catalog_lockfile_path(&dot_flox));
            assert!(
                !builder_path.to_string_lossy().contains(char::is_whitespace),
                "a builder file path must be whitespace-free: {}",
                builder_path.display()
            );
            let builder_content = std::fs::read_to_string(&builder_path).unwrap();
            assert_eq!(
                builder_content,
                render_builder_lock(lock.build_lock()).unwrap()
            );
            drop(lock);

            assert_eq!(builder_path.exists(), retain);
            if retain {
                assert_eq!(
                    std::fs::read_to_string(&builder_path).unwrap(),
                    builder_content
                );
                std::fs::remove_file(&builder_path).unwrap();
            }
            assert_eq!(
                std::fs::read_to_string(catalog_lockfile_path(&dot_flox)).unwrap(),
                COMMITTED_LOCK
            );
        }
    }
}
