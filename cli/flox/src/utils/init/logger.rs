use log::{debug, error};
use once_cell::sync::OnceCell;
use tracing_subscriber::filter::Filtered;
use tracing_subscriber::fmt::Layer;
use tracing_subscriber::prelude::*;
use tracing_subscriber::Registry;

use crate::commands::Verbosity;
use crate::utils::metrics::MetricsLayer;
use crate::utils::TERMINAL_STDERR;

struct LockingTerminalStderr;
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LockingTerminalStderr {
    type Writer = LockingTerminalStderr;

    fn make_writer(&'a self) -> Self::Writer {
        LockingTerminalStderr
    }
}

impl std::io::Write for LockingTerminalStderr {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let buf_vec = buf.to_vec();
        if let Ok(mut guard) = TERMINAL_STDERR.lock() {
            guard.write_all(buf_vec.as_slice())?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Ok(mut guard) = TERMINAL_STDERR.lock() {
            guard.flush()?
        }
        Ok(())
    }
}

#[allow(clippy::type_complexity)]
static LOGGER_HANDLE: OnceCell<
    tracing_subscriber::reload::Handle<
        Filtered<
            Layer<
                Registry,
                tracing_subscriber::fmt::format::DefaultFields,
                tracing_subscriber::fmt::format::Format,
                LockingTerminalStderr,
            >,
            tracing_subscriber::EnvFilter,
            Registry,
        >,
        Registry,
    >,
> = OnceCell::new();
pub fn init_logger(verbosity: Option<Verbosity>) {
    let verbosity = verbosity.unwrap_or_default();

    let log_filter = match verbosity {
        // Show only errors
        Verbosity::Quiet => "off,flox=error",
        // Only show warnings
        Verbosity::Verbose(0) => "off,flox=warn",
        // Show our own info logs
        Verbosity::Verbose(1) => "off,flox=info",
        // Also show debug from our libraries
        Verbosity::Verbose(2) => "off,flox=debug,flox-rust-sdk=debug",
        // Also show trace from our libraries and POSIX
        Verbosity::Verbose(3) => "off,flox=trace,flox-rust-sdk=trace",
        // Also show trace from our libraries and POSIX
        Verbosity::Verbose(4) => "debug,flox=trace,flox-rust-sdk=trace",
        Verbosity::Verbose(_) => "trace",
    };

    let filter_handle = LOGGER_HANDLE.get_or_init(|| {
        debug!("Initializing logger (how are you seeing this?)");

        // The first time this layer is set it establishes an upper boundary for `log` verbosity.
        // If you try to `modify` this layer later, `log` will not accept any higher verbosity events.
        //
        // Before we used to replace both the fmt layer _and_ this layer.
        // That purged enough internal state to reset the `log` verbosity filter.
        // For simplicity, we'll now just set the filter to `trace`,
        // and then modify it later to the actual level below.
        // Logs are being passed through by the `log` crate and correctly filtered by `tracing`.
        let filter = tracing_subscriber::filter::EnvFilter::try_new("trace").unwrap();

        let fmt_filtered = tracing_subscriber::fmt::layer()
            .with_writer(LockingTerminalStderr)
            .event_format(tracing_subscriber::fmt::format())
            .with_filter(filter);

        let (fmt_filtered, fmt_reload_handle) =
            tracing_subscriber::reload::Layer::new(fmt_filtered);

        let metrics_layer = MetricsLayer::new();
        let sentry_layer = sentry::integrations::tracing::layer();

        tracing_subscriber::registry()
            .with(fmt_filtered)
            .with(metrics_layer)
            .with(sentry_layer)
            .init();

        fmt_reload_handle
    });

    let result = filter_handle.modify(|layer| {
        match tracing_subscriber::filter::EnvFilter::try_from_default_env()
            .or_else(|_| tracing_subscriber::EnvFilter::try_new(log_filter))
        {
            Ok(new_filter) => *layer.filter_mut() = new_filter,
            Err(err) => {
                error!("Updating logger filter failed: {}", err);
            },
        };
    });
    if let Err(err) = result {
        error!("Updating logger filter failed: {}", err);
    }
}
