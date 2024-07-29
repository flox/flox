use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, Context};
use clap::Parser;
use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::utils::{maybe_traceable_path, traceable_path};
use listen::{
    listen, signal_listener, spawn_signal_listener, spawn_termination_listener, target_pid,
};
use logger::init_logger;
use nix::errno::Errno;
use nix::sys::signal::kill;
use nix::unistd::{getpgid, getpid, setsid, Pid};
use once_cell::sync::Lazy;
use sentry::init_sentry;
use tracing::{debug, error, info, instrument};

mod listen;
mod logger;
mod sentry;

type Error = anyhow::Error;

const SHORT_HELP: &str = "Monitors activation lifecycle to perform cleanup.";
const LONG_HELP: &str = "Monitors activation lifecycle to perform cleanup.

The watchdog (klaus) is spawned during activation to aid in service cleanup
when the final activation of an environment has terminated. This cleanup can
be manually triggered via signal (SIGUSR1), but otherwise runs automatically.";

#[derive(Debug, Parser)]
#[command(version = Lazy::get(&FLOX_VERSION).map(|v| v.as_str()).unwrap_or("0.0.0"))]
#[command(about = SHORT_HELP, long_about = LONG_HELP)]
pub struct Cli {
    /// The PID of the process to monitor.
    ///
    /// Note: this has no effect on Linux
    #[arg(short, long, value_name = "PID")]
    pub pid: Option<i32>,

    /// The path to the environment registry
    #[arg(short, long = "registry", value_name = "PATH")]
    pub registry_path: PathBuf,

    /// The hash of the environment's .flox path
    #[arg(short, long = "dot-flox-hash", value_name = "DOT_FLOX_HASH")]
    pub dot_flox_hash: String,

    /// The path to the process-compose socket
    #[arg(short, long = "socket", value_name = "PATH")]
    pub socket_path: Option<PathBuf>,

    /// Where to store watchdog logs
    #[arg(short, long = "logs", value_name = "PATH")]
    pub log_path: Option<PathBuf>,
}

#[tokio::main]
#[instrument("watchdog",
    skip_all,
    fields(
        pid = tracing::field::Empty,
        registry = tracing::field::Empty,
        dot_flox_hash = tracing::field::Empty,
        socket = tracing::field::Empty,
        log = tracing::field::Empty))]
async fn main() -> Result<(), Error> {
    // Initialization
    let args = Cli::parse();
    init_logger(&args.log_path).context("failed to initialize logger")?;
    if let Err(err) = ensure_process_group_leader() {
        error!(%err, "failed to ensure watchdog is detached from terminal");
    }
    let _sentry_guard = init_sentry();
    let span = tracing::Span::current();
    span.record("pid", args.pid);
    span.record("registry", traceable_path(&args.registry_path));
    span.record("dot_flox_hash", &args.dot_flox_hash);
    span.record("socket", maybe_traceable_path(&args.socket_path));
    span.record("log", maybe_traceable_path(&args.log_path));
    if let Some(ref path) = args.socket_path {
        debug!(socket_path = traceable_path(&path), "was provided a socket");
    }

    debug!("starting");

    // The parent may have already died, in which case we just want to exit
    if let Some(ref pid) = args.pid {
        // TODO: re-use the method from ActivationPid after merging both PRs
        let parent_is_running = match kill(Pid::from_raw(*pid), None) {
            // These semantics come from kill(2).
            Ok(_) => true,              // Process received the signal and is running.
            Err(Errno::EPERM) => true,  // No permission to send a signal but we know it's running.
            Err(Errno::ESRCH) => false, // No process running to receive the signal.
            Err(_) => false,            // Unknown error, assume no running process.
        };
        if !parent_is_running {
            return Err(anyhow!("detected that watchdog had unexpected parent"));
        }
    }

    // Start the listeners
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let signal_listener = signal_listener()?;
    let signal_task = spawn_signal_listener(signal_listener, shutdown_flag.clone())?;
    let pid = target_pid(&args);
    let termination_task = spawn_termination_listener(pid, shutdown_flag.clone());

    info!(
        this_pid = nix::unistd::getpid().as_raw(),
        target_pid = pid.as_raw(),
        "watchdog is on duty"
    );

    // Listen for a notification
    let _action = listen(signal_task, termination_task, shutdown_flag).await;

    // Exit
    info!("exiting");
    Ok(())
}

/// We want to make sure that the watchdog is detached from the terminal in case it sends
/// any signals to the activation. A terminal sends signals to all processes in a process group,
/// and we want to make sure that the watchdog is in its own process group to avoid receiving any
/// signals intended for the shell.
///
/// From local testing I haven't been able to deliver signals to the watchdog by sending signals to
/// the activation, so this is more of a "just in case" measure.
fn ensure_process_group_leader() -> Result<(), Error> {
    let pid = getpid();
    // Trivia:
    // You can't create a new session if you're already a session leader, the reason being that
    // the other processes in the group aren't automatically moved to the new session. You're supposed
    // to have this invariant: all processes in a process group share the same controllling terminal.
    // If you were able to create a new session as session leader and leave behind the other processes
    // in the group in the old session, it would be possible for processes in this group to be in two
    // different sessions and therefore have two different controlling terminals.
    if pid != getpgid(None).context("failed to get process group leader")? {
        setsid().context("failed to create new session")?;
    }
    Ok(())
}
