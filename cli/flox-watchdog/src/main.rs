use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use clap::Parser;
use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::models::env_registry::{
    deregister_activation,
    read_environment_registry,
    register_activation,
    ActivationPid,
    EnvRegistryError,
};
use flox_rust_sdk::providers::services::process_compose_down;
use flox_rust_sdk::utils::{maybe_traceable_path, traceable_path};
use logger::init_logger;
use nix::libc::{SIGINT, SIGQUIT, SIGTERM, SIGUSR1};
use nix::unistd::{getpgid, getpid, setsid};
use once_cell::sync::Lazy;
use sentry::init_sentry;
use tracing::{debug, error, info, instrument};

mod logger;
mod sentry;

type Error = anyhow::Error;

const SHORT_HELP: &str = "Monitors activation lifecycle to perform cleanup.";
const LONG_HELP: &str = "Monitors activation lifecycle to perform cleanup.

The watchdog (fka. klaus) is spawned during activation to aid in service cleanup
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

    /// The directory to store logs
    #[arg(short, long = "log-dir", value_name = "PATH")]
    pub log_dir: Option<PathBuf>,

    /// Disable metric reporting
    #[arg(long)]
    pub disable_metrics: bool,
}

#[instrument("watchdog",
    skip_all,
    fields(
        pid = tracing::field::Empty,
        registry = tracing::field::Empty,
        dot_flox_hash = tracing::field::Empty,
        socket = tracing::field::Empty,
        log_dir = tracing::field::Empty))]
fn main() -> Result<(), Error> {
    // Initialization
    let args = Cli::parse();
    let log_file = &args
        .log_dir
        .as_ref()
        .map(|dir| dir.join(format!("watchdog.{}.log", args.pid)));
    init_logger(log_file).context("failed to initialize logger")?;
    if let Err(err) = ensure_process_group_leader() {
        error!(%err, "failed to ensure watchdog is detached from terminal");
    }
    let _sentry_guard = (!args.disable_metrics).then(init_sentry);
    let span = tracing::Span::current();
    span.record("pid", args.pid);
    span.record("registry", traceable_path(&args.registry_path));
    span.record("dot_flox_hash", &args.dot_flox_hash);
    span.record("socket", traceable_path(&args.socket_path));
    span.record("log_dir", maybe_traceable_path(&args.log_dir));
    debug!("starting");

    // Set the signal handler
    let should_clean_up = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGUSR1, Arc::clone(&should_clean_up))
        .context("failed to set SIGUSR1 signal handler")?;
    let should_terminate = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGINT, Arc::clone(&should_terminate))
        .context("failed to set SIGINT signal handler")?;
    signal_hook::flag::register(SIGTERM, Arc::clone(&should_terminate))
        .context("failed to set SIGTERM signal handler")?;
    signal_hook::flag::register(SIGQUIT, Arc::clone(&should_terminate))
        .context("failed to set SIGQUIT signal handler")?;

    // Ensure that we'll get sent SIGUSR1 on Linux when the parent terminates
    #[cfg(target_os = "linux")]
    nix::sys::prctl::set_pdeathsig(Some(nix::sys::signal::Signal::SIGUSR1))
        .context("set_pdeathsig failed")?;

    #[cfg(target_os = "macos")]
    let watcher = {
        let mut watcher = kqueue::Watcher::new()?;
        watcher.add_pid(
            args.pid,
            kqueue::EventFilter::EVFILT_PROC,
            kqueue::FilterFlag::NOTE_EXIT,
        )?;
        watcher.watch().context("failed to register watcher")?;
        watcher
    };
    debug!("registered termination interest");

    debug!(
        path = traceable_path(&args.socket_path),
        exists = &args.socket_path.exists(),
        "checked socket"
    );

    // Register activation PID so that we can track last one out
    let activation = ActivationPid::from(args.pid);
    // Check if our original parent has already exited.
    // If it has, we can go ahead and cleanup.
    // Note that this check should come after we call prctl or kqueue so that
    // there isn't a race between checking ppid and calling prctl or kqueue.
    if !activation.is_current_process_parent() {
        debug!("parent has already exited");
        let reg = read_environment_registry(&args.registry_path)?.unwrap_or_default();
        let entry = reg
            .entry_for_hash(&args.dot_flox_hash)
            .ok_or(EnvRegistryError::UnknownKey(args.dot_flox_hash))?;
        if entry.activations.is_empty() {
            cleanup(args.socket_path);
        }
        return Ok(());
    }
    register_activation(&args.registry_path, &args.dot_flox_hash, activation)?;

    info!(
        this_pid = nix::unistd::getpid().as_raw(),
        target_pid = args.pid,
        "watchdog is on duty"
    );

    // Listen for a notification, getting an error if we should terminate
    #[cfg(target_os = "macos")]
    let res = wait_for_termination(watcher, should_clean_up, should_terminate);

    #[cfg(target_os = "linux")]
    let res = wait_for_termination(should_clean_up, should_terminate);

    // If we get a SIGINT/SIGTERM/SIGQUIT/SIGKILL we leave behind the activation in the registry,
    // but there's not much we can do about that because we don't know who sent us one of those
    // signals or why.
    if res.is_err() {
        error!("received stop signal, exiting");
        return res;
    }

    // Now we proceed with cleanup assuming we've gotten a notification that the target process
    // has terminated.

    let remaining_activations =
        deregister_activation(&args.registry_path, &args.dot_flox_hash, activation)
            .context("failed to deregister activation")?;
    debug!(n = remaining_activations, "remaining activations");
    if remaining_activations == 0 {
        cleanup(args.socket_path);
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

// If the activation for a watchdog gets removed from the registry as stale by a different watchdog,
// multiple watchdogs could perform cleanup.
// The following can be run multiple times without issue.
fn cleanup(socket_path: impl AsRef<Path>) {
    debug!("running cleanup");
    let socket_path = socket_path.as_ref();
    if socket_path.exists() {
        if let Err(err) = process_compose_down(socket_path) {
            error!(%err, "failed to run process-compose shutdown command");
        }
    } else {
        debug!(reason = "no socket", "did not shut down process-compose");
    }
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

#[cfg(target_os = "macos")]
fn wait_for_termination(
    watcher: kqueue::Watcher,
    proceed_flag: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
) -> Result<(), Error> {
    loop {
        if proceed_flag.load(std::sync::atomic::Ordering::SeqCst) {
            debug!("observed proceed flag");
            break Ok(());
        }
        if stop_flag.load(std::sync::atomic::Ordering::SeqCst) {
            break Err(anyhow!("received stop signal"));
        }
        if let Some(_event) = watcher.poll(None) {
            debug!("received termination event, will proceed");
            break Ok(());
        }
        std::thread::sleep(CHECK_INTERVAL);
    }
}

#[cfg(target_os = "linux")]
fn wait_for_termination(
    proceed_flag: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
) -> Result<(), Error> {
    loop {
        if proceed_flag.load(std::sync::atomic::Ordering::SeqCst) {
            debug!("observed flag, will proceed");
            break Ok(());
        }
        if stop_flag.load(std::sync::atomic::Ordering::SeqCst) {
            break Err(anyhow!("received stop signal"));
        }
        std::thread::sleep(CHECK_INTERVAL);
    }
}
