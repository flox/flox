use clap::Parser;
use flox_activations::cli::Cli;
use flox_activations::logger::Verbosity;
use flox_activations::{Error, cli};
use log::debug;

fn main() -> Result<(), Error> {
    let args = Cli::parse();

    let mut builder = env_logger::Builder::default();
    if let Some(filter) = Verbosity::filter_from_env_and_arg(args.verbosity) {
        builder.parse_filters(&filter);
    }
    builder.init();
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
