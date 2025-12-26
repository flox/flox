use std::fs::{self};
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Result, anyhow, bail};
use clap::Args;
use flox_core::activate::context::{ActivateCtx, InvocationType};
use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
use flox_core::activations::state_json_path;
use indoc::formatdoc;
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{Pid, getpid};
use serde::{Deserialize, Serialize};
use signal_hook::consts::{SIGCHLD, SIGUSR1};
use signal_hook::iterator::Signals;
use tracing::debug;

use crate::activate_script_builder::{FLOX_ENV_DIRS_VAR, assemble_command_for_start_script};
use crate::attach::{attach, quote_run_args};
use crate::cli::executive::ExecutiveCtx;
use crate::env_diff::EnvDiff;
use crate::process_compose::start_services_blocking;

pub const NO_REMOVE_ACTIVATION_FILES: &str = "_FLOX_NO_REMOVE_ACTIVATION_FILES";

#[derive(Debug, Args)]
pub struct ActivateArgs {
    /// Path to JSON file containing activation data
    #[arg(long)]
    pub activate_data: PathBuf,

    /// Additional arguments used to provide a command to run.
    /// NOTE: this is only relevant for containerize activations.
    #[arg(allow_hyphen_values = true)]
    pub cmd: Option<Vec<String>>,
}

impl ActivateArgs {
    pub fn handle(self, subsystem_verbosity: u32) -> Result<(), anyhow::Error> {
        let contents = fs::read_to_string(&self.activate_data)?;
        let mut context: ActivateCtx = serde_json::from_str(&contents)?;

        if context.remove_after_reading
            && !std::env::var(NO_REMOVE_ACTIVATION_FILES).is_ok_and(|val| val == "true")
        {
            fs::remove_file(&self.activate_data)?;
        } else {
            debug!(
                "Leaving activation context file at {:?}",
                &self.activate_data
            );
        }

        // In the case of containerize, you can't bake-in the invocation type or the
        // `run_args`, so you need to do that detection at runtime. Here we do that
        // by modifying the `ActivateCtx` passed to us in the container's
        // EntryPoint.
        let run_args = self
            .cmd
            .as_ref()
            .and_then(|args| if args.is_empty() { None } else { Some(args) });

        match (context.invocation_type.as_ref(), run_args) {
            // This is a container invocation, and we need to set the invocation type
            // based on the presence of command arguments.
            (None, None) => context.invocation_type = Some(InvocationType::Interactive),
            // This is a container invocation, and we need to set the invocation type
            // based on the presence of command arguments.
            (None, Some(args)) => {
                context.invocation_type = Some(InvocationType::ShellCommand(quote_run_args(args)));
            },
            // The following two cases are normal shell activations, and don't need
            // to modify the activation context.
            (Some(_), None) => {},
            (Some(_), Some(_)) => {},
        }
        // For any case where `invocation_type` is None, we should have detected that above
        // and set it to Some.
        let invocation_type = context
            .invocation_type
            .clone()
            .expect("invocation type should have been some");

        if let Ok(shell_force) = std::env::var("_FLOX_SHELL_FORCE") {
            context.shell = PathBuf::from(shell_force).as_path().try_into()?;
        }
        // Unset FLOX_SHELL to detect the parent shell anew with each flox invocation.
        unsafe { std::env::remove_var("FLOX_SHELL") };

        let vars_from_env = VarsFromEnvironment::get()?;

        let start_id = self.start_or_attach(
            &context,
            &invocation_type,
            subsystem_verbosity,
            &vars_from_env,
        )?;

        // Create legacy StartOrAttachResult for attach() compatibility
        let start_or_attach = Self::start_identifier_to_legacy_result(&start_id, &context)?;

        attach(
            context,
            invocation_type,
            subsystem_verbosity,
            vars_from_env,
            start_or_attach,
            start_id,
        )
    }

    /// Temporary helper to convert StartIdentifier to legacy StartOrAttachResult.
    fn start_identifier_to_legacy_result(
        start_id: &flox_core::activations::rewrite::StartIdentifier,
        context: &ActivateCtx,
    ) -> Result<crate::cli::start_or_attach::StartOrAttachResult, anyhow::Error> {
        let activation_id = format!(
            "{}.{}",
            start_id.store_path.file_name().unwrap().to_string_lossy(),
            *start_id.timestamp
        );
        let activation_state_dir = start_id.state_dir_path(
            &context.attach_ctx.flox_runtime_dir,
            &context.attach_ctx.dot_flox_path,
        )?;
        Ok(crate::cli::start_or_attach::StartOrAttachResult {
            attach: false,
            activation_state_dir,
            activation_id,
        })
    }

