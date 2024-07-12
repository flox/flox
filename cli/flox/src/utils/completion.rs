use anyhow::{bail, Result};
use flox_rust_sdk::flox::{Flox, Floxhub, DEFAULT_FLOXHUB_URL};
use log::debug;
use tempfile::TempDir;

use super::init::{init_access_tokens, init_catalog_client};
use crate::config::Config;

pub(crate) trait FloxCompletionExt
where
    Self: Sized,
{
    /// Create a [Self] ([Flox]) instance in the constrained
    /// context of the [bpaf] completion engine
    fn completion_instance() -> Result<Self>;
}

impl FloxCompletionExt for Flox {
    fn completion_instance() -> Result<Flox> {
        let config = Config::parse()
            .map_err(|e| debug!("Failed to load config: {e}"))
            .unwrap();

        let process_dir = config.flox.cache_dir.join("process");
        match std::fs::create_dir_all(&process_dir) {
            Ok(_) => {},
            Err(e) => {
                bail!("Failed to create process dir: {e}");
            },
        };

        let temp_dir = match TempDir::new_in(process_dir) {
            Ok(x) => x,
            Err(e) => {
                bail!("Failed to create temp_dir: {e}");
            },
        };

        let access_tokens = init_access_tokens(
            config
                .nix
                .as_ref()
                .map(|nix_config| &nix_config.access_tokens),
        )
        .map_err(|e| debug!("Failed to initialize access tokens: {e}"))
        .unwrap_or_default();

        let netrc_file = dirs::home_dir()
            .expect("User must have a home directory")
            .join(".netrc");

        let catalog_client = init_catalog_client(&config)?;

        Ok(Flox {
            cache_dir: config.flox.cache_dir,
            data_dir: config.flox.data_dir,
            config_dir: config.flox.config_dir,
            temp_dir: temp_dir.into_path(),
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            netrc_file,
            access_tokens,
            uuid: uuid::Uuid::nil(),
            floxhub_token: None,
            floxhub: Floxhub::new(DEFAULT_FLOXHUB_URL.clone(), None)?,
            catalog_client,
            flake_locking: Default::default(),
            features: config.features.clone().unwrap_or_default(),
        })
    }
}
