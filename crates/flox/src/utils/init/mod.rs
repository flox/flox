use std::collections::{BTreeMap, HashMap};
use std::env;
use std::path::Path;

use anyhow::{Context, Result};
use indexmap::IndexMap;
use indoc::indoc;
use log::{debug, info};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

pub async fn init_git_conf(temp_dir: &Path, config_dir: &Path) -> Result<()> {
    let flox_global_conf_path = config_dir.join("gitconfig");

    // the flox specific git config
    let git_config = format!(
        include_str!("./gitConfig.in"),
        flox_gh_bin = env!("FLOX_GH_BIN"),
        gh_bin = env!("GH_BIN")
    );

    // write or update gitconfig if needed
    if !flox_global_conf_path.exists() || {
        let mut contents = String::new(); // todo: allocate once with some room

        tokio::fs::OpenOptions::new()
            .read(true)
            .open(&flox_global_conf_path)
            .await?
            .read_to_string(&mut contents)
            .await?;

        contents != git_config
    } {
        // create a file in the process directory containing the git config
        let temp_global_conf_path = temp_dir.join("gitconfig");
        tokio::fs::OpenOptions::new()
            .write(true)
            .mode(0o600)
            .create_new(true)
            .open(&temp_global_conf_path)
            .await?
            .write_all(git_config.as_bytes())
            .await?;

        info!("Updating {:#?}", &flox_global_conf_path);
        tokio::fs::rename(temp_global_conf_path, &flox_global_conf_path).await?;
    }

    Ok(())
}
