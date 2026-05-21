use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use flox_catalog::ClientTrait;
use flox_core::Version;
use indexmap::IndexMap;
use nef_catalog_refs::{collect_transitive, parse_dir};
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
    /// Path to scan for catalog attribute-path references (typically the `pkgs/` directory).
    /// When set, only referenced packages are included in each FloxHub catalog lock.
    /// None = lock all packages (legacy behavior).
    pub pkgs_dir: Option<PathBuf>,
}

/// Lock a [BuildConfig] using the default Flox conventions.
#[tracing::instrument(skip_all)]
pub async fn lock_config(
    config: &BuildConfig,
    client: &(impl ClientTrait + Send + Sync),
) -> Result<BuildLock> {
    lock_config_with_options(config, client, &LockOptions {
        nef_base_dir: Some(".flox".to_string()),
        pkgs_dir: None,
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

    // Scan pkgs_dir once for catalog refs when configured.
    // Partitioned by catalog ID; the remainder after "catalogs.<id>." is the pkg attr-path.
    let scanned_refs: Option<HashMap<CatalogId, BTreeSet<String>>> =
        if let Some(pkgs_dir) = &options.pkgs_dir {
            let roots = std::iter::once("catalogs".to_string()).collect();
            let db = parse_dir(pkgs_dir, &roots);
            let all_refs = collect_transitive(db, pkgs_dir, &roots);

            let mut by_catalog: HashMap<CatalogId, BTreeSet<String>> = HashMap::new();
            for r in &all_refs {
                let mut parts = r.splitn(3, '.');
                if let (Some("catalogs"), Some(catalog_name), Some(pkg_path)) =
                    (parts.next(), parts.next(), parts.next())
                {
                    by_catalog
                        .entry(CatalogId(catalog_name.to_string()))
                        .or_default()
                        .insert(pkg_path.to_string());
                }
            }
            Some(by_catalog)
        } else {
            None
        };

    let empty_refs: BTreeSet<String> = BTreeSet::new();
    let mut locked_catalogs = BTreeMap::new();

    for (name, catalog) in catalog_spec {
        let _guard = tracing::info_span!(
            "lock-catalog",
            progress = format!("Locking catalog '{name}'")
        )
        .entered();

        let locked_catalog = match catalog {
            CatalogSpec::FloxHub {} => {
                let referenced = scanned_refs
                    .as_ref()
                    .map(|m| m.get(name).unwrap_or(&empty_refs));
                lock_floxhub_catalog(client, name, referenced).await?
            },
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

/// Lock a FloxHub catalog by fetching locked sources and building a tree.
///
/// `referenced_packages` — when `Some`, only packages whose attr-path (dot-joined)
/// appears in the set are included in the lock.  `None` includes everything.
/// When the catalog API gains a targeted-query endpoint, only this function changes.
#[instrument(skip(client, referenced_packages))]
async fn lock_floxhub_catalog(
    client: &(impl ClientTrait + Send + Sync),
    catalog_id: &CatalogId,
    referenced_packages: Option<&BTreeSet<String>>,
) -> Result<CatalogLock> {
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

    let mut builder = PackageTreeBuilder::new();
    for item in locked_items.results {
        if let Some(refs) = referenced_packages
            && !refs.contains(&item.attr_path_components.join("."))
        {
            continue;
        }
        let source = NixFlakeref::try_from(serde_json::to_value(item.source)?)?;
        builder.add_package(item.attr_path_components, item.build_type, source)?;
    }
    let tree_node = builder.into_root();

    Ok(CatalogLock::FloxHub {
        packages: tree_node,
    })
}
