use clap::Parser;
use flox_activations::cli::Cli;
use flox_activations::{cli, logging, proctitle, Error};
use log::debug;

fn main() -> Result<(), Error> {
    // Initialize process title system before any forks
    proctitle::init();

    let args = Cli::parse();
    logging::init_logger(args.verbose);
    debug!("{args:?}");

    match args.command {
        cli::Command::StartOrAttach(args) => {
            args.handle()?;
        },
        cli::Command::SetReady(args) => args.handle()?,
        cli::Command::Attach(args) => args.handle()?,
        cli::Command::Activate(activate_args) => activate_args.handle(args.verbose)?,
        cli::Command::FixPaths(args) => args.handle()?,
        cli::Command::SetEnvDirs(args) => args.handle()?,
        cli::Command::ProfileScripts(args) => args.handle()?,
        cli::Command::PrependAndDedup(args) => args.handle(),
        cli::Command::FixFpath(args) => args.handle(),
    }
    Ok(())
}
