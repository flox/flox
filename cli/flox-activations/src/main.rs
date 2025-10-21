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

fn caller_fn(record: &log::Record) -> Option<String> {
    use backtrace::{Backtrace, SymbolName};
    let rec_file = record.file()?;
    let rec_line = record.line()?;

    let bt = Backtrace::new();
    for frame in bt.frames() {
        for sym in frame.symbols() {
            // Heuristic: match the frame that points to the same file & line as the log call.
            if let (Some(file), Some(line)) = (sym.filename(), sym.lineno()) {
                if file.ends_with(rec_file) && line == rec_line {
                    // Demangle (e.g. "flox::commands::activate")
                    return Some(match sym.name() {
                        Some(name) => format!("{name:#}()"), // pretty/demangled
                        None => "<unknown>".to_string(),
                    });
                }
            }
        }
    }
    None
}

fn init_logging() {
    // 13:07:42.123456 Use `digits:3` for ms, `digits:6` for µs, or `digits:9` for ns.
    let time_fmt = format_description!("[hour]:[minute]:[second].[subsecond digits:6]");
    let mut builder = env_logger::Builder::from_env(Env::default().default_filter_or("info"));

    // Uncomment to force color always:
    builder.write_style(env_logger::fmt::WriteStyle::Always);

    builder
        .format(move |buf, record| {
            let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
            let ts = now.format(&time_fmt).unwrap();

            let ts_style = Style::new().fg_color(Some(AnsiColor::Magenta.into()));
            let pid_style = Style::new().fg_color(Some(AnsiColor::Cyan.into()));
            let lvl_style = buf.default_level_style(record.level());
            let target_style = Style::new().fg_color(Some(AnsiColor::Green.into()));

            // Only pay the backtrace cost if explicitly enabled (or for DEBUG level).
            let want_fn =
                std::env::var_os("LOG_FN").is_some() || record.level() <= log::Level::Debug;
            let who = if want_fn {
                caller_fn(record).unwrap_or_else(|| record.target().to_string())
            } else {
                record.target().to_string()
            };

            writeln!(
                buf,
                "{ts_style}{ts}{ts_style:#} \
             {pid_style}pid={pid}{pid_style:#} \
             {lvl_style}{lvl}{lvl_style:#} \
             {target_style}{who}{target_style:#}: {msg}",
                pid = std::process::id(),
                lvl = record.level(),
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
