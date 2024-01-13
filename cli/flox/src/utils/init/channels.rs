use std::collections::BTreeMap;
use std::str::FromStr;

use anyhow::{Context, Result};
use flox_rust_sdk::prelude::{Channel, ChannelRegistry};
use flox_rust_sdk::nix::flake_ref::FlakeRef;
use flox_rust_sdk::nix::flake_ref::git_service::{GitServiceRef, GitServiceAttributes};
use indoc::formatdoc;
use once_cell::sync::Lazy;

/// Default channels that are aways vendored with flox and can't be overridden
pub static DEFAULT_CHANNELS: Lazy<BTreeMap<&'static str, FlakeRef>> = Lazy::new(|| {
    [
        ("flox",
         FlakeRef::Github(GitServiceRef::new("flox".to_string(), "floxpkgs".to_string(), GitServiceAttributes { reference: Some("master".to_string()), ..Default::default()}))
         ),
        ("nixpkgs-flox",
         FlakeRef::Github(GitServiceRef::new("flox".to_string(), "nixpkgs-flox".to_string(), GitServiceAttributes { reference: Some("master".to_string()), ..Default::default()}))
         ),
    ]
    .into()
});

/// Hidden channels that are used for stability setup
pub static HIDDEN_CHANNELS: Lazy<BTreeMap<&'static str, FlakeRef>> = Lazy::new(|| {
    [
        (
            "nixpkgs",
            // overridden if stability is known.
            // globalizing stability is outstanding.
            FlakeRef::Github(GitServiceRef::new("flox".to_string(), "nixpkgs".to_string(), GitServiceAttributes { reference: Some("stable".to_string()), ..Default::default()}))
        ),
        ("nixpkgs-stable",
         FlakeRef::Github(GitServiceRef::new("flox".to_string(), "nixpkgs".to_string(), GitServiceAttributes { reference: Some("stable".to_string()), ..Default::default()}))
         ),
        ("nixpkgs-unstable",
         FlakeRef::Github(GitServiceRef::new("flox".to_string(), "nixpkgs".to_string(), GitServiceAttributes { reference: Some("unstable".to_string()), ..Default::default()}))
         ),
        ("nixpkgs-staging",
         FlakeRef::Github(GitServiceRef::new("flox".to_string(), "nixpkgs".to_string(), GitServiceAttributes { reference: Some("staging".to_string()), ..Default::default()}))
         ),
    ]
    .into()
});

/// Setup the **in-memory** channels registry.
///
/// The registry is later written to a file to be passed to nix.
///
/// Channels that have been subscribed to are passed as argument.
/// They are typically read from the floxUserMeta.json configuration in the user's floxmeta.
pub fn init_channels(user_channels: BTreeMap<String, String>) -> Result<ChannelRegistry> {
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
    for (name, flakeref) in DEFAULT_CHANNELS.iter() {
        channels.register_channel(name, flakeref.clone().into())
    }

    // hidden channels
    for (name, flakeref) in HIDDEN_CHANNELS.iter() {
        channels.register_channel(name, flakeref.clone().into())
    }

    Ok(channels)
}
