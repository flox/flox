use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::nix;
use crate::nix::nix_base_command;

#[derive(Debug, Clone)]
pub struct NixFlakeref {
    url: Url,
    parsed: Value,
}

impl TryFrom<&str> for NixFlakeref {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> std::result::Result<Self, Self::Error> {
        let expr = format!("builtins.parseFlakeRef \"{value}\"");

        let mut command = nix::nix_base_command();
        command.arg("eval").arg("--json").arg("--expr").arg(expr);

        let output = command
            .output()
            .with_context(|| format!("failed to run '{command:?}')"))?;

        if !output.status.success() {
            return Err(anyhow::Error::msg(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }

        let parsed = Value::from_str(&String::from_utf8(output.stdout)?)
            .context("could not parse nix flakeref structure")?;

        // normalize the url by formatting the parsed struct back as a url
        parsed.try_into()
    }
}
/// Convert the catalog spec into a URL **using Nix' builtin flakeRef formatting**.
/// The Nix cli only accepts `flakeRef`s rather than structural source descriptors.
impl TryFrom<Value> for NixFlakeref {
    type Error = anyhow::Error;

    fn try_from(value: Value) -> std::result::Result<Self, Self::Error> {
        let catalog_json = serde_json::to_string(&value)?;

        let expr = format!(
            "let flakeRef = builtins.fromJSON ''{catalog_json}''; in builtins.flakeRefToString flakeRef"
        );

        let mut command = nix::nix_base_command();
        command.arg("eval").arg("--raw").arg("--expr").arg(expr);

        let output = command
            .output()
            .with_context(|| format!("failed to run '{command:?}')"))?;

        if !output.status.success() {
            return Err(anyhow::Error::msg(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }

        let url = Url::parse(&String::from_utf8(output.stdout)?)
            .context("could not parse nix flakeref")?;

        Ok(NixFlakeref { url, parsed: value })
    }
}

impl NixFlakeref {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        format!("path:{}", path.as_ref().to_string_lossy())
            .as_str()
            .try_into()
    }

    pub fn from_git_with_dir(url: &Url, dir: Option<&Path>) -> Result<Self> {
        let mut map = serde_json::Map::new();
        map.insert("type".into(), json!("git"));
        map.insert("url".into(), json!(url));
        if let Some(d) = dir {
            map.insert("dir".into(), json!(d));
        }
        Value::Object(map).try_into()
    }

    /// Build a `git+file://` flake ref pointing at a local repository at a
    /// specific revision.  Nix resolves this without network access, imports
    /// the tree into the store, so builds can run with full sandbox isolation.
    pub fn from_local_git(path: impl AsRef<Path>, rev: &str, dir: Option<&Path>) -> Result<Self> {
        let url = Url::from_file_path(path.as_ref())
            .map_err(|()| anyhow::anyhow!("path is not absolute: {}", path.as_ref().display()))?;
        let mut map = serde_json::Map::new();
        map.insert("type".into(), json!("git"));
        map.insert("url".into(), json!(url));
        map.insert("rev".into(), json!(rev));
        if let Some(d) = dir {
            map.insert("dir".into(), json!(d));
        }
        Value::Object(map).try_into()
    }

    pub fn as_url(&self) -> &Url {
        &self.url
    }

    /// Get the parsed flake reference as a Value
    pub fn as_parsed(&self) -> &Value {
        &self.parsed
    }
}

/// The raw attribute-set form of a locked nix flakeref (a "source ref"),
/// carried verbatim as JSON.
///
/// Unlike [NixFlakeref], this performs no nix-based parsing or validation: the
/// catalog `/build-inputs/lookup` endpoint returns sources already locked
/// server-side, and this type carries that JSON through unchanged into the
/// build lock, where the NEF feeds it to `builtins.fetchTree`. It is a marker
/// for the "assumed-locked, stored-verbatim" invariant — the source is not
/// re-validated client-side.
///
/// Serialized transparently, so the lockfile shape is just the inner object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RawNixFlakerefAttrs(Value);

impl RawNixFlakerefAttrs {
    /// Wrap an already-locked source value (e.g. a catalog lookup result)
    /// without validating it — the caller asserts it is a locked flakeref.
    pub fn new_unchecked(value: Value) -> Self {
        Self(value)
    }
}

impl From<floxhub_client::LockedGitSource> for RawNixFlakerefAttrs {
    fn from(value: floxhub_client::LockedGitSource) -> Self {
        Self::new_unchecked(serde_json::to_value(value).expect("deserialized from json body"))
    }
}

/// Why [lock_flakeref] could not lock a flakeref.
#[derive(Debug, thiserror::Error)]
pub enum LockFlakerefError {
    #[error("Failed to call 'nix flake prefetch'")]
    CallNix(#[source] std::io::Error),
    /// Nix refused the flakeref or could not fetch it; carries its stderr.
    #[error("Caught Nix error while locking the flakeref:\n{0}")]
    Nix(String),
    #[error("Failed to parse the output of 'nix flake prefetch'")]
    ParseOutput(#[source] serde_json::Error),
}

/// Lock `flakeref` as the source of a NEF project, for a source the catalog
/// did not resolve — one the user names on the command line.
///
/// The flakeref reaches nix as a single argument and is never spliced into
/// an expression, so its text cannot change what nix evaluates.
pub fn lock_flakeref(
    flakeref: &Url,
    nef_base_dir: &str,
) -> Result<RawNixFlakerefAttrs, LockFlakerefError> {
    Ok(nix_prefetch_url(flakeref)?.into_nef_source(nef_base_dir))
}

/// The part of `nix flake prefetch --json` output a lock needs: the locked
/// source attributes and the hash of the tree they were fetched to. The
/// output also carries `original` and `storePath`, which have no consumer.
#[derive(Debug, Clone, Deserialize)]
struct NixPrefetchResult {
    hash: String,
    locked: serde_json::Map<String, Value>,
}

impl NixPrefetchResult {
    /// The locked attribute set the NEF fetches, as it does a
    /// catalog-resolved source.
    ///
    /// `dir` points at the project's `nef_base_dir` (`.flox`) beneath any
    /// `dir` the flakeref itself names: that is where the NEF looks for the
    /// `pkgs/` holding the package expressions, as it is for every source
    /// the catalog pins.
    ///
    /// `narHash` pins the tree that was prefetched. Nix reports none for a
    /// source that names no revision (a `path:`, a dirty git tree), and
    /// without one every fetch during the build would read the live tree
    /// again, possibly a different one each time.
    fn into_nef_source(self, nef_base_dir: &str) -> RawNixFlakerefAttrs {
        let NixPrefetchResult { hash, mut locked } = self;
        let dir = match locked.get("dir").and_then(Value::as_str) {
            Some(prefix) => format!("{prefix}/{nef_base_dir}"),
            None => nef_base_dir.to_string(),
        };
        locked.insert("dir".to_string(), dir.into());
        locked.entry("narHash").or_insert(hash.into());
        RawNixFlakerefAttrs::new_unchecked(Value::Object(locked))
    }
}

/// Lock a flakeref url using `nix flake prefetch`.
/// This resolves the url, downloads the source and reports the locked
/// source attributes along with the hash and store path of the tree.
///
/// Example:
///
/// ```shell
/// $ nix flake prefetch git+ssh://git@github.com/flox/flox --json
/// {
///   "hash": "sha256-LdMMBff1PCXQQl3I5Dvg5U2s4l+7l9lemAncUCjJUY8=",
///   "locked": {
///     "lastModified": 1770220825,
///     "ref": "refs/heads/main",
///     "rev": "a6250c34313d184c5c5be7ad824ad0bbc7610e38",
///     "revCount": 4546,
///     "type": "git",
///     "url": "ssh://git@github.com/flox/flox"
///   },
///   "original": {
///     "type": "git",
///     "url": "ssh://git@github.com/flox/flox"
///   },
///   "storePath": "/nix/store/pihgq0g5vnrzlx2g5lzdn7dh7aqfbl7g-source"
/// }
/// ```
fn nix_prefetch_url(url: &Url) -> Result<NixPrefetchResult, LockFlakerefError> {
    let mut command = nix_base_command();
    command.arg("flake").arg("prefetch").arg("--json");
    // A flakeref that names no revision names something that moves (a
    // branch, a working tree): ask for its current state, not a cached one.
    if !url.query_pairs().any(|(key, _)| key == "rev") {
        command.arg("--refresh");
    }
    command.arg(url.as_str());

    let output = command.output().map_err(LockFlakerefError::CallNix)?;
    if !output.status.success() {
        return Err(LockFlakerefError::Nix(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    serde_json::from_slice(&output.stdout).map_err(LockFlakerefError::ParseOutput)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefetched(locked: Value) -> NixPrefetchResult {
        serde_json::from_value(json!({ "hash": "sha256-prefetched", "locked": locked })).unwrap()
    }

    /// The NEF base dir lies beneath the `dir` the flakeref names, and the
    /// prefetched tree's hash pins a source that reports none of its own.
    #[test]
    fn nef_source_points_at_the_base_dir_and_pins_the_prefetched_tree() {
        let source = prefetched(json!({ "type": "path", "path": "/src/hello" }));
        assert_eq!(
            source.into_nef_source(".flox"),
            RawNixFlakerefAttrs::new_unchecked(json!({
                "type": "path",
                "path": "/src/hello",
                "dir": ".flox",
                "narHash": "sha256-prefetched",
            }))
        );

        let source = prefetched(json!({
            "type": "git",
            "url": "file:///src/monorepo",
            "dir": "sub",
            "narHash": "sha256-reported",
        }));
        assert_eq!(
            source.into_nef_source(".flox"),
            RawNixFlakerefAttrs::new_unchecked(json!({
                "type": "git",
                "url": "file:///src/monorepo",
                "dir": "sub/.flox",
                "narHash": "sha256-reported",
            }))
        );
    }
}