    fn start_or_attach(
        &self,
        context: &ActivateCtx,
        invocation_type: &InvocationType,
        subsystem_verbosity: u32,
        vars_from_env: &VarsFromEnvironment,
    ) -> Result<flox_core::activations::rewrite::StartIdentifier, anyhow::Error> {
        use flox_core::activations::rewrite::StartOrAttachResult;

        let mut retries = 30; // 30 * 200ms = 6 seconds for concurrent start blocking

        loop {
            match self.try_start_or_attach(
                context,
                invocation_type,
                subsystem_verbosity,
                vars_from_env,
            )? {
                StartOrAttachResult::Start { start_id, .. }
                | StartOrAttachResult::Attach { start_id, .. } => {
                    return Ok(start_id);
                },
                StartOrAttachResult::AlreadyStarting {
                    pid: blocking_pid, ..
                } if retries > 0 => {
                    debug!(
                        pid = blocking_pid,
                        retries = retries,
                        "Another activation is starting",
                    );

                    retries -= 1;
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    continue;
                },
                StartOrAttachResult::AlreadyStarting {
                    pid: blocking_pid, ..
                } => {
                    anyhow::bail!(
                        "Timed out waiting for concurrent activation to complete (blocked by PID {})",
                        blocking_pid
                    );
                },
            }
        }
    }

    /// Try to start or attach to an activation.
    ///
    /// Returns StartOrAttachResult indicating whether we started, attached, or should retry.
    fn try_start_or_attach(
        &self,
        context: &ActivateCtx,
        invocation_type: &InvocationType,
        subsystem_verbosity: u32,
        vars_from_env: &VarsFromEnvironment,
    ) -> Result<flox_core::activations::rewrite::StartOrAttachResult, anyhow::Error> {
        use flox_core::activations::rewrite::{
            ActivationState,
            StartOrAttachResult,
            read_activations_json,
            write_activations_json,
        };

        let activations_json_path = state_json_path(
            &context.attach_ctx.flox_runtime_dir,
            &context.attach_ctx.dot_flox_path,
        );

        let (activations_opt, lock) = read_activations_json(&activations_json_path)?;
        let mut activations =
            activations_opt.unwrap_or_else(|| ActivationState::new(&context.mode));

        // TODO: what if attached pids are dead?
        if activations.mode() != &context.mode {
            let pids = activations.attached_pids_running();
            anyhow::bail!(
                "Environment already activated in {} mode. \
                 Exit activations with PIDs {:?} to activate in {} mode.",
                activations.mode(),
                pids,
                context.mode
            );
        }

        let pid = std::process::id() as i32;
        let result = activations.start_or_attach(pid, &context.flox_activate_store_path);

        // Early return for AlreadyStarting - no write needed
        if matches!(result, StartOrAttachResult::AlreadyStarting { .. }) {
            drop(lock);
            return Ok(result);
        }

        let (needs_new_executive, start_id) = match &result {
            StartOrAttachResult::Start {
                needs_new_executive,
                start_id,
            }
            | StartOrAttachResult::Attach {
                needs_new_executive,
                start_id,
            } => (*needs_new_executive, start_id),
            _ => unreachable!(),
        };

        // Create legacy StartOrAttachResult
        let start_or_attach = Self::start_identifier_to_legacy_result(start_id, context)?;

        let new_exec_pid = if needs_new_executive {
            let exec_pid = self.spawn_executive(context, start_id)?;
            activations.set_executive_pid(exec_pid.as_raw());
            Some(exec_pid)
        } else {
            None
        };

        write_activations_json(&activations, &activations_json_path, lock)?;

        if let Some(exec_pid) = new_exec_pid {
            Self::wait_for_executive(exec_pid)?;
        }

        match &result {
            StartOrAttachResult::Start { start_id, .. } => {
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
                    // hook.on-activate may have already printed to stderr
                    bail!("Running hook.on-activate failed");
                }

                // Re-acquire lock to mark ready
                let (activations_opt, lock) = read_activations_json(&activations_json_path)?;
                let mut activations = activations_opt.expect("activations.json should exist");
                activations.set_ready(start_id);
                write_activations_json(&activations, &activations_json_path, lock)?;
            },
            StartOrAttachResult::Attach { .. } => {
                // TODO: should this be here?
                if *invocation_type == InvocationType::Interactive {
                    eprintln!(
                        "{}",
                        formatdoc! {"✅ Attached to existing activation of environment '{}'
                                 To stop using this environment, type 'exit'
                                ",
                        context.attach_ctx.env_description,
                        }
                    );
                }
            },
            StartOrAttachResult::AlreadyStarting { .. } => unreachable!(),
        }

        if context.attach_ctx.flox_activate_start_services {
            let diff = EnvDiff::from_files(&start_or_attach.activation_state_dir)?;
            start_services_blocking(
                &context.attach_ctx,
                subsystem_verbosity,
                vars_from_env.clone(),
                &start_or_attach,
                diff,
            )?;
        };

