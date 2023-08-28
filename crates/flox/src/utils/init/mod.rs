use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result};
use indexmap::IndexMap;
use indoc::indoc;
use log::debug;
use serde::Deserialize;

mod logger;
mod metrics;

pub use logger::*;
pub use metrics::*;

mod channels;

pub use channels::{init_channels, DEFAULT_CHANNELS, HIDDEN_CHANNELS};

pub fn init_access_tokens(
    config_tokens: &HashMap<String, String>,
) -> Result<Vec<(String, String)>> {
    use std::io::{BufRead, BufReader};

    #[derive(Deserialize)]
    struct GhHost {
        oauth_token: Option<String>,
    }

    let gh_config_file = xdg::BaseDirectories::with_prefix("gh")?.get_config_file("hosts.yml");
    let gh_tokens: BTreeMap<String, String> = if gh_config_file.exists() {
        serde_yaml::from_reader::<_, IndexMap<String, GhHost>>(std::fs::File::open(
            &gh_config_file,
        )?)
        .context("Could not read `gh` config file")?
        .into_iter()
        .filter_map(|(host, v)| {
            if v.oauth_token.is_none() {
                debug!(
                    indoc! {"
                    gh config ({gh_config_file:?}): {host}: no `oauth_token` specified
                "},
                    gh_config_file = gh_config_file,
                    host = host
                );
            }
            v.oauth_token.map(|token| (host, token))
        })
        .collect()
    } else {
        Default::default()
    };

    let nix_tokens_file = xdg::BaseDirectories::with_prefix("nix")?.get_config_file("nix.conf");
    let nix_tokens: Vec<(String, String)> = if nix_tokens_file.exists() {
        let mut tokens = Vec::new();
        for line in BufReader::new(std::fs::File::open(nix_tokens_file)?).lines() {
            let line = line.unwrap();
            let (k, v) = if let Some(l) = line.split_once('=') {
                l
            } else {
                continue;
            };

            match (k.trim(), v.trim()) {
                ("access-tokens", tt) | ("extra-access-tokens", tt) => {
                    tokens.extend(tt.split_ascii_whitespace().map(|t| {
                        let (tk, tv) = t.split_once('=').unwrap();
                        (tk.to_string(), tv.to_string())
                    }));
                },
                _ => {},
            }
        }
        tokens
    } else {
        debug!("no default user nix.conf found - weird");
        Default::default()
    };

    let mut tokens = Vec::new();

    tokens.extend(nix_tokens.into_iter());
    tokens.extend(gh_tokens.into_iter());
    tokens.extend(config_tokens.clone().into_iter());
    tokens.dedup();

    Ok(tokens)
}
