//# An attempt at defining a domain model for flox
//# Mostly ignore this for now, but feel free to put data models in here
//# that we should share between applications.

use std::fmt::Display;

use anyhow::Result;
use tokio::sync::RwLock;

pub mod catalog;
pub mod channels;
pub mod flake_ref;
pub mod flox_package;
pub mod registry;

use catalog::*;

///
/// Flox base instance
/// Ignore for now
pub struct Flox<Storage, Package> {
    storage: RwLock<Storage>,
    package_provider: RwLock<Package>,
    channels: Vec<FloxChannel>,
    catalog: RwLock<FloxCatalog>,
}

#[derive(Debug, Clone)]
pub struct Package {
    name: String,
    description: String,
}

#[derive(Debug, Clone)]
pub struct Environment {
    name: String,
    path: String,
    system: TargetSystem,
    generations: Vec<Generation>,
}

#[derive(Debug, Clone)]
pub struct Generation {
    number: i64,
    path: String,
}

pub struct History {}

// Probably going to use Flywheel instead of this approach, but I'l leave it here for now.
impl<Storage, Provider> Flox<Storage, Provider> {
    fn subscribe(_name: &str) -> Result<()> {
        Ok(())
    }
    fn unsubscribe(_name: &str) -> Result<()> {
        Ok(())
    }
}

pub struct CreateResult {
    message: String,
}

impl CreateResult {
    pub fn new(message: &str) -> Self {
        CreateResult {
            message: message.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    packages: Vec<Package>,
}
impl SearchResult {
    pub fn new(packages: Vec<Package>) -> Self {
        SearchResult { packages }
    }
}

#[derive(Copy, Clone)]
pub struct InstallResult {}
impl InstallResult {
    fn new() -> Self {
        InstallResult {}
    }
}
#[derive(Copy, Clone)]
pub struct PublishResult {}
impl PublishResult {
    fn new() -> Self {
        PublishResult {}
    }
}
#[derive(Clone, Default)]
pub struct InitResult {
    pub message: String,
}
impl InitResult {
    pub fn new(message: &str) -> Self {
        InitResult {
            message: message.to_string(),
        }
    }
}
pub struct FloxConfig {}

pub struct FloxChannel {}

pub(crate) trait CacheProvider {}

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
    Custom(String),
}

impl From<String> for FloxBuilder {
    fn from(builder: String) -> Self {
        match builder.as_str() {
            "buildBazelPackage" | "bazel" => FloxBuilder::Bazel,
            "buildGoModule" | "go" => FloxBuilder::GoModule,
            "buildPerlPackage" | "perl" => FloxBuilder::PerlPackage,
            "buildPythonPackage" | "python" => FloxBuilder::PythonPackage,
            "buildRustPackage" | "rust" => FloxBuilder::RustPackage,
            "floxEnv" | "env" => FloxBuilder::FloxEnv,
            "mkRelease" | "release" => FloxBuilder::Mix,
            "mkDerivation" | "drv" | "derivation" => FloxBuilder::Derivation,
            "mkDerivation-java" | "drv.java" => FloxBuilder::DerivationJava,
            "mkDerivation-ruby" | "drv.ruby" => FloxBuilder::DerivationRuby,
            "mkYarnPackage" | "yarn" => FloxBuilder::YarnPackage,
            "python-withPackages" | "python-with-packages" => FloxBuilder::PythonWithPackages,
            _ => FloxBuilder::Custom(builder),
        }
    }
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
        write!(f, "{}", flox_name)
    }
}

// pub trait RuntimeProvider {
//     fn activate() -> Result<()>;
//     fn environments() -> Result<()>;
// }
