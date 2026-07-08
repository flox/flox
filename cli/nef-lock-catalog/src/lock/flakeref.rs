use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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

impl From<floxhub_client::LockedGitSource> for RawNixFlakerefAttrs {
    fn from(value: floxhub_client::LockedGitSource) -> Self {
        Self::new_unchecked(serde_json::to_value(value).expect("deserialized from json body"))
    }
}
