use std::fs::OpenOptions;
use std::path::PathBuf;

use anyhow::Context;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

pub(crate) fn init_logger(file_path: Option<PathBuf>) -> Result<(), anyhow::Error> {
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(EnvFilter::from_default_env());
    let file_layer = if let Some(path) = file_path {
        let path = if path.is_relative() {
            std::env::current_dir()?.join(path)
        } else {
            path
        };
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("failed to open log file {}", path.display()))?;
        Some(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(file)
                .with_filter(EnvFilter::try_new("debug").expect("'debug' should work")),
        )
    } else {
        None
    };
    let sentry_layer = sentry::integrations::tracing::layer().enable_span_attributes();
    tracing_subscriber::registry()
        .with(file_layer)
        .with(sentry_layer)
        .with(stderr_layer)
        .init();
    Ok(())
}
