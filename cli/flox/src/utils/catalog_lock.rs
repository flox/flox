//! The catalog lock a build consumes, and its lifetime.
//!
//! Lock *resolution* belongs to `nef-lock-catalog`; what lives here is the
//! CLI-level decision of which lock a given invocation builds against, and
//! the ownership of an ephemeral lock's file. Without a committed
//! `.flox/catalog.lock` the project builds locklessly: the CLI resolves a
//! fresh lock into a temp file that lives exactly as long as the build, and
//! nothing is ever written into the project tree. Input overrides are
//! applied the same way: whichever lock the invocation starts from, the
//! overridden copy is ephemeral and the committed file is left as found.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use flox_rust_sdk::providers::build::nix_expression_dir_in;
use floxhub_client::CatalogClientTrait;
use nef_lock_catalog::{
    BuildLock,
    CATALOG_LOCKFILE_NAME,
    InputKey,
    InputOverride,
    NixFlakeref,
    RawNixFlakerefAttrs,
    catalog_lockfile_path,
    read_lock,
    resolve_lock,
    scan_references,
    write_lock,
};
use tracing::debug;
use url::Url;

/// An input override as given on the command line, with its flakeref
/// parsed by nix so that what reaches the lock is exactly the attribute
/// set the NEF will fetch.
#[derive(Debug, Clone)]
pub struct ResolvedOverride {
    key: InputKey,
    flakeref: NixFlakeref,
}

impl ResolvedOverride {
    /// Parse `flakeref` for the input `key`. A bare or `path:` path is
    /// canonicalized against the working directory first: nix refuses a
    /// relative path in `fetchTree`, and one through a symlink (`/tmp` on
    /// macOS), and the eval that fetches runs from the project directory,
    /// not the user's.
    pub fn new(key: InputKey, flakeref: &str) -> Result<Self> {
        let cwd = std::env::current_dir().context("Could not determine the working directory.")?;
        let canonical = canonicalize_flakeref_path(flakeref, &cwd)?;
        let flakeref = NixFlakeref::try_from(canonical.as_str())
            .with_context(|| format!("'{flakeref}' is not a flakeref usable for input '{key}'."))?;
        Ok(ResolvedOverride { key, flakeref })
    }

    pub fn key(&self) -> &InputKey {
        &self.key
    }

    /// The flakeref as nix renders it, for messages.
    pub fn url(&self) -> &Url {
        self.flakeref.as_url()
    }

    pub fn into_override(self) -> InputOverride {
        InputOverride {
            key: self.key,
            source: RawNixFlakerefAttrs::new_unchecked(self.flakeref.as_parsed().clone()),
        }
    }
}

/// Whether `value` starts with a URL scheme (`git+file:`, `github:`, …),
/// as opposed to a bare path.
fn has_url_scheme(value: &str) -> bool {
    let Some((scheme, _)) = value.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
}

/// `value` with a bare or `path:` path canonicalized — resolved against
/// `cwd`, symlinks and `..` resolved — which fails if the path does not
/// exist. Any other flakeref is returned as given.
fn canonicalize_flakeref_path(value: &str, cwd: &Path) -> Result<String> {
    let (prefix, rest) = match value.strip_prefix("path:") {
        Some(rest) => ("path:", rest),
        None if has_url_scheme(value) => return Ok(value.to_string()),
        None => ("", value),
    };
    let (path, query) = rest
        .split_once('?')
        .map_or((rest, None), |(path, query)| (path, Some(query)));
    if path.is_empty() {
        return Ok(value.to_string());
    }
    let canonical = cwd
        .join(path)
        .canonicalize()
        .with_context(|| format!("'{path}' does not exist."))?;
    Ok(match query {
        Some(query) => format!("{prefix}{}?{query}", canonical.display()),
        None => format!("{prefix}{}", canonical.display()),
    })
}

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
    /// The input overrides applied to the lock, in the order given.
    overrides: Vec<InputOverride>,
}

