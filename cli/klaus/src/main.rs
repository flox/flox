use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use logger::init_logger;
use tracing::debug;

mod logger;

#[derive(Debug, Parser)]
#[command()]
pub struct Cli {
    /// The PID of the process to monitor.
    ///
    /// Note: this has no effect on Linux
    #[arg(short, long)]
    pub parent_pid: Option<u32>,

    /// The path to the environment registry
    #[arg(short, long = "registry")]
    pub registry_path: PathBuf,

    /// The path to the process-compose socket
    #[arg(short, long = "socket")]
    pub socket_path: PathBuf,

    /// Where to store watchdog logs
    #[arg(short, long = "logs")]
    pub log_path: Option<PathBuf>,
}

fn main() -> Result<(), anyhow::Error> {
    let args = Cli::parse();
    init_logger(args.log_path).context("failed to init logger")?;
    debug!("started");
    Ok(())
}
