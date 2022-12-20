use std::{path::Path, str::FromStr};

use anyhow::{Context, Ok, Result};
use crossterm::tty::IsTty;
use flox_rust_sdk::{
    flox::Flox,
    prelude::{Channel, ChannelRegistry},
};
use indoc::indoc;
use inquire::ui::{Attributes, RenderConfig, StyleSheet, Styled};
use itertools::Itertools;
use log::warn;
use once_cell::sync::Lazy;

use std::collections::HashSet;

use flox_rust_sdk::flox::FloxInstallable;
use flox_rust_sdk::prelude::Installable;

pub mod colors;
pub mod dialog;
pub mod init;
use std::borrow::Cow;

use regex::Regex;

use crate::utils::dialog::InquireExt;

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
        let choices: Vec<String> = matches
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

        if !std::io::stderr().is_tty() {
            return Err(anyhow!(
                indoc! {"
                You must address a specific {derivation_type}. For example with:

                    $ flox {subcommand} {first_choice},

                The available packages are:
                {choices_list}
            "},
                derivation_type = derivation_type,
                subcommand = subcommand,
                first_choice = choices.get(0).expect("Expected at least one choice"),
                choices_list = choices
                    .iter()
                    .map(|choice| format!("  - {choice}"))
                    .join("\n")
            ))
            .context(format!(
                "No terminal to prompt for {derivation_type} choice"
            ));
        }

        // Prompt for the user to select match
        let sel = inquire::Select::new(
            &format!("Select a {} for flox {}", derivation_type, subcommand),
            choices,
        )
        .with_flox_theme()
        .raw_prompt()
        .with_context(|| format!("Failed to prompt for {} choice", derivation_type))?;

        let installable = matches.remove(sel.index).installable();

        warn!(
            "HINT: avoid selecting a {} next time with:",
            derivation_type
        );
        warn!(
            "$ flox {} {}",
            subcommand,
            shell_escape::escape(sel.value.into())
        );

        installable
    } else if matches.len() == 1 {
        matches.remove(0).installable()
    } else {
        return Err(anyhow!("No matching installables found"));
    })
}
