use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use nef_lock_catalog::{LockOptions, lock_config_with_options, read_config, write_lock};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Parser)]
struct Cli {
    /// Path to the nix-builds.toml config file
    config: PathBuf,

    /// Relative path from source root to packages directory
    #[arg(long, default_value = ".flox/pkgs")]
    pkgs_dir: String,

    /// Relative path from source root to catalog lock file
    #[arg(long, default_value = ".flox/nix-builds.lock")]
    catalogs_lock: String,
}

fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_span_events(FmtSpan::ENTER))
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    let options = LockOptions {
        pkgs_dir: cli.pkgs_dir,
        catalogs_lock: cli.catalogs_lock,
    };

    let config = read_config(&cli.config)?;
    let lockfile = lock_config_with_options(&config, &options)?;

    write_lock(&lockfile, cli.config.with_extension("lock"))?;
    Ok(())
}
