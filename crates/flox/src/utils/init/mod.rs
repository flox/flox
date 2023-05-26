use std::collections::{BTreeMap, HashMap};
use std::env;
use std::path::Path;

use anyhow::{Context, Result};
use indexmap::IndexMap;
use indoc::indoc;
use log::{debug, info, warn};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

mod logger;
mod metrics;

pub use logger::*;
pub use metrics::*;

const ENV_GIT_CONFIG_SYSTEM: &str = "GIT_CONFIG_SYSTEM";
const ENV_FLOX_ORIGINAL_GIT_CONFIG_SYSTEM: &str = "FLOX_ORIGINAL_GIT_CONFIG_SYSTEM";

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
                warn!(
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
                    tokens.extend(tt.split_ascii_whitespace().into_iter().map(|t| {
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
    let flox_system_conf_path = config_dir.join("gitconfig");

    // Get the backed up `GIT_CONFIG_SYSTEM` set by a parent invocation of `flox`
    // May be empty if `GIT_CONFIG_SYSTEM` not set outside of flox.
    // If not empty is expected to point to an existing file.
    let backed_up_system_conf = match env::var(ENV_FLOX_ORIGINAL_GIT_CONFIG_SYSTEM) {
        Result::Ok(c) => Some(c),
        _ => None,
    };

    // `GIT_CONFIG_SYSTEM` as outside flox or by parent flox instance.
    // Ignored if absent, empty or pointing to a non-existent file.
    let current_system_conf = match env::var(ENV_GIT_CONFIG_SYSTEM) {
        Result::Ok(c) if !c.is_empty() && Path::new(&c).exists() => Some(c),
        _ => None,
    };

    // Recall or load the system config if it exists
    let system_conf = match (
        current_system_conf.as_deref(),
        backed_up_system_conf.as_deref(),
    ) {
        // Use `GIT_CONFIG_SYSTEM` if `FLOX_ORIGINAL_GIT_CONFIG_SYSTEM` is not set.
        // Ignore if `GIT_CONFIG_SYSTEM` is set to flox/gitconfgi to avoid circular imports.
        // Corresponds to first/"outermost" invocation of flox.
        (Some(c), None) if Path::new(c) != flox_system_conf_path => Some(c),

        // No prior backed up system gitconfig
        (_, Some("")) => None,

        // If an original configuration was backed up, use that one.
        // `GIT_CONFIG_SYSTEM` would refer to the one set by a parent flox instance
        (_, Some(c)) => Some(c),

        // If no backed up config extists, use the default global config file
        // _ if Path::new("/etc/gitconfig").exists() => Some("/etc/gitconfig"),
        _ if tokio::fs::metadata("/etc/gitconfig").await.is_ok() => Some("/etc/gitconfig"),

        // if neither exists, no other system config is applied
        _ => None,
    };

    // the flox specific git config
    let git_config = format!(
        include_str!("./gitConfig.in"),
        original_include = system_conf
            .as_ref()
            .map(|c| format!(
                indoc::indoc!(
                    "

[include]
	path = {}

"
                ),
                c
            ))
            .unwrap_or_default()
    );

    // write or update gitconfig if needed
    if !flox_system_conf_path.exists() || {
        let mut contents = String::new(); // todo: allocate once with some room

        tokio::fs::OpenOptions::new()
            .read(true)
            .open(&flox_system_conf_path)
            .await?
            .read_to_string(&mut contents)
            .await?;

        contents != git_config
    } {
        // create a file in the process directory containing the git config
        let temp_system_conf_path = temp_dir.join("gitconfig");
        tokio::fs::OpenOptions::new()
            .write(true)
            .mode(0o600)
            .create_new(true)
            .open(&temp_system_conf_path)
            .await?
            .write_all(git_config.as_bytes())
            .await?;

        info!("Updating git config {:#?}", &flox_system_conf_path);
        tokio::fs::rename(temp_system_conf_path, &flox_system_conf_path).await?;
    }

    // Set system config variable
    env::set_var(ENV_GIT_CONFIG_SYSTEM, flox_system_conf_path);
    // Set the `FLOX_ORIGINAL_GIT_CONFIG_SYSTEM` variable.
    // This will be empty, if no system wide configuration is applied.
    // In an inner invocation the existence of this variable means that `GIT_CONFIG_SYSTEM` was
    // set by flox.
    env::set_var(
        ENV_FLOX_ORIGINAL_GIT_CONFIG_SYSTEM,
        system_conf.unwrap_or_default(),
    );

    Ok(())
}
