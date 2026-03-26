use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::CatalogId;
use crate::lock::lock_url_with_options;
use crate::nix_build_lock::{BuildLock, CatalogLock};

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
    Full(NixSourceTypeSpec),
    Url { url: Url },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NixSourceTypeSpec {
    #[serde(rename = "type")]
    type_: CatalogType,
    #[serde(flatten)]
    attrs: toml::Table,
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

/// Options for controlling the paths written into lock entries.
/// Defaults to the flox convention (`.flox/pkgs` and `.flox/nix-builds.lock`).
#[derive(Debug, Clone, Default)]
pub struct LockOptions {
    /// Relative path from source root to nef base directory (containing pkgs/, nix-builds.lock).
    /// Appended after any `dir` prefix from the flakeref.
    pub nef_base_dir: Option<String>,
}

/// Lock a [BuildConfig] using the default Flox conventions.
#[tracing::instrument(skip_all)]
pub fn lock_config(config: &BuildConfig) -> Result<BuildLock> {
    lock_config_with_options(config, &LockOptions {
        nef_base_dir: Some(".flox".to_string()),
    })
}

/// Lock a [BuildConfig] with explicit path options.
#[tracing::instrument(skip_all)]
pub fn lock_config_with_options(config: &BuildConfig, options: &LockOptions) -> Result<BuildLock> {
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

        let locked_catalog = match catalog {
            CatalogSpec::Full(source_type) => {
                let prefetch = lock_url_with_options(
                    &serde_json::to_value(source_type)?.try_into()?,
                    options,
                )?;
                CatalogLock::Nix { prefetch }
            },
            CatalogSpec::Url { url } => {
                let prefetch = lock_url_with_options(&url.as_str().try_into()?, options)?;
                CatalogLock::Nix { prefetch }
            },
        };

        locked_catalogs.insert(name.clone(), locked_catalog);
    }

    Ok(BuildLock {
        catalogs: locked_catalogs,
    })
}
