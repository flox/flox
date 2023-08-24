use std::str::FromStr;

use derive_more::Display;
use runix::arguments::flake::OverrideInput;
use runix::flake_ref::FlakeRef;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Serialize, Deserialize, Display, PartialEq, Eq, Ord, PartialOrd, Default,
)]
#[serde(rename_all = "camelCase")]
pub enum Stability {
    #[default]
    #[display(fmt = "stable")]
    Stable,
    #[display(fmt = "unstable")]
    Unstable,
    #[display(fmt = "staging")]
    Staging,
}

impl Stability {
    pub fn as_override(&self) -> OverrideInput {
        (
            "flox-floxpkgs/nixpkgs/nixpkgs".into(),
            format!("flake:nixpkgs-{self}").parse().unwrap(),
        )
            .into()
    }

    pub fn as_flakeref(&self) -> FlakeRef {
        format!("github:flox/nixpkgs/{}", self).parse().unwrap() // known valid ref
    }
}

impl FromStr for Stability {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(serde_json::Value::String(s.to_string()))
    }
}
