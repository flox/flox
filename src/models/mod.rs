
use std::fmt::Display;

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
impl InstallResult {
    fn new() -> Self {
        InstallResult {  }
    }
}
pub struct PublishResult {}
impl PublishResult {
    fn new() -> Self {
        PublishResult {  }
    }
}
pub struct InitResult {
    pub message: String
}
impl InitResult {
    pub fn new(message: &str) -> Self {
        InitResult { message: message.to_string() }
    }
}
pub struct FloxConfig {}

pub struct FloxChannel {

}

pub(crate) trait CacheProvider {

}


pub(crate) trait StorageProvider {
    fn destroy(package: Package) -> Result<PublishResult>;
}

// these are actually dynamic but for now we'll list them out
pub enum FloxBuilder {
    Bazel,
    GoModule,
    PerlPackage,
    PythonPackage,
    RustPackage,
    FloxEnv,
    Mix,
    Derivation,
    DerivationJava,
    DerivationRuby,
    YarnPackage,
    PythonWithPackages,
    Custom(String)
}

impl Display for FloxBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let flox_name = match self {
            FloxBuilder::Bazel => "buildBazelPackage",
            FloxBuilder::GoModule => "buildGoModule",
            FloxBuilder::PerlPackage => "buildPerlPackage",
            FloxBuilder::PythonPackage => "buildPythonPackage",
            FloxBuilder::RustPackage => "buildRustPackage",
            FloxBuilder::FloxEnv => "floxEnv",
            FloxBuilder::Mix => "mkRelease",
            FloxBuilder::Derivation => "mkDerivation",
            FloxBuilder::DerivationJava => "mkDerivation-java",
            FloxBuilder::DerivationRuby => "mkDerivation-ruby",
            FloxBuilder::YarnPackage => "mkYarnPackage",
            FloxBuilder::PythonWithPackages => "python-withPackages",
            FloxBuilder::Custom(s) => s,
        };
        return write!(f, "{}", flox_name)
    }
}

/// A provider for package actions. 
/// 
/// 
#[async_trait]
pub trait PackageProvider {
    async fn init(&self, package_name: &str, builder: FloxBuilder) -> Result<InitResult>;
    async fn create(&self, package_name: &str) -> Result<CreateResult>;
    async fn search(&self, query: &str) -> Result<SearchResult>;
    async fn install(&self, ) -> Result<InstallResult>;
    async fn shell(&self) -> Result<()>;
}

pub trait RuntimeProvider {
    fn activate() -> Result<()>;
    fn environments() -> Result<()>;
}

