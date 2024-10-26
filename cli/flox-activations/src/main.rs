use clap::Parser;
use cli::Cli;
use log::debug;
use xdg::BaseDirectories;

mod cli;

pub type Error = anyhow::Error;

fn main() -> Result<(), Error> {
    env_logger::init();

    let args = Cli::parse();
    debug!("{args:?}");

    let runtime_dir = match args.runtime_dir {
        Some(runtime_dir) => runtime_dir,
        None => {
            let dirs = BaseDirectories::with_prefix("flox")?;
            match dirs.get_runtime_directory() {
                Ok(runtime_dir) => runtime_dir.to_path_buf(),
                Err(_) => dirs.get_cache_home().join("run"),
            }
        },
    };

    match args.command {
        cli::Command::StartOrAttach(args) => args.handle(runtime_dir)?,
        cli::Command::SetReady(args) => args.handle(runtime_dir)?,
        cli::Command::Attach(args) => args.handle(runtime_dir)?,
    }
    Ok(())
}
