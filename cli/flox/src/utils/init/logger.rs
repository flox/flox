use std::io::Write;

use log::{debug, error};
use once_cell::sync::OnceCell;
use sentry::integrations::tracing::layer as sentry_layer;
use tracing_subscriber::prelude::*;

use crate::commands::Verbosity;
use crate::utils::logger::{self, LogFormatter};
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

type LayerType = tracing_subscriber::layer::Layered<MetricsLayer, tracing_subscriber::Registry>;
type ReloadHandle<T> = tracing_subscriber::reload::Handle<T, LayerType>;

#[allow(clippy::type_complexity)]
static LOGGER_HANDLE: OnceCell<(
    ReloadHandle<tracing_subscriber::EnvFilter>,
    ReloadHandle<
        tracing_subscriber::fmt::Layer<
            LayerType,
            tracing_subscriber::fmt::format::DefaultFields,
            LogFormatter,
            LockingTerminalStderr,
        >,
    >,
)> = OnceCell::new();

pub fn init_logger(verbosity: Option<Verbosity>) {
    let verbosity = verbosity.unwrap_or_default();
    let debug = matches!(verbosity, Verbosity::Verbose(1..));

    let log_filter = match verbosity {
        // Show only errors
        Verbosity::Quiet => "off,flox=error",
        // Show our own info logs
        Verbosity::Verbose(0) => "off,flox=info",
        // Also show debug from our libraries
        Verbosity::Verbose(1) => "off,flox=debug,flox-rust-sdk=debug",
        // Also show trace from our libraries and POSIX
        Verbosity::Verbose(2) => "off,flox=trace,flox-rust-sdk=trace",
        // Also show trace from our libraries and POSIX
        Verbosity::Verbose(3) => "debug,flox=trace,flox-rust-sdk=trace",
        Verbosity::Verbose(_) => "trace",
    };

    let (filter_handle, fmt_handle) = LOGGER_HANDLE.get_or_init(|| {
        debug!("Initializing logger (how are you seeing this?)");

        let filter = tracing_subscriber::filter::EnvFilter::try_from_default_env()
            .or_else(|_| tracing_subscriber::filter::EnvFilter::try_new(log_filter))
            .unwrap();
        let (filter_reloadable, filter_reload_handle) =
            tracing_subscriber::reload::Layer::new(filter);

        let fmt = tracing_subscriber::fmt::layer()
            .with_writer(LockingTerminalStderr)
            .event_format(logger::LogFormatter { debug });

        let (fmt_reloadable, fmt_reload_handle) = tracing_subscriber::reload::Layer::new(fmt);

        let fmt_filtered = fmt_reloadable.with_filter(filter_reloadable);

        tracing_subscriber::registry()
            .with(MetricsLayer::new())
            .with(fmt_filtered)
            .with(sentry_layer())
            .init();

        (filter_reload_handle, fmt_reload_handle)
    });

    if let Err(err) = filter_handle.modify(|layer| {
        *layer = tracing_subscriber::filter::EnvFilter::try_from_default_env()
            .or_else(|_| tracing_subscriber::filter::EnvFilter::try_new(log_filter))
            .unwrap();
    }) {
        error!("Updating logger filter failed: {}", err);
    }

    if let Err(err) = fmt_handle.modify(|layer| {
        *layer = tracing_subscriber::fmt::layer()
            .with_writer(LockingTerminalStderr)
            .event_format(logger::LogFormatter { debug });
    }) {
        error!("Updating logger filter failed: {}", err);
    }
}

pub fn flush_logger() {
    if let Some((_, fmt_handle)) = LOGGER_HANDLE.get() {
        let _ = fmt_handle.modify(|l| {
            let _ = l.writer_mut().flush();
        });
    }
}
