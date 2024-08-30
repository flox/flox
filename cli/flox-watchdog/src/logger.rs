use std::fs::OpenOptions;
use std::path::PathBuf;
use std::thread::{sleep, spawn};
use std::time::Duration;

use anyhow::Context;
use tracing::debug;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(3600);

/// Initializes a logger that persists logs to an optional file in addition to `stderr`
pub(crate) fn init_logger(file_path: &Option<PathBuf>) -> Result<(), anyhow::Error> {
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(EnvFilter::from_default_env());
    let file_layer = if let Some(path) = file_path {
        let path = if path.is_relative() {
            std::env::current_dir()?.join(path)
        } else {
            path.clone()
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
                .with_filter(EnvFilter::from_env("_FLOX_WATCHDOG_LOG_LEVEL")),
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

/// Starts a background thread which emits a log entry at an interval. This is
/// used as an indication of whether a watchdog's log file can be garbage
/// collected. The thread will run until the watchdog exits.
pub(crate) fn spawn_heartbeat_log() {
    /// Assert that HEARTBEAT_INTERVAL falls in the range of KEEP_WATCHDOG_DAYS at compile time.
    const _: () = assert!(
        HEARTBEAT_INTERVAL.as_secs() < duration_from_days(KEEP_WATCHDOG_DAYS).as_secs(),
        "`HEARTBEAT_INTERVAL` must be less than `KEEP_WATCHDOG_DAYS` days"
    );

    spawn(|| loop {
        debug!("still watching, woof woof");
        sleep(HEARTBEAT_INTERVAL);
    });
}
