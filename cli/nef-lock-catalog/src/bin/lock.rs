use std::env::args;
use std::path::{Path, PathBuf};

use anyhow::Result;
use nef_lock_catalog::{lock_config, read_config, write_lock};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_span_events(FmtSpan::ENTER))
        .with(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut args = args().skip(1);

    let config_path = PathBuf::from(args.next().unwrap());

    let config = read_config(&config_path)?;
    let lockfile = lock_config(&config)?;

    write_lock(&lockfile, config_path.with_extension("lock"))?;
    Ok(())
}
