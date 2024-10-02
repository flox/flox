use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

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
}

#[derive(Debug, Args)]
pub struct StartOrAttachArgs {
    #[arg(help = "The PID of the shell registering interest in the activation.")]
    #[arg(short, long, value_name = "PID")]
    pub pid: i32,
    #[arg(help = "The path to the .flox directory for the environment.")]
    #[arg(short, long, value_name = "PATH")]
    pub flox_env: PathBuf,
    #[arg(help = "The store path of the rendered environment for this activation.")]
    #[arg(short, long, value_name = "PATH")]
    pub store_path: String,
}

#[derive(Debug, Args)]
pub struct SetReadyArgs {
    #[arg(help = "The path to the .flox directory for the environment.")]
    #[arg(short, long, value_name = "PATH")]
    pub flox_env: PathBuf,
    #[arg(help = "The UUID for this particular activation of this environment.")]
    #[arg(short, long, value_name = "UUID")]
    pub id: String,
}

#[derive(Debug, Args)]
pub struct AttachArgs {
    #[arg(help = "The PID of the shell registering interest in the activation.")]
    #[arg(short, long, value_name = "PID")]
    pub pid: i32,
    #[arg(help = "The path to the .flox directory for the environment.")]
    #[arg(short, long, value_name = "PATH")]
    pub flox_env: PathBuf,
    #[arg(help = "The UUID for this particular activation of this environment.")]
    #[arg(short, long, value_name = "UUID")]
    pub id: String,
    #[command(flatten)]
    pub exclusive: AttachExclusiveArgs,
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub struct AttachExclusiveArgs {
    #[arg(help = "How long to wait between termination of this PID and cleaning up its interest.")]
    #[arg(short, long, value_name = "TIME_MS")]
    pub timeout_ms: Option<u32>,
    #[arg(help = "Remove the specified PID when attaching to this activation.")]
    #[arg(short, long, value_name = "PID")]
    pub remove_pid: Option<i32>,
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
