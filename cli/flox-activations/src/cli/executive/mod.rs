use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;
use flox_core::activate::context::ActivateCtx;
use nix::sys::signal::Signal::SIGUSR1;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
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

        let args = monitoring::Args {
            dot_flox_path: context.attach_ctx.dot_flox_path.clone(),
            flox_env: context.attach_ctx.env.clone().into(),
            runtime_dir: context.attach_ctx.flox_runtime_dir.clone().into(),
            socket_path: socket_path.into(),
            log_dir: log_dir.into(),
        };
        debug!(?args, "starting monitoring loop");
        monitoring::run(args)
    }
}
