use std::path::PathBuf;

use attach::AttachArgs;
use clap::{Parser, Subcommand};

pub mod activate;
pub mod attach;
pub mod executive;
mod fix_fpath;
pub mod fix_paths;
mod prepend_and_dedup;
mod profile_scripts;
pub mod set_env_dirs;
mod set_ready;
pub mod start_or_attach;

use activate::ActivateArgs;
use executive::ExecutiveArgs;
use fix_fpath::FixFpathArgs;
use fix_paths::FixPathsArgs;
use prepend_and_dedup::PrependAndDedupArgs;
use profile_scripts::ProfileScriptsArgs;
use set_env_dirs::SetEnvDirsArgs;
pub use set_ready::SetReadyArgs;
pub use start_or_attach::StartOrAttachArgs;

const SHORT_HELP: &str = "Monitors activation lifecycle to perform cleanup.";
const LONG_HELP: &str = "Monitors activation lifecycle to perform cleanup.";

#[derive(Debug, Parser)]
// #[command(version = Lazy::get(&FLOX_VERSION).map(|v| v.as_str()).unwrap_or("0.0.0"))]
#[command(about = SHORT_HELP, long_about = LONG_HELP)]
pub struct Cli {
    #[arg(
        short = 'v',
        long = "verbosity",
        help = "What level of output to display."
    )]
    pub verbosity: Option<u32>,
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
    #[command(about = "Activate a Flox environment.")]
    Activate(ActivateArgs),
    #[command(about = "Start an activation and then run the executive")]
    Executive(ExecutiveArgs),
    #[command(about = "Print sourceable output fixing PATH and MANPATH for a shell.")]
    FixPaths(FixPathsArgs),
    #[command(about = "Print sourceable output setting FLOX_ENV_DIRS.")]
    SetEnvDirs(SetEnvDirsArgs),
    #[command(about = "Print sourceable output that sources the user's profile scripts.")]
    ProfileScripts(ProfileScriptsArgs),
    #[command(
        about = "Prepends and dedups environment dirs, optionally pruning directories that aren't from environments"
    )]
    PrependAndDedup(PrependAndDedupArgs),
    #[command(about = "Print sourceable output fixing fpath/FPATH for zsh.")]
    FixFpath(FixFpathArgs),
}

/// Splits PATH-like variables into individual paths, removing any empty strings.
fn separate_dir_list(joined: &str) -> Vec<PathBuf> {
    let joined = if joined == "empty" { "" } else { joined };
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
