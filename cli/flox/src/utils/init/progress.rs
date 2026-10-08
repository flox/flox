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
                write!(f, "👻 How can you see me?")
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
