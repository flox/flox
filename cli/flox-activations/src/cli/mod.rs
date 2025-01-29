use std::path::PathBuf;

use attach::AttachArgs;
use clap::{Parser, Subcommand};

pub mod attach;
mod fix_paths;
mod set_env_dirs;
mod set_ready;
mod start_or_attach;

use fix_paths::FixPathsArgs;
use set_env_dirs::SetEnvDirsArgs;
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
}

#[derive(Debug, Subcommand)]
pub enum Command {
    #[command(about = "Start a new activation or attach to an existing one.")]
    StartOrAttach(StartOrAttachArgs),
    #[command(about = "Set that the activation is ready to be attached to.")]
    SetReady(SetReadyArgs),
    #[command(about = "Attach to an existing activation.")]
    Attach(AttachArgs),
    #[command(about = "Print sourceable output fixing PATH and MANPATH for a shell.")]
    FixPaths(FixPathsArgs),
    #[command(about = "Print sourceable output setting FLOX_ENV_DIRS.")]
    SetEnvDirs(SetEnvDirsArgs),
}

/// Splits PATH-like variables into individual paths, removing any empty strings.
fn separate_dir_list(joined: &str) -> Vec<PathBuf> {
    let split = std::env::split_paths(joined).collect::<Vec<_>>();
    if (split.len() == 1) && (split[0] == PathBuf::from("")) {
        vec![]
    } else {
        split
    }
}

fn join_dir_list(dirs: Vec<PathBuf>) -> String {
    dirs.into_iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(":")
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
        let activations_json_path = activations::activations_json_path(runtime_dir, flox_env);
        let (activations, lock) =
            activations::read_activations_json(&activations_json_path).unwrap();
        let mut activations = activations
            .map(|a| a.check_version())
            .transpose()
            .unwrap()
            .unwrap_or_default();

        let res = f(&mut activations);

        activations::write_activations_json(&activations, &activations_json_path, lock).unwrap();
        res
    }

    pub(crate) fn read_activations<T>(
        runtime_dir: impl AsRef<Path>,
        flox_env: impl AsRef<Path>,
        f: impl FnOnce(&Activations) -> T,
    ) -> Option<T> {
        let activations_json_path = activations::activations_json_path(runtime_dir, flox_env);
        let (activations, _lock) =
            activations::read_activations_json(&activations_json_path).unwrap();
        activations.map(|activations| f(&activations.check_version().unwrap()))
    }

    #[test]
    fn cli_works() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