        Ok(result)
    }

    fn spawn_executive(
        &self,
        context: &ActivateCtx,
        start_id: &flox_core::activations::rewrite::StartIdentifier,
    ) -> Result<Pid, anyhow::Error> {
        let parent_pid = getpid();

        // Get activation state directory using new format
        let activation_state_dir = start_id.state_dir_path(
            &context.attach_ctx.flox_runtime_dir,
            &context.attach_ctx.dot_flox_path,
        )?;

        // Create the directory
        std::fs::create_dir_all(&activation_state_dir)?;

        // For now, create old-style StartOrAttachResult for ExecutiveCtx compatibility
        // (Will be replaced in Phase 2 with StartIdentifier)
        let activation_id = format!(
            "{}.{}",
            start_id.store_path.file_name().unwrap().to_string_lossy(),
            *start_id.timestamp
        );

        let old_start_or_attach = crate::cli::start_or_attach::StartOrAttachResult {
            attach: false,
            activation_state_dir: activation_state_dir.clone(),
            activation_id,
        };

        // Serialize ExecutiveCtx
        let executive_ctx = ExecutiveCtx {
            context: context.clone(),
            start_or_attach: old_start_or_attach,
            parent_pid: parent_pid.as_raw(),
        };

        let temp_file =
            tempfile::NamedTempFile::with_prefix_in("executive_ctx_", &activation_state_dir)?;
        serde_json::to_writer(&temp_file, &executive_ctx)?;
        let executive_ctx_path = temp_file.path().to_path_buf();
        temp_file.keep()?;

        // Spawn executive
        let mut executive = Command::new((*FLOX_ACTIVATIONS_BIN).clone());
        executive.args([
            "executive",
            "--dot-flox-path",
            &context.attach_ctx.dot_flox_path.to_string_lossy(),
            "--executive-ctx",
            &executive_ctx_path.to_string_lossy(),
        ]);

        debug!(
            "Spawning executive process to start activation: {:?}",
            executive
        );
        let child = executive.spawn()?;
        Ok(Pid::from_raw(child.id() as i32))
    }

    /// Wait for the executive to signal that it has started by sending SIGUSR1.
    /// If the executive dies, then we error.
    fn wait_for_executive(child_pid: Pid) -> Result<(), anyhow::Error> {
        debug!(
            "Awaiting SIGUSR1 from child process with PID: {}",
            child_pid
        );

        let mut signals = Signals::new([SIGCHLD, SIGUSR1])?;
        // I think the executive will always either successfully send SIGUSR1,
        // or it will exit sending SIGCHLD
        // If I'm wrong, this will loop forever
        loop {
            let pending = signals.wait();
            // We want to handle SIGUSR1 rather than SIGCHLD if both
            // are received
            // I'm not 100% confident SIGCHLD couldn't be delivered prior to
            // SIGUSR1 or SIGUSR2,
            // but I haven't seen that since switching to signals.wait() instead
            // of signals.forever()
            // If that does happen, the user would see
            // "Error: Activation process {} terminated unexpectedly"
            // which isn't a huge problem
            let signals = pending.collect::<Vec<_>>();
            // Proceed after receiving SIGUSR1
            if signals.contains(&SIGUSR1) {
                debug!(
                    "Received SIGUSR1 (executive started successfully) from child process {}",
                    child_pid
                );
                return Ok(());
            } else if signals.contains(&SIGCHLD) {
                // SIGCHLD can come from any child process, not just ours.
                // Use waitpid with WNOHANG to check if OUR child has exited.
                match waitpid(child_pid, Some(WaitPidFlag::WNOHANG)) {
                    Ok(WaitStatus::StillAlive) => {
                        // Our child is still alive, SIGCHLD was from a different process
                        debug!(
                            "Received SIGCHLD but child {} is still alive, continuing to wait",
                            child_pid
                        );
                        continue;
                    },
                    Ok(status) => {
                        // Our child has exited
                        return Err(anyhow!(
                            // TODO: we should print the path to the log file
                            "Executive {} terminated unexpectedly with status: {:?}",
                            child_pid,
                            status
                        ));
                    },
                    Err(nix::errno::Errno::ECHILD) => {
                        // Child already reaped, this shouldn't happen but handle gracefully
                        return Err(anyhow!(
                            "Executive {} terminated unexpectedly (already reaped)",
                            child_pid
                        ));
                    },
                    Err(e) => {
                        // Unexpected error from waitpid
                        return Err(anyhow!(
                            "Failed to check status of executive process {}: {}",
                            child_pid,
                            e
                        ));
                    },
                }
            } else {
                unreachable!("Received unexpected signal or empty iterator over signals");
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VarsFromEnvironment {
    pub flox_env_dirs: Option<String>,
    pub path: String,
    pub manpath: Option<String>,
}

impl VarsFromEnvironment {
    fn get() -> Result<Self> {
        let flox_env_dirs = std::env::var(FLOX_ENV_DIRS_VAR).ok();
        let path = match std::env::var("PATH") {
            Ok(path) => path,
            Err(e) => {
                return Err(anyhow!("failed to get PATH from environment: {}", e));
            },
        };
        let manpath = std::env::var("MANPATH").ok();

        Ok(Self {
            flox_env_dirs,
            path,
            manpath,
        })
    }
}
