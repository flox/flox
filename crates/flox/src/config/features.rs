use anyhow::Result;
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

#[derive(Clone, Debug, Deserialize, Serialize, Default, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SearchStrategy {
    Match,
    #[default]
    MatchName,
}