impl BuildLockGuard {
    /// The committed `.flox/catalog.lock` exactly as found when one exists;
    /// otherwise a fresh ephemeral lock resolving the union of the
    /// references of the expressions named by `rel_file_paths` (relative to
    /// the project's expression directory), written to a randomly named
    /// temp file that is removed when the returned value is dropped.
    ///
    /// With `overrides`, the lock the invocation starts from — committed or
    /// freshly resolved — has them applied and is always written to an
    /// ephemeral file; the committed lock is never rewritten. An override
    /// naming an input the lock does not pin fails before anything is
    /// written.
    pub async fn new_existing_or_ephemeral(
        client: &impl CatalogClientTrait,
        dot_flox_path: impl AsRef<Path>,
        rel_file_paths: impl IntoIterator<Item = impl AsRef<Path>>,
        overrides: Vec<InputOverride>,
    ) -> Result<BuildLockGuard> {
        let dot_flox_path = dot_flox_path.as_ref();
        let committed = catalog_lockfile_path(dot_flox_path);
        let lock = if committed.exists() {
            let lock = read_lock(&committed)?;
            if overrides.is_empty() {
                // The path handed to make is *relative to the project
                // directory* make is started in (`--directory`), composed
                // of two constant components — so a project path containing
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
                    overrides,
                });
            }
            debug!(path = %committed.display(), "build starts from the committed catalog lock");
            lock
        } else {
            let references = scan_references(nix_expression_dir_in(dot_flox_path), rel_file_paths)?;
            resolve_lock(client, references).await?
        };
        Self::ephemeral(lock, overrides)
    }

    /// `lock` with `overrides` applied, written to a temp file that is
    /// removed when the returned value is dropped.
    fn ephemeral(mut lock: BuildLock, overrides: Vec<InputOverride>) -> Result<BuildLockGuard> {
        lock.override_inputs(overrides.iter().cloned())?;
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
        debug!(
            path = %temp_path.display(),
            overrides = overrides.len(),
            "build consumes an ephemeral catalog lock"
        );
        Ok(BuildLockGuard {
            path: temp_path.to_path_buf(),
            lock,
            _ephemeral: Some(temp_path),
            overrides,
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

    /// The input overrides applied to this lock, in the order given. A
    /// lock with any is never fit to publish: its sources are not what
    /// the catalog resolved.
    pub fn overrides(&self) -> &[InputOverride] {
        &self.overrides
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
            overrides: Vec::new(),
        }
    }

    impl BuildLockGuard {
        /// This guard with `overrides` recorded, for tests of consumers
        /// that refuse an overridden lock.
        pub fn with_overrides(mut self, overrides: Vec<InputOverride>) -> Self {
            self.overrides = overrides;
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use floxhub_client::client::test_helpers::new_noop;
    use nef_lock_catalog::{RawNixFlakerefAttrs, scan_package};
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
  "catalogs": {
    "myorg": {
      "type": "floxhub",
      "packages": {
        "type": "package_set",
        "entries": {
          "hello": {
            "type": "package",
            "build_type": "nef",
            "source": {
              "dir": ".",
              "ref": "refs/heads/main",
              "rev": "0000000000000000000000000000000000000000",
              "type": "git",
              "url": "https://example.com/repo"
            }
          }
        }
      }
    }
  }
}
"#;

    fn path_override(key: &str, path: &str) -> InputOverride {
        InputOverride {
            key: key.parse().unwrap(),
            source: RawNixFlakerefAttrs::new_unchecked(
                serde_json::json!({ "type": "path", "path": path }),
            ),
        }
    }

    fn project_with_expression(expression: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let project = tempdir().unwrap();
        let dot_flox = project.path().join(".flox");
        let pkgs_dir = nix_expression_dir_in(&dot_flox);
        std::fs::create_dir_all(&pkgs_dir).unwrap();
        std::fs::write(pkgs_dir.join("hello.nix"), expression).unwrap();
        (project, dot_flox, pkgs_dir)
    }

    /// Bare and `path:` values are canonicalized: relative to the working
    /// directory, `..` and symlinks resolved. Other flakerefs pass through.
    #[test]
    fn canonicalize_flakeref_path_resolves_bare_and_path_values_only() {
        let root = tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let cwd = root_path.join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(root_path.join("desco")).unwrap();
        std::os::unix::fs::symlink(root_path.join("desco"), root_path.join("link")).unwrap();
        let desco = root_path.join("desco").display().to_string();
        let link = root_path.join("link").display().to_string();

        let cases = [
            ("../desco", desco.clone()),
            ("../link", desco.clone()),
            (link.as_str(), desco.clone()),
            ("path:../desco", format!("path:{desco}")),
            ("path:../desco?dir=.flox", format!("path:{desco}?dir=.flox")),
            (
                "git+file:///abs/desco?dir=.flox",
                "git+file:///abs/desco?dir=.flox".to_string(),
            ),
            (
                "github:org/repo/branch",
                "github:org/repo/branch".to_string(),
            ),
            ("path:", "path:".to_string()),
        ];
        for (given, expected) in cases {
            assert_eq!(
                canonicalize_flakeref_path(given, &cwd).unwrap(),
                expected,
                "for '{given}'"
            );
        }

        let err = canonicalize_flakeref_path("../missing", &cwd).unwrap_err();
        assert!(
            err.to_string().contains("'../missing' does not exist"),
            "got: {err}"
        );
    }

    /// A project whose expressions make no catalog references resolves an
    /// empty ephemeral lock without any catalog request: the no-op client
    /// fails every request it is asked to make, so reaching the network at
    /// all fails this test.
    #[tokio::test]
    async fn no_references_resolve_without_a_catalog_request() {
        let (_project, dot_flox, _pkgs_dir) =
            project_with_expression("{ runCommand }: runCommand \"hello\" { } \"\"");
        let lock = BuildLockGuard::new_existing_or_ephemeral(
            &new_noop(),
            &dot_flox,
            ["hello.nix"],
            vec![],
        )
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
        let lock = BuildLockGuard::new_existing_or_ephemeral(
            &new_noop(),
            &dot_flox,
            ["hello.nix"],
            vec![],
        )
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
        let lock = BuildLockGuard::new_existing_or_ephemeral(
            &new_noop(),
            &dot_flox,
            ["hello.nix"],
            vec![],
        )
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

    /// A committed lock with overrides is never rewritten: the build
    /// consumes an ephemeral copy carrying the overridden source, and the
    /// committed file stays byte-identical.
    #[tokio::test]
    async fn committed_lock_with_overrides_is_consumed_from_an_ephemeral_copy() {
        let (_project, dot_flox, _pkgs_dir) =
            project_with_expression("{ catalogs }: catalogs.myorg.hello");
        std::fs::write(catalog_lockfile_path(&dot_flox), COMMITTED_LOCK).unwrap();

        let lock =
            BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"], vec![
                path_override("myorg/hello", "/src/hello"),
            ])
            .await
            .unwrap();

        assert!(!lock.is_existing());
        assert_ne!(lock.path(), Path::new(".flox").join(CATALOG_LOCKFILE_NAME));
        assert_eq!(lock.overrides().len(), 1);
        assert_eq!(
            std::fs::read_to_string(catalog_lockfile_path(&dot_flox)).unwrap(),
            COMMITTED_LOCK,
            "the committed lock must not be rewritten"
        );
        let ephemeral: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(lock.path()).unwrap()).unwrap();
        assert_eq!(
            ephemeral["catalogs"]["myorg"]["packages"]["entries"]["hello"]["source"],
            serde_json::json!({ "type": "path", "path": "/src/hello", "dir": "." })
        );
    }

    /// An override naming an input the lock does not pin fails by name
    /// before any lock is written.
    #[tokio::test]
    async fn override_of_an_unknown_input_fails_by_name() {
        let (_project, dot_flox, _pkgs_dir) =
            project_with_expression("{ catalogs }: catalogs.myorg.hello");
        std::fs::write(catalog_lockfile_path(&dot_flox), COMMITTED_LOCK).unwrap();

        let err =
            BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"], vec![
                path_override("myorg/missing", "/src/missing"),
            ])
            .await
            .expect_err("an unknown input cannot be overridden");

        let message = format!("{err:#}");
        assert!(
            message.contains("myorg/missing") && message.contains("myorg/hello"),
            "the unknown key and the available inputs must be named, got: {message}"
        );
    }
}
