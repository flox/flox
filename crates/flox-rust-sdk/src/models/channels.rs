use derive_more::FromStr;
use runix::registry::RegistryEntry;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::flake_ref::FlakeRef;
use super::registry::Registry;

#[derive(Error, Debug)]
pub enum ChannelError {
    #[error("Couldn't Parse channel Url: {0}")]
    ParseUrl(#[from] url::ParseError),
}

#[derive(Debug, FromStr, PartialEq, Eq, Clone)]
pub struct Channel {
    flake_ref: FlakeRef,
}

impl Channel {}

impl From<FlakeRef> for Channel {
    fn from(flake_ref: FlakeRef) -> Self {
        Channel { flake_ref }
    }
}

/// Todo: ensure some channels cannot be overridden
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelRegistry {
    #[serde(flatten)]
    registry: Registry,
}

impl ChannelRegistry {
    pub fn register_channel(&mut self, name: impl ToString, channel: Channel) {
        self.registry.set(name, channel.flake_ref)
    }

    pub fn iter(&self) -> impl Iterator<Item = &RegistryEntry> {
        self.registry.entries()
    }

    pub fn iter_names(&self) -> impl Iterator<Item = &String> {
        self.iter().map(|entry| &entry.from.id)
    }

    pub fn get_entry(&self, name: impl AsRef<str>) -> Option<&RegistryEntry> {
        self.iter().find(|entry| entry.from.id == name.as_ref())
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
