use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use clap::Args;
use flox_core::activate::context::{ActivateCtx, InvocationType};
use flox_core::activate::sandbox_mode::SandboxMode;
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
use shell_gen::ShellWithPath;
use tracing::debug;

use crate::attach::attach;
use crate::message::{info, updated};
use crate::sandbox;
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
            // Container invocation with no command: start an interactive shell.
            (None, None) => context.invocation_type = Some(InvocationType::Interactive),
            // Container invocation with command arguments: exec the command
            // directly without routing through a shell. OCI CMD semantics are
            // exec, not shell — argv must reach the command verbatim. Joining
            // into a shell string would give every dollar sign, glob, and
            // command substitution an extra expansion pass against the
            // activation environment.
            (None, Some(args)) => {
                context.invocation_type = Some(InvocationType::ExecCommand(args.to_vec()));
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
        let vars_from_env = VarsFromEnvironment::get_with_snapshot()?;

        // Unset FLOX_SHELL to detect the parent shell anew with each flox invocation.
        unsafe { std::env::remove_var("FLOX_SHELL") };

        let (start_id, resolved_sandbox_mode) = self.start_or_attach(
            &context,
            &invocation_type,
            subsystem_verbosity,
            &vars_from_env,
        )?;

        // An attach that omitted --sandbox inherits the mode recorded on the
        // active activation. Record the resolved mode on the context so the
        // env injection (double_set_envs) and the generated rc scripts
        // re-export the active session's sandbox policy rather than the
        // omitted-flag default of Off.
        context.attach_ctx.sandbox_mode = resolved_sandbox_mode;

        // A sandboxed session must not exec a shell libsandbox cannot load
        // into: macOS strips DYLD_INSERT_LIBRARIES when exec'ing a
        // SIP-protected shell such as /bin/zsh, so the shell's own builtins
        // and redirections (`echo secret > ~/file`) would escape the policy
        // even though the children it spawns are mediated. Swap in the
        // mediable bash bundled with flox instead. This runs after
        // sandbox-mode resolution so an attach that inherited the mode is
        // covered, and only swaps unmediable shells, so a FLOX_SHELL pointed
        // at a mediable shell (one installed in the environment, say) is
        // honored unchanged.
        if context.attach_ctx.sandbox_mode != SandboxMode::Off && context.project_ctx.is_some() {
            if let Some(bash) = sandboxed_session_shell_swap(
                context.attach_ctx.sandbox_mode,
                context.project_ctx.is_some(),
                &context.shell,
            ) {
                // The session shell is only exec'd for interactive and
                // shell-command invocations; an exec-command invocation runs
                // the user's argv directly, so there the swap only feeds the
                // SHELL rewrite below and a user-facing notice would be
                // misleading noise.
                if matches!(
                    invocation_type,
                    InvocationType::Interactive | InvocationType::ShellCommand(_)
                ) {
                    info(formatdoc! {"
                        Cannot mediate '{shell}' inside the sandbox; using the bash bundled with Flox for this session.
                          To use a different shell, set FLOX_SHELL to a shell the sandbox can mediate, such as one installed in the environment.",
                        shell = context.shell.exe_path().display(),
                    });
                } else {
                    debug!(
                        original_shell = %context.shell.exe_path().display(),
                        "swapped unmediable shell for the bundled bash in a sandboxed activation"
                    );
                }
                context.shell = ShellWithPath::Bash(bash);
            }
            // Processes inside the session inherit the activation's
            // environment, so rewriting SHELL to the (now guaranteed
            // mediable) session shell is what makes `$SHELL`-spawning tools
            // (editors, tmux, git) launch a mediated shell rather than
            // re-opening the SIP hole through the user's original value.
            unsafe { std::env::set_var("SHELL", context.shell.exe_path()) };
        }

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
    ) -> Result<(StartIdentifier, SandboxMode), anyhow::Error> {
        let retry_delay = Duration::from_millis(200);
        let warning_interval = Duration::from_secs(5);
        let mut last_warning: Option<Instant> = None;

        let deactivate_hint = "To stop using this environment, run 'flox deactivate'";

        loop {
            let (result, resolved_sandbox_mode) =
                self.try_start_or_attach(context, subsystem_verbosity, vars_from_env)?;
            match result {
                StartOrAttachResult::Start { start_id, .. } => {
                    if *invocation_type == InvocationType::Interactive {
                        updated(
                            formatdoc! {"You are now using the environment '{env_description}'
                                     {deactivate_hint}
                                     ",
                            env_description = context.attach_ctx.env_description,
                            },
                        );
                    }
                    return Ok((start_id, resolved_sandbox_mode));
                },
                StartOrAttachResult::Attach { start_id, .. } => {
                    if *invocation_type == InvocationType::Interactive {
                        updated(
                            formatdoc! {"Attached to existing activation of environment '{env_description}'
                                     {deactivate_hint}
                                     ",
                            env_description = context.attach_ctx.env_description,
                            },
                        );
                    }
                    return Ok((start_id, resolved_sandbox_mode));
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
    /// Returns the [`StartOrAttachResult`] (started, attached, or retry) plus
    /// the resolved sandbox mode for this activation. The resolved mode is
    /// the requested mode for a fresh start or an explicit match, and the
    /// inherited active mode when `--sandbox` was omitted while a sandboxed
    /// session is already running.
    fn try_start_or_attach(
        &self,
        context: &ActivateCtx,
        subsystem_verbosity: u32,
        vars_from_env: &VarsFromEnvironment,
    ) -> Result<(StartOrAttachResult, SandboxMode), anyhow::Error> {
        // Use the pre-computed activation state directory
        let activations_json_path = state_json_path(&context.activation_state_dir);

        let (activations_opt, lock) = read_activations_json(&activations_json_path)?;

        // Get dot_flox_path for ActivationState.info (human debugging)
        // - Project activations: actual .flox path
        // - Containers: None
        let dot_flox_path = context.project_ctx.as_ref().map(|p| &p.dot_flox_path);

        let sandbox_mode = context.attach_ctx.sandbox_mode;

        let mut activations = activations_opt.unwrap_or_else(|| {
            debug!("no existing activation state, creating new one");
            ActivationState::new(&context.mode, dot_flox_path, &context.attach_ctx.env)
                .with_sandbox_mode(sandbox_mode)
        });

        // Reset state (but leave start state dirs) if executive is not running.
        // For containers this is the first activation; if for any reason the
        // runtime dir is preserved across container states then we'll start
        // again.
        if !activations.executive_running() {
            debug!("discarding activation state due to executive not running");
            activations =
                ActivationState::new(&context.mode, dot_flox_path, &context.attach_ctx.env)
                    .with_sandbox_mode(sandbox_mode);
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

        // Mirror the activation-mode mismatch handling for the sandbox mode.
        // An attach that requests no sandbox (the omitted-flag default of
        // `Off`) inherits the active mode with an info line. An attach that
        // explicitly requests a different non-`Off` mode is rejected, since a
        // single activation cannot run under two sandbox policies at once.
        let active_sandbox_mode = *activations.sandbox_mode();
        // The mode this activation actually runs under. Omitting --sandbox
        // (the `Off` default) while a sandboxed session is active inherits
        // that session's mode; otherwise the requested mode stands (it equals
        // the active mode on an explicit match, and is the start mode on a
        // fresh start).
        let resolved_sandbox_mode = if sandbox_mode == SandboxMode::Off {
            if active_sandbox_mode != SandboxMode::Off {
                updated(format!(
                    "Attaching with the active sandbox mode '{active_sandbox_mode}'."
                ));
            }
            active_sandbox_mode
        } else if sandbox_mode != active_sandbox_mode {
            return Err(anyhow::anyhow!(formatdoc! {"
                environment '{env}' is already active with sandbox mode '{active_sandbox_mode}'.
                Exit the existing session, or omit --sandbox to attach with the active mode.",
                env = context.attach_ctx.env_description,
            }));
        } else {
            sandbox_mode
        };

        let pid = std::process::id() as i32;
        let result = match activations.start_or_attach(pid, &context.flox_activate_store_path) {
            StartOrAttachResult::Start { start_id } => start(
                context,
                subsystem_verbosity,
                vars_from_env,
                start_id,
                &mut activations,
                &activations_json_path,
                lock,
            )?,
            StartOrAttachResult::Attach { start_id } => {
                write_activations_json(&activations, &activations_json_path, lock)?;
                StartOrAttachResult::Attach { start_id }
            },
            StartOrAttachResult::AlreadyStarting { pid, start_id } => {
                drop(lock); // Explicit for clarity only.
                StartOrAttachResult::AlreadyStarting { pid, start_id }
            },
        };
        Ok((result, resolved_sandbox_mode))
    }
}

/// The bundled-bash swap for a sandboxed session: `Some(bash)` when the
/// session shell must be replaced because the sandbox cannot mediate it,
/// `None` to keep the detected shell. Unsandboxed activations and
/// activations without a project context (containers, where the sandbox env
/// is never injected) always keep their shell.
fn sandboxed_session_shell_swap(
    sandbox_mode: SandboxMode,
    has_project: bool,
    shell: &ShellWithPath,
) -> Option<std::path::PathBuf> {
    (sandbox_mode != SandboxMode::Off && has_project && !sandbox::mediable_shell(shell.exe_path()))
        .then(sandbox::bundled_bash)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn shell_swap_keeps_shell_when_unsandboxed_or_projectless() {
        let zsh = ShellWithPath::Zsh(PathBuf::from("/bin/zsh"));
        assert_eq!(
            sandboxed_session_shell_swap(SandboxMode::Off, true, &zsh),
            None
        );
        assert_eq!(
            sandboxed_session_shell_swap(SandboxMode::Enforce, false, &zsh),
            None
        );
    }

    #[test]
    fn shell_swap_keeps_mediable_shells() {
        let nix_zsh = ShellWithPath::Zsh(PathBuf::from("/nix/store/abc-zsh-5.9/bin/zsh"));
        assert_eq!(
            sandboxed_session_shell_swap(SandboxMode::Enforce, true, &nix_zsh),
            None
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn shell_swap_replaces_sip_shells_in_active_modes() {
        let zsh = ShellWithPath::Zsh(PathBuf::from("/bin/zsh"));
        for mode in [SandboxMode::Warn, SandboxMode::Enforce, SandboxMode::Prompt] {
            assert_eq!(
                sandboxed_session_shell_swap(mode, true, &zsh),
                Some(sandbox::bundled_bash())
            );
        }
    }

    /// Container argv containing shell-special characters must survive
    /// to the InvocationType verbatim. OCI CMD semantics are exec, not
    /// shell — the container runtime passes argv elements directly to
    /// the entrypoint without a shell interpretation pass.
    #[test]
    fn container_run_args_produce_exec_command_verbatim() {
        let shell_special_args = vec![
            "bash".to_string(),
            "-c".to_string(),
            "echo $VAR ${arr[0]} $(date) * 'quoted'".to_string(),
            "an arg with $dollar".to_string(),
        ];

        let args = ActivateArgs {
            activate_data: PathBuf::from("/does/not/exist"),
            cmd: Some(shell_special_args.clone()),
        };

        // run_args detection mirrors the inline logic in handle():
        // cmd.as_ref().and_then(|a| if a.is_empty() { None } else { Some(a) })
        let run_args = args
            .cmd
            .as_ref()
            .and_then(|a| if a.is_empty() { None } else { Some(a) });

        // Simulate the container arm: invocation_type starts as None
        let invocation_type = match (None::<&InvocationType>, run_args) {
            (None, Some(a)) => InvocationType::ExecCommand(a.to_vec()),
            (None, None) => InvocationType::Interactive,
            (Some(t), _) => t.clone(),
        };

        assert_eq!(
            invocation_type,
            InvocationType::ExecCommand(shell_special_args),
        );
    }

    /// Empty command list with no pre-set invocation type yields Interactive,
    /// matching the container entrypoint behaviour when no CMD is given.
    #[test]
    fn container_no_run_args_produces_interactive() {
        let args = ActivateArgs {
            activate_data: PathBuf::from("/does/not/exist"),
            cmd: None,
        };

        let run_args = args
            .cmd
            .as_ref()
            .and_then(|a| if a.is_empty() { None } else { Some(a) });

        let invocation_type = match (None::<&InvocationType>, run_args) {
            (None, Some(a)) => InvocationType::ExecCommand(a.to_vec()),
            (None, None) => InvocationType::Interactive,
            (Some(t), _) => t.clone(),
        };

        assert_eq!(invocation_type, InvocationType::Interactive);
    }
}
