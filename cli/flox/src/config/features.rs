use miette::Result;
use flox_rust_sdk::models::search::SearchStrategy;
use serde::{Deserialize, Serialize};

use super::Config;

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct Features {
    /// Which matching logic to use when searching for packages
    #[serde(default)]
    pub search_strategy: SearchStrategy,
    #[serde(default)]
    pub use_catalog: UseCatalog,
}

#[derive(Copy, Clone, Debug, Deserialize, Serialize, derive_more::Deref)]
pub struct UseCatalog(bool);

impl Default for UseCatalog {
    fn default() -> Self {
        UseCatalog(true)
    }
}

impl Features {
    pub fn parse() -> Result<Self> {
        Ok(Config::parse()?.features.unwrap_or_default())
    }
}
