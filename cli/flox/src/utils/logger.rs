use std::fmt;

use crossterm::style::Stylize;

#[derive(Default, Debug)]
struct LogFields {
    message: Option<String>,
    target: Option<String>,
    module: Option<String>,
    file: Option<String>,
    line: Option<String>,
}

struct LoggerVisitor<'a>(&'a mut LogFields);

impl<'a> tracing::field::Visit for LoggerVisitor<'a> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        match field.name() {
            "message" => self.0.message = Some(value.to_string()),
            "log.target" => self.0.target = Some(value.to_string()),
            "log.file" => self.0.file = Some(value.to_string()),
            "log.line" => self.0.line = Some(value.to_string()),
            "log.module_path" => self.0.module = Some(value.to_string()),
            _ => {},
        }
    }

    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        if field.name() == "message" {
            (self.0).message = Some(value.to_string());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.record_str(field, &format!("{value:#?}"));
    }
}

pub struct LogFormatter {
    pub debug: bool,
}

impl<S, N> tracing_subscriber::fmt::FormatEvent<S, N> for LogFormatter
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    N: for<'a> tracing_subscriber::fmt::FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &tracing_subscriber::fmt::FmtContext<'_, S, N>,
        mut f: tracing_subscriber::fmt::format::Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();

        // Only handle "log" events
        if metadata.target() != "log" {
            return Ok(());
        }

        // Read relevant "log.*" fields from the event
        // theses fields are populated by the `log::{error, warn, info, debug, trace}!` macros
        let mut fields = LogFields::default();
        let mut visitor = LoggerVisitor(&mut fields);
        event.record(&mut visitor);

        // If for any reason the message is not present,
        // we don't have anything to log
        let message = match fields.message {
            Some(m) => m,
            None => return Ok(()),
        };

        // Unless debug output is explicitly requested or tthe verbosity is set high enough,
        // simply print the message
        if !self.debug {
            writeln!(f, "{message}")?;
            return Ok(());
        }

        // Produce debug output
        //
        // The output will look like this:
        //
        // ERROR 2021-08-25T14:00:00.000000000Z /path/to/file.rs:42
        // <message>

        let level_prefix = {
            let level = metadata.level();
            let level_prefix = level.as_str();

            match *level {
                tracing::Level::ERROR => level_prefix.red(),
                tracing::Level::WARN => level_prefix.yellow(),
                _ => level_prefix.black(),
            }
        };

        let time_prefix: chrono::DateTime<chrono::Local> = chrono::Local::now();

        let origin_prefix = {
            let line = fields.line.as_deref().unwrap_or("??");
            let file = fields.file.as_deref().unwrap_or("<unknown file>");

            format!("{}:{}", file, line)
        };

        let head = format!("{level_prefix} {time_prefix} {origin_prefix}").bold();

        let message = format!("{head}: {message}");

        writeln!(f, "{}", message)?;

        Ok(())
    }
}
