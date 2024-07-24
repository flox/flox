use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::Context;
use clap::Parser;
use futures::StreamExt;
use logger::init_logger;
use nix::libc::{SIGINT, SIGQUIT, SIGTERM};
use signal_hook_tokio::Signals;
use tokio::task::JoinHandle;
use tracing::debug;

mod listen;
mod listen_orig;
mod logger;

static SHUTDOWN_FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();
type Error = anyhow::Error;

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

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args = Cli::parse();
    init_logger(args.log_path).context("failed to init logger")?;
    debug!("started");
    Ok(())
}

/// Takes action based on the delivery of a signal.
async fn handle_signals(mut signals: Signals, shutdown_flag: Arc<AtomicBool>) {
    while let Some(signal) = signals.next().await {
        match signal {
            SIGTERM | SIGINT | SIGQUIT => shutdown_flag.store(true, Ordering::SeqCst),
            _ => unreachable!(),
        }
    }
}

/// Spawns a task that resolves on the delivery of a signal of interest.
fn init_shutdown_handler() -> Result<JoinHandle<()>, Error> {
    let shutdown_flag = SHUTDOWN_FLAG.get_or_init(|| Arc::new(AtomicBool::new(false)));
    let signals =
        Signals::new([SIGTERM, SIGINT, SIGQUIT]).context("couldn't install signal handler")?;
    let signals_stream_handle = signals.handle();
    Ok(tokio::spawn(handle_signals(signals, shutdown_flag.clone())))
}
