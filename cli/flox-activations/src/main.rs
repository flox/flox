use anyhow::Context;
use clap::Parser;
use flox_activations::cli::Cli;
use flox_activations::{Error, cli, logger};
use tracing::debug;

fn main() -> Result<(), Error> {
    let args = Cli::parse();

    let logger_handle =
        logger::init_logger(args.verbosity).context("failed to initialize logger")?;
    debug!("{args:?}");

    match args.command {
        cli::Command::StartOrAttach(args) => {
            args.handle()?;
        },
        cli::Command::SetReady(args) => args.handle()?,
        cli::Command::Attach(args) => args.handle()?,
        cli::Command::Activate(args) => args.handle(logger_handle.subsystem_verbosity)?,
        cli::Command::Executive(cmd_args) => cmd_args.handle(logger_handle.reload_handle)?,
        cli::Command::FixPaths(args) => args.handle()?,
        cli::Command::SetEnvDirs(args) => args.handle()?,
        cli::Command::ProfileScripts(args) => args.handle()?,
        cli::Command::PrependAndDedup(args) => args.handle(),
        cli::Command::FixFpath(args) => args.handle(),
    }
    Ok(())
}
