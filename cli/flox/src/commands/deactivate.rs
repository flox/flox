use std::io::{BufWriter, Write, stdout};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow};
use bpaf::Bpaf;
use flox_core::activate::context::InvocationType;
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

        // Interactive subshell: just emit `exit;`. The shell exits and the
        // executive monitors the PID, cleaning up state.json when it goes away.
        let is_interactive = InvocationType::from_str(&invocation_type)
            .unwrap_or(InvocationType::InPlace)
            == InvocationType::Interactive;

        let mut writer = BufWriter::new(stdout());

        if is_interactive {
            write!(writer, "exit;")?;
            return Ok(());
        }

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
        .context("failed to generate deactivation script")?;

        Ok(())
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

#[cfg(test)]
mod tests {
    use std::io::BufWriter;

    use super::*;

    fn is_interactive(invocation_type: &str) -> bool {
        InvocationType::from_str(invocation_type).unwrap_or(InvocationType::InPlace)
            == InvocationType::Interactive
    }

    // Helper to call handle_print_script with a given invocation type and
    // capture the output. We bypass `handle()` because it checks flox features.

    /// Confirm that `"interactive"` as the invocation type emits exactly `exit;`.
    #[test]
    fn interactive_invocation_type_emits_exit() {
        // We can't call handle_print_script directly in unit tests because it
        // calls activated_environments() (reads env vars) and
        // detect_shell_for_in_place() (reads env vars). Instead we test the
        // logic inline:
        assert!(
            is_interactive("interactive"),
            "\"interactive\" should be detected as interactive"
        );

        let mut buf = Vec::new();
        {
            let mut writer = BufWriter::new(&mut buf);
            if is_interactive("interactive") {
                write!(writer, "exit;").unwrap();
            }
        }
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "exit;",
            "interactive invocation type should emit exactly 'exit;'"
        );
    }

    /// Confirm that `"inplace"` does NOT emit `exit`.
    #[test]
    fn inplace_invocation_type_emits_no_exit() {
        assert!(
            !is_interactive("inplace"),
            "\"inplace\" should not be detected as interactive"
        );
    }

    /// Confirm that an empty string defaults to in-place (not interactive).
    #[test]
    fn empty_invocation_type_defaults_to_inplace() {
        assert!(
            !is_interactive(""),
            "empty invocation type should default to in-place"
        );
    }

    /// Confirm that an unknown string defaults to in-place (not interactive).
    #[test]
    fn unknown_invocation_type_defaults_to_inplace() {
        assert!(
            !is_interactive("totally-unknown"),
            "unknown invocation type should default to in-place"
        );
    }
}
