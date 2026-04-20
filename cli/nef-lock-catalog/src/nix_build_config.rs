use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use flox_catalog::ClientTrait;
use flox_core::Version;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use tracing::instrument;
use url::Url;

use crate::CatalogId;
use crate::lock::{NixFlakeref, lock_url_with_options};
use crate::nix_build_lock::{BuildLock, CatalogLock};
use crate::tree::PackageTreeBuilder;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum CatalogType {
    #[serde(rename = "floxhub")]
    FloxHub,
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
#[serde(tag = "type", rename_all = "lowercase")]
enum CatalogSpec {
    FloxHub {},
    #[serde(untagged)]
    Full(NixSourceTypeSpec),
    #[serde(untagged)]
    Url {
        url: Url,
    },
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
    #[serde(rename = "version")]
    _version: Version<1>,

    // Using an IndexMap to lock in user defined order.
    // This is no hard requirement, the lockfile will be sorted.
    catalogs: IndexMap<CatalogId, CatalogSpec>,
}

/// Read a [BuildConfig] at the given `path`.
pub fn read_config(path: impl AsRef<Path>) -> Result<BuildConfig> {
    let config = fs::read(&path)
        .with_context(|| format!("failed to read {path:?}", path = path.as_ref()))?;

    #[derive(Debug, Clone, Deserialize, Serialize)]
    #[serde(untagged)]
    enum ConfigVersionCompat {
        V1(BuildConfig),
        VX { version: toml::Value },
    }

    let config: ConfigVersionCompat =
        toml::from_slice(&config).context("failed to parse config")?;
    let config = match config {
        ConfigVersionCompat::V1(config) => config,
        ConfigVersionCompat::VX { version } => {
            anyhow::bail!("unsupported config version: {version}")
        },
    };

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
pub async fn lock_config(
    config: &BuildConfig,
    client: &(impl ClientTrait + Send + Sync),
) -> Result<BuildLock> {
    lock_config_with_options(config, client, &LockOptions {
        nef_base_dir: Some(".flox".to_string()),
    })
    .await
}

/// Lock a [BuildConfig] with explicit path options.
#[tracing::instrument(skip_all)]
pub async fn lock_config_with_options(
    config: &BuildConfig,
    client: &(impl ClientTrait + Send + Sync),
    options: &LockOptions,
) -> Result<BuildLock> {
    let BuildConfig {
        _version: Version,
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
            CatalogSpec::FloxHub {} => lock_floxhub_catalog(client, name).await?,
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
        _version: Version,
        catalogs: locked_catalogs,
    })
}

/// Lock a FloxHub catalog by fetching locked sources and building tree structure
#[instrument(skip(client))]
async fn lock_floxhub_catalog(
    client: &(impl ClientTrait + Send + Sync),
    catalog_id: &CatalogId,
) -> Result<CatalogLock> {
    // Fetch locked sources from FloxHub
    let locked_items = client
        .get_catalog_locked_sources(&catalog_id.0)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to fetch locked sources for catalog '{}': {}",
                catalog_id.0,
                e
            )
        })?;

    // Build tree structure from locked items using PackageTreeBuilder
    let mut builder = PackageTreeBuilder::new();
    for item in locked_items.results {
        // Convert LockedSourceItem to NixFlakeref for the builder
        let source = NixFlakeref::try_from(serde_json::to_value(item.source)?)?;
        builder.add_package(item.attr_path_components, item.build_type, source)?;
    }
    let tree_node = builder.into_root();

    Ok(CatalogLock::FloxHub {
        packages: tree_node,
    })
}
