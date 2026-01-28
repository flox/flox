use anyhow::Context;
use clap::Parser;
use flox_activations::cli::Cli;
use flox_activations::{Error, cli, logger, message};
use tracing::{debug, debug_span};

fn main() {
    if let Err(e) = try_main() {
        message::error(format!("{e:#}"));
        std::process::exit(1);
    }
}

fn try_main() -> Result<(), Error> {
    let args = Cli::parse();

    if let cli::Command::Executive(executive_args) = args.command {
        return executive_args.handle(args.verbosity);
    };

    let subsystem_verbosity =
        logger::init_stderr_logger(args.verbosity).context("failed to initialize logger")?;

    // Propagate PID field to all spans.
    // We can set this eagerly because the PID doesn't change after this entry
    // point. Re-execs of activate->executive will cross this entry point again.
    let pid = std::process::id();
    let root_span = debug_span!("flox_activations", pid = pid);
    let _guard = root_span.entered();

    debug!("{args:?}");

    match args.command {
        cli::Command::Attach(args) => args.handle(),
        cli::Command::Activate(args) => args.handle(subsystem_verbosity),
        cli::Command::Executive(_) => {
            unreachable!("executive already handled above")
        },
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
