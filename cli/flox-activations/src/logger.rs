use std::io;

use flox_core::activate::vars::FLOX_ACTIVATIONS_VERBOSITY_VAR;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt, reload};

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
        match self.inner {
            0 => "flox_activations=error",
            1 => "flox_activations=debug",
            2 => "flox_activations=trace",
            _ => "flox_activations=trace",
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

pub fn init_logger(verbosity_arg: Option<u32>) -> Result<u32, anyhow::Error> {
    let (subsystem_verbosity, filter) = Verbosity::verbosity_from_env_and_arg(verbosity_arg);
    let env_filter = EnvFilter::try_new(filter)?;

    let stderr_layer = fmt::layer()
        .with_writer(io::stderr)
        .with_ansi(true) // TODO: Interactive only?
        .with_target(true).boxed();
    let (reloadable, _reload_handle) = reload::Layer::new(stderr_layer);

    tracing_subscriber::registry()
        .with(reloadable)
        .with(env_filter)
        .init();

    Ok(subsystem_verbosity)
}
