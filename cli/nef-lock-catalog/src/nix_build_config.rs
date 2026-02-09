use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::nix::nix_base_command;
use crate::nix_build_lock::{BuildLock, CatalogLock};
use crate::{CatalogId, nix};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum CatalogType {
    #[serde(untagged)]
    Nix(String),
}

/// A descriptor for a Catalog to be locked and provided to builds
/// using the NEF.
/// This can be a (typically) unlocked source-type [1].
/// Source types can be described as a URL or as a structured description
/// similar to the `inputs` section in a Nix flake.
///
/// Examples:
///
/// As a URL:
/// ```
/// url = "git+https://github.com/foo/bar?ref=<ref>"
/// ```
///
/// As a structured description:
/// ```
/// type = "git"
/// url = "https://github.com/foo/bar"
/// ref = "<ref>"
/// ```
///
/// [1]: https://nix.dev/manual/nix/2.31/language/builtins.html#source-types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum CatalogSpec {
    Full {
        #[serde(rename = "type")]
        type_: CatalogType,
        #[serde(flatten)]
        attrs: toml::Table,
    },
    Url {
        url: Url,
    },
}

impl CatalogSpec {
    /// Convert the catalog spec into a URL **using Nix' builtin flakeRef formatting**.
    /// The Nix cli only accepts `flakeRef`s rather than structural source descriptors.
    fn to_url(&self) -> Result<Url> {
        if let CatalogSpec::Url { url } = self {
            return Ok(url.clone());
        };

        let catalog_json = serde_json::to_string(&self)?;

        let expr = format!(
            "let flakeRef = builtins.fromJSON ''{catalog_json}''; in builtins.flakeRefToString flakeRef"
        );

        let mut command = nix::nix_base_command();
        command.arg("eval").arg("--raw").arg("--expr").arg(expr);

        let output = command
            .output()
            .with_context(|| format!("failed to run '{command:?}')"))?;

        if !output.status.success() {
            bail!(String::from_utf8_lossy(&output.stderr).into_owned());
        }

        Url::parse(&String::from_utf8(output.stdout)?).context("could not parse nix flakeref")
    }
}

/// A representation of the [BuildConfig] i.e. catalogs to be locked
/// and provided to builds using the NEF.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BuildConfig {
    // Using an IndexMap to lock in user defined order.
    // This is no hard requirement, the lockfile will be sorted.
    catalogs: IndexMap<CatalogId, CatalogSpec>,
}

/// Read a [BuildConfig] at the given `path`.
pub fn read_config(path: impl AsRef<Path>) -> Result<BuildConfig> {
    let config = fs::read(&path)
        .with_context(|| format!("failed to read {path:?}", path = path.as_ref()))?;
    let config = toml::from_slice(&config).context("failed to parse config")?;
    Ok(config)
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
///
/// The url is either provided by the user directly
/// or computed using [CatalogSpec::to_url].
///
/// The lock result above is stored _as is_ in the lockfile,
/// the NEF expects the `.locked` subset of this structure
/// as the input to `builtins.fetchTree`.
fn lock_url(url: &Url) -> Result<serde_json::Value> {
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

#[tracing::instrument(skip_all)]
pub fn lock_config(config: &BuildConfig) -> Result<BuildLock> {
    let BuildConfig {
        catalogs: catalog_spec,
    } = config;

    let mut locked_catalogs = BTreeMap::new();

    for (name, catalog) in catalog_spec {
        let _guard = tracing::info_span!(
            "lock-catalog",
            progress = format!("Locking catalog '{name}'")
        )
        .entered();

        #[allow(clippy::match_single_binding)] // extension point for floxhub catalogs
        let locked_catalog = match catalog {
            nix_spec => {
                let catalog_url = nix_spec.to_url()?;
                let mut prefetch = lock_url(&catalog_url)?;

                // Extract and remove `dir` from the locked ref.
                // Replaced by explicit pkgsDir/catalogsLock fields.
                let dir = prefetch
                    .get_mut("locked")
                    .and_then(|l| l.as_object_mut())
                    .and_then(|l| l.remove("dir"))
                    .and_then(|d| d.as_str().map(String::from))
                    .unwrap_or_default();

                let prefix = if dir.is_empty() {
                    String::new()
                } else {
                    format!("{dir}/")
                };

                CatalogLock::Nix {
                    pkgs_dir: format!("{prefix}.flox/pkgs"),
                    catalogs_lock: Some(format!("{prefix}.flox/nix-builds.lock")),
                    prefetch,
                }
            },
        };

        locked_catalogs.insert(name.clone(), locked_catalog);
    }

    Ok(BuildLock {
        catalogs: locked_catalogs,
    })
}
