use clap::Parser;
use flox_activations::cli::Cli;
use flox_activations::logger::Verbosity;
use flox_activations::{Error, cli};
use log::debug;

fn main() -> Result<(), Error> {
    let args = Cli::parse();

    let verbosity = Verbosity::from(args.verbosity);
    env_logger::Builder::default()
        .parse_filters(verbosity.env_filter())
        .init();
    debug!("{args:?}");

    match args.command {
        cli::Command::StartOrAttach(args) => {
            args.handle()?;
        },
        cli::Command::SetReady(args) => args.handle()?,
        cli::Command::Attach(args) => args.handle()?,
        cli::Command::Activate(args) => args.handle()?,
        cli::Command::FixPaths(args) => args.handle()?,
        cli::Command::SetEnvDirs(args) => args.handle()?,
        cli::Command::ProfileScripts(args) => args.handle()?,
        cli::Command::PrependAndDedup(args) => args.handle(),
        cli::Command::FixFpath(args) => args.handle(),
    }
    Ok(())
}
