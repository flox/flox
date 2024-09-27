use clap::Parser;
use cli::Cli;

mod cli;

type Error = anyhow::Error;

fn main() -> Result<(), Error> {
    let args = Cli::parse();
    eprintln!("{args:?}");
    match args.command {
        cli::Command::StartOrAttach(_args) => {},
        cli::Command::SetReady(_args) => {},
        cli::Command::Attach(_args) => {},
    }
    Ok(())
}
