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

const INITIAL_ACTIVATION_RETRY_DELAY: Duration = Duration::from_millis(25);
const MAX_ACTIVATION_RETRY_DELAY: Duration = Duration::from_secs(2);
const ACTIVATION_WARNING_INITIAL_DELAY: Duration = Duration::from_secs(3);
const ACTIVATION_WARNING_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug)]
struct ActivationRetryBackoff {
    next_retry_delay: Duration,
    blocked_since: Instant,
    next_warning_at: Instant,
}

impl ActivationRetryBackoff {
    fn new(now: Instant) -> Self {
        Self {
            next_retry_delay: INITIAL_ACTIVATION_RETRY_DELAY,
            blocked_since: now,
            next_warning_at: now + ACTIVATION_WARNING_INITIAL_DELAY,
        }
    }

    fn next_wait(&mut self, now: Instant) -> ActivationRetryWait {
        let retry_delay = self.next_retry_delay;
        self.next_retry_delay = self
            .next_retry_delay
            .saturating_mul(2)
            .min(MAX_ACTIVATION_RETRY_DELAY);

        let blocked_for = if now >= self.next_warning_at {
            self.next_warning_at = now + ACTIVATION_WARNING_INTERVAL;
            Some(now.duration_since(self.blocked_since))
        } else {
            None
        };

        ActivationRetryWait {
            retry_delay,
            blocked_for,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ActivationRetryWait {
    retry_delay: Duration,
    blocked_for: Option<Duration>,
}

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

        // Capture env snapshot *before* modifying the process environment so
        // the diff reflects the true pre-activation state.
        let vars_from_env = if context.capture_env_diff {
            VarsFromEnvironment::get_with_snapshot()?
        } else {
            VarsFromEnvironment::get()?
        };

        // Unset FLOX_SHELL to detect the parent shell anew with each flox invocation.
        unsafe { std::env::remove_var("FLOX_SHELL") };

        let start_id = self.start_or_attach(
            &context,
            &invocation_type,
            subsystem_verbosity,
            &vars_from_env,
        )?;

        // Only start services if project context exists
        if let Some(project) = &context.project_ctx
            && !project.services_to_start.is_empty()
        {
            start_services_with_new_process_compose(
                &context.activation_state_dir,
                &project.process_compose_bin,
                &project.flox_services_socket,
                &project.services_to_start,
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
        let mut retry_backoff: Option<ActivationRetryBackoff> = None;

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
                    let wait = retry_backoff
                        .get_or_insert_with(|| ActivationRetryBackoff::new(now))
                        .next_wait(now);

                    if let Some(blocked_for) = wait.blocked_for {
                        eprintln!(
                            "⚠️  Activation is blocked by another startup (PID {blocking_pid}).\nRetrying automatically after waiting {} seconds.",
                            blocked_for.as_secs()
                        );
                    }

                    std::thread::sleep(wait.retry_delay);
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
        // Use the pre-computed activation state directory
        let activations_json_path = state_json_path(&context.activation_state_dir);

        let (activations_opt, lock) = read_activations_json(&activations_json_path)?;

        // Get dot_flox_path for ActivationState.info (human debugging)
        // - Project activations: actual .flox path
        // - Containers: None
        let dot_flox_path = context.project_ctx.as_ref().map(|p| &p.dot_flox_path);

        let mut activations = activations_opt.unwrap_or_else(|| {
            debug!("no existing activation state, creating new one");
            ActivationState::new(&context.mode, dot_flox_path, &context.attach_ctx.env)
        });

        // Reset state (but leave start state dirs) if executive is not running.
        // For containers this is the first activation; if for any reason the
        // runtime dir is preserved across container states then we'll start
        // again.
        if !activations.executive_running() {
            debug!("discarding activation state due to executive not running");
            activations =
                ActivationState::new(&context.mode, dot_flox_path, &context.attach_ctx.env);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_retry_backoff_grows_to_cap() {
        let start = Instant::now();
        let mut backoff = ActivationRetryBackoff::new(start);

        let waits = (0..9)
            .map(|_| backoff.next_wait(start).retry_delay)
            .collect::<Vec<_>>();

        assert_eq!(waits, vec![
            Duration::from_millis(25),
            Duration::from_millis(50),
            Duration::from_millis(100),
            Duration::from_millis(200),
            Duration::from_millis(400),
            Duration::from_millis(800),
            Duration::from_millis(1600),
            Duration::from_secs(2),
            Duration::from_secs(2),
        ]);
    }

    #[test]
    fn activation_retry_warning_is_delayed_and_repeated_less_often() {
        let start = Instant::now();
        let mut backoff = ActivationRetryBackoff::new(start);

        assert_eq!(backoff.next_wait(start), ActivationRetryWait {
            retry_delay: Duration::from_millis(25),
            blocked_for: None,
        });
        assert_eq!(
            backoff.next_wait(start + ACTIVATION_WARNING_INITIAL_DELAY),
            ActivationRetryWait {
                retry_delay: Duration::from_millis(50),
                blocked_for: Some(ACTIVATION_WARNING_INITIAL_DELAY),
            }
        );
        assert_eq!(
            backoff.next_wait(start + ACTIVATION_WARNING_INITIAL_DELAY + Duration::from_secs(14)),
            ActivationRetryWait {
                retry_delay: Duration::from_millis(100),
                blocked_for: None,
            }
        );
        assert_eq!(
            backoff
                .next_wait(start + ACTIVATION_WARNING_INITIAL_DELAY + ACTIVATION_WARNING_INTERVAL),
            ActivationRetryWait {
                retry_delay: Duration::from_millis(200),
                blocked_for: Some(ACTIVATION_WARNING_INITIAL_DELAY + ACTIVATION_WARNING_INTERVAL),
            }
        );
    }
}
