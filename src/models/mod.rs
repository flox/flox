
use async_trait::async_trait;
use anyhow::{anyhow,Result};
use tokio::sync::RwLock;

pub mod catalog;

use catalog::*;

///
/// Flox base instance
/// 
pub struct Flox <Storage, Package>{
    storage: RwLock<Storage>,
    package_provider: RwLock<Package>,
    channels: Vec<FloxChannel>,
    catalog: RwLock<FloxCatalog>
}


#[derive(Debug)]
pub struct Package {
    name: String,
    description: String
}

pub struct Environment {}
pub struct Generation {}


impl <Storage, Provider> Flox<Storage, Provider> {
    fn subscribe(name: &str) -> Result<()> {
        return Ok(())
    }
    fn unsubscribe(name: &str) -> Result<()> {
        return Ok(())
    }
}

pub struct CreateResult {
     message: String
}

impl CreateResult {
    pub fn new(message: &str) -> Self {
         CreateResult { message:message.to_string() }
    }
}

#[derive(Debug)]
pub struct SearchResult {
    packages: Vec<Package>
}
impl SearchResult {
    pub fn new(packages: Vec<Package>) -> Self {
        SearchResult { packages:packages }
    }
}
pub struct InstallResult {}
pub struct PublishResult {}
pub struct FloxConfig {}

pub struct FloxChannel {

}

pub(crate) trait CacheProvider {

}


pub(crate) trait StorageProvider {
    fn publish(package: Package) -> Result<PublishResult>;
    fn destroy(package: Package) -> Result<PublishResult>;
}

/// A provider for package actions. 
/// 
/// 
#[async_trait]
pub trait PackageProvider {
   async fn create(&self, package_name: &str) -> Result<CreateResult>;
   async fn search(&self, query: &str) -> Result<SearchResult>;
   async fn install(&self, ) -> Result<InstallResult>;
   async fn shell(&self) -> Result<()>;
}

pub trait RuntimeProvider {
    fn activate() -> Result<()>;
    fn environments() -> Result<()>;
}

