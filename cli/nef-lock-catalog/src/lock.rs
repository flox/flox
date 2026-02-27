use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::nix::nix_base_command;
use crate::{LockOptions, nix};

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
        json!({
            "type": "git",
            "url": url,
            "dir": dir,
        })
        .try_into()
    }

    pub fn as_url(&self) -> &Url {
        &self.url
    }

    pub fn as_json_value(&self) -> &Value {
        &self.parsed
    }
}

/// Lock a flakeref url
pub fn lock_url_with_options(
    flakeref: &NixFlakeref,
    options: &LockOptions,
) -> Result<NixPrefetchResult> {
    let mut prefetch = nix_prefetch_url(flakeref.as_url())?;

    // Extract and remove `dir` from the locked ref.
    // Replaced by explicit pkgsDir/catalogsLock fields.
    {
        let locked = prefetch
            .locked
            .as_object_mut()
            .expect("'locked' attribute should be a map");

        if let Some(ref nef_base_dir) = options.nef_base_dir {
            let prefix = locked.get("dir").and_then(Value::as_str).unwrap_or(".");
            locked.insert("dir".to_string(), format!("{prefix}/{nef_base_dir}").into());
        }
    }

    Ok(prefetch)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NixPrefetchResult {
    hash: String,
    locked: Value,
    original: Value,
    #[serde(rename = "storePath")]
    store_path: PathBuf,
}

impl NixPrefetchResult {
    pub fn locked_flake_ref(&self) -> NixFlakeref {
        self.locked
            .clone()
            .try_into()
            .expect("valid flakeref by construction")
    }

    pub fn store_path(&self) -> &Path {
        &self.store_path
    }
}

/// Lock a flakeref url using `nix flake prefetch`.
/// This resolves urls, downloads the source and returns
/// a locked source type as well as source information,
/// such as hash and storePath.
///
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
pub(crate) fn nix_prefetch_url(url: &Url) -> Result<NixPrefetchResult> {
    let mut command = nix_base_command();
    command
        .arg("flake")
        .arg("prefetch")
        .arg("--refresh")
        .arg("--json")
        .arg(url.as_str());

    let output = command
        .output()
        .with_context(|| format!("failed to run '{command:?}')"))?;

    if !output.status.success() {
        Err(anyhow::anyhow!(
            String::from_utf8_lossy(&output.stderr).into_owned()
        ))
        .with_context(|| format!("failed to lock {url}"))?;
    }

    serde_json::from_slice(&output.stdout).context("could not parse nix prefetch")
}
