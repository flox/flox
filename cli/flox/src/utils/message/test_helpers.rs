//! Unit-test transport for user messages. Production output never uses this adapter.

use std::fmt::{self, Write as _};
use std::io::{self, Write};

use flox_rust_sdk::utils::logging::test_helpers::CollectingWriter;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;

use super::Output;

pub(crate) const CAPTURE_TARGET: &str = "flox::test::message";

#[derive(Clone, Debug)]
struct TracingWriter {
    stream: &'static str,
}

impl Write for TracingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.write_fmt(format_args!("{}", String::from_utf8_lossy(bytes)))?;
        Ok(bytes.len())
    }

    // Keep a formatted write intact, including newlines, rather than recording
    // each formatting fragment as a separate tracing event.
    fn write_fmt(&mut self, args: fmt::Arguments<'_>) -> io::Result<()> {
        // TRACE is transport metadata, not diagnostic severity. Sentry ignores it.
        tracing::trace!(target: CAPTURE_TARGET, stream = self.stream, message = %args);
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(crate) fn tracing_output() -> Output {
    Output::new(TracingWriter { stream: "stdout" }, TracingWriter {
        stream: "stderr",
    })
}

#[derive(Clone, Debug, Default)]
struct MessageFields {
    stdout: bool,
    text: String,
}

impl Visit for MessageFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "stream" => self.stdout = value == "stdout",
            "message" => self.text.push_str(value),
            _ => {},
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.text, "{value:?}");
        }
    }
}

#[derive(Clone, Debug)]
struct CaptureLayer {
    stdout: CollectingWriter,
    stderr: CollectingWriter,
}

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        if event.metadata().target() != CAPTURE_TARGET {
            return;
        }
        let mut fields = MessageFields::default();
        event.record(&mut fields);
        let mut writer = if fields.stdout {
            &self.stdout
        } else {
            &self.stderr
        };
        let _ = writer.write_all(fields.text.as_bytes());
    }
}

pub(crate) fn capture_output<S: Subscriber>() -> (impl Layer<S>, CollectingWriter, CollectingWriter)
{
    let stdout = CollectingWriter::default();
    let stderr = CollectingWriter::default();
    let layer = CaptureLayer {
        stdout: stdout.clone(),
        stderr: stderr.clone(),
    };
    (layer, stdout, stderr)
}

pub(crate) fn test_subscriber_message_only() -> (impl Subscriber, CollectingWriter) {
    let (capture, _, stderr) = capture_output();
    (tracing_subscriber::registry().with(capture), stderr)
}
