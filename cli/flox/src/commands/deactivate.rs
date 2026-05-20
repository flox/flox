/// Explicit deactivation of persistent Flox Agent environments.
///
/// `flox deactivate -d <path>` tears down a persistent environment started
/// with `flox activate --persistent`.  It locates the executive PID from
/// the activation state file and sends SIGTERM to gracefully stop it.
///
/// For interactive (non-persistent) environments, the user should just
/// type `exit` in their shell.  This command is specifically for the
/// Flox Agent prototype's persistent daemon environments.
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use flox_core::activations::{activation_state_dir_path, read_activations_json, state_json_path};
use flox_core::proc_status::pid_is_running;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::find_dot_flox;
use indoc::formatdoc;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use shell_gen::Shell;
use tracing::debug;

use crate::commands::recap::persistent_marker_path;
use crate::utils::message;

#[derive(Bpaf, Clone, Debug)]
pub struct Deactivate {
    /// Path containing the .flox/ directory of the environment to deactivate
    #[bpaf(long("dir"), short('d'), argument("path"), optional)]
    pub dir: Option<PathBuf>,

    /// Emit shell code to restore PS1 (for eval by shell wrapper function)
    #[bpaf(long("shell-eval"), switch)]
    pub shell_eval: bool,

    /// Shell to emit PS1 restoration code for (bash, zsh)
    #[bpaf(long("shell"), argument("SHELL"), optional)]
    pub shell: Option<String>,
}

impl Deactivate {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        let dot_flox_path = match self.dir {
            Some(ref path) => {
                // Resolve to the .flox directory within the given path.
                let canonical = path
                    .canonicalize()
                    .with_context(|| format!("Could not resolve path: {}", path.display()))?;
                find_dot_flox(&canonical)
                    .with_context(|| {
                        format!(
                            "No .flox directory found at or above {}",
                            canonical.display()
                        )
                    })?
                    .map(|d| d.path)
                    .unwrap_or_else(|| canonical.join(".flox"))
            },
            None => {
                // No --dir given: look in current directory.
                let cwd = std::env::current_dir()?;
                find_dot_flox(&cwd)
                    .with_context(|| "No .flox directory found in current directory")?
                    .map(|d| d.path)
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "No environment found in current directory.\nSpecify a path with 'flox deactivate -d <path>'."
                        )
                    })?
            },
        };

        let state_dir = activation_state_dir_path(&flox.runtime_dir, &dot_flox_path);

        let state_json = state_json_path(&state_dir);
        if !state_json.exists() {
            bail!(formatdoc! {"
                No active session found for this environment.
                The environment may not be running or may have already exited."
            });
        }

        // Read activation state to find the executive PID.
        let (activations, _lock) = read_activations_json(&state_json)
            .with_context(|| "Could not read activation state file")?;

        let Some(activations) = activations else {
            bail!("No activation state found for this environment.");
        };

        let exec_pid = activations.executive_pid();
        if exec_pid == 0 {
            bail!("No running executive process found for this environment.");
        }

        if !pid_is_running(exec_pid) {
            message::warning("Executive process is no longer running. Cleaning up state.");
            // Nothing to kill, but we report success so the user isn't confused.
            return Ok(());
        }

        debug!(exec_pid, "Sending SIGTERM to executive process");

        kill(Pid::from_raw(exec_pid), Signal::SIGTERM)
            .with_context(|| format!("Could not send SIGTERM to executive (pid {exec_pid})"))?;

        // Remove the persistent marker (if any) so the environment no longer
        // shows up as [persistent] in `flox envs` after explicit deactivation.
        let marker = persistent_marker_path(&flox.cache_dir, &dot_flox_path);
        if marker.exists()
            && let Err(e) = std::fs::remove_file(&marker)
        {
            message::warning(format!("Could not remove persistent marker: {e}"));
        }

        // Emit shell eval code to restore PS1 if --shell-eval was requested.
        if self.shell_eval
            && let Some(ref shell_str) = self.shell
            && let Ok(shell) = shell_str.parse::<Shell>()
        {
            print!("{}", ps1_restore_code(shell));
            return Ok(());
        }

        message::plain(format!(
            "✅  Environment deactivated (executive pid {exec_pid} terminated)."
        ));
        Ok(())
    }
}

/// Generate shell code to restore PS1 after deactivation.
///
/// When `flox deactivate --shell-eval --shell <shell>` is called via the
/// `flox()` wrapper function, this code is eval'd in the parent shell to
/// restore PS1 from the saved value set by `set-prompt.bash/zsh`.
pub(crate) fn ps1_restore_code(shell: Shell) -> String {
    match shell {
        Shell::Bash => {
            r#"PS1="${FLOX_SAVE_BASH_PS1:-$PS1}"; unset FLOX_SAVE_BASH_PS1;"#.to_string()
        },
        Shell::Zsh => r#"PS1="${FLOX_SAVE_ZSH_PS1:-$PS1}"; unset FLOX_SAVE_ZSH_PS1;"#.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps1_restore_code_bash_restores_from_save_var() {
        let code = ps1_restore_code(Shell::Bash);
        assert!(
            code.contains("FLOX_SAVE_BASH_PS1"),
            "should reference bash save var"
        );
        assert!(code.contains("PS1="), "should reassign PS1");
        assert!(
            code.contains("unset FLOX_SAVE_BASH_PS1"),
            "should clean up save var"
        );
    }

    #[test]
    fn ps1_restore_code_zsh_restores_from_save_var() {
        let code = ps1_restore_code(Shell::Zsh);
        assert!(
            code.contains("FLOX_SAVE_ZSH_PS1"),
            "should reference zsh save var"
        );
        assert!(code.contains("PS1="), "should reassign PS1");
        assert!(
            code.contains("unset FLOX_SAVE_ZSH_PS1"),
            "should clean up save var"
        );
    }
}
