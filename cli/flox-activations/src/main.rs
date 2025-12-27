use anyhow::Context;
use clap::Parser;
use flox_activations::cli::Cli;
use flox_activations::{Error, cli, logger};
use tracing::{debug, debug_span};

fn main() {
    if let Err(e) = try_main() {
        // TODO: should we share code with CLI formatting?
        eprintln!("❌ ERROR: {e:#}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<(), Error> {
    let args = Cli::parse();

    let logger_handle =
        logger::init_logger(args.verbosity).context("failed to initialize logger")?;

    // Propagate PID field to all spans.
    // We can set this eagerly because the PID doesn't change after this entry
    // point. Re-execs of activate->executive will cross this entry point again.
    let pid = std::process::id();
    let root_span = debug_span!("flox_activations", pid = pid);
    let _guard = root_span.entered();

    debug!("{args:?}");

    match args.command {
        cli::Command::SetReady(args) => args.handle(),
        cli::Command::Attach(args) => args.handle(),
        cli::Command::Activate(args) => args.handle(logger_handle.subsystem_verbosity),
        cli::Command::Executive(cmd_args) => cmd_args.handle(logger_handle.reload_handle),
        cli::Command::FixPaths(args) => args.handle(),
        cli::Command::SetEnvDirs(args) => args.handle(),
        cli::Command::ProfileScripts(args) => args.handle(),
        cli::Command::PrependAndDedup(args) => {
            args.handle();
            Ok(())
        },
        cli::Command::FixFpath(args) => {
            args.handle();
            Ok(())
        },
    }
}
