use crossterm::style::Attribute;
use crossterm::style::ContentStyle;
use crossterm::style::Stylize;
use std::fmt::{self, Write};

use crate::utils::colors;

#[derive(Default, Debug)]
struct LogFields {
    message: Option<String>,
    target: Option<String>,
}

struct LoggerVisitor<'a>(&'a mut LogFields);

impl<'a> tracing::field::Visit for LoggerVisitor<'a> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            (*self.0).message = Some(value.to_string());
        } else if field.name() == "log.target" {
            (*self.0).target = Some(value.to_string());
        }
    }

    fn record_f64(&mut self, _field: &tracing::field::Field, _value: f64) {}
    fn record_i64(&mut self, _field: &tracing::field::Field, _value: i64) {}
    fn record_u64(&mut self, _field: &tracing::field::Field, _value: u64) {}
    fn record_bool(&mut self, _field: &tracing::field::Field, _value: bool) {}

    fn record_error(
        &mut self,
        field: &tracing::field::Field,
        value: &(dyn std::error::Error + 'static),
    ) {
        if field.name() == "message" {
            (*self.0).message = Some(value.to_string());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            (*self.0).message = Some(format!("{:?}", value));
        }
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
        let mut first = true;
        for chunk in s.split('\n') {
            if !first {
                write!(self.buf, "\n{:width$}", "", width = 4)?;
            }
            self.buf.write_str(chunk)?;
            first = false;
        }

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

        let message = match fields.message {
            Some(m) => m,
            None => return Ok(()),
        };

        let line = if let Some(light_peach) = colors::LIGHT_PEACH.to_crossterm() {
            let mut line_style = ContentStyle::new();
            match *level {
                tracing::Level::TRACE => {
                    line_style.foreground_color = Some(light_peach);
                    line_style.attributes.set(Attribute::Bold);
                }
                tracing::Level::ERROR | tracing::Level::WARN => {
                    line_style.attributes.set(Attribute::Bold);
                }
                _ => {}
            }

            line_style.apply(message).to_string()
        } else {
            message
        };

        if self.debug {
            let target_prefix = if let Some(target) = fields.target {
                // TODO add flox colors for all levels and for target
                let target_name = match supports_color::on(supports_color::Stream::Stderr) {
                    Some(supports_color::ColorLevel {
                        has_basic: true, ..
                    }) => target.bold().to_string(),
                    _ => target,
                };
                format!("[{target_name}] ")
            } else {
                "".to_owned()
            };

            let bare_level_name = match *level {
                tracing::Level::TRACE => "TRACE",
                tracing::Level::DEBUG => "DEBUG",
                tracing::Level::INFO => "INFO",
                tracing::Level::WARN => "WARN",
                tracing::Level::ERROR => "ERROR",
            };

            // TODO add flox colors for all levels and for target
            let level_name = match supports_color::on(supports_color::Stream::Stderr) {
                Some(supports_color::ColorLevel {
                    has_basic: true, ..
                }) => (match *level {
                    tracing::Level::TRACE => bare_level_name.cyan(),
                    tracing::Level::DEBUG => bare_level_name.blue(),
                    tracing::Level::INFO => bare_level_name.green(),
                    tracing::Level::WARN => bare_level_name.yellow(),
                    tracing::Level::ERROR => bare_level_name.red(),
                })
                .to_string(),
                _ => bare_level_name.to_string(),
            };

            write!(
                IndentWrapper { buf: &mut f },
                "[{level_name}] {target_prefix}{line}",
            )?;
            writeln!(f)?;
        } else {
            write!(
                IndentWrapper { buf: &mut f },
                "{level_prefix}{line}",
                level_prefix = match (*level, colors::LIGHT_PEACH.to_crossterm()) {
                    (tracing::Level::ERROR, Some(light_peach)) =>
                        "ERROR: ".with(light_peach).bold().to_string(),
                    _ => "".to_string(),
                },
            )?;
            writeln!(f)?;
        }

        Ok(())
    }
}
