use anyhow::Result;
use log::debug;
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
    pub fn implementation(&self) -> Result<Impl> {
        let map = Config::parse()?.features;

        Ok(match self {
            Feature::All => *map.get(self).unwrap_or(&Impl::Bash),
            Feature::Env => *map
                .get(self)
                .or_else(|| map.get(&Self::All))
                .unwrap_or(&Impl::Bash),
            Feature::Nix => *map
                .get(self)
                .or_else(|| map.get(&Self::All))
                .unwrap_or(&Impl::Rust),
            Feature::Develop | Feature::Publish | Feature::Channels => *map
                .get(self)
                .or_else(|| map.get(&Self::All))
                .unwrap_or(&Impl::Bash),
        })
    }

    pub fn is_forwarded(&self) -> Result<bool> {
        if self.implementation()? == Impl::Bash {
            let env_name = format!(
                "FLOX_FEATURES_{}",
                serde_variant::to_variant_name(self)?.to_uppercase()
            );
            debug!("`{env_name}` unset or not \"rust\", falling back to legacy flox");
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Impl {
    Rust,
    Bash,
}
