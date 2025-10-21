use std::io::Write;

use clap::Parser;
use env_logger::Env;
use env_logger::fmt::WriteStyle;
use env_logger::fmt::style::{AnsiColor, Style};
use flox_activations::cli::Cli;
use flox_activations::{Error, cli};
use log::debug;
use time::OffsetDateTime;
use time::macros::format_description;

fn init_logging() {
    // 13:07:42.123456 Use `digits:3` for ms, `digits:6` for µs, or `digits:9` for ns.
    let time_fmt = format_description!("[hour]:[minute]:[second].[subsecond digits:6]");

    let mut builder = env_logger::Builder::from_env(Env::default().default_filter_or("info"));
    // Force color even when not a TTY (optional):
    builder.write_style(WriteStyle::Always);

    builder
        .format(move |buf, record| {
            let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
            let ts = now.format(&time_fmt).unwrap();

            // Colors
            let ts_style = Style::new().fg_color(Some(AnsiColor::Magenta.into()));
            let pid_style = Style::new().fg_color(Some(AnsiColor::Cyan.into()));
            let lvl_style = buf.default_level_style(record.level());
            let target_style = Style::new().fg_color(Some(AnsiColor::Green.into()));

            writeln!(
                buf,
                "{ts_style}{ts}{ts_style:#} \
                 {pid_style}pid={pid}{pid_style:#} \
                 {lvl_style}{level}{lvl_style:#} \
                 {target_style}{target}{target_style:#}: {msg}",
                pid = std::process::id(),
                level = record.level(),
                target = record.target(),
                msg = record.args(),
            )
        })
        .init();
}

fn main() -> Result<(), Error> {
    init_logging();

    let args = Cli::parse();
    debug!("{args:?}");

    match args.command {
        cli::Command::StartOrAttach(args) => {
            args.handle()?;
        },
        cli::Command::SetReady(args) => args.handle()?,
        cli::Command::Attach(args) => args.handle()?,
        cli::Command::Activate(args) => args.handle()?,
        cli::Command::FixPaths(args) => args.handle()?,
        cli::Command::SetEnvDirs(args) => args.handle()?,
        cli::Command::ProfileScripts(args) => args.handle()?,
        cli::Command::PrependAndDedup(args) => args.handle(),
        cli::Command::FixFpath(args) => args.handle(),
    }
    Ok(())
}
