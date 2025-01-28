use clap::Parser;
use flox_activations::cli::Cli;
use flox_activations::{cli, Error};
use log::debug;

fn main() -> Result<(), Error> {
    env_logger::init();

    let args = Cli::parse();
    debug!("{args:?}");

    let runtime_dir = &args.runtime_dir;

    match args.command {
        cli::Command::StartOrAttach(args) => {
            args.handle(runtime_dir)?;
        },
        cli::Command::SetReady(args) => args.handle(runtime_dir)?,
        cli::Command::Attach(args) => args.handle(runtime_dir)?,
        cli::Command::FixPaths(args) => args.handle()?,
        cli::Command::SetEnvDirs(args) => args.handle()?,
    }
    Ok(())
}
