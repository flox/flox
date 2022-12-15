mod flake;

use std::{path::Path, str::FromStr};

use anyhow::{Context, Result};
pub use flake::*;
use flox_rust_sdk::{
    flox::Flox,
    prelude::{Channel, ChannelRegistry},
};
use log::{info, warn};
use once_cell::sync::Lazy;

use std::collections::HashSet;

pub use flake::*;
use flox_rust_sdk::flox::FloxInstallable;
use flox_rust_sdk::prelude::Installable;

use std::borrow::Cow;

use regex::Regex;

static NIX_IDENTIFIER_SAFE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^[a-zA-Z0-9_-]+$"#).unwrap());

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

fn nix_str_safe<'a>(s: &'a str) -> Cow<'a, str> {
    if NIX_IDENTIFIER_SAFE.is_match(s) {
        s.into()
    } else {
        format!("{:?}", s).into()
    }
}

pub async fn resolve_installable(
    flox: &Flox,
    flox_installable: FloxInstallable,
    default_flakerefs: &[&str],
    default_attr_prefixes: &[(&str, bool)],
    subcommand: &str,
    derivation_type: &str,
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
            flakerefs.insert(m.flakeref.to_string());
            prefixes.insert(m.prefix.to_string());
        }

        // Complile a list of choices for the user to choose from
        let mut choices: Vec<String> = matches
            .iter()
            .map(
                // Format the results according to how verbose we have to be for disambiguation, only showing the flakeref or prefix when multiple are used
                |m| {
                    let nix_safe_key = m
                        .key
                        .iter()
                        .map(|s| nix_str_safe(s.as_str()))
                        .collect::<Vec<_>>()
                        .join(".");

                    match (flakerefs.len() > 1, prefixes.len() > 1) {
                        (false, false) => nix_safe_key,
                        (true, false) => {
                            format!("{}#{}", m.flakeref, nix_safe_key)
                        }
                        (true, true) => {
                            format!(
                                "{}#{}.{}",
                                m.flakeref,
                                nix_str_safe(&m.prefix),
                                nix_safe_key
                            )
                        }
                        (false, true) => {
                            format!("{}.{}", m.prefix, nix_safe_key)
                        }
                    }
                },
            )
            .collect();

        if !dialoguer::console::user_attended_stderr() {
            warn!(
                "No terminal found, you must address a specific {}. For example with:",
                derivation_type
            );
            warn!(
                "$ flox {} {}",
                subcommand,
                choices.get(0).expect("Expected at least one choice")
            );

            info!("The available packages are:");
            for choice in choices {
                info!("- {}", choice);
            }

            return Err(anyhow!(
                "No terminal to prompt for {} choice",
                derivation_type
            ));
        }

        warn!("Select a {} for flox {}", derivation_type, subcommand);

        // Prompt for the user to select match
        let sel_i = dialoguer::Select::with_theme(&dialoguer::theme::ColorfulTheme::default())
            .with_prompt("Select a package")
            .default(0)
            .max_length(5)
            .items(&choices)
            .interact()
            .with_context(|| format!("Failed to prompt for {} choice", derivation_type))?;

        let installable = matches.remove(sel_i).installable();

        warn!(
            "HINT: avoid selecting a {} next time with:",
            derivation_type
        );
        warn!("$ flox {} {}", subcommand, choices.remove(sel_i));

        installable
    } else if matches.len() == 1 {
        matches.remove(0).installable()
    } else {
        return Err(anyhow!("No matching installables found"));
    })
}
