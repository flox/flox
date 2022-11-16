use crate::models::{InstallResult, SearchResult};
use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait Initializer {}

#[async_trait]
pub trait Publisher {}

#[async_trait]
pub trait Builder {}

#[async_trait]
pub trait Installer {
    async fn install(&self) -> Result<InstallResult>;
    async fn search(&self, query: &str) -> Result<SearchResult>;
}

#[async_trait]
pub trait Sharer {}

#[async_trait]
pub trait Developer {
    async fn shell(&self) -> Result<()>;
}

/// A Quick example of building the "flywheel" with the traits that we built
#[async_trait]
trait Flywheel: Initializer + Publisher + Installer + Sharer + Builder + Developer {}

// A provider for package actions.
// #[async_trait]
// pub trait PackageProvider {
//     async fn init(&self, package_name: &str, builder: FloxBuilder) -> Result<InitResult>;
//     async fn environments(&self) -> Result<Vec<Environment>>;
//     async fn search(&self, query: &str) -> Result<SearchResult>;
//     async fn install(&self, ) -> Result<InstallResult>;
//     async fn shell(&self) -> Result<()>;
// }
