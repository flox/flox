use async_trait::async_trait;
use anyhow::Result;
use crate::models::{FloxBuilder, InitResult};

#[async_trait]
pub trait Initializer {
    async fn init(&self, package_name: &str, builder: FloxBuilder) -> Result<InitResult>;
}

// A provider for package actions. 
// #[async_trait]
// pub trait PackageProvider {
//     async fn init(&self, package_name: &str, builder: FloxBuilder) -> Result<InitResult>;
//     async fn environments(&self) -> Result<Vec<Environment>>;
//     async fn search(&self, query: &str) -> Result<SearchResult>;
//     async fn install(&self, ) -> Result<InstallResult>;
//     async fn shell(&self) -> Result<()>;
// }