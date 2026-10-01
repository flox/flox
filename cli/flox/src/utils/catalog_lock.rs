//! The catalog lock a build consumes, and its lifetime.
//!
//! Lock *resolution* belongs to `nef-lock-catalog`; what lives here is the
//! CLI-level decision of which lock a given invocation builds against, and
//! the ownership of an ephemeral lock's file. Without a committed
//! `.flox/catalog.lock` the project builds locklessly: the CLI resolves a
//! fresh lock into a temp file that lives exactly as long as the build, and
//! nothing is ever written into the project tree. Input overrides work the
//! same way: whichever lock the invocation starts from, the copy with the
//! overrides applied is ephemeral and the committed file is left as found.

use std::collections::BTreeSet;
use std::fmt::Display;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use flox_rust_sdk::models::environment::DOT_FLOX;
use flox_rust_sdk::providers::build::nix_expression_dir_in;
use flox_rust_sdk::providers::git::{GitCommandProvider, GitProvider};
use floxhub_client::CatalogClientTrait;
use indoc::formatdoc;
use nef_lock_catalog::{
    BuildLock,
    CATALOG_LOCKFILE_NAME,
    CatalogRef,
    RawNixFlakerefAttrs,
    catalog_lockfile_path,
    lock_flakeref,
    read_lock,
    resolve_lock,
    scan_references,
    write_lock,
};
use tracing::debug;
use url::{Url, form_urlencoded};

use crate::utils::errors::display_chain;

/// A `--override-input <REFERENCE>=<FLAKEREF>` value: fetch the catalog
/// input `reference` from `flakeref` for this invocation instead of the
/// source the lock pins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputOverride {
    /// The input as the expressions name it, e.g. `catalogs.myorg.hello`.
    pub reference: CatalogRef,
    /// The flakeref nix is to lock; a path the user gave has become the
    /// flakeref it stands for (see [path_flakeref]).
    pub flakeref: Url,
}

impl InputOverride {
    /// The locked source to fetch the input from: the override's flakeref
    /// as `nix flake prefetch` locks it, pointing at the `.flox` of the
    /// project it names.
    fn lock_source(&self) -> Result<RawNixFlakerefAttrs> {
        lock_flakeref(&self.flakeref, DOT_FLOX).map_err(|err| {
            anyhow!(formatdoc! {"
                Could not fetch {self}.
                {err}", err = display_chain(&err)})
        })
    }
}

impl FromStr for InputOverride {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let Some((reference, source)) = s.split_once('=') else {
            bail!("'{s}' is not of the form '<REFERENCE>=<FLAKEREF>'.");
        };
        if source.is_empty() {
            bail!("'{s}' names no flakeref after '='.");
        }
        let reference = reference.parse().with_context(|| {
            format!("'{reference}' is not a catalog reference like 'catalogs.<catalog>.<package>'.")
        })?;
        // Anything with a URL scheme (`git+file:`, `github:`, …) is a
        // flakeref for nix to judge; anything else is a path.
        let flakeref = match Url::parse(source) {
            Ok(flakeref) => flakeref,
            Err(_) => path_flakeref(source)?,
        };
        Ok(InputOverride {
            reference,
            flakeref,
        })
    }
}

impl Display for InputOverride {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "'{}' from '{}'", self.reference, self.flakeref)
    }
}

/// The flakeref a path stands for, as nix itself reads one.
///
/// Within a git repository that is the repository, with `dir` pointing at
/// the path: only tracked files are fetched, uncommitted changes to them
/// included, as for the expressions of the project being built. Elsewhere
/// it is a `path:` flakeref, which copies the directory whole into the
/// store.
///
/// The path is canonical — nix refuses a `path:` input that is relative or
/// passes through a symlink (`/tmp` on macOS is one) — and percent-encoded,
/// so a space, `?` or `#` in it survives as part of the path.
fn path_flakeref(path: &str) -> Result<Url> {
    let path = Path::new(path)
        .canonicalize()
        .map_err(|err| match err.kind() {
            ErrorKind::NotFound => anyhow!("'{path}' does not exist."),
            _ => anyhow!("Could not read '{path}': {err}"),
        })?;
    let file_url = |path: &Path| {
        Url::from_file_path(path)
            .map_err(|()| anyhow!("'{}' is not an absolute path.", path.display()))
    };

    let repository = GitCommandProvider::discover(&path).ok();
    let in_repository = repository.as_ref().and_then(|git| {
        let workdir = git.workdir()?;
        Some((workdir, path.strip_prefix(workdir).ok()?))
    });
    let Some((workdir, dir)) = in_repository else {
        return Ok(Url::parse(&format!("path:{}", file_url(&path)?.path()))?);
    };

    let mut flakeref = Url::parse(&format!("git+{}", file_url(workdir)?))?;
    if !dir.as_os_str().is_empty() {
        // Form encoding writes a space as `+`, which nix would read
        // literally; a literal `+` is itself encoded, so every `+` left is
        // a space.
        let dir: String =
            form_urlencoded::byte_serialize(dir.as_os_str().as_encoded_bytes()).collect();
        flakeref.set_query(Some(&format!("dir={}", dir.replace('+', "%20"))));
    }
    Ok(flakeref)
}

