use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod attach;
mod set_ready;
mod start_or_attach;

pub use attach::AttachArgs;
pub use set_ready::SetReadyArgs;
pub use start_or_attach::StartOrAttachArgs;

const SHORT_HELP: &str = "Monitors activation lifecycle to perform cleanup.";
const LONG_HELP: &str = "Monitors activation lifecycle to perform cleanup.";

#[derive(Debug, Parser)]
// #[command(version = Lazy::get(&FLOX_VERSION).map(|v| v.as_str()).unwrap_or("0.0.0"))]
#[command(about = SHORT_HELP, long_about = LONG_HELP)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    #[arg(
        short,
        long,
        value_name = "PATH",
        help = "The path to the cache directory."
    )]
    pub cache_dir: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Start a new activation or attach to an existing one.")]
    StartOrAttach(StartOrAttachArgs),
    #[command(about = "Set that the activation is ready to be attached to.")]
    SetReady(SetReadyArgs),
    #[command(about = "Attach to an existing activation.")]
    Attach(AttachArgs),
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn cli_works() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
