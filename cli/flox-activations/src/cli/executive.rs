use std::path::PathBuf;
use std::{env, fs};

use anyhow::{Result, bail};
use clap::Args;
use flox_core::activate::context::{ActivateCtx, InvocationType};
use flox_core::traceable_path;
use flox_core::vars::FLOX_DISABLE_METRICS_VAR;
use nix::libc::{STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO};
use nix::sys::signal::Signal::{SIGUSR1, SIGUSR2};
use nix::sys::signal::kill;
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::activate_script_builder::assemble_command_for_start_script;
use crate::cli::activate::{NO_REMOVE_ACTIVATION_FILES, VarsFromEnvironment};
use crate::cli::start_or_attach::StartOrAttachResult;
use crate::env_diff::EnvDiff;
use crate::logger;
use crate::process_compose::start_services_blocking;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutiveCtx {
    pub context: ActivateCtx,
    pub subsystem_verbosity: u32,
    pub vars_from_env: VarsFromEnvironment,
    pub start_or_attach: StartOrAttachResult,
    pub invocation_type: InvocationType,
    pub parent_pid: i32,
}

#[derive(Debug, Args)]
pub struct ExecutiveArgs {
    /// Path to JSON file containing executive context
    #[arg(long)]
    pub executive_ctx: PathBuf,
}

impl ExecutiveArgs {
    pub fn handle(self, reload_handle: logger::ReloadHandle) -> Result<(), anyhow::Error> {
        let contents = fs::read_to_string(&self.executive_ctx)?;
        let ExecutiveCtx {
            context,
            subsystem_verbosity,
            vars_from_env,
            start_or_attach,
            invocation_type,
            parent_pid,
        } = serde_json::from_str(&contents)?;
        if !std::env::var(NO_REMOVE_ACTIVATION_FILES).is_ok_and(|val| val == "true") {
            fs::remove_file(&self.executive_ctx)?;
        }

        let mut start_command = assemble_command_for_start_script(
            context.clone(),
            subsystem_verbosity,
            vars_from_env.clone(),
            &start_or_attach,
            invocation_type.clone(),
        );
        debug!("spawning start.bash: {:?}", start_command);
        let status = start_command.spawn()?.wait()?;
        if !status.success() {
            kill(Pid::from_raw(parent_pid), SIGUSR2)?;
            // hook.on-activate may have already printed to stderr
            // We're still sharing stderr with `flox-activations activate`
            bail!("Running hook.on-activate failed");
        }
        if context.flox_activate_start_services {
            let diff = EnvDiff::from_files(&start_or_attach.activation_state_dir)?;
            let result = start_services_blocking(
                &context,
                subsystem_verbosity,
                vars_from_env,
                &start_or_attach,
                diff,
            );
            if let Err(e) = result {
                kill(Pid::from_raw(parent_pid), SIGUSR2)?;
                // We're still sharing stderr with `flox-activations activate`
                return Err(e);
            }
        };

        // Signal the parent that activation is ready
        debug!("sending SIGUSR1 to parent {}", parent_pid);
        kill(Pid::from_raw(parent_pid), SIGUSR1)?;

        // TODO: Use types to group the mutually optional fields for containers.
        if !context.run_monitoring_loop {
            debug!("monitoring loop disabled, exiting executive");
            return Ok(());
        }
        let Some(log_dir) = &context.flox_env_log_dir else {
            unreachable!("flox_env_log_dir must be set in activation context");
        };
        let Some(socket_path) = &context.flox_services_socket else {
            unreachable!("flox_services_socket must be set in activation context");
        };

        let watchdog = flox_watchdog::Cli {
            flox_env: context.env.clone().into(),
            runtime_dir: context.flox_runtime_dir.clone().into(),
            activation_id: start_or_attach.activation_id.clone(),
            socket_path: socket_path.into(),
            log_dir: log_dir.into(),
            disable_metrics: env::var(FLOX_DISABLE_METRICS_VAR).is_ok(),
        };

        // NB: If we rename this log file then we also need to update the globs
        // for GC and continue to cover the old names for a period of time.
        let log_file = format!("watchdog.{}.log", &watchdog.activation_id);
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
