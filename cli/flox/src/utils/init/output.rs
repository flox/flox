//! CLI startup owns terminal output, progress coordination, and subscriber installation.

use std::sync::OnceLock;

use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::reload::Handle;
use tracing_subscriber::{EnvFilter, Registry};

use super::{logger, progress};
use crate::commands::Verbosity;
use crate::utils::message::{self, Output};
use crate::utils::metrics::MetricsLayer;

struct CliOutput {
    filter: Handle<EnvFilter, Registry>,
    output: Output,
}

static CLI_OUTPUT: OnceLock<CliOutput> = OnceLock::new();

pub(crate) fn init_output(verbosity: Option<Verbosity>) -> Output {
    let verbosity = verbosity.unwrap_or_default();
    let cli = CLI_OUTPUT.get_or_init(|| {
        let (subscriber, filter, output) = create_registry_and_filter_reload_handle();
        let output = output.with_quiet(matches!(verbosity, Verbosity::Quiet));
        subscriber.init();
        message::set_default_output(output.clone());
        CliOutput { filter, output }
    });
    if let Err(err) = logger::update_filters(&cli.filter, logger::verbosity_filter(verbosity)) {
        tracing::error!(error = %err, "Updating logger filter failed");
    }
    cli.output.clone()
}

pub(crate) fn create_registry_and_filter_reload_handle() -> (
    impl tracing_subscriber::layer::SubscriberExt,
    Handle<EnvFilter, Registry>,
    Output,
) {
    create_registry_with_console_writer(None)
}

fn create_registry_with_console_writer(
    console_writer: Option<BoxMakeWriter>,
) -> (
    impl tracing_subscriber::layer::SubscriberExt,
    Handle<EnvFilter, Registry>,
    Output,
) {
    let (progress_layer, writer) = progress::progress_layer();
    let output = Output::new(std::io::stdout(), writer.clone());
    let (log_layer, filter_reload_handle) =
        logger::console_layer(console_writer.unwrap_or_else(|| BoxMakeWriter::new(writer)));
    let metrics_layer = MetricsLayer::new();
    let sentry_layer = logger::sentry_layer();
    // Filtered layer must come first.
    // This appears to be the only way to avoid logs of the `flox_command` trace
    // which is processed by the `log_layer` irrespective of the filter applied to it.
    // My current understanding is, that it because the `metrics_layer` (at least) is
    // registering `Interest` for the event and that somehow bypasses the filter?!
    let registry = tracing_subscriber::registry()
        .with(log_layer)
        .with(progress_layer)
        .with(metrics_layer)
        .with(sentry_layer);

    (registry, filter_reload_handle, output)
}
