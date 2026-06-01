use std::io::{BufWriter, Write, stdout};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use bpaf::Bpaf;
use flox_core::activate::context::InvocationKind;
use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
use flox_core::activations::activation_state_dir_path;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::utils::FLOX_INTERPRETER;
use indoc::{formatdoc, indoc};

use super::{activated_environments, uninitialized_environment_description};
use crate::subcommand_metric;
use crate::utils::detect_shell::detect_shell_for_in_place;
use crate::utils::message;

#[derive(Bpaf, Clone)]
pub struct Deactivate {
    /// Invocation type for print-script mode (hidden, for shell hook use).
    ///
    /// When provided, emits a deactivation script to stdout. The value
    /// determines the exit strategy:
    /// - `"interactive"` → emit `exit;` (subshell will exit and clean up)
    /// - anything else   → emit in-place env-var restoration + detach command
    #[bpaf(long("print-script"), argument("INVOCATION_TYPE"), optional, hide)]
    pub print_script: Option<String>,
}

impl Deactivate {
    pub fn handle(self, flox: Flox) -> Result<()> {
        if !flox.features.auto_activate {
            return self.old_exit(flox);
        }

        subcommand_metric!("deactivate");

        if let Some(invocation_type) = self.print_script.clone() {
            return self.handle_print_script(flox, invocation_type);
        }

        // Interactive mode - print instructions
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

    /// Handle `flox deactivate --print-script <INVOCATION_TYPE>`.
    ///
    /// Emits a script that either exits the calling shell (for interactive
    /// subshell activations) or performs an in-place deactivation (env-var
    /// restoration + a `flox-activations detach` command).
    ///
    /// The `invocation_type` argument comes from the `_FLOX_INVOCATION_TYPE`
    /// shell variable set during activation, removing the need to read
    /// state.json inside this binary.
    fn handle_print_script(self, flox: Flox, invocation_type: String) -> Result<()> {
        let shell = detect_shell_for_in_place()?;

        // Get the .flox directory path from the active environment, opening
        // the concrete environment so managed and remote environments are
        // also supported (not just local path environments).
        let last_active = activated_environments()
            .last_active()
            .ok_or_else(|| anyhow!("No environment active."))?;
        let dot_flox_path = last_active
            .into_concrete_environment(&flox, None)
            .context("failed to open active environment for deactivation")?
            .dot_flox_path()
            .to_path_buf();

        let activation_state_dir = activation_state_dir_path(&flox.runtime_dir, &dot_flox_path);

        let mut writer = BufWriter::new(stdout());

        if invocation_type.is_empty() {
            bail!("cannot deactivate for empty INVOCATION_TYPE")
        }

        let invocation_kind = InvocationKind::from_str(&invocation_type)
            .context("could not determine invocation type".to_string())?;

        match invocation_kind {
            InvocationKind::Interactive => {
                // Interactive subshell: just emit `exit;`. The shell exits and the
                // executive monitors the PID, cleaning up state.json when it goes
                // away.
                write!(writer, "exit;")?;
                Ok(())
            },
            InvocationKind::InPlace | InvocationKind::ShellCommand => {
                // In-place activation: restore env vars first, then emit a shell
                // command that calls `flox-activations detach` so state.json is
                // updated after the script is eval'd by the caller.
                flox_activations::deactivate::generate_deactivate_script(
                    shell,
                    &mut writer,
                    &*FLOX_INTERPRETER,
                    &FLOX_ACTIVATIONS_BIN,
                    &activation_state_dir,
                )
                .context("failed to generate deactivation script")
            },
            InvocationKind::ExecCommand => {
                // This should be unreachable because we shouldn't set _FLOX_INVOCATION_TYPE to exec_command
                bail!("cannot deactivate an exec command activation");
            },
        }
    }

    pub fn old_exit(self, _flox: Flox) -> Result<()> {
        subcommand_metric!("exit");

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
}
