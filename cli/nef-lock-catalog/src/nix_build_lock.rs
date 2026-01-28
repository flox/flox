use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use tracing::debug;

use crate::CatalogId;

pub(crate) type LockedCatalog = serde_json::Value;

/// A `BuildLock` is a collection of locked sources for each catalog.
/// It is used to ensure reproducibility of builds by locking the
/// sources of declared dependencies.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BuildLock {
    pub(crate) catalogs: BTreeMap<CatalogId, CatalogLock>,
}

impl BuildLock {}

/// Write a `BuildLock` to the specified file.
/// The file is written in a pretty-printed JSON format
/// and consumed by the NEF.
pub fn write_lock(lock: &BuildLock, path: impl AsRef<Path>) -> Result<()> {
    debug!(path = %path.as_ref().display(),"writing build lock");

    let json = serde_json::to_string_pretty(&lock).context("failed to serialize lockfile")?;
    fs::write(&path, &json)
        .with_context(|| format!("failed to write {path:?}", path = path.as_ref()))?;
    Ok(())
}
