use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Args;
use flox_core::activate::context::{ActivateCtx, InvocationType};
use flox_core::activations::{
    ActivationState,
    ModeMismatch,
    StartIdentifier,
    StartOrAttachResult,
    read_activations_json,
    state_json_path,
    write_activations_json,
};
use indoc::formatdoc;
use tracing::debug;

use crate::attach::{attach, quote_run_args};
use crate::message::updated;
use crate::start::{start, start_services_with_new_process_compose};
use crate::vars_from_env::VarsFromEnvironment;

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
        // Common setup: read file, detect run_args, get environment vars
        let contents = fs::read_to_string(&self.activate_data)?;
        let run_args = self
            .cmd
            .as_ref()
            .and_then(|args| if args.is_empty() { None } else { Some(args) });
        let vars_from_env = VarsFromEnvironment::get()?;

        // Unset FLOX_SHELL to detect the parent shell anew with each flox invocation.
        unsafe { std::env::remove_var("FLOX_SHELL") };

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

        // Detect invocation type from context or run_args
        let invocation_type = Self::resolve_invocation_type(&mut context.invocation_type, run_args);

        Self::apply_shell_override(&mut context.core.shell)?;

        let start_id = self.start_or_attach(
            &context,
            &invocation_type,
            subsystem_verbosity,
            &vars_from_env,
        )?;

        if !context.project.services_to_start.is_empty() {
            start_services_with_new_process_compose(
                &context.core.flox_runtime_dir,
                &context.project.dot_flox_path,
                &context.project.process_compose_bin,
                &context.project.flox_services_socket,
                &context.project.services_to_start,
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

    /// Apply shell override from `_FLOX_SHELL_FORCE` environment variable.
    fn apply_shell_override(shell: &mut shell_gen::ShellWithPath) -> Result<(), anyhow::Error> {
        if let Ok(shell_force) = std::env::var("_FLOX_SHELL_FORCE") {
            *shell = PathBuf::from(shell_force).as_path().try_into()?;
        }
        Ok(())
    }

    /// Compute invocation type from run_args (for containers or when context has None).
    fn invocation_type_from_args(run_args: Option<&Vec<String>>) -> InvocationType {
        match run_args {
            None => InvocationType::Interactive,
            Some(args) => InvocationType::ShellCommand(quote_run_args(args)),
        }
    }

    /// Resolve invocation type from context or run_args.
    /// Updates context.invocation_type if it was None.
    fn resolve_invocation_type(
        ctx_invocation_type: &mut Option<InvocationType>,
        run_args: Option<&Vec<String>>,
    ) -> InvocationType {
        match (ctx_invocation_type.as_ref(), run_args) {
            // Context has None - detect from run_args (container-like behavior)
            (None, _) => {
                let invocation_type = Self::invocation_type_from_args(run_args);
                *ctx_invocation_type = Some(invocation_type.clone());
                invocation_type
            },
            // Context already has invocation type - use it
            (Some(invocation_type), _) => invocation_type.clone(),
        }
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
                            env_description = context.core.env_description,
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
                            env_description = context.core.env_description,
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
            &context.core.flox_runtime_dir,
            &context.project.dot_flox_path,
        );

        let (activations_opt, lock) = read_activations_json(&activations_json_path)?;
        let mut activations = activations_opt.unwrap_or_else(|| {
            debug!("no existing activation state, creating new one");
            ActivationState::new(
                &context.core.mode,
                &context.project.dot_flox_path,
                &context.core.env,
            )
        });

        // Reset state (but leave start state dirs) if executive is not running.
        if !activations.executive_running() {
            debug!("discarding activation state due to executive not running");
            activations = ActivationState::new(
                &context.core.mode,
                &context.project.dot_flox_path,
                &context.core.env,
            );
        }

        if activations.mode() != &context.core.mode {
            let running = activations
                .running_processes()
                // State (and thus mode) would have been reset if there was no executive.
                .expect("mode mismatch implies running processes (executive or attachments)");

            return Err(ModeMismatch::from_running_processes(
                activations.mode().clone(),
                context.core.mode.clone(),
                running,
            )
            .into());
        }

        let pid = std::process::id() as i32;
        match activations.start_or_attach(pid, &context.core.flox_activate_store_path) {
            StartOrAttachResult::Start { start_id } => start(
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
}
