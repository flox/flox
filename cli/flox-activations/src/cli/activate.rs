use std::fs::{self, DirBuilder};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use clap::Args;
use flox_core::activate::context::{ActivateCtx, InvocationType};
use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
use flox_core::activations::{
    ActivationState,
    ModeMismatch,
    StartIdentifier,
    StartOrAttachResult,
    read_activations_json,
    state_json_path,
    write_activations_json,
};
use fslock::LockFile;
use indoc::formatdoc;
use nix::sys::signal::{Signal, kill};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{Pid, getpid};
use serde::{Deserialize, Serialize};
use signal_hook::consts::{SIGCHLD, SIGUSR1};
use signal_hook::iterator::Signals;
use tracing::{debug, error};

use crate::activate_script_builder::{FLOX_ENV_DIRS_VAR, assemble_activate_command};
use crate::attach::{attach, quote_run_args};
use crate::cli::executive::ExecutiveCtx;
use crate::message::updated;
use crate::process_compose::{
    process_compose_down,
    start_services_via_socket,
    wait_for_socket_ready,
};

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

        if !context.attach_ctx.services_to_start.is_empty() {
            let socket_path = context
                .attach_ctx
                .flox_services_socket
                .as_ref()
                .expect("flox_services_socket must be set to start services");
            let process_compose_bin = context
                .attach_ctx
                .process_compose_bin
                .as_ref()
                .expect("process_compose_bin must be set to start services");
            Self::start_services_with_new_process_compose(
                &context.attach_ctx.flox_runtime_dir,
                &context.attach_ctx.dot_flox_path,
                // Unwrapped values that shouldn't be taken from context again.
                process_compose_bin,
                socket_path,
                &context.attach_ctx.services_to_start,
            )?;
        }

        attach(
            context,
            invocation_type,
            subsystem_verbosity,
            vars_from_env,
            start_id,
        )
    }

    fn start_or_attach(
        &self,
        context: &ActivateCtx,
        invocation_type: &InvocationType,
        subsystem_verbosity: u32,
        vars_from_env: &VarsFromEnvironment,
    ) -> Result<StartIdentifier, anyhow::Error> {
        let retry_delay = Duration::from_millis(200);
        let warning_interval = Duration::from_secs(5);
        let mut last_warning: Option<Instant> = None;

        loop {
            match self.try_start_or_attach(context, subsystem_verbosity, vars_from_env)? {
                StartOrAttachResult::Start { start_id, .. } => {
                    if *invocation_type == InvocationType::Interactive {
                        updated(
                            formatdoc! {"You are now using the environment '{env_description}'
                                     To stop using this environment, type 'exit'
                                     ",
                            env_description = context.attach_ctx.env_description,
                            },
                        );
                    }
                    return Ok(start_id);
                },
                StartOrAttachResult::Attach { start_id, .. } => {
                    if *invocation_type == InvocationType::Interactive {
                        updated(
                            formatdoc! {"Attached to existing activation of environment '{env_description}'
                                     To stop using this environment, type 'exit'
                                     ",
                            env_description = context.attach_ctx.env_description,
                            },
                        );
                    }
                    return Ok(start_id);
                },
                StartOrAttachResult::AlreadyStarting {
                    pid: blocking_pid, ..
                } => {
                    let now = Instant::now();
                    let should_warn =
                        last_warning.is_none_or(|t| now.duration_since(t) >= warning_interval);

                    if should_warn {
                        eprintln!(
                            "⚠️  Waiting for another activation to complete (blocked by PID {})...",
                            blocking_pid
                        );
                        last_warning = Some(now);
                    }

                    std::thread::sleep(retry_delay);
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
        subsystem_verbosity: u32,
        vars_from_env: &VarsFromEnvironment,
    ) -> Result<StartOrAttachResult, anyhow::Error> {
        let activations_json_path = state_json_path(
            &context.attach_ctx.flox_runtime_dir,
            &context.attach_ctx.dot_flox_path,
        );

        let (activations_opt, lock) = read_activations_json(&activations_json_path)?;
        let mut activations = activations_opt.unwrap_or_else(|| {
            debug!("no existing activation state, creating new one");
            ActivationState::new(
                &context.mode,
                &context.attach_ctx.dot_flox_path,
                &context.attach_ctx.env,
            )
        });

        // Reset state (but leave start state dirs) if executive is not running.
        if !activations.executive_running() {
            debug!("discarding activation state due to executive not running");
            activations = ActivationState::new(
                &context.mode,
                &context.attach_ctx.dot_flox_path,
                &context.attach_ctx.env,
            );
        }

        if activations.mode() != &context.mode {
            let running = activations
                .running_processes()
                // State (and thus mode) would have been reset if there was no executive.
                .expect("mode mismatch implies running processes (executive or attachments)");

            return Err(ModeMismatch::from_running_processes(
                activations.mode().clone(),
                context.mode.clone(),
                running,
            )
            .into());
        }

        let pid = std::process::id() as i32;
        match activations.start_or_attach(pid, &context.flox_activate_store_path) {
            StartOrAttachResult::Start { start_id } => Self::start(
                context,
                subsystem_verbosity,
                vars_from_env,
                start_id,
                &mut activations,
                &activations_json_path,
                lock,
            ),
            StartOrAttachResult::Attach { start_id } => {
                write_activations_json(&activations, &activations_json_path, lock)?;
                Ok(StartOrAttachResult::Attach { start_id })
            },
            StartOrAttachResult::AlreadyStarting { pid, start_id } => {
                drop(lock); // Explicit for clarity only.
                Ok(StartOrAttachResult::AlreadyStarting { pid, start_id })
            },
        }
    }

    // Start a new activation because we either have a:
    // - different store path
    // - fresh state file, which could be caused by no executive
    fn start(
        context: &ActivateCtx,
        subsystem_verbosity: u32,
        vars_from_env: &VarsFromEnvironment,
        start_id: StartIdentifier,
        activations: &mut ActivationState,
        activations_json_path: &Path,
        lock: LockFile,
    ) -> Result<StartOrAttachResult, anyhow::Error> {
        let start_state_dir = start_id.state_dir_path(
            &context.attach_ctx.flox_runtime_dir,
            &context.attach_ctx.dot_flox_path,
        )?;
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&start_state_dir)?;

        let new_executive = if !activations.executive_started() {
            // Register signal handler BEFORE spawning executive to avoid race condition
            // where SIGUSR1 arrives before handler is registered
            let signals = Signals::new([SIGCHLD, SIGUSR1])?;
            let exec_pid = self.spawn_executive(context, &start_state_dir)?;
            activations.set_executive_pid(exec_pid.as_raw());
            Some((exec_pid, signals))
        } else {
            None
        };

        write_activations_json(activations, activations_json_path, lock)?;

        if let Some((exec_pid, signals)) = new_executive {
            Self::wait_for_executive(exec_pid, signals)?;
        }

        let mut start_command = assemble_activate_command(
            context.clone(),
            subsystem_verbosity,
            vars_from_env.clone(),
            &start_id,
        );
        debug!("spawning activate script: {:?}", start_command);
        let status = start_command.spawn()?.wait()?;
        if !status.success() {
            // hook.on-activate may have already printed to stderr
            bail!("Running hook.on-activate failed");
        }

        // Re-acquire lock to mark ready
        let (activations_opt, lock) = read_activations_json(activations_json_path)?;
        let mut activations = activations_opt.expect("activations.json should exist");
        activations.set_ready(&start_id);
        write_activations_json(&activations, activations_json_path, lock)?;

        Ok(StartOrAttachResult::Start { start_id })
    }

    /// Start services with a new process-compose instance.
    ///
    /// The CLI has already decided that a new process-compose is needed.
    /// This function starts process-compose and then starts the specified services.
    fn start_services_with_new_process_compose(
        runtime_dir: &str,
        dot_flox_path: &Path,
        process_compose_bin: &Path,
        socket_path: &Path,
        services: &[String],
    ) -> Result<(), anyhow::Error> {
        let activations_json_path = state_json_path(runtime_dir, dot_flox_path);
        let (activations_opt, lock) = read_activations_json(&activations_json_path)?;
        let activations = activations_opt.expect("state.json should exist");
        let executive_pid = activations.executive_pid();
        // Don't hold a lock because the executive will need it when starting `process-compose`
        drop(lock);

        debug!("starting new process-compose for services");
        Self::signal_new_process_compose(process_compose_bin, socket_path, executive_pid)?;
        start_services_via_socket(process_compose_bin, socket_path, services)?;

        Ok(())
    }

    /// Start a new process-compose instance by signaling the executive.
    fn signal_new_process_compose(
        process_compose_bin: &Path,
        socket_path: &Path,
        executive_pid: i32,
    ) -> Result<(), anyhow::Error> {
        // Stop first, if running, to ensure that we wait on the socket from the new instance.
        if socket_path.exists() {
            debug!("shutting down old process-compose");
            if let Err(err) = process_compose_down(process_compose_bin, socket_path) {
                error!(%err, "failed to stop process-compose");
            }
        }

        debug!(
            executive_pid,
            "sending SIGUSR1 to executive to start new process-compose",
        );
        kill(Pid::from_raw(executive_pid), Signal::SIGUSR1)?;

        let activation_timeout = std::env::var("_FLOX_SERVICES_ACTIVATE_TIMEOUT")
            .ok()
            .and_then(|t| t.parse().ok())
            .map(Duration::from_secs_f64)
            .unwrap_or(Duration::from_secs(2));
        let socket_ready =
            wait_for_socket_ready(process_compose_bin, socket_path, activation_timeout)?;
        if !socket_ready {
            // TODO: We used to print the services log (if it exists) here to
            // help users debug the failure but we no longer have the path
            // available now that it's started by the executive.
            bail!("Failed to start services: process-compose socket not ready");
        }

        Ok(())
    }

    fn spawn_executive(
        &self,
        context: &ActivateCtx,
        start_state_dir: &Path,
    ) -> Result<Pid, anyhow::Error> {
        let parent_pid = getpid();

        // Serialize ExecutiveCtx
        let executive_ctx = ExecutiveCtx {
            context: context.clone(),
            parent_pid: parent_pid.as_raw(),
        };

        let temp_file = tempfile::NamedTempFile::with_prefix_in("executive_ctx_", start_state_dir)?;
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
        executive
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        debug!(
            "Spawning executive process to start activation: {:?}",
            executive
        );
        let child = executive.spawn()?;
        Ok(Pid::from_raw(child.id() as i32))
    }

    /// Wait for the executive to signal that it has started by sending SIGUSR1.
    /// If the executive dies, then we error.
    /// Signals should have been registered for SIGCHLD and SIGUSR1
    fn wait_for_executive(child_pid: Pid, mut signals: Signals) -> Result<(), anyhow::Error> {
        debug!(
            "Awaiting SIGUSR1 from child process with PID: {}",
            child_pid
        );

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
    // TODO: move now that it's also used by executive
    pub fn get() -> Result<Self> {
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
