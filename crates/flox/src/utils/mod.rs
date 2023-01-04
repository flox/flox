use std::{collections::HashMap, str::FromStr};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use crossterm::tty::IsTty;
use flox_rust_sdk::{
    flox::{Flox, ResolvedInstallableMatch},
    prelude::{Channel, ChannelRegistry},
};
use indoc::indoc;
use itertools::Itertools;
use log::{debug, error, warn};
use once_cell::sync::Lazy;
use std::borrow::Cow;
use tempfile::TempDir;

use std::collections::HashSet;

use flox_rust_sdk::flox::FloxInstallable;
use flox_rust_sdk::prelude::Installable;

pub mod colors;
pub mod dialog;
pub mod init;
pub mod metrics;

use regex::Regex;

use crate::config::Config;
use crate::utils::dialog::InquireExt;

static NIX_IDENTIFIER_SAFE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"^[a-zA-Z0-9_-]+$"#).unwrap());

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

fn nix_str_safe(s: &str) -> Cow<str> {
    if NIX_IDENTIFIER_SAFE.is_match(s) {
        s.into()
    } else {
        format!("{:?}", s).into()
    }
}

#[async_trait]
pub trait InstallableDef: FromStr + Default + Clone {
    const DEFAULT_PREFIXES: &'static [(&'static str, bool)];
    const DEFAULT_FLAKEREFS: &'static [&'static str];
    const INSTALLABLE: fn(&Self) -> String;
    const SUBCOMMAND: &'static str;
    const DERIVATION_TYPE: &'static str;
    const ARG_FLAG: Option<&'static str> = None;

    async fn resolve_matches(&self, flox: &Flox) -> Result<Vec<ResolvedInstallableMatch>> {
        Ok(flox
            .resolve_matches(
                &[Self::INSTALLABLE(self).parse()?],
                Self::DEFAULT_FLAKEREFS,
                Self::DEFAULT_PREFIXES,
                false,
            )
            .await?)
    }

    async fn resolve_installable(&self, flox: &Flox) -> Result<Installable> {
        Ok(resolve_installable_from_matches(
            Self::SUBCOMMAND,
            Self::DERIVATION_TYPE,
            Self::ARG_FLAG,
            self.resolve_matches(flox).await?,
        )
        .await?)
    }

    fn complete_inst(&self) -> Vec<(String, Option<String>)> {
        let inst = Self::INSTALLABLE(self);

        let config = Config::parse()
            .map_err(|e| debug!("Failed to load config: {e}"))
            .unwrap_or_default();

        let channels = init_channels()
            .map_err(|e| debug!("Failed to initialize channels: {e}"))
            .unwrap_or_default();

        let process_dir = config.flox.cache_dir.join("process");
        match std::fs::create_dir_all(&process_dir) {
            Ok(_) => {}
            Err(e) => {
                debug!("Failed to create process dir: {e}");
                return vec![];
            }
        };

        let temp_dir = match TempDir::new_in(process_dir) {
            Ok(x) => x,
            Err(e) => {
                debug!("Failed to create temp_dir: {e}");
                return vec![];
            }
        };

        let access_tokens = init::init_access_tokens(&config.nix.access_tokens)
            .map_err(|e| debug!("Failed to initialize access tokens: {e}"))
            .unwrap_or_default();

        let netrc_file = dirs::home_dir()
            .expect("User must have a home directory")
            .join(".netrc");

        let flox = Flox {
            cache_dir: config.flox.cache_dir,
            data_dir: config.flox.data_dir,
            config_dir: config.flox.config_dir,
            channels,
            temp_dir: temp_dir.path().to_path_buf(),
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            netrc_file,
            access_tokens,
            uuid: uuid::Uuid::nil(),
        };

        let default_prefixes = Self::DEFAULT_PREFIXES;
        let default_flakerefs = Self::DEFAULT_FLAKEREFS;

        let inst = inst;
        let handle = tokio::runtime::Handle::current();
        let comp = std::thread::spawn(move || {
            handle
                .block_on(complete_installable(
                    &flox,
                    &inst,
                    default_flakerefs,
                    default_prefixes,
                ))
                .map_err(|e| debug!("Failed to complete installable: {e}"))
                .unwrap_or_default()
        })
        .join()
        .unwrap();

        comp.into_iter().map(|a| (a, None)).collect()
    }
}

