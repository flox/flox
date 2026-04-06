use std::sync::OnceLock;

use tracing::{debug, error};
use tracing_indicatif::util::FilteredFormatFields;
use tracing_subscriber::prelude::*;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry, filter};

use crate::commands::Verbosity;
use crate::utils::init::logger::indicatif::PROGRESS_TAG;
use crate::utils::message::stderr_supports_color;
use crate::utils::metrics::MetricsLayer;

/// Type-erased filter update function, so we can store a handle regardless
/// of whether the profiling chrome layer changes the subscriber type.
type FilterUpdateFn = Box<dyn Fn(&str) + Send + Sync>;

static LOGGER_HANDLE: OnceLock<FilterUpdateFn> = OnceLock::new();

/// Holds the Chrome tracing FlushGuard so it lives until process exit/exec.
/// When dropped (or before exec), the guard flushes trace data to disk.
#[cfg(feature = "profiling")]
static CHROME_GUARD: std::sync::Mutex<Option<tracing_chrome::FlushGuard>> =
    std::sync::Mutex::new(None);

/// Drop the Chrome FlushGuard to flush trace data to disk.
/// Call this before exec() to ensure trace data is written.
pub fn flush_chrome_trace() {
    #[cfg(feature = "profiling")]
    {
        let mut guard = CHROME_GUARD.lock().expect("chrome guard mutex poisoned");
        // Dropping the guard flushes the trace file
        guard.take();
    }
}

pub(crate) fn init_logger(verbosity: Option<Verbosity>) {
    let verbosity = verbosity.unwrap_or_default();

    let log_filter = match verbosity {
        // Show only errors
        Verbosity::Quiet => "off,flox=error",
        // Only show warnings, and user facing messages
        Verbosity::Verbose(0) => "warn,flox::utils::message=info",
        // Show internal info logs
        Verbosity::Verbose(1) => "warn,flox=info,flox-rust-sdk=info,flox-core=info",
        // Show debug logs from our libraries
        Verbosity::Verbose(2) => "warn,flox=debug,flox-rust-sdk=debug,flox-core=debug",
        // Show trace logs from our libraries
        Verbosity::Verbose(3) => "warn,flox=trace,flox-rust-sdk=trace,flox-core=trace",
        // Show trace for all libraries
        Verbosity::Verbose(_) => "trace",
    };

    let update_fn = LOGGER_HANDLE.get_or_init(|| {
        #[cfg(feature = "profiling")]
        {
            init_subscriber_with_profiling()
        }

        #[cfg(not(feature = "profiling"))]
        {
            init_subscriber()
        }
    });

    update_fn(log_filter);
}

