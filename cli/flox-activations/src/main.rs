use clap::Parser;
use flox_activations::cli::Cli;
use flox_activations::{Error, cli};
use log::debug;

fn main() -> Result<(), Error> {
    env_logger::init();

    let args = Cli::parse();
    debug!("{args:?}");

    match args.command {
        cli::Command::StartOrAttach(args) => {
            args.handle()?;
        },
        cli::Command::SetReady(args) => args.handle()?,
        cli::Command::Attach(args) => args.handle()?,
        cli::Command::FixPaths(args) => args.handle()?,
        cli::Command::SetEnvDirs(args) => args.handle()?,
    }
    Ok(())
}
