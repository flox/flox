use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context, Result};
use clap::Args;
use flox_core::activate::context::ActivateCtx;
use log_gc::{spawn_heartbeat_log, spawn_logs_gc_threads};
use nix::libc::{SIGCHLD, SIGINT, SIGQUIT, SIGTERM};
use nix::sys::signal::Signal::SIGUSR1;
use nix::sys::signal::kill;
use nix::unistd::{Pid, getpgid, getpid, setsid};
use serde::{Deserialize, Serialize};
use signal_hook::iterator::Signals;
use tracing::{debug, debug_span};

use crate::cli::activate::NO_REMOVE_ACTIVATION_FILES;
use crate::logger;

mod log_gc;
mod monitoring;
mod reaper;
mod watcher;
// TODO: Re-enable sentry after fixing OpenSSL dependency issues
// mod sentry;

#[cfg(target_os = "linux")]
use reaper::linux::SubreaperGuard;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutiveCtx {
    pub context: ActivateCtx,
    pub parent_pid: i32,
}

#[derive(Debug, Args)]
pub struct ExecutiveArgs {
    /// .flox directory path
    // This isn't consumed and serves only to identify in process listings which
    // environment the executive is responsible for.
    #[arg(long)]
    pub dot_flox_path: PathBuf,

    /// Path to JSON file containing executive context
    #[arg(long)]
    pub executive_ctx: PathBuf,
}

impl ExecutiveArgs {
    pub fn handle(self, subsystem_verbosity: Option<u32>) -> Result<(), anyhow::Error> {
        let contents = fs::read_to_string(&self.executive_ctx)?;
        let ExecutiveCtx {
            context,
            parent_pid,
        } = serde_json::from_str(&contents)?;
        if !std::env::var(NO_REMOVE_ACTIVATION_FILES).is_ok_and(|val| val == "true") {
            fs::remove_file(&self.executive_ctx)?;
        }

        // Set as subreaper immediately. The guard ensures cleanup on all exit paths.
        #[cfg(target_os = "linux")]
        let _subreaper_guard = SubreaperGuard::new()?;

        // Ensure the executive is detached from the terminal
        ensure_process_group_leader()
            .context("failed to ensure executive is detached from terminal")?;

        // Signal the parent that the executive is ready
        debug!("sending SIGUSR1 to parent {}", parent_pid);
        kill(Pid::from_raw(parent_pid), SIGUSR1)?;

        let Some(log_dir) = &context.attach_ctx.flox_env_log_dir else {
            unreachable!("flox_env_log_dir must be set in activation context");
        };
        let log_file = format!("executive.{}.log", std::process::id());
        logger::init_file_logger(subsystem_verbosity, log_file, log_dir)
            .context("failed to initialize logger")?;

        // Propagate PID field to all spans.
        // We can set this eagerly because the PID doesn't change after this entry
        // point. Re-execs of activate->executive will cross this entry point again.
        let pid = std::process::id();
        let root_span = debug_span!("flox_activations_executive", pid = pid);
        let _guard = root_span.entered();

        debug!("{self:?}");

        // TODO: Enable earlier in `flox-activations` rather than just when detached?
        // TODO: Re-enable sentry after fixing OpenSSL dependency issues
        // let disable_metrics = env::var(FLOX_DISABLE_METRICS_VAR).is_ok();
        // let _sentry_guard = (!disable_metrics).then(sentry::init_sentry);

        // TODO: Use types to group the mutually optional fields for containers.
        if !context.run_monitoring_loop {
            debug!("monitoring loop disabled, exiting executive");
            return Ok(());
        }
        let Some(socket_path) = &context.attach_ctx.flox_services_socket else {
            unreachable!("flox_services_socket must be set in activation context");
        };

        spawn_heartbeat_log();
        spawn_logs_gc_threads(log_dir);

        // Set up signal handlers just before entering the monitoring loop.
        // Doing this too early could prevent the executive from being killed if
        // it gets stuck.
        let should_clean_up = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(nix::libc::SIGUSR1, Arc::clone(&should_clean_up))
            .context("failed to set SIGUSR1 signal handler")?;
        let should_terminate = Arc::new(AtomicBool::new(false));
        signal_hook::flag::register(SIGINT, Arc::clone(&should_terminate))
            .context("failed to set SIGINT signal handler")?;
        signal_hook::flag::register(SIGTERM, Arc::clone(&should_terminate))
            .context("failed to set SIGTERM signal handler")?;
        signal_hook::flag::register(SIGQUIT, Arc::clone(&should_terminate))
            .context("failed to set SIGQUIT signal handler")?;
        // This compliments the SubreaperGuard setup above.
        // WARNING: You cannot reliably use Command::wait after we've entered the
        // monitoring loop, including concurrent threads like GCing logs, because
        // children will be reaped automatically.
        let should_reap = Signals::new([SIGCHLD])?;

        let args = monitoring::Args {
            dot_flox_path: context.attach_ctx.dot_flox_path.clone(),
            runtime_dir: context.attach_ctx.flox_runtime_dir.clone().into(),
            socket_path: socket_path.into(),
        };
        debug!(?args, "starting monitoring loop");
        monitoring::run_monitoring_loop(args, should_terminate, should_clean_up, should_reap)
    }
}

/// Ensures the executive is detached from the terminal by becoming a process group leader.
///
/// We want to make sure that the executive is detached from the terminal in case it sends
/// any signals to the activation. A terminal sends signals to all processes in a process group,
/// and we want to make sure that the executive is in its own process group to avoid receiving any
/// signals intended for the shell.
///
/// From local testing I haven't been able to deliver signals to the executive by sending signals to
/// the activation, so this is more of a "just in case" measure.
fn ensure_process_group_leader() -> Result<(), anyhow::Error> {
    let pid = getpid();
    // Trivia:
    // You can't create a new session if you're already a session leader, the reason being that
    // the other processes in the group aren't automatically moved to the new session. You're supposed
    // to have this invariant: all processes in a process group share the same controlling terminal.
    // If you were able to create a new session as session leader and leave behind the other processes
    // in the group in the old session, it would be possible for processes in this group to be in two
    // different sessions and therefore have two different controlling terminals.
    if pid != getpgid(None).context("failed to get process group leader")? {
        setsid().context("failed to create new session")?;
    }
    Ok(())
}
