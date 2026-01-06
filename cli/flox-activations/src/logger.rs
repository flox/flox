use std::io;
use std::io::IsTerminal;
use std::path::Path;

use flox_core::activate::vars::FLOX_ACTIVATIONS_VERBOSITY_VAR;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, Registry, fmt, reload};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Verbosity {
    inner: u32,
}

impl From<u32> for Verbosity {
    fn from(value: u32) -> Self {
        Self { inner: value }
    }
}

impl Verbosity {
    pub fn env_filter(&self) -> &'static str {
        // watchdog is more conservative because it backgrounds and writes to a file
        match self.inner {
            0 => "flox_activations=error,flox_watchdog=info",
            1 => "flox_activations=debug,flox_watchdog=info",
            2 => "flox_activations=trace,flox_watchdog=debug",
            _ => "flox_activations=trace,flox_watchdog=trace",
        }
    }

    /// Returns (number for subsystem verbosity, filter string)
    pub fn verbosity_from_env_and_arg(arg: Option<u32>) -> (u32, String) {
        // Try to get verbosity from environment variable
        let our_variable = std::env::var(FLOX_ACTIVATIONS_VERBOSITY_VAR)
            .ok()
            .and_then(|value| value.parse::<u32>().ok());

        // Build filter string from each source, trying in priority order
        let filter = std::env::var("RUST_LOG")
            .ok()
            .or_else(|| our_variable.map(|v| Verbosity::from(v).env_filter().to_string()))
            .or_else(|| arg.map(|v| Verbosity::from(v).env_filter().to_string()))
            .unwrap_or_else(|| Verbosity::from(0).env_filter().to_string());

        let subsystem_verbosity = our_variable.or(arg).unwrap_or(0);
        (subsystem_verbosity, filter)
    }
}

pub type ReloadHandle = reload::Handle<Box<dyn Layer<Registry> + Send + Sync>, Registry>;

pub struct LoggerHandle {
    pub subsystem_verbosity: u32,
    pub reload_handle: ReloadHandle,
}

/// Initialize logging to STDERR.
pub fn init_stderr_logger(verbosity_arg: Option<u32>) -> Result<u32, anyhow::Error> {
    let (subsystem_verbosity, filter) = Verbosity::verbosity_from_env_and_arg(verbosity_arg);
    let env_filter = EnvFilter::try_new(filter)?;

    let stderr_layer = fmt::layer()
        .with_writer(io::stderr)
        .with_ansi(io::stderr().is_terminal())
        .with_target(true)
        .boxed();

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(env_filter)
        .init();

    Ok(subsystem_verbosity)
}

/// Replace existing logging with a file. Used by long-living child processes.
pub fn init_file_logger(
    verbosity_arg: Option<u32>,
    log_file: impl AsRef<str>,
    log_dir: impl AsRef<Path>,
) -> Result<u32, anyhow::Error> {
    let (subsystem_verbosity, filter) = Verbosity::verbosity_from_env_and_arg(verbosity_arg);
    let env_filter = EnvFilter::try_new(filter)?;

    let file_appender = tracing_appender::rolling::daily(log_dir, log_file.as_ref());

    let file_layer = fmt::layer()
        .with_writer(file_appender)
        .with_ansi(false)
        .with_target(true)
        .boxed();

    tracing_subscriber::registry()
        .with(file_layer)
        .with(env_filter)
        .init();

    Ok(subsystem_verbosity)
}
