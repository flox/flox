use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crossterm::style::{ResetColor, Stylize};
use flox_core::util::message::stderr_supports_color;
use sentry::integrations::tracing::EventFilter;
use tracing::{Level, Subscriber};
use tracing_indicatif::util::FilteredFormatFields;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::reload::Handle;
use tracing_subscriber::{EnvFilter, Registry};

use super::progress::PROGRESS_TAG;
use crate::commands::Verbosity;

pub(super) fn verbosity_filter(verbosity: Verbosity) -> &'static str {
    match verbosity {
        Verbosity::Quiet | Verbosity::Verbose(0) => "off",
        Verbosity::Verbose(1) => "warn,flox=info,flox_rust_sdk=info,flox_core=info",
        Verbosity::Verbose(2) => "warn,flox=debug,flox_rust_sdk=debug,flox_core=debug",
        Verbosity::Verbose(3) => "warn,flox=trace,flox_rust_sdk=trace,flox_core=trace",
        Verbosity::Verbose(_) => "trace",
    }
}

pub fn update_filters(
    filter_handle: &Handle<EnvFilter, Registry>,
    log_filter: &str,
) -> anyhow::Result<()> {
    let filter = EnvFilter::try_from_env("FLOX_LOG").or_else(|_| EnvFilter::try_new(log_filter))?;
    filter_handle.modify(|layer| *layer = filter)?;
    Ok(())
}

pub(super) fn sentry_layer<S>() -> impl tracing_subscriber::Layer<S>
where
    S: Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    sentry::integrations::tracing::layer()
        .enable_span_attributes()
        .event_filter(sentry_event_filter)
}

fn sentry_event_filter(metadata: &tracing::Metadata<'_>) -> EventFilter {
    match *metadata.level() {
        Level::ERROR => EventFilter::Event,
        Level::WARN | Level::INFO => EventFilter::Breadcrumb,
        Level::DEBUG | Level::TRACE => EventFilter::Ignore,
    }
}

/// A timer for the debug log layer that colours the timestamp based on the
/// gap since the previous log line:
///   ≥ 500 ms → red    (significant delay worth investigating)
///   ≥ 100 ms → yellow (noticeable pause)
///   < 100 ms → no colour
///
/// When ANSI is disabled (piped output) the timestamp is written plain.
#[derive(Debug, Clone)]
struct DeltaTimer {
    // TODO: do we need Arc and Mutex?
    last: Arc<Mutex<Option<Instant>>>,
    use_colors: bool,
}

impl DeltaTimer {
    fn new(use_colors: bool) -> Self {
        Self {
            last: Arc::new(Mutex::new(None)),
            use_colors,
        }
    }
}

impl FormatTime for DeltaTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> fmt::Result {
        let now = Instant::now();
        let ts = chrono::Local::now().format("%H:%M:%S%.3f");
        let delta_ms = match self.last.lock() {
            Ok(mut guard) => {
                let ms = guard.map(|prev| now.duration_since(prev).as_millis());
                *guard = Some(now);
                ms
            },
            Err(_) => None,
        };

        if self.use_colors {
            // ResetColor clears dim styling applied by tracing-subscriber so
            // the highlighted timestamps are bright, not faint.
            // It must be a separate write before the colour — chaining .reset()
            // on the same StyledContent puts the Reset attribute before the
            // colour in crossterm's output, which clears the colour.
            match delta_ms {
                Some(ms) if ms >= 500 => {
                    write!(w, "{}{}", ResetColor, ts.to_string().red().bold())
                },
                Some(ms) if ms >= 100 => {
                    write!(w, "{}{}", ResetColor, ts.to_string().yellow().bold())
                },
                _ => write!(w, "{}", ts),
            }
        } else {
            write!(w, "{}", ts)
        }
    }
}

pub(super) fn console_layer(
    writer: BoxMakeWriter,
) -> (
    impl tracing_subscriber::Layer<Registry>,
    Handle<EnvFilter, Registry>,
) {
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
    let use_colors = stderr_supports_color();

    // Tracing layer that handles all other logs.
    //
    // Span data is added to _all_ events within them, and "stack" if multiple spans are active.
    // While the JSON formatter seems to support to suppress this span information,
    // the same is not possible with either of the other builtin formatters.
    //
    // An existing issue on that upstream appears not to have active development:
    // <https://github.com/tokio-rs/tracing/issues/3254>
    //
    // What is possible however is to filter out the _"progress" fields_,
    // so that spans are still printed but we don't repeat the messages.
    // That is using the `FilteredFormatFields` utility from `tracing_indicative`,
    // which is a visitor implementation that just drops fields based on a filter function,
    // here: a test for the field name "progress".
    let log_layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(use_colors)
        // Without this, colored output is broken,
        // see https://github.com/tokio-rs/tracing/issues/3369
        .with_ansi_sanitization(false)
        .with_timer(DeltaTimer::new(use_colors))
        .map_fmt_fields(|format| {
            FilteredFormatFields::new(format, |field| field.name() != PROGRESS_TAG)
        })
        .with_filter(filter);

    (log_layer, filter_reload_handle)
}
