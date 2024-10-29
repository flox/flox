use clap::Parser;
use cli::Cli;
use log::debug;

mod cli;

pub type Error = anyhow::Error;

fn main() -> Result<(), Error> {
    env_logger::init();

    let args = Cli::parse();
    debug!("{args:?}");

    let runtime_dir = &args.runtime_dir;

    match args.command {
        cli::Command::StartOrAttach(args) => args.handle(runtime_dir)?,
        cli::Command::SetReady(args) => args.handle(runtime_dir)?,
        cli::Command::Attach(args) => args.handle(runtime_dir)?,
    }
    Ok(())
}