fn make_update_fn<S>(
    handle: tracing_subscriber::reload::Handle<EnvFilter, S>,
) -> FilterUpdateFn
where
    S: Send + Sync + 'static,
{
    Box::new(move |log_filter: &str| {
        let result = handle.modify(|layer| {
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
    })
}

fn init_subscriber() -> FilterUpdateFn {
    debug!("Initializing logger (how are you seeing this?)");

    let (progress_layer, writer) = indicatif::progress_layer();
    let filter = tracing_subscriber::filter::EnvFilter::try_new("trace").unwrap();
    let (filter, filter_reload_handle) = tracing_subscriber::reload::Layer::new(filter);
    let use_colors = stderr_supports_color();

    let message_fmt = tracing_subscriber::fmt::format()
        .compact()
        .without_time()
        .with_level(false)
        .with_target(false);
    let message_layer = tracing_subscriber::fmt::layer()
        .with_writer(writer.clone())
        .with_ansi(use_colors)
        .event_format(message_fmt)
        .with_filter(filter::filter_fn(|meta| {
            meta.target().starts_with("flox::utils::message")
        }));

    let log_layer = tracing_subscriber::fmt::layer()
        .with_writer(writer.clone())
        .with_ansi(use_colors)
        .map_fmt_fields(|format| {
            FilteredFormatFields::new(format, |field| field.name() != PROGRESS_TAG)
        })
        .with_filter(filter::filter_fn(|meta| {
            !meta.target().starts_with("flox::utils::message")
        }));

    let combined_log_layer = log_layer.and_then(message_layer).with_filter(filter);
    let metrics_layer = MetricsLayer::new();
    let sentry_layer = sentry::integrations::tracing::layer().enable_span_attributes();

    tracing_subscriber::registry()
        .with(combined_log_layer)
        .with(progress_layer)
        .with(metrics_layer)
        .with(sentry_layer)
        .init();

    make_update_fn(filter_reload_handle)
}

#[cfg(feature = "profiling")]
fn init_subscriber_with_profiling() -> FilterUpdateFn {
    debug!("Initializing logger (how are you seeing this?)");

    let (progress_layer, writer) = indicatif::progress_layer();
    let filter = tracing_subscriber::filter::EnvFilter::try_new("trace").unwrap();
    let (filter, filter_reload_handle) = tracing_subscriber::reload::Layer::new(filter);
    let use_colors = stderr_supports_color();

    let message_fmt = tracing_subscriber::fmt::format()
        .compact()
        .without_time()
        .with_level(false)
        .with_target(false);
    let message_layer = tracing_subscriber::fmt::layer()
        .with_writer(writer.clone())
        .with_ansi(use_colors)
        .event_format(message_fmt)
        .with_filter(filter::filter_fn(|meta| {
            meta.target().starts_with("flox::utils::message")
        }));

    let log_layer = tracing_subscriber::fmt::layer()
        .with_writer(writer.clone())
        .with_ansi(use_colors)
        .map_fmt_fields(|format| {
            FilteredFormatFields::new(format, |field| field.name() != PROGRESS_TAG)
        })
        .with_filter(filter::filter_fn(|meta| {
            !meta.target().starts_with("flox::utils::message")
        }));

    let combined_log_layer = log_layer.and_then(message_layer).with_filter(filter);
    let metrics_layer = MetricsLayer::new();
    let sentry_layer = sentry::integrations::tracing::layer().enable_span_attributes();

    let (chrome_layer, guard) =
        flox_core::profiling::create_chrome_layer::<tracing_subscriber::Registry>("flox-cli");
    *CHROME_GUARD.lock().expect("chrome guard mutex poisoned") = guard;

    // Chrome layer closest to registry so its S=Registry type param matches
    tracing_subscriber::registry()
        .with(chrome_layer)
        .with(combined_log_layer)
        .with(progress_layer)
        .with(metrics_layer)
        .with(sentry_layer)
        .init();

    make_update_fn(filter_reload_handle)
}

/// Update the log filter on a reload handle.
/// Used by tests that create their own subscriber via [create_registry_and_filter_reload_handle].
pub fn update_filters(
    filter_handle: &tracing_subscriber::reload::Handle<EnvFilter, Registry>,
    log_filter: &str,
) {
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

/// Create a subscriber with a reloadable filter handle.
/// Used by tests that need their own subscriber.
pub fn create_registry_and_filter_reload_handle() -> (
    impl tracing_subscriber::layer::SubscriberExt + tracing::Subscriber,
    tracing_subscriber::reload::Handle<EnvFilter, Registry>,
) {
    let (progress_layer, writer) = indicatif::progress_layer();
    let filter = tracing_subscriber::filter::EnvFilter::try_new("trace").unwrap();
    let (filter, filter_reload_handle) = tracing_subscriber::reload::Layer::new(filter);
    let use_colors = stderr_supports_color();

    let message_fmt = tracing_subscriber::fmt::format()
        .compact()
        .without_time()
        .with_level(false)
        .with_target(false);
    let message_layer = tracing_subscriber::fmt::layer()
        .with_writer(writer.clone())
        .with_ansi(use_colors)
        .event_format(message_fmt)
        .with_filter(filter::filter_fn(|meta| {
            meta.target().starts_with("flox::utils::message")
        }));

    let log_layer = tracing_subscriber::fmt::layer()
        .with_writer(writer.clone())
        .with_ansi(use_colors)
        .map_fmt_fields(|format| {
            FilteredFormatFields::new(format, |field| field.name() != PROGRESS_TAG)
        })
        .with_filter(filter::filter_fn(|meta| {
            !meta.target().starts_with("flox::utils::message")
        }));

    let combined_log_layer = log_layer.and_then(message_layer).with_filter(filter);
    let metrics_layer = MetricsLayer::new();
    let sentry_layer = sentry::integrations::tracing::layer().enable_span_attributes();

    let registry = tracing_subscriber::registry()
        .with(combined_log_layer)
        .with(progress_layer)
        .with(metrics_layer)
        .with(sentry_layer);

    (registry, filter_reload_handle)
}

// region: indicatif
mod indicatif {
    use std::fmt::{self, Display, Write};

    use indicatif::{ProgressState, ProgressStyle};
    use tracing::Subscriber;
    use tracing::field::{Field, Visit};
    use tracing_indicatif::IndicatifWriter;
    use tracing_subscriber::field::RecordFields;
    use tracing_subscriber::fmt::FormatFields;
    use tracing_subscriber::fmt::format::Writer;
    use tracing_subscriber::layer::Layer;
    use tracing_subscriber::registry;

    pub(super) const PROGRESS_TAG: &str = "progress";

    pub fn progress_layer<S>() -> (impl tracing_subscriber::Layer<S>, IndicatifWriter)
    where
        S: Subscriber + for<'span> registry::LookupSpan<'span> + 'static,
    {
        #[derive(Debug, Default)]
        struct Visitor {
            message: Option<String>,
        }
        impl Display for Visitor {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                if let Some(message) = &self.message {
                    write!(f, "{message}")
                } else {
                    write!(f, "\u{1F47B} How can you see me?")
                }
            }
        }
        impl Visit for Visitor {
            fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
                self.record_str(field, &format!("{:?}", value));
            }

            fn record_str(&mut self, field: &Field, value: &str) {
                if field.name() == PROGRESS_TAG {
                    self.message = Some(value.to_string());
                }
            }
        }

        struct Formatter;
        impl<'writer> FormatFields<'writer> for Formatter {
            /// Format the provided `fields` to the provided [`Writer`], returning a result.
            fn format_fields<R: RecordFields>(
                &self,
                mut writer: Writer<'writer>,
                fields: R,
            ) -> fmt::Result {
                let mut visitor = Visitor::default();
                fields.record(&mut visitor);

                write!(&mut writer, "{visitor}")?;

                Ok(())
            }
        }

        // The progress bar style, a spinner the progress message
        // and the elapsed time if it's running longer than 1 second.
        let style =
            ProgressStyle::with_template("{span_child_prefix}{spinner} {span_fields} {wide_msg}")
                .unwrap()
                .with_key(
                    "elapsed",
                    |state: &ProgressState, writer: &mut dyn Write| {
                        if state.elapsed() > std::time::Duration::from_secs(1) {
                            let seconds = state.elapsed().as_secs();
                            let sub_seconds = (state.elapsed().as_millis() % 1000) / 100;
                            let _ = writer.write_str(&format!("{}.{}s", seconds, sub_seconds));
                        }
                    },
                );

        let layer = tracing_indicatif::IndicatifLayer::new()
            .with_progress_style(style)
            .with_span_field_formatter(Formatter);

        let writer = layer.get_stderr_writer();

        let filtered = layer.with_filter(tracing_subscriber::filter::FilterFn::new(|meta| {
            meta.fields()
                .iter()
                .any(|field| field.name() == PROGRESS_TAG)
        }));

        (filtered, writer)
    }
}
// endregion: indicatif
