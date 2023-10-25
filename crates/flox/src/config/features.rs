use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::Config;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, Hash)]
pub enum Feature {
    #[serde(rename = "all")]
    All,
    #[serde(rename = "env")]
    Env,
    #[serde(rename = "nix")]
    Nix,
    #[serde(rename = "develop")]
    Develop,
    #[serde(rename = "publish")]
    Publish,
    #[serde(rename = "channels")]
    Channels,
}

impl Feature {
    // Leaving this code as it may be useful for feature flagging, but it's
    // currently dead as it's a remnant of bash passthrough
    #[allow(unused)]
    pub fn implementation(&self) -> Result<Impl> {
        let map = Config::parse()?.features;

        Ok(match self {
            Feature::All => *map.get(self).unwrap_or(&Impl::Rust),
            Feature::Env => *map
                .get(self)
                .or_else(|| map.get(&Self::All))
                .unwrap_or(&Impl::Rust),
            Feature::Nix => *map
                .get(self)
                .or_else(|| map.get(&Self::All))
                .unwrap_or(&Impl::Rust),
            Feature::Develop | Feature::Publish | Feature::Channels => *map
                .get(self)
                .or_else(|| map.get(&Self::All))
                .unwrap_or(&Impl::Rust),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Impl {
    Rust,
}
