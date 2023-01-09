use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::str::FromStr;
use std::{env, io, iter};

use anyhow::{Context, Result};
use crossterm::tty::IsTty;
use flox_rust_sdk::prelude::{Channel, ChannelRegistry};
use fslock::LockFile;
use indoc::indoc;
use log::{debug, info, trace};
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing_subscriber::prelude::*;
use tracing_subscriber::Layer;

use super::dialog::InquireExt;
use super::metrics::{PosthogLayer, METRICS_UUID_FILE_NAME};
use crate::commands::Verbosity;
use crate::utils::logger;
use crate::utils::metrics::METRICS_LOCK_FILE_NAME;

const ENV_GIT_CONFIG_SYSTEM: &str = "GIT_CONFIG_SYSTEM";
const ENV_FLOX_ORIGINAL_GIT_CONFIG_SYSTEM: &str = "FLOX_ORIGINAL_GIT_CONFIG_SYSTEM";

async fn write_metrics_uuid(uuid_path: &Path, consent: bool) -> Result<()> {
    let mut file = tokio::fs::File::create(&uuid_path).await?;
    if consent {
        let uuid = uuid::Uuid::new_v4();
        file.write_all(uuid.to_string().as_bytes()).await?;
    }
    Ok(())
}

pub async fn init_telemetry_consent(data_dir: &Path, cache_dir: &Path) -> Result<()> {
    tokio::fs::create_dir_all(data_dir).await?;

    if !std::io::stderr().is_tty() || !std::io::stdin().is_tty() {
        // Can't prompt user now, do it another time
        return Ok(());
    }

    let mut metrics_lock = LockFile::open(&cache_dir.join(METRICS_LOCK_FILE_NAME))?;
    tokio::task::spawn_blocking(move || metrics_lock.lock()).await??;

    let uuid_path = data_dir.join(METRICS_UUID_FILE_NAME);

    match tokio::fs::File::open(&uuid_path).await {
        Ok(_) => return Ok(()),
        Err(err) => match err.kind() {
            std::io::ErrorKind::NotFound => {},
            _ => return Err(err.into()),
        },
    }

    debug!("Metrics consent not recorded");

    let bash_user_meta_path = xdg::BaseDirectories::with_prefix("flox")
        .context("Unable to find config dir")?
        .find_config_file("floxUserMeta.json")
        .context("Unable to find floxUserMeta.json")?;

    if let Ok(mut file) = tokio::fs::File::open(&bash_user_meta_path).await {
        trace!("Attempting to extract metrics consent value from bash flox");

        let mut bash_user_meta_json = String::new();
        file.read_to_string(&mut bash_user_meta_json).await?;

        let json: serde_json::Value = serde_json::from_str(&bash_user_meta_json)?;

        if let Some(x) = json["floxMetricsConsent"].as_u64() {
            debug!("Using metrics consent value from bash flox");
            write_metrics_uuid(&uuid_path, x == 1).await?;
            return Ok(());
        }
    }

    trace!("Prompting user for metrics consent");

    let consent = inquire::Confirm::new("Do you consent to the collection of basic usage metrics?")
        .with_help_message(indoc! {"
            flox collects basic usage metrics in order to improve the user experience,
            including a record of the subcommand invoked along with a unique token.
            It does not collect any personal information."})
        .with_flox_theme()
        .prompt()?;

    if consent {
        write_metrics_uuid(&uuid_path, true).await?;
        info!("\nThank you for helping to improve flox!\n");
    } else {
        let _consent_refusal =
            inquire::Confirm::new("Can we log your refusal?")
                .with_help_message("Doing this helps us keep track of our user count, it would just be a single anonymous request")
                .with_flox_theme()
                .prompt()?;

        // TODO log if Refuse

        write_metrics_uuid(&uuid_path, false).await?;
        info!("\nUnderstood. If you change your mind you can change your election\nat any time with the following command: flox reset-metrics\n");
    }

    Ok(())
}

pub async fn init_uuid(data_dir: &Path) -> Result<uuid::Uuid> {
    tokio::fs::create_dir_all(data_dir).await?;

    let uuid_file_path = data_dir.join("uuid");

    match tokio::fs::File::open(&uuid_file_path).await {
        Ok(mut uuid_file) => {
            debug!("Reading uuid from file");
            let mut uuid_str = String::new();
            uuid_file.read_to_string(&mut uuid_str).await?;
            Ok(uuid::Uuid::try_parse(&uuid_str)?)
        },
        Err(err) => match err.kind() {
            std::io::ErrorKind::NotFound => {
                debug!("Creating new uuid");
                let uuid = uuid::Uuid::new_v4();
                let mut file = tokio::fs::File::create(&uuid_file_path).await?;
                file.write_all(uuid.to_string().as_bytes()).await?;

                Ok(uuid)
            },
            _ => Err(err.into()),
        },
    }
}

pub fn init_logger(verbosity: Verbosity, debug: bool) {
    let log_filter = match (verbosity, debug) {
        (Verbosity::Quiet, false) => "off,flox=error",
        (Verbosity::Quiet, true) => "off,flox=error,posix=debug",
        (Verbosity::Verbose(0), false) => "off,flox=info",
        (Verbosity::Verbose(0), true) => "off,flox=debug,posix=debug",
        (Verbosity::Verbose(1), false) => "off,flox=info,flox-rust-sdk=info,runix=info",
        (Verbosity::Verbose(1), true) => {
            "off,flox=debug,flox-rust-sdk=debug,runix=debug,posix=debug"
        },
        (Verbosity::Verbose(2), _) => "debug",
        (Verbosity::Verbose(_), _) => "trace",
    };

    let env_filter = tracing_subscriber::filter::EnvFilter::try_from_default_env()
        .or_else(|_| tracing_subscriber::filter::EnvFilter::try_new(log_filter))
        .unwrap();

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(io::stderr)
        .event_format(logger::LogFormatter { debug })
        .with_filter(env_filter);

    tracing_subscriber::registry()
        .with(PosthogLayer::new())
        .with(fmt_layer)
        .init();
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

pub fn init_access_tokens(
    config_tokens: &HashMap<String, String>,
) -> Result<HashMap<String, String>> {
    #[derive(Deserialize)]
    struct GhHost {
        oauth_token: String,
    }

    let gh_config_file = xdg::BaseDirectories::with_prefix("gh")?.get_config_file("hosts.yml");
    let gh_tokens: HashMap<String, String> = if gh_config_file.exists() {
        serde_yaml::from_reader::<_, HashMap<String, GhHost>>(File::open(gh_config_file)?)?
            .into_iter()
            .map(|(k, v)| (k, v.oauth_token))
            .collect()
    } else {
        Default::default()
    };

    let nix_tokens_file = xdg::BaseDirectories::with_prefix("nix")?.get_config_file("nix.conf");
    let nix_tokens: HashMap<String, String> = if nix_tokens_file.exists() {
        let mut tokens = HashMap::new();
        for line in BufReader::new(File::open(nix_tokens_file)?).lines() {
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

    let beta_access = [
        "github.com/flox/capacitor",
        "github.com/flox/nixpkgs-flox",
        "github.com/flox/nixpkgs-catalog",
        "github.com/flox/catalog-ingest",
        "github.com/flox/flox-extras",
        "github.com/flox/bundlers",
    ]
    .into_iter()
    .map(String::from)
    .zip(iter::repeat(env!("BETA_ACCESS_TOKEN").to_string()));

    let mut tokens = HashMap::new();

    tokens.extend(gh_tokens.into_iter());
    tokens.extend(nix_tokens.into_iter());
    tokens.extend(config_tokens.clone().into_iter());
    tokens.extend(beta_access);

    Ok(tokens)
}

pub async fn init_git_conf(temp_dir: &Path) -> Result<()> {
    // Get the backed up `GIT_CONFIG_SYSTEM` set by a parent invocation of `flox`
    // May be empty if `GIT_CONFIG_SYSTEM` not set outside of flox.
    // If not empty is expected to point to an existing file.
    let backed_system_conf = match env::var(ENV_FLOX_ORIGINAL_GIT_CONFIG_SYSTEM) {
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
        backed_system_conf.as_deref(),
    ) {
        // Use `GIT_CONFIG_SYSTEM` if `FLOX_ORIGINAL_GIT_CONFIG_SYSTEM` is not set.
        // Corresponds to first/"outermost" invocation of flox.
        (Some(c), None) => Some(c),

        // No prior backed up system gitconfig
        (_, Some("")) => None,

        // If an original configuration was backed up, use that one.
        // `GIT_CONFIG_SYSTEM` would refer to the one set by a parent flox instance
        (_, Some(c)) => Some(c),

        // If no backed up config extists, use the default global config file
        _ if Path::new("/etc/gitconfig").exists() => Some("/etc/gitconfig"),

        // if neither exists, no other system config is applied
        _ => None,
    };

    // the flox specific git config
    let git_config = format!(
        include_str!("./gitConfig.in"),
        betaToken = env!("BETA_ACCESS_TOKEN"),
        original_include = system_conf
            .as_ref()
            .map(|c| format!("path = {c}"))
            .unwrap_or_else(|| "; no original system git config".to_string())
    );

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

    // Set system config variable
    env::set_var(ENV_GIT_CONFIG_SYSTEM, temp_system_conf_path);
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
