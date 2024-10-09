use clap::Parser;
use cli::Cli;
use xdg::BaseDirectories;

mod activations;
mod cli;

pub type Error = anyhow::Error;

fn main() -> Result<(), Error> {
    let args = Cli::parse();
    eprintln!("{args:?}");

    let cache_dir = match args.cache_dir {
        Some(cache_dir) => cache_dir,
        None => {
            let dirs = BaseDirectories::with_prefix("flox")?;
            dirs.create_cache_directory("")?
        },
    };

    match args.command {
        cli::Command::StartOrAttach(args) => args.handle(cache_dir)?,
        cli::Command::SetReady(args) => args.handle(cache_dir)?,
        cli::Command::Attach(args) => args.handle(cache_dir)?,
    }
    Ok(())
}
