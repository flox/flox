use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result};
use flox_rust_sdk::prelude::{Channel, ChannelRegistry};
use indoc::formatdoc;
use log::warn;
use serde::Deserialize;

/// Setup the **in-memory** channels registry.
///
/// The registry is later written to a file to be passed to nix.
///
/// Channels that have been subscribed to are read from the floxUserMeta.json file.
pub fn init_channels(config_dir: &Path) -> Result<ChannelRegistry> {
    // TODO: figure out how/where we handle FloxUserMeta during and after the rewrite
    // For now we "only" need the channels.
    // Editing of this file is left to the bash implementation.
    let flox_user_meta_path = config_dir.join("floxUserMeta.json");

    let user_channels = if flox_user_meta_path.exists() {
        let parsed_user_meta: UserMeta = serde_json::from_reader(File::open(flox_user_meta_path)?)?;
        parsed_user_meta.channels
    } else {
        warn!("Did not find {flox_user_meta_path:?}, continuing without user channels");
        HashMap::default()
    };

    let mut channels = ChannelRegistry::default();

    // user synched channels
    for (name, flakeref) in user_channels.iter() {
        channels.register_channel(
            name,
            Channel::from_str(flakeref).with_context(|| {
                formatdoc! {"
                Channel url for {name} ({flakeref}) could not parsed as a flake reference
                "}
            })?,
        );
    }

    // default channels
    channels.register_channel("flox", Channel::from_str("github:flox/floxpkgs/master")?);
    channels.register_channel(
        "nixpkgs-flox",
        Channel::from_str("github:flox/nixpkgs-flox/master")?,
    );

    // always add these:
    channels.register_channel(
        "nixpkgs",
        // overridden if stability is known.
        // globalizing stability is outstanding.
        Channel::from_str("github:flox/nixpkgs/stable")?,
    );
    channels.register_channel(
        "nixpkgs-stable",
        Channel::from_str("github:flox/nixpkgs/stable")?,
    );
    channels.register_channel(
        "nixpkgs-staging",
        Channel::from_str("github:flox/nixpkgs/staging")?,
    );
    channels.register_channel(
        "nixpkgs-unstable",
        Channel::from_str("github:flox/nixpkgs/unstable")?,
    );

    Ok(channels)
}

#[derive(Deserialize)]
struct UserMeta {
    /// User provided channels
    /// TODO: transition to runix flakeRefs
    channels: HashMap<String, String>,
}
