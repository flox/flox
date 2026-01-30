use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use tracing::debug;

use crate::CatalogId;

/// Locked source information to nix expression catalog.
/// That is either:
/// 1. a locked source-type [1], referencing a source + optional (sub)
///    with `.flox/pkgs`
/// 2. a package attribute hierarchy with a locked source per package
///    at its leaves (phase 3, WIP)
///
/// [1]: https://nix.dev/manual/nix/2.31/language/builtins.html#source-types
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub(crate) enum CatalogLock {
    #[serde(rename = "floxhub")]
    FloxHub(crate::catalog::CatalogSnapshot),
    #[serde(rename = "nix")]
    Nix(serde_json::Value),
}

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
