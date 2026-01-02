use std::path::PathBuf;
use std::{env, fs, process};

use anyhow::Result;
use clap::Args;
use flox_core::activate::context::ActivateCtx;
use flox_core::traceable_path;
use flox_core::vars::FLOX_DISABLE_METRICS_VAR;
#[cfg(target_os = "linux")]
use flox_watchdog::reaper::linux::SubreaperGuard;
use nix::libc::{STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO};
use nix::sys::signal::Signal::SIGUSR1;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::cli::activate::NO_REMOVE_ACTIVATION_FILES;
use crate::cli::start_or_attach::StartOrAttachResult;
use crate::logger;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutiveCtx {
    pub context: ActivateCtx,
    pub start_or_attach: StartOrAttachResult,
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
    pub fn handle(self, reload_handle: logger::ReloadHandle) -> Result<(), anyhow::Error> {
        let contents = fs::read_to_string(&self.executive_ctx)?;
        let ExecutiveCtx {
            context,
            start_or_attach,
            parent_pid,
        } = serde_json::from_str(&contents)?;
        if !std::env::var(NO_REMOVE_ACTIVATION_FILES).is_ok_and(|val| val == "true") {
            fs::remove_file(&self.executive_ctx)?;
        }

        // Set as subreaper immediately. The guard ensures cleanup on all exit paths.
        #[cfg(target_os = "linux")]
        let _subreaper_guard = SubreaperGuard::new()?;

        // Signal the parent that the executive is ready
        debug!("sending SIGUSR1 to parent {}", parent_pid);
        kill(Pid::from_raw(parent_pid), SIGUSR1)?;

        // TODO: Use types to group the mutually optional fields for containers.
        if !context.run_monitoring_loop {
            debug!("monitoring loop disabled, exiting executive");
            return Ok(());
        }
        let Some(log_dir) = &context.attach_ctx.flox_env_log_dir else {
            unreachable!("flox_env_log_dir must be set in activation context");
        };
        let Some(socket_path) = &context.attach_ctx.flox_services_socket else {
            unreachable!("flox_services_socket must be set in activation context");
        };

        let watchdog = flox_watchdog::Cli {
            dot_flox_path: context.attach_ctx.dot_flox_path.clone(),
            flox_env: context.attach_ctx.env.clone().into(),
            runtime_dir: context.attach_ctx.flox_runtime_dir.clone().into(),
            socket_path: socket_path.into(),
            log_dir: log_dir.into(),
            disable_metrics: env::var(FLOX_DISABLE_METRICS_VAR).is_ok(),
        };

        let log_file = format!("executive.{}.log", process::id());
        debug!(
            log_dir = traceable_path(&watchdog.log_dir),
            log_file, "switching to file logging"
        );
        logger::switch_to_file_logging(reload_handle, log_file, log_dir)?;

        // Close stdin, stdout, stderr to detach from terminal
        for fd in &[STDIN_FILENO, STDOUT_FILENO, STDERR_FILENO] {
            let _ = nix::unistd::close(*fd);
        }

        // TODO: Enable earlier in `flox-activations` rather than just when detached?
        // TODO: Re-enable sentry after fixing OpenSSL dependency issues
        // let _sentry_guard = (!watchdog.disable_metrics).then(flox_watchdog::init_sentry);

        debug!(watchdog = ?watchdog, "starting watchdog");
        flox_watchdog::run(watchdog)
    }
}