pub async fn complete_installable(
    flox: &Flox,
    installable_str: &String,
    default_flakerefs: &[&str],
    default_attr_prefixes: &[(&str, bool)],
) -> Result<Vec<String>> {
    let mut flox_installables: Vec<FloxInstallable> = vec![];

    if installable_str != "." {
        let trimmed = installable_str.trim_end_matches(|c| c == '.' || c == '#');

        if let Ok(flox_installable) = trimmed.parse() {
            flox_installables.push(flox_installable);
        }

        match trimmed.rsplit_once(|c| c == '.' || c == '#') {
            Some((s, _)) if s != trimmed => flox_installables.push(s.parse()?),
            None => flox_installables.push("".parse()?),
            Some(_) => {}
        };
    } else {
        flox_installables.push(FloxInstallable {
            source: Some(".".to_string()),
            attr_path: vec![],
        });
    };

    let matches = flox
        .resolve_matches(
            flox_installables.as_slice(),
            default_flakerefs,
            default_attr_prefixes,
            true,
        )
        .await?;

    let mut prefixes_with: HashMap<String, HashSet<String>> = HashMap::new();
    let mut flakerefs_with: HashMap<String, HashSet<String>> = HashMap::new();

    for m in &matches {
        let k1 = m.key.get(0).expect("match is missing key");

        flakerefs_with
            .entry(k1.clone())
            .or_insert_with(HashSet::new)
            .insert(m.flakeref.clone());

        prefixes_with
            .entry(k1.clone())
            .or_insert_with(HashSet::new)
            .insert(m.prefix.clone());
    }

    let mut completions: Vec<String> = matches
        .iter()
        .flat_map(|m| {
            let nix_safe_key = m
                .key
                .iter()
                .map(|s| nix_str_safe(s.as_str()))
                .collect::<Vec<_>>()
                .join(".");

            let mut t = vec![format!(
                "{}#{}.{}",
                m.flakeref,
                nix_str_safe(&m.prefix),
                nix_safe_key
            )];

            let k1 = m.key.get(0).expect("match is missing key");
            let flakerefs = flakerefs_with.get(k1).map(HashSet::len).unwrap_or(0);
            let prefixes = flakerefs_with.get(k1).map(HashSet::len).unwrap_or(0);

            if let (true, Some(system)) = (m.explicit_system, &m.system) {
                t.push(format!(
                    "{}#{}.{}.{}",
                    m.flakeref,
                    nix_str_safe(&m.prefix),
                    nix_str_safe(system),
                    nix_safe_key
                ));

                if flakerefs <= 1 {
                    t.push(format!(
                        "{}.{}.{}",
                        nix_str_safe(&m.prefix),
                        nix_str_safe(system),
                        nix_safe_key
                    ));
                }
            }

            if flakerefs <= 1 && prefixes <= 1 {
                t.push(nix_safe_key.clone());
            }

            if prefixes <= 1 {
                t.push(format!("{}#{}", m.flakeref, nix_safe_key));
            }

            if flakerefs <= 1 {
                t.push(format!("{}.{}", nix_str_safe(&m.prefix), nix_safe_key));
            }

            t
        })
        .filter(|c| c.starts_with(installable_str))
        .collect();

    completions.sort();
    completions.dedup();

    Ok(completions)
}

pub async fn resolve_installable_from_matches(
    subcommand: &str,
    derivation_type: &str,
    arg_flag: Option<&str>,
    mut matches: Vec<ResolvedInstallableMatch>,
) -> Result<Installable> {
    match matches.len() {
        0 => {
            bail!("No matching installables found");
        }
        1 => Ok(matches.remove(0).installable()),
        _ => {
            let mut prefixes_with: HashMap<String, HashSet<String>> = HashMap::new();
            let mut flakerefs_with: HashMap<String, HashSet<String>> = HashMap::new();

            for m in &matches {
                let k1 = m.key.get(0).expect("match is missing key");

                flakerefs_with
                    .entry(k1.clone())
                    .or_insert_with(HashSet::new)
                    .insert(m.flakeref.clone());

                prefixes_with
                    .entry(k1.clone())
                    .or_insert_with(HashSet::new)
                    .insert(m.prefix.clone());
            }

            // Complile a list of choices for the user to choose from, and shorter choices for suggestions
            let mut choices: Vec<(String, String)> = matches
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

                        let k1 = m.key.get(0).expect("match is missing key");

                        let flakerefs = flakerefs_with.get(k1).map(HashSet::len).unwrap_or(0);
                        let prefixes = flakerefs_with.get(k1).map(HashSet::len).unwrap_or(0);

                        let prefixes_total = prefixes_with.values().fold(0, |a, p| a + p.len());

                        let flakeref_str: Cow<str> = if flakerefs > 1 {
                            format!("{}#", m.flakeref).into()
                        } else {
                            "".into()
                        };

                        let prefix_strs: (Cow<str>, Cow<str>) = if prefixes_total > 1 {
                            let long: Cow<str> = format!("{}.", nix_str_safe(&m.prefix)).into();

                            let short = if prefixes > 1 {
                                long.clone()
                            } else {
                                "".into()
                            };

                            (long, short)
                        } else {
                            ("".into(), "".into())
                        };

                        (
                            format!("{}{}{}", flakeref_str, prefix_strs.0, nix_safe_key),
                            format!("{}{}{}", flakeref_str, prefix_strs.1, nix_safe_key),
                        )
                    },
                )
                .collect();

            let full_subcommand: Cow<str> = match arg_flag {
                Some(f) => format!("{subcommand} {f}").into(),
                None => subcommand.into(),
            };

            if !std::io::stderr().is_tty() || !std::io::stdin().is_tty() {
                error!(
                    indoc! {"
                    You must address a specific {derivation_type}. For example with:

                      $ flox {full_subcommand} {first_choice},

                    The available packages are:
                    {choices_list}
                "},
                    derivation_type = derivation_type,
                    full_subcommand = full_subcommand,
                    first_choice = choices.get(0).expect("Expected at least one choice").1,
                    choices_list = choices
                        .iter()
                        .map(|(choice, _)| format!("  - {choice}"))
                        .join("\n")
                );

                bail!("No terminal to prompt for {derivation_type} choice");
            }

            // Prompt for the user to select match
            let sel = inquire::Select::new(
                &format!("Select a {} for flox {}", derivation_type, subcommand),
                choices.iter().map(|(long, _)| long).collect(),
            )
            .with_flox_theme()
            .raw_prompt()
            .with_context(|| format!("Failed to prompt for {} choice", derivation_type))?;

            let installable = matches.remove(sel.index).installable();

            warn!(
                "HINT: avoid selecting a {} next time with:\n  $ flox {} {}",
                derivation_type,
                full_subcommand,
                shell_escape::escape(choices.remove(sel.index).1.into())
            );

            Ok(installable)
        }
    }
}
