use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use flox_catalog::{CatalogClient, CatalogClientConfig, DEFAULT_CATALOG_URL};
use nef_lock_catalog::{LockOptions, lock_config_with_options, read_config, write_lock};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Parser)]
struct Cli {
    /// Path to the nix-builds.toml config file
    config: PathBuf,

    /// Relative path from source root to packages directory
    #[arg(long)]
    nef_base_dir: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_span_events(FmtSpan::ENTER))
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    let options = LockOptions {
        nef_base_dir: cli.nef_base_dir,
    };

    let client = {
        let catalog_url =
            std::env::var("FLOX_CATALOG_URL").unwrap_or_else(|_| DEFAULT_CATALOG_URL.to_string());
        let floxhub_token = std::env::var("FLOXHUB_TOKEN")
            .ok()
            .map(|token| token.parse())
            .transpose()?;
        let auth = flox_catalog::auth_strategy_from_method(
            &flox_catalog::AuthMethod::Auth0,
            floxhub_token,
            catalog_url.clone(),
        );

        let config = CatalogClientConfig {
            catalog_url,
            extra_headers: Default::default(),
            mock_mode: flox_catalog::CatalogMockMode::None,
            auth_strategy: auth,
            user_agent: None,
        };

        CatalogClient::new(config)?
    };

    let config = read_config(&cli.config)?;
    let lockfile = lock_config_with_options(&config, &client, &options).await?;

    write_lock(&lockfile, cli.config.with_extension("lock"))?;
    Ok(())
}
