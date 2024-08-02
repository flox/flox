use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use clap::Parser;
use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::models::env_registry::{
    deregister_activation,
    register_activation,
    ActivationPid,
};
use flox_rust_sdk::providers::services::PROCESS_COMPOSE_BIN;
use flox_rust_sdk::utils::{maybe_traceable_path, traceable_path};
use logger::init_logger;
use nix::libc::{SIGINT, SIGQUIT, SIGTERM};
use nix::unistd::{getpgid, getpid, setsid};
use once_cell::sync::Lazy;
use sentry::init_sentry;
use tracing::{debug, error, info, instrument, trace};

mod logger;
mod sentry;

type Error = anyhow::Error;

const SHORT_HELP: &str = "Monitors activation lifecycle to perform cleanup.";
const LONG_HELP: &str = "Monitors activation lifecycle to perform cleanup.

The watchdog (klaus) is spawned during activation to aid in service cleanup
when the final activation of an environment has terminated. This cleanup can
be manually triggered via signal (SIGUSR1), but otherwise runs automatically.";
const CHECK_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Parser)]
#[command(version = Lazy::get(&FLOX_VERSION).map(|v| v.as_str()).unwrap_or("0.0.0"))]
#[command(about = SHORT_HELP, long_about = LONG_HELP)]
pub struct Cli {
    /// The PID of the process to monitor.
    #[arg(short, long, value_name = "PID")]
    pub pid: i32,

    /// The path to the environment registry
    #[arg(short, long = "registry", value_name = "PATH")]
    pub registry_path: PathBuf,

    /// The hash of the environment's .flox path
    #[arg(short, long = "hash", value_name = "DOT_FLOX_HASH")]
    pub dot_flox_hash: String,

    /// The path to the process-compose socket
    #[arg(short, long = "socket", value_name = "PATH")]
    pub socket_path: PathBuf,

    /// Where to store watchdog logs
    #[arg(short, long = "logs", value_name = "PATH")]
    pub log_path: Option<PathBuf>,
}

#[instrument("watchdog",
    skip_all,
    fields(
        pid = tracing::field::Empty,
        registry = tracing::field::Empty,
        dot_flox_hash = tracing::field::Empty,
        socket = tracing::field::Empty,
        log = tracing::field::Empty))]
fn main() -> Result<(), Error> {
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
    span.record("socket", traceable_path(&args.socket_path));
    span.record("log", maybe_traceable_path(&args.log_path));

    debug!("starting");

    // Set signal handlers for graceful shutdown
    let should_stop = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGINT, Arc::clone(&should_stop))
        .context("failed to set SIGINT signal handler")?;
    signal_hook::flag::register(SIGTERM, Arc::clone(&should_stop))
        .context("failed to set SIGTERM signal handler")?;
    signal_hook::flag::register(SIGQUIT, Arc::clone(&should_stop))
        .context("failed to set SIGQUIT signal handler")?;

    debug!(
        path = traceable_path(&args.socket_path),
        exists = &args.socket_path.exists(),
        "checked socket"
    );

    // Register activation PID so that we can track last one out
    let activation = ActivationPid::from(args.pid);
    if !activation.is_current_process_parent() {
        return Err(anyhow!("detected that watchdog had unexpected parent"));
    }
    register_activation(&args.registry_path, &args.dot_flox_hash, activation)?;

    info!(
        this_pid = nix::unistd::getpid().as_raw(),
        target_pid = args.pid,
        "watchdog is on duty"
    );

    // Wait for the target process to terminate
    loop {
        trace!(pid = args.pid, "checking whether process is alive");
        if !activation.is_running() {
            info!("target process terminated");
            break;
        }
        // If we got a SIGINT/SIGTERM/SIGQUIT we exit gracefully and leave the activation in the registry,
        // but there's not much we can do about that because we don't know who sent us the signal
        // or why. If this is the last activation, then services won't get cleaned up. If it's _not_
        // the last activation, then we're fine because another watchdog will remove any stale entries
        // from the registry and eventually the services will get cleaned up.
        if should_stop.load(Ordering::SeqCst) {
            info!("received signal, exiting");
            return Ok(());
        }
        std::thread::sleep(CHECK_INTERVAL);
    }

    // Now we proceed assuming that the target process terminated.
    let remaining_activations =
        deregister_activation(&args.registry_path, &args.dot_flox_hash, activation)
            .context("failed to deregister activation")?;
    debug!(n = remaining_activations, "remaining activations");
    if remaining_activations == 0 {
        if args.socket_path.exists() {
            let mut cmd = Command::new(&*PROCESS_COMPOSE_BIN);
            cmd.arg("down");
            cmd.arg("--unix-socket");
            cmd.arg(&args.socket_path);
            cmd.env("NO_COLOR", "1");
            match cmd.output() {
                Ok(output) => {
                    if !output.status.success() {
                        error!(
                            code = output.status.code(),
                            "failed to run process-compose shutdown command"
                        );
                    }
                },
                Err(err) => {
                    error!(%err, "failed to run process-compose shutdown command");
                },
            }
        } else {
            debug!(reason = "no socket", "did not shut down process-compose");
        }
    } else {
        debug!(
            reason = "remaining activations",
            "did not shut down process-compose"
        );
    }

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
