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
        help = "The path to the runtime directory keeping activation data.\n\
                Defaults to XDG_RUNTIME_DIR/flox or XDG_CACHE_HOME/flox if not provided."
    )]
    pub runtime_dir: Option<PathBuf>,
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
    use std::path::Path;

    use flox_core::activations::{self, Activations};

    use super::*;

    pub(crate) fn write_activations<T>(
        runtime_dir: impl AsRef<Path>,
        flox_env: impl AsRef<Path>,
        f: impl FnOnce(&mut Activations) -> T,
    ) -> T {
        let activations_json_path =
            activations::activations_json_path(runtime_dir, flox_env).unwrap();
        let (activations, lock) =
            activations::read_activations_json(&activations_json_path).unwrap();
        let mut activations = activations.unwrap_or_default();

        let res = f(&mut activations);

        activations::write_activations_json(&activations, &activations_json_path, lock).unwrap();
        res
    }

    pub(crate) fn read_activations<T>(
        runtime_dir: impl AsRef<Path>,
        flox_env: impl AsRef<Path>,
        f: impl FnOnce(&Activations) -> T,
    ) -> Option<T> {
        let activations_json_path =
            activations::activations_json_path(runtime_dir, flox_env).unwrap();
        let (activations, _lock) =
            activations::read_activations_json(&activations_json_path).unwrap();
        activations.map(|activations| f(&activations))
    }

    #[test]
    fn cli_works() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
