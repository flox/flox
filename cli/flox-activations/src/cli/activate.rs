use std::fs::{self};
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::Args;
use flox_core::activate::context::{ActivateCtx, InvocationType};
use indoc::formatdoc;
use log::debug;
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{ForkResult, Pid, fork, getpid};
use signal_hook::consts::{SIGCHLD, SIGUSR1};
use signal_hook::iterator::Signals;

use super::StartOrAttachArgs;
use crate::activate_script_builder::FLOX_ENV_DIRS_VAR;
use crate::attach::attach;
use crate::executive::executive;

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
        }

        // In the case of containerize, you can't bake-in the invocation type or the
        // `run_args`, so you need to do that detection at runtime. Here we do that
        // by modifying the `ActivateCtx` passed to us in the container's
        // EntryPoint.
        let run_args = self
            .cmd
            .as_ref()
            .or(Some(&context.run_args))
            .and_then(|args| if args.is_empty() { None } else { Some(args) });

        match (context.invocation_type.as_ref(), run_args) {
            // This is a container invocation, and we need to set the invocation type
            // based on the presence of command arguments.
            (None, None) => context.invocation_type = Some(InvocationType::Interactive),
            // This is a container invocation, and we need to set the invocation type
            // based on the presence of command arguments.
            (None, Some(args)) => {
                context.invocation_type = Some(InvocationType::Command);
                context.run_args = args.clone();
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
            .expect("invocation type should have been some");

        if let Ok(shell_force) = std::env::var("_FLOX_SHELL_FORCE") {
            context.shell = PathBuf::from(shell_force).as_path().try_into()?;
        }
        // Unset FLOX_SHELL to detect the parent shell anew with each flox invocation.
        unsafe { std::env::remove_var("FLOX_SHELL") };

        let start_or_attach = StartOrAttachArgs {
            pid: std::process::id() as i32,
            flox_env: PathBuf::from(&context.env),
            store_path: context.flox_activate_store_path.clone(),
            runtime_dir: PathBuf::from(&context.flox_runtime_dir),
        }
        .handle_inner()?;

        let vars_from_env = VarsFromEnvironment::get()?;

        if start_or_attach.attach {
            debug!(
                "Attaching to existing activation in state dir {:?}, id {}",
                start_or_attach.activation_state_dir, start_or_attach.activation_id
            );
            if invocation_type == InvocationType::Interactive {
                eprintln!(
                    "{}",
                    formatdoc! {"✅ Attached to existing activation of environment '{}'
                             To stop using this environment, type 'exit'
                            ",
                    context.env_description,
                    }
                );
            }
        } else {
            let parent_pid = getpid();
            match unsafe { fork() } {
                Ok(ForkResult::Child) => {
                    #[cfg(target_os = "linux")]
                    unsafe {
                        setsid();
                        // register as subreaper on Linux only
                        prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);
                    }
                    return executive(
                        context,
                        subsystem_verbosity,
                        vars_from_env,
                        start_or_attach,
                        invocation_type,
                        parent_pid,
                    );
                },
                Ok(ForkResult::Parent { child: child_pid }) => Self::wait_for_start(child_pid)?,
                Err(e) => {
                    // Fork failed
                    return Err(anyhow!("Fork failed: {}", e));
                },
            }
            debug!("Finished spawning activation - proceeding to attach");
        }

        attach(
            context,
            invocation_type,
            subsystem_verbosity,
            vars_from_env,
            start_or_attach,
        )
    }

    // Wait for the executive to start the activation, mark it ready, and send SIGUSR1.
    fn wait_for_start(child_pid: Pid) -> Result<(), anyhow::Error> {
        debug!(
            "Awaiting SIGUSR1 from child process with PID: {}",
            child_pid
        );

        // Set up signal handler to await the death of the child.
        // If the child dies, then we should error out. We expect
        // to receive SIGUSR1 from the child when it's ready.
        let mut signals = Signals::new([SIGCHLD, SIGUSR1])?;
        for signal in signals.forever() {
            match signal {
                SIGUSR1 => {
                    debug!("Received SIGUSR1 from child process {}", child_pid);
                    return Ok(()); // Proceed after receiving SIGUSR1
                },
                SIGCHLD => {
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
                                "Activation process {} terminated unexpectedly with status: {:?}",
                                child_pid,
                                status
                            ));
                        },
                        Err(nix::errno::Errno::ECHILD) => {
                            // Child already reaped, this shouldn't happen but handle gracefully
                            return Err(anyhow!(
                                "Activation process {} terminated unexpectedly (already reaped)",
                                child_pid
                            ));
                        },
                        Err(e) => {
                            // Unexpected error from waitpid
                            return Err(anyhow!(
                                "Failed to check status of activation process {}: {}",
                                child_pid,
                                e
                            ));
                        },
                    }
                },
                _ => unreachable!(),
            }
        }

        unreachable!();
    }
}

#[derive(Clone, Debug)]
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
