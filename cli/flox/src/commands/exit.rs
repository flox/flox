use anyhow::Result;
use bpaf::Bpaf;
use flox_core::activate::vars::FLOX_ACTIVE_ENVIRONMENTS_VAR;
use flox_core::hook_state::{
    HOOK_VAR_ACTIVATIONS,
    HOOK_VAR_DIFF,
    HOOK_VAR_DIRS,
    HOOK_VAR_SUPPRESSED,
    HOOK_VAR_WATCHES,
    HookState,
};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::UninitializedEnvironment;
use indoc::{formatdoc, indoc};
use shell_gen::{GenerateShell, SetVar, Shell, UnsetVar};

use super::hook_env::{emit_revert, spawn_auto_detach};
use super::{activated_environments, uninitialized_environment_description};
use crate::subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Clone)]
pub struct Exit {
    /// Shell to generate deactivation commands for
    #[bpaf(long, argument("shell"), optional)]
    pub shell: Option<String>,
}

impl Exit {
    pub fn handle(self, _flox: Flox) -> Result<()> {
        subcommand_metric!("exit");

        let in_auto_activation = std::env::var(HOOK_VAR_DIRS).is_ok_and(|v| !v.is_empty());

        if in_auto_activation {
            return self.handle_auto_deactivate();
        }

        let active_environments = activated_environments();
        let last_active = active_environments.last_active();

        let Some(last_active) = last_active else {
            message::info(indoc! {"
                No environment active!
                Exit active environments by typing 'exit' to exit your current shell or close your terminal.
                Environments can be activated using `flox activate`.
            "});

            return Ok(());
        };

        message::info(formatdoc! {"
            Exit the currently active environment {} by typing 'exit' to exit your current shell or close your terminal.
        ", uninitialized_environment_description(&last_active)?});

        Ok(())
    }

    fn handle_auto_deactivate(self) -> Result<()> {
        let shell: Shell = self
            .shell
            .as_deref()
            .unwrap_or("bash")
            .parse()
            .map_err(|_| {
                anyhow::anyhow!(
                    "unsupported shell: {}",
                    self.shell.as_deref().unwrap_or("bash")
                )
            })?;

        let state = HookState::from_env()?;
        let mut stdout = std::io::stdout().lock();

        // Revert the diff and restore the prompt.
        emit_revert(&state.diff, shell, &mut stdout)?;

        // Only suppress the innermost (CWD-nearest) directory so that
        // `flox deactivate` peels off one environment at a time.
        let mut suppressed = state.suppressed_dirs.clone();
        if let Some(innermost) = state.active_dirs.last() {
            if !suppressed.contains(innermost) {
                suppressed.push(innermost.clone());
            }

            // Detach shell PID from the activation state for this environment.
            let shell_pid = std::os::unix::process::parent_id() as i32;
            if let Some(info) = state.activation_tracking.entries.get(innermost) {
                spawn_auto_detach(shell_pid, &info.activation_state_dir);
            }

            // Remove the deactivated env from _FLOX_ACTIVE_ENVIRONMENTS
            // so that a subsequent `flox activate` doesn't see it as
            // "already active".
            let mut active_envs = activated_environments();
            active_envs.retain(|env| {
                if let UninitializedEnvironment::DotFlox(d) = env {
                    d.path != *innermost
                } else {
                    true
                }
            });
            SetVar::exported_no_expansion(FLOX_ACTIVE_ENVIRONMENTS_VAR, active_envs.to_string())
                .generate_with_newline(shell, &mut stdout)?;
        }
        let suppressed_str = HookState::format_path_list(&suppressed);
        SetVar::exported_no_expansion(HOOK_VAR_SUPPRESSED, &suppressed_str)
            .generate_with_newline(shell, &mut stdout)?;

        // Clear hook state variables.
        UnsetVar::new(HOOK_VAR_DIFF).generate_with_newline(shell, &mut stdout)?;
        UnsetVar::new(HOOK_VAR_DIRS).generate_with_newline(shell, &mut stdout)?;
        UnsetVar::new(HOOK_VAR_WATCHES).generate_with_newline(shell, &mut stdout)?;
        UnsetVar::new(HOOK_VAR_ACTIVATIONS).generate_with_newline(shell, &mut stdout)?;

        // Hint: `flox deactivate` suppresses auto-activation for this
        // session only. Suggest `flox deny` for persistent denial.
        eprintln!(
            "flox: environment deactivated for this session. \
             Use 'flox deny' to permanently prevent auto-activation."
        );

        Ok(())
    }
}
