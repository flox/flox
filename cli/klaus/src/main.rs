use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use futures::future::Either;
use listen::{spawn_signal_listener, spawn_termination_listener, target_pid};
use logger::init_logger;
use tracing::{debug, error, info};

mod listen;
// mod listen_orig;
mod logger;

type Error = anyhow::Error;

const SHORT_HELP: &str = "Monitors activation lifecycle to perform cleanup.";
const LONG_HELP: &str = "Monitors activation lifecycle to perform cleanup.

The watchdog (klaus) is spawned during activation to aid in service cleanup
when the final activation of an environment has terminated. This cleanup can
be manually triggered via signal (SIGUSR1), but otherwise runs automatically.";

#[derive(Debug, Parser)]
#[command(version, about = SHORT_HELP, long_about = LONG_HELP)]
pub struct Cli {
    /// The PID of the process to monitor.
    ///
    /// Note: this has no effect on Linux
    #[arg(short, long, value_name = "PID")]
    pub pid: Option<i32>,

    /// The path to the environment registry
    #[arg(short, long = "registry", value_name = "PATH")]
    pub registry_path: PathBuf,

    /// The path to the process-compose socket
    #[arg(short, long = "socket", value_name = "PATH")]
    pub socket_path: PathBuf,

    /// Where to store watchdog logs
    #[arg(short, long = "logs", value_name = "PATH")]
    pub log_path: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args = Cli::parse();
    init_logger(&args.log_path).context("failed to initialize logger")?;
    debug!("starting");
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let signal_task = spawn_signal_listener(shutdown_flag.clone())?;

    let pid = target_pid(&args);

    let termination_task = spawn_termination_listener(pid, shutdown_flag.clone());

    info!(
        this_pid = nix::unistd::getpid().as_raw(),
        target_pid = pid.as_raw(),
        "watchdog is on duty"
    );

    match futures::future::select(termination_task, signal_task).await {
        Either::Left((maybe_term_action, unresolved_signal_task)) => {
            info!("received termination, setting shutdown flag");
            shutdown_flag.store(true, Ordering::SeqCst);
            // Let the signal task shut down gracefully
            debug!("waiting for signal task to abort");
            let _ = unresolved_signal_task.await;
            match maybe_term_action {
                Ok(Ok(action)) => {
                    debug!(%action, "termination task completed successfully");
                },
                Ok(Err(err)) => {
                    error!(%err, "error encountered in termination task");
                },
                Err(err) => {
                    error!(%err, "termination task was cancelled");
                },
            }
        },
        Either::Right((maybe_signal_action, unresolved_termination_task)) => {
            info!("received signal, setting shutdown flag");
            shutdown_flag.store(true, Ordering::SeqCst);
            // Let the signal task shut down gracefully
            debug!("waiting for termination task to shut down");
            let _ = unresolved_termination_task.await;
            match maybe_signal_action {
                Ok(Ok(action)) => {
                    debug!(%action, "signal task completed successfully");
                },
                Ok(Err(err)) => {
                    error!(%err, "error encountered in signal task");
                },
                Err(err) => {
                    error!(%err, "signal task was cancelled");
                },
            }
        },
    }
    Ok(())
}
