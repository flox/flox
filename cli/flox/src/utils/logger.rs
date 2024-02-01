use std::fmt::{self, Write};

use crossterm::style::Stylize;
use tracing::Level;

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

struct IndentWrapper<'a, 'b> {
    buf: &'a mut tracing_subscriber::fmt::format::Writer<'b>,
}

impl std::fmt::Write for IndentWrapper<'_, '_> {
    fn write_str(&mut self, s: &str) -> Result<(), std::fmt::Error> {
        write!(self.buf, "{}", textwrap::fill(s, 80))?;
        Ok(())
    }
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

        if metadata.target() != "log" {
            return Ok(());
        }

        let level = metadata.level();

        let mut fields = LogFields::default();
        let mut visitor = LoggerVisitor(&mut fields);
        event.record(&mut visitor);

        let mut message = match fields.message {
            Some(m) => m,
            None => return Ok(()),
        };

        let is_posix = fields.target.iter().any(|x| x == "posix");

        let is_flox = fields.target.map_or(false, |target| {
            target == "flox" || target.starts_with("flox::")
        });

        // pretend all messages from `flox` are user facing
        // unless they are posix command prints
        if is_flox && !self.debug && !is_posix {
            let wrap_options = if *level > Level::DEBUG {
                textwrap::Options::new(80)
            } else {
                textwrap::Options::with_termwidth()
            };

            let message = textwrap::fill(&message, wrap_options);
            writeln!(f, "{message}")?;
            return Ok(());
        }

        // VVV  debug mode, posix or not flox  VVV

        let origin_prefix = {
            let line = fields.line.as_deref().unwrap_or("??");
            let file = fields.file.as_deref().unwrap_or("<unknown file>");

            format!("{}:{}", file, line)
        };

        let level_prefix = {
            let level_prefix = level.as_str();

            match *level {
                tracing::Level::ERROR => level_prefix.red(),
                tracing::Level::WARN => level_prefix.yellow(),
                _ => level_prefix.black(),
            }
        };

        if is_posix {
            let styled_message = message.bold();

            if self.debug {
                let head = format!("{level_prefix} {origin_prefix}:").bold();
                writeln!(f, "{head}")?;
            }
            writeln!(f, "+ {styled_message}")?;
            return Ok(());
        }

        // VVV  debug mode, not posix, both flox and not flox  VVV

        // todo: filter this out in the `EnvFilter` `Layer` if possible
        if !self.debug && !is_flox {
            return Ok(());
        }

        // VVV  debug  VVV

        let time_prefix: chrono::DateTime<chrono::Local> = chrono::Local::now();

        let head = format!("{level_prefix} {time_prefix} {origin_prefix}").bold();

        let combined_message = format!("{head}:\n{message}");

        let wrap_options = textwrap::Options::with_termwidth().break_words(false);

        message = textwrap::fill(&combined_message, wrap_options);

        writeln!(f, "{}", message)?;

        Ok(())
    }
}