/// Check every override against `lock` before any source is fetched, so
/// that a mistyped reference fails without a download.
fn ensure_overridable(lock: &BuildLock, overrides: &[InputOverride]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for InputOverride { reference, .. } in overrides {
        if !seen.insert(reference) {
            bail!(formatdoc! {"
                '{reference}' is overridden more than once.
                Pass a single '--override-input' for each catalog input."});
        }
        lock.ensure_pins(reference)?;
    }
    Ok(())
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
}

impl BuildLockGuard {
    /// The committed `.flox/catalog.lock` exactly as found when one exists;
    /// otherwise a fresh ephemeral lock resolving the union of the
    /// references of the expressions named by `rel_file_paths` (relative to
    /// the project's expression directory), written to a randomly named
    /// temp file that is removed when the returned value is dropped.
    ///
    /// With `overrides`, the lock the invocation starts from — committed or
    /// freshly resolved — has each override's flakeref locked and put in
    /// place of the pinned source, and is always written to an ephemeral
    /// file; the committed lock is never rewritten. An override the lock
    /// cannot take fails before any is fetched.
    pub async fn new_existing_or_ephemeral(
        client: &impl CatalogClientTrait,
        dot_flox_path: impl AsRef<Path>,
        rel_file_paths: impl IntoIterator<Item = impl AsRef<Path>>,
        overrides: &[InputOverride],
    ) -> Result<BuildLockGuard> {
        let dot_flox_path = dot_flox_path.as_ref();
        let committed = catalog_lockfile_path(dot_flox_path);
        let committed_lock = committed
            .exists()
            .then(|| read_lock(&committed))
            .transpose()?;
        let mut lock = match committed_lock {
            Some(lock) if overrides.is_empty() => {
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
                });
            },
            Some(lock) => {
                debug!(path = %committed.display(), "build starts from the committed catalog lock");
                lock
            },
            None => {
                let references =
                    scan_references(nix_expression_dir_in(dot_flox_path), rel_file_paths)?;
                resolve_lock(client, references).await?
            },
        };

        ensure_overridable(&lock, overrides)?;
        for input_override in overrides {
            lock.override_input(&input_override.reference, input_override.lock_source()?)?;
        }
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
    use indoc::indoc;
    use nef_lock_catalog::{OverrideInputError, scan_package};
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

    fn hello_from(source: impl Display) -> Result<InputOverride> {
        format!("catalogs.myorg.hello={source}").parse()
    }

    fn hello_override(flakeref: &str) -> InputOverride {
        InputOverride {
            reference: "catalogs.myorg.hello".parse().unwrap(),
            flakeref: Url::parse(flakeref).unwrap(),
        }
    }

    /// Outside a git repository a path is a `path:` flakeref of its
    /// canonical form, with the characters a URL gives meaning to encoded.
    /// A `:` after a `/` does not make the value a URL.
    #[test]
    fn path_outside_a_git_repository_is_a_path_flakeref() {
        let dir = tempdir().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        let path = dir.path().join("a:b c#d");
        std::fs::create_dir(&path).unwrap();

        assert_eq!(
            hello_from(path.display()).unwrap(),
            hello_override(&format!("path:{}/a:b%20c%23d", canonical.display()))
        );
    }

    /// Inside a git repository a path is the repository itself, with `dir`
    /// naming where in it the path lies.
    #[test]
    fn path_inside_a_git_repository_is_a_git_flakeref() {
        let dir = tempdir().unwrap();
        let canonical = dir.path().canonicalize().unwrap();
        GitCommandProvider::init(dir.path(), false).unwrap();
        let subdir = dir.path().join("sub dir").join("a+b");
        std::fs::create_dir_all(&subdir).unwrap();

        assert_eq!(
            hello_from(dir.path().display()).unwrap(),
            hello_override(&format!("git+file://{}", canonical.display()))
        );
        assert_eq!(
            hello_from(subdir.display()).unwrap(),
            hello_override(&format!(
                "git+file://{}?dir=sub%20dir%2Fa%2Bb",
                canonical.display()
            ))
        );
    }

    /// A value with a URL scheme is nix's to judge, whether or not it names
    /// anything that exists.
    #[test]
    fn value_with_a_url_scheme_is_taken_as_a_flakeref() {
        for flakeref in [
            "git+file:///src/hello?dir=sub&ref=main",
            "github:myorg/hello/my-branch",
            "flake:nixpkgs",
        ] {
            assert_eq!(hello_from(flakeref).unwrap(), hello_override(flakeref));
        }
    }

    #[test]
    fn malformed_input_override_says_what_is_wrong() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("missing");

        let message = |value: String| value.parse::<InputOverride>().unwrap_err().to_string();

        assert_eq!(
            message("catalogs.myorg.hello".to_string()),
            "'catalogs.myorg.hello' is not of the form '<REFERENCE>=<FLAKEREF>'."
        );
        assert_eq!(
            message("catalogs.myorg.hello=".to_string()),
            "'catalogs.myorg.hello=' names no flakeref after '='."
        );
        assert_eq!(
            message(format!("myorg={}", dir.path().display())),
            "'myorg' is not a catalog reference like 'catalogs.<catalog>.<package>'."
        );
        assert_eq!(
            message(format!("catalogs.myorg.hello={}", missing.display())),
            format!("'{}' does not exist.", missing.display())
        );
    }

    fn project_with_committed_lock() -> (tempfile::TempDir, PathBuf) {
        let (project, dot_flox, _pkgs_dir) =
            project_with_expression("{ catalogs }: catalogs.myorg.hello");
        std::fs::write(catalog_lockfile_path(&dot_flox), COMMITTED_LOCK).unwrap();
        (project, dot_flox)
    }

    /// A committed lock with an override is never rewritten: the build
    /// consumes an ephemeral copy pinning the override's locked source, and
    /// the committed file stays byte-identical.
    #[tokio::test]
    async fn committed_lock_with_an_override_is_consumed_from_an_ephemeral_copy() {
        let (_project, dot_flox) = project_with_committed_lock();
        let override_dir = tempdir().unwrap();
        let override_path = override_dir.path().canonicalize().unwrap();

        let lock =
            BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"], &[
                hello_from(override_dir.path().display()).unwrap(),
            ])
            .await
            .unwrap();

        assert!(!lock.is_existing());
        assert_eq!(
            std::fs::read_to_string(catalog_lockfile_path(&dot_flox)).unwrap(),
            COMMITTED_LOCK,
            "the committed lock must not be rewritten"
        );
        let mut ephemeral: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(lock.path()).unwrap()).unwrap();
        let source = ephemeral["catalogs"]["myorg"]["packages"]["entries"]["hello"]["source"]
            .as_object_mut()
            .unwrap();
        // What the tree is pinned to is nix's to say.
        assert!(source.remove("narHash").is_some());
        assert!(source.remove("lastModified").is_some());
        assert_eq!(
            serde_json::Value::Object(source.clone()),
            serde_json::json!({
                "type": "path",
                "path": override_path,
                "dir": ".flox",
            })
        );
    }

    /// An override of a reference the lock does not pin fails by name,
    /// before anything is fetched: the flakeref here could not be.
    #[tokio::test]
    async fn override_of_an_unpinned_reference_fails_before_any_fetch() {
        let (_project, dot_flox) = project_with_committed_lock();
        let overrides = [
            hello_override("path:/does/not/exist"),
            "catalogs.myorg.missing=path:/does/not/exist"
                .parse()
                .unwrap(),
        ];

        let err = BuildLockGuard::new_existing_or_ephemeral(
            &new_noop(),
            &dot_flox,
            ["hello.nix"],
            &overrides,
        )
        .await
        .expect_err("the lock does not pin the reference");

        assert_eq!(
            err.downcast_ref::<OverrideInputError>(),
            Some(&OverrideInputError::NotPinned {
                reference: "catalogs.myorg.missing".parse().unwrap(),
                pinned: vec!["catalogs.myorg.hello".parse().unwrap()],
            })
        );
    }

    #[tokio::test]
    async fn overriding_a_reference_twice_is_refused() {
        let (_project, dot_flox) = project_with_committed_lock();
        let overrides = [
            hello_override("path:/src/one"),
            hello_override("path:/src/other"),
        ];

        let err = BuildLockGuard::new_existing_or_ephemeral(
            &new_noop(),
            &dot_flox,
            ["hello.nix"],
            &overrides,
        )
        .await
        .expect_err("one input cannot come from two sources");

        assert_eq!(err.to_string(), indoc! {"
            'catalogs.myorg.hello' is overridden more than once.
            Pass a single '--override-input' for each catalog input."});
    }

    /// A source nix cannot lock fails naming the override, with nix's own
    /// words introduced as such.
    #[tokio::test]
    async fn override_nix_cannot_lock_relays_the_nix_error() {
        let (_project, dot_flox) = project_with_committed_lock();

        let err =
            BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"], &[
                hello_override("path:/does/not/exist"),
            ])
            .await
            .expect_err("nix cannot fetch a path that does not exist");

        let introduction = indoc! {"
            Could not fetch 'catalogs.myorg.hello' from 'path:/does/not/exist'.
            Caught Nix error while locking the flakeref:
            error:"};
        let message = err.to_string();
        assert!(
            message.starts_with(introduction),
            "unexpected message: {message}"
        );
    }

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
            BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"], &[])
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
        let lock =
            BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"], &[])
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
        let lock =
            BuildLockGuard::new_existing_or_ephemeral(&new_noop(), &dot_flox, ["hello.nix"], &[])
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
