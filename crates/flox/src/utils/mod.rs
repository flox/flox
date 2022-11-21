mod flake;

use std::{path::Path, str::FromStr};

use anyhow::Result;
pub use flake::*;
use flox_rust_sdk::{
    flox::Flox,
    prelude::{Channel, ChannelRegistry},
};

use std::collections::HashSet;

pub use flake::*;
use flox_rust_sdk::flox::FloxInstallable;
use flox_rust_sdk::prelude::Installable;

struct Flake {}

impl Flake {
    fn determine_default_flake(path_str: String) {
        let _path = Path::new(&path_str);
    }
}

pub fn init_channels() -> Result<ChannelRegistry> {
    let mut channels = ChannelRegistry::default();
    channels.register_channel("flox", Channel::from_str("github:flox/floxpkgs")?);
    channels.register_channel("nixpkgs", Channel::from_str("github:flox/nixpkgs/stable")?);
    channels.register_channel(
        "nixpkgs-flox",
        Channel::from_str("github:flox/nixpkgs-flox/master")?,
    );

    // generate these dynamically based on <?>
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

pub async fn resolve_installable(
    flox: &Flox,
    flox_installable: FloxInstallable,
    default_flakerefs: &[&str],
    default_attr_prefixes: &[(&str, bool)],
) -> Result<Installable> {
    let mut matches = flox
        .resolve_matches(flox_installable, default_flakerefs, default_attr_prefixes)
        .await?;

    Ok(if matches.len() > 1 {
        // Create set of used prefixes and flakerefs to determine how many are in use
        let mut flakerefs: HashSet<String> = HashSet::new();
        let mut prefixes: HashSet<String> = HashSet::new();

        // Populate the flakerefs and prefixes sets
        for m in &matches {
            flakerefs.insert(m.installable.flakeref.to_string());
            prefixes.insert(m.prefix.to_string());
        }

        // Complile a list of choices for the user to choose from
        let choices: Vec<String> = matches
            .iter()
            .map(
                // Format the results according to how verbose we have to be for disambiguation, only showing the flakeref or prefix when multiple are used
                |m| match (flakerefs.len() > 1, prefixes.len() > 1) {
                    (false, false) => m.key.join("."),
                    (true, false) => {
                        format!("{}#{}", m.installable.flakeref, m.key.join("."))
                    }
                    (true, true) => {
                        format!(
                            "{}#{}.{}",
                            m.installable.flakeref,
                            m.prefix,
                            m.key.join(".")
                        )
                    }
                    (false, true) => {
                        format!("{}.{}", m.prefix, m.key.join("."))
                    }
                },
            )
            .collect();

        // Prompt for the user to select match
        let sel_i = dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt("Select a package")
            .default(0)
            .max_length(5)
            .items(&choices)
            .interact()
            .unwrap();

        matches.remove(sel_i).installable
    } else if matches.len() == 1 {
        matches.remove(0).installable
    } else {
        return Err(anyhow!("No matching installables found"));
    })
}
