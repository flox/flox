use log::{debug, error};
use once_cell::sync::OnceCell;
use tracing_subscriber::prelude::*;
use tracing_subscriber::reload::Handle;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

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

static LOGGER_HANDLE: OnceCell<Handle<EnvFilter, Registry>> = OnceCell::new();

pub(crate) fn init_logger(verbosity: Option<Verbosity>) {
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
        let (subscriber, reload_handle) = create_registry_and_filter_reload_handle();
        subscriber.init();
        reload_handle
    });

    update_filters(filter_handle, log_filter);
}

pub fn update_filters(filter_handle: &Handle<EnvFilter, Registry>, log_filter: &str) {
    let result = filter_handle.modify(|layer| {
        match EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new(log_filter)) {
            Ok(new_filter) => *layer = new_filter,
            Err(err) => {
                error!("Updating logger filter failed: {}", err);
            },
        };
    });
    if let Err(err) = result {
        error!("Updating logger filter failed: {}", err);
    }
}

pub fn create_registry_and_filter_reload_handle() -> (
    impl tracing_subscriber::layer::SubscriberExt,
    Handle<EnvFilter, Registry>,
) {
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
    let (filter, filter_reload_handle) = tracing_subscriber::reload::Layer::new(filter);
    let log_layer = tracing_subscriber::fmt::layer()
        .with_writer(LockingTerminalStderr)
        .event_format(tracing_subscriber::fmt::format())
        .with_filter(filter);
    let metrics_layer = MetricsLayer::new();
    let sentry_layer = sentry::integrations::tracing::layer();
    // Filtered layer must come first.
    // This appears to be the only way to avoid logs of the `flox_command` trace
    // which is processed by the `log_layer` irrepective of the filter applied to it.
    // My current understanding is, that it because the `metrics_layer` (at least) is
    // registering `Interest` for the event and that somehow bypasses the filter?!
    let registry = tracing_subscriber::registry()
        .with(log_layer)
        .with(metrics_layer)
        .with(sentry_layer);

    (registry, filter_reload_handle)
}
