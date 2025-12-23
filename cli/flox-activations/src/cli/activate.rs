use std::fs::{self};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Result, anyhow, bail};
use clap::Args;
use flox_core::activate::context::{ActivateCtx, InvocationType};
use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
use indoc::formatdoc;
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{Pid, getpid};
use serde::{Deserialize, Serialize};
use signal_hook::consts::{SIGCHLD, SIGUSR1};
use signal_hook::iterator::Signals;
use tracing::debug;

use super::StartOrAttachArgs;
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

        let start_or_attach = StartOrAttachArgs {
            pid: std::process::id() as i32,
            dot_flox_path: context.attach_ctx.dot_flox_path.clone(),
            store_path: context.flox_activate_store_path.clone(),
            runtime_dir: PathBuf::from(&context.attach_ctx.flox_runtime_dir),
        }
        .handle_inner()?;

        let vars_from_env = VarsFromEnvironment::get()?;

        if start_or_attach.attach {
            debug!(
                "Attaching to existing activation in state dir {:?}, id {}",
                start_or_attach.activation_state_dir, start_or_attach.activation_id
            );
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
            if invocation_type == InvocationType::Interactive {
                eprintln!(
                    "{}",
                    formatdoc! {"✅ Attached to existing activation of environment '{}'
                             To stop using this environment, type 'exit'
                            ",
                    context.attach_ctx.env_description,
                    }
                );
            }
        } else {
            let parent_pid = getpid();

            // Serialize ExecutiveCtx before forking
            let executive_ctx = ExecutiveCtx {
                context: context.clone(),
                start_or_attach: start_or_attach.clone(),
                parent_pid: parent_pid.as_raw(),
            };

            let temp_file = tempfile::NamedTempFile::with_prefix_in(
                "executive_ctx_",
                &start_or_attach.activation_state_dir,
            )?;
            serde_json::to_writer(&temp_file, &executive_ctx)?;
            let executive_ctx_path = temp_file.path().to_path_buf();
            temp_file.keep()?;

            let mut executive = Command::new((*FLOX_ACTIVATIONS_BIN).clone());
            executive.args([
                "executive",
                "--dot-flox-path",
                &context.attach_ctx.dot_flox_path.to_string_lossy(),
                "--executive-ctx",
                &executive_ctx_path.to_string_lossy(),
            ]);
            // The executive daemonizes so shouldn't need stdin/stdout/stderr
            executive
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            debug!(
                "Spawning executive process to start activation: {:?}",
                executive
            );
            let child = executive.spawn()?;
            Self::wait_for_executive(Pid::from_raw(child.id() as i32))?;

            let mut start_command = assemble_command_for_start_script(
                context.clone(),
                subsystem_verbosity,
                vars_from_env.clone(),
                &start_or_attach,
                invocation_type.clone(),
            );
            debug!("spawning start.bash: {:?}", start_command);
            let status = start_command.status()?;
            if !status.success() {
                // hook.on-activate may have already printed to stderr
                bail!("Running hook.on-activate failed");
            }
            if context.attach_ctx.flox_activate_start_services {
                // TODO: replace with signal executive
                let diff = EnvDiff::from_files(&start_or_attach.activation_state_dir)?;
                start_services_blocking(
                    &context.attach_ctx,
                    subsystem_verbosity,
                    vars_from_env.clone(),
                    &start_or_attach,
                    diff,
                )?;
            };
        }

        attach(
            context,
            invocation_type,
            subsystem_verbosity,
            vars_from_env,
            start_or_attach,
        )
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
