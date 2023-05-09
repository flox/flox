use std::convert::Infallible;
use std::str::FromStr;

use derive_more::Display;
use runix::arguments::flake::OverrideInput;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Display, PartialEq, Eq, Ord, PartialOrd)]
#[serde(rename_all = "camelCase")]
pub enum Stability {
    #[display(fmt = "stable")]
    Stable,
    #[display(fmt = "unstable")]
    Unstable,
    #[display(fmt = "staging")]
    Staging,
    #[display(fmt = "{_0}")]
    Other(String), // will need custom deserializer for this
}

impl Stability {
    pub fn as_override(&self) -> OverrideInput {
        (
            "flox-floxpkgs/nixpkgs/nixpkgs".into(),
            format!("flake:nixpkgs-{self}").parse().unwrap(),
        )
            .into()
    }
}

impl Default for Stability {
    fn default() -> Self {
        Stability::Stable
    }
}

// TODO: fix serde stuff for Stability...
impl FromStr for Stability {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "stable" => Ok(Stability::Stable),
            "unstable" => Ok(Self::Unstable),
            "staging" => Ok(Stability::Staging),
            _ => Ok(Stability::Other(s.to_string())),
        }
    }
}
