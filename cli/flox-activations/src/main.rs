use std::io::Write;

use clap::Parser;
use flox_activations::cli::Cli;
use flox_activations::{activate, cli, Error};
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
        cli::Command::ActivatePhase1(args) => {
            let buffer = activate::phase_one(&args)?;
            let mut stdout = std::io::stdout();
            stdout.write_all(&buffer)?;
        },
    }
    Ok(())
}
