use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::Name;

pub(crate) type LockedCatalog = serde_json::Value;

#[derive(Debug, Clone, Default, Serialize)]
pub struct BuildLock {
    pub(crate) catalogs: BTreeMap<Name, LockedCatalog>,
}

impl BuildLock {}

#[tracing::instrument(skip_all, fields(path = %path.as_ref().display()))]
pub fn write_lock(lock: &BuildLock, path: impl AsRef<Path>) -> Result<()> {
    let json = serde_json::to_string_pretty(&lock).context("failed to serialize lockfile")?;
    fs::write(&path, &json)
        .with_context(|| format!("failed to write {path:?}", path = path.as_ref()))?;
    Ok(())
}
