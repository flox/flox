use anyhow::Result;
use flox_rust_sdk::models::search::SearchStrategy;
use serde::{Deserialize, Serialize};

use super::Config;

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct Features {
    pub search_strategy: SearchStrategy,
}

impl Features {
    pub fn parse() -> Result<Self> {
        Ok(Config::parse()?.features)
    }
}
