use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use flox_core::Version;
use floxhub_client::LockedInputEntry;
use serde::Serialize;
use tracing::{debug, instrument};

use super::tree::PackageTreeNode;
use crate::CatalogId;

/// Locked source information for a catalog: a package attribute hierarchy with
/// a locked source per package at its leaves, as returned by the catalog
/// `/build-inputs/lookup` endpoint.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub(crate) enum CatalogLock {
    #[serde(rename = "floxhub")]
    FloxHub {
        /// Tree structure of locked packages from FloxHub
        packages: PackageTreeNode,
    },
}

/// A `BuildLock` is a collection of locked sources for each catalog.
/// It is used to ensure reproducibility of builds by locking the
/// sources of declared dependencies.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BuildLock {
    #[serde(rename = "version")]
    pub(crate) _version: Version<1>,
    /// The direct (first-order) catalog inputs, keyed by their locked-input
    /// reference. Publish records this subset on the build.
    ///
    /// Note: `LockedInputEntry` is generated from the catalog OpenAPI spec.
    /// Serializing it verbatim into this on-disk lock couples the persisted
    /// format to the generated schema, so regenerating the client can change
    /// the lock format. A later increment should insulate this with a
    /// hand-owned domain type (cf. the `BaseCatalogInfo` newtype and `From`
    /// impls in `floxhub-client`).
    pub(crate) direct_catalog_inputs: HashMap<String, LockedInputEntry>,
    pub(crate) catalogs: BTreeMap<CatalogId, CatalogLock>,
}

impl BuildLock {}

/// Serialize a `BuildLock` to the pretty-printed JSON format consumed by the
/// NEF. Shared by [write_lock] and callers that stream the lock elsewhere
/// (e.g. stdout).
pub fn render_lock(lock: &BuildLock) -> Result<String> {
    serde_json::to_string_pretty(lock).context("failed to serialize lockfile")
}

/// Write a `BuildLock` to the specified file.
/// The file is written in a pretty-printed JSON format
/// and consumed by the NEF.
#[instrument(skip(lock), fields(path = %path.as_ref().display()))]
pub fn write_lock(lock: &BuildLock, path: impl AsRef<Path>) -> Result<()> {
    let json = format!("{}\n", render_lock(lock)?);
    fs::write(&path, &json)
        .with_context(|| format!("failed to write {path:?}", path = path.as_ref()))?;
    debug!(bytes = json.len(), "wrote build lock");
    Ok(())
}
