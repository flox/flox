use std::io::Write;

use anyhow::{Context, anyhow};
use env_logger::fmt::style::{AnsiColor, Style};
use flox_core::activate::vars::FLOX_ACTIVATIONS_VERBOSITY_VAR;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Verbosity {
    inner: u32,
}

impl From<u32> for Verbosity {
    fn from(value: u32) -> Self {
        Self { inner: value }
    }
}

impl Verbosity {
    pub fn env_filter(&self) -> &'static str {
        match self.inner {
            0 => "flox_activations=error",
            1 => "flox_activations=debug",
            2 => "flox_activations=trace",
            _ => "flox_activations=trace",
        }
    }

    /// Returns (number for subsystem verbosity, optional filter string)
    pub fn verbosity_from_env_and_arg(arg: Option<u32>) -> (u32, Option<String>) {
        let rust_log = std::env::var("RUST_LOG").context("RUST_LOG not present");
        let our_variable = std::env::var(FLOX_ACTIVATIONS_VERBOSITY_VAR)
            .context("verbosity variable not present")
            .and_then(|value| {
                value
                    .parse::<u32>()
                    .context("failed to parse verbosity as int")
            });
        let our_variable_filter = our_variable
            .as_ref()
            .map(|v| Verbosity::from(*v))
            .map(|v| v.env_filter().to_string());

        let explicit_arg_filter = arg.map(Verbosity::from).map(|v| v.env_filter().to_string());
        let filter = rust_log
            .or(our_variable_filter)
            .or(explicit_arg_filter.ok_or(anyhow!("no arg provided")));
        let subsystem_verbosity = our_variable.ok().or(arg).unwrap_or(0);
        (subsystem_verbosity, filter.ok())
    }
}

pub fn init_logger(verbosity_arg: Option<u32>) -> Result<u32, anyhow::Error> {
    let mut builder = env_logger::Builder::default();
    let (subsystem_verbosity, filter) = Verbosity::verbosity_from_env_and_arg(verbosity_arg);
    if let Some(filter) = filter {
        builder.parse_filters(&filter);
    }
    let format = time::format_description::parse("[hour]:[minute]:[second].[subsecond digits:6]")
        .context("failed to create formatter")?;
    builder.format(move |buf, record| {
        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        let ts = now.format(&format).expect("failed to format timestamp");

        // Colors
        let ts_style = Style::new().fg_color(Some(AnsiColor::Magenta.into()));
        let pid_style = Style::new().fg_color(Some(AnsiColor::Cyan.into()));
        let lvl_style = buf.default_level_style(record.level());
        let target_style = Style::new().fg_color(Some(AnsiColor::Green.into()));

        writeln!(
            buf,
            "{ts_style}{ts}{ts_style:#} \
             {lvl_style}{level}{lvl_style:#} \
             {target_style}{target}{target_style:#} \
             {pid_style}pid={pid}{pid_style:#}: {msg}",
            pid = std::process::id(),
            level = record.level(),
            target = record.target(),
            msg = record.args(),
        )
    });
    builder.init();
    Ok(subsystem_verbosity)
}
