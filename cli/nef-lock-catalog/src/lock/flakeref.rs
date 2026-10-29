use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::instrument;
use url::Url;

use crate::nix;

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

/// A catalog-resolved source is stored as the catalog returned it.
impl From<floxhub_client::LockedGitSource> for RawNixFlakerefAttrs {
    fn from(value: floxhub_client::LockedGitSource) -> Self {
        Self::new_unchecked(serde_json::to_value(value).expect("serializes to a JSON object"))
    }
}

/// A source that is not a locked git source, such as one overridden for a
/// build, cannot be submitted to the catalog.
#[derive(Debug, thiserror::Error)]
#[error("not a locked git source: {0}")]
pub struct NotAGitSourceError(Value);

impl TryFrom<&RawNixFlakerefAttrs> for floxhub_client::LockedGitSource {
    type Error = NotAGitSourceError;

    fn try_from(value: &RawNixFlakerefAttrs) -> Result<Self, Self::Error> {
        if value.0.get("type").and_then(Value::as_str) != Some("git") {
            return Err(NotAGitSourceError(value.0.clone()));
        }
        serde_json::from_value(value.0.clone()).map_err(|_| NotAGitSourceError(value.0.clone()))
    }
}

/// The directory beneath a repository's root (or its flakeref `dir`) that
/// holds a NEF project's `pkgs/`.
pub(crate) const NEF_BASE_DIR: &str = ".flox";

/// A flakeref fetched for `--override-input`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FetchedOverride {
    /// The source to put in the lock: `locked` as `nix flake prefetch`
    /// reports it, with `dir` pointing at [NEF_BASE_DIR] beneath any `dir`
    /// the flakeref names.
    pub source: RawNixFlakerefAttrs,
    /// The fetched tree, to scan for catalog references.
    pub store_path: PathBuf,
}

/// The part of `nix flake prefetch --json` output an override needs.
#[derive(Debug, Deserialize)]
struct NixPrefetchResult {
    locked: serde_json::Map<String, Value>,
    #[serde(rename = "storePath")]
    store_path: PathBuf,
}

impl From<NixPrefetchResult> for FetchedOverride {
    fn from(value: NixPrefetchResult) -> Self {
        let mut locked = value.locked;
        let dir = match locked.get("dir").and_then(Value::as_str) {
            Some(dir) => format!("{dir}/{NEF_BASE_DIR}"),
            None => NEF_BASE_DIR.to_string(),
        };
        locked.insert("dir".to_string(), dir.into());
        Self {
            source: RawNixFlakerefAttrs::new_unchecked(Value::Object(locked)),
            store_path: value.store_path,
        }
    }
}

impl FetchedOverride {
    /// The `pkgs` directory of the NEF project in the fetched tree.
    pub fn pkgs_dir(&self) -> PathBuf {
        let dir = self.source.0["dir"]
            .as_str()
            .expect("the dir is set on construction");
        self.store_path.join(dir).join("pkgs")
    }
}

/// Why [fetch_override] could not fetch a flakeref.
#[derive(Debug, thiserror::Error)]
pub enum PrefetchError {
    #[error("failed to call 'nix flake prefetch'")]
    CallNix(#[source] std::io::Error),
    /// Nix refused the flakeref or could not fetch it; carries its stderr.
    #[error("nix could not fetch the flakeref:\n{0}")]
    Nix(String),
    #[error("failed to parse the output of 'nix flake prefetch'")]
    ParseOutput(#[source] serde_json::Error),
}

/// Fetch `flakeref` with `nix flake prefetch`.
///
/// `--refresh` makes nix fetch a branch or a local tree again rather than
/// reuse a cached copy, so the override sees the current state of the
/// source.
#[instrument(skip_all, fields(progress = format!("Fetching '{}'", flakeref.as_url())))]
pub(crate) fn fetch_override(flakeref: &NixFlakeref) -> Result<FetchedOverride, PrefetchError> {
    let mut command = nix::nix_base_command();
    command
        .args(["flake", "prefetch", "--refresh", "--json"])
        .arg(flakeref.as_url().as_str());

    let output = command.output().map_err(PrefetchError::CallNix)?;
    if !output.status.success() {
        return Err(PrefetchError::Nix(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    let result: NixPrefetchResult =
        serde_json::from_slice(&output.stdout).map_err(PrefetchError::ParseOutput)?;
    Ok(result.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prefetched(locked: Value) -> NixPrefetchResult {
        serde_json::from_value(json!({
            "hash": "sha256-prefetched",
            "locked": locked,
            "original": {},
            "storePath": "/nix/store/abc-source",
        }))
        .unwrap()
    }

    /// `locked` is used as nix reports it, `.flox` added beneath its `dir`.
    #[test]
    fn fetched_override_uses_the_locked_source_beneath_the_nef_base_dir() {
        assert_eq!(
            FetchedOverride::from(prefetched(json!({
                "type": "path",
                "path": "/src/hello",
                "lastModified": 1,
            }))),
            FetchedOverride {
                source: RawNixFlakerefAttrs::new_unchecked(json!({
                    "type": "path",
                    "path": "/src/hello",
                    "lastModified": 1,
                    "dir": ".flox",
                })),
                store_path: PathBuf::from("/nix/store/abc-source"),
            }
        );
        assert_eq!(
            FetchedOverride::from(prefetched(json!({
                "type": "git",
                "url": "file:///src/mono",
                "rev": "abc",
                "narHash": "sha256-reported",
                "dir": "sub",
            })))
            .source,
            RawNixFlakerefAttrs::new_unchecked(json!({
                "type": "git",
                "url": "file:///src/mono",
                "rev": "abc",
                "narHash": "sha256-reported",
                "dir": "sub/.flox",
            }))
        );
    }
}
