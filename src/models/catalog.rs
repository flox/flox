use serde::{Serialize, Deserialize};
use getset::{Getters, CopyGetters};
use async_trait::async_trait;
use anyhow::Result;

use super::{Package, PublishResult};

#[async_trait]
pub trait Catalog {
    async fn publish(package: &Package) -> Result<PublishResult>;
}

pub trait PublishProvider {}

pub struct FloxCatalog {
    publish_provider: Box<dyn PublishProvider>
}

#[async_trait]
impl Catalog for FloxCatalog {
    async fn publish(package: &Package) -> Result<PublishResult> {
        return Ok(PublishResult::new())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SourceType {
    Repository(String),
    File(String),
    Directory(String),
    Unknown
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Stability {
    Stable,
    Unstable,
    Staging,
    Other(String), // will need custom deserializer for this
    Unknown
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TargetSystem {
    #[serde(rename = "x86_64-linux")]
    X86_64Linux,
    #[serde(rename = "x86_64-darwin")]
    X86_64Darwin,
    #[serde(rename = "aarch64-darwin")]
    Aarch64Darwin,
    Other(String),
    Unknown
}

impl Default for SourceType {
    fn default() -> Self {
        SourceType::Unknown
    }
}
impl Default for TargetSystem {
    fn default() -> Self {
        TargetSystem::Unknown
    }
}
impl Default for Stability {
    fn default() -> Self {
        Stability::Unknown
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogManifest {
    #[serde(skip)]
    name: String,
    #[serde(skip)]
    source_type: SourceType,
    pub element: CatalogElement,
    pub cache: Vec<CatalogCache>,
    pub build: CatalogBuild,
    pub eval: CatalogEval,
    pub source: CatalogSource

}

impl CatalogManifest {
    
}
#[derive(Debug, Clone, Serialize, Deserialize,Getters)]
#[getset(get = "pub")]
#[serde(rename_all = "camelCase")]
pub struct CatalogElement{
    store_paths: Vec<String>,
    attr_path: Vec<String>,
    active: bool
}
#[derive(Debug, Clone, Serialize, Deserialize, Getters)]
#[serde(rename_all = "camelCase")]
#[getset(get = "pub")]
pub struct CatalogCache{
    cache_url: String,
    state: String, // move to enum?
    narinfo: Vec<NarInfo>
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogBuild{
    // TODO
}
#[derive(Debug, Clone, Serialize, Deserialize, Getters)]
#[serde(rename_all = "camelCase")]
#[getset(get = "pub")]
pub struct CatalogEval{
    attr_path: Vec<String>,
    drv_path: String,
    meta: CatalogMetadata,
    name: String,
    outputs: NixOutput,
    pname: String,
    stability: Stability,
    system: TargetSystem,
    version: String
}

#[derive(Debug, Clone, Serialize, Deserialize, Getters)]
#[serde(rename_all = "camelCase")]
#[getset(get = "pub")]
pub struct NixOutput {
    out: String // could be vec of strings? May have to flatten
}

#[derive(Debug, Clone, Serialize, Deserialize, Getters)]
#[getset(get = "pub")]
#[serde(rename_all = "camelCase")]
pub struct CatalogMetadata {
    available: bool,
    broken: bool,
    insecure: bool,
    name: String,
    outputs_to_install: Vec<String>,
    position: String,
    unfree: bool,
    unsupported: bool
}


#[derive(Debug, Clone, Serialize, Deserialize, Getters)]
#[getset(get = "pub")]
pub struct CatalogSource{
    locked: SourceEntry,
    original: SourceEntry
}

#[derive(Debug, Clone, Serialize, Deserialize, Getters)]
#[getset(get = "pub")]
#[serde(rename_all = "camelCase")]
pub struct SourceEntry{
    #[serde(default)]
    last_modified: u64,    
    #[serde(default)]
    nar_hash: String,
    #[serde(default)]
    owner: String,
    #[serde(default)]
    repo: String,
    #[serde(default)]
    rev: String,
    #[serde(rename="ref", default)]
    reference: String,
    #[serde(rename="type",default)]
    lock_type: String
}


#[derive(Debug, Clone, Serialize, Deserialize, Getters)]
#[getset(get = "pub")]
#[serde(rename_all = "camelCase")]
pub struct NarInfo {
    valid: bool,
    path: String,
    download_hash: String,
    url: String,
    nar_size: u64,
    signatures: Vec<String>,
    deriver: String,
    download_size: u64,
    nar_hash: String,
    references: Vec<String>
}

#[cfg(test)]
mod test {
    use anyhow::Result;
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn test_catalog_model() -> Result<()> {
        let json_file = &fs::read_to_string("tests/catalog_example.json")?;
       
        let manifest: CatalogManifest = 
            serde_json::from_str(json_file)?;

        // Test all of the roots and see if we
        // get what we expect.

        assert_eq!(manifest.element.attr_path, vec!["x86_64-linux",
        "stable",
        "flox"]);

        assert_eq!(manifest.cache.first().unwrap().cache_url, "https://flox-store-public.s3.us-east-1.amazonaws.com?trusted=1");
        assert_eq!(manifest.source.locked.nar_hash, "sha256-edcFhtk4qxHc3r13yTpBRFTRShbZyMDigT9OgeJuCDw=");
        assert_eq!(manifest.eval.meta.position, "/nix/store/40nz8qcma443z2lvp64ryah09i96qyhs-source/default.nix:67");
        let json = serde_json::to_string(&manifest)?;
       
        // check some serialization 
        assert!(json.contains("\"name\":\"flox-0.0.2-r212\""));

        Ok(())
    }
}