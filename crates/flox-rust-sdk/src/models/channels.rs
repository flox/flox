use derive_more::FromStr;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::flake_ref::ToFlakeRef;
use super::registry::Registry;

#[derive(Error, Debug)]
pub enum ChannelError {
    #[error("Couldn't Parse channel Url: {0}")]
    ParseUrl(#[from] url::ParseError),
}

#[derive(Debug, FromStr)]
pub struct Channel {
    flake_ref: ToFlakeRef,
}

impl Channel {}

impl From<ToFlakeRef> for Channel {
    fn from(flake_ref: ToFlakeRef) -> Self {
        Channel { flake_ref }
    }
}

/// Todo: ensure some channels cannot be overriden
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ChannelRegistry {
    #[serde(flatten)]
    registry: Registry,
}

impl ChannelRegistry {
    pub fn register_channel(&mut self, name: impl ToString, channel: Channel) {
        self.registry.set(name, channel.flake_ref)
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn github_url() {
        Channel::from_str("github:flox/floxpkgs").expect("parses");
    }
}
