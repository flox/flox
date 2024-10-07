use clap::Parser;
use cli::Cli;
use xdg::BaseDirectories;

mod activations;
mod cli;

pub type Error = anyhow::Error;

fn main() -> Result<(), Error> {
    let args = Cli::parse();

    let dirs = BaseDirectories::with_prefix("flox")?;
    let cache_dir = dirs.get_cache_home();

    eprintln!("{args:?}");
    match args.command {
        cli::Command::StartOrAttach(args) => args.handle(cache_dir)?,
        cli::Command::SetReady(args) => args.handle(cache_dir)?,
        cli::Command::Attach(args) => args.handle(cache_dir)?,
    }
    Ok(())
}
