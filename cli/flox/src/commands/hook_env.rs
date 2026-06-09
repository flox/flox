use std::borrow::Cow;
use std::io::{BufWriter, Write, stdout};

use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use flox_core::activate::context::InvocationKind;
use flox_core::hook_actions::{HookAction, take_hook_actions};
use flox_rust_sdk::flox::Flox;
use shell_gen::{Shell, ShellWithPath};

use super::deactivate::{emit_deactivate_script, flox_activate_tracelevel};
use crate::subcommand_metric;

#[derive(Debug, Clone, Bpaf)]
pub struct HookEnv {
    /// Shell to emit hook-env code for (bash, zsh, fish, tcsh)
    #[bpaf(long("shell"), argument("SHELL"))]
    shell: Shell,

    /// PID of the calling interactive shell ($$ / $fish_pid).
    ///
    /// The shell expands this before invoking `hook-env`, so it identifies the
    /// interactive shell even though `hook-env` itself runs in a command
    /// substitution subshell. It keys the prompt-hook action file this shell
    /// reads.
    #[bpaf(long("shell-pid"), argument("PID"))]
    shell_pid: i32,

    /// Invocation type of the activation the hook is running in
    /// (`$_FLOX_INVOCATION_TYPE`), used when emitting a deactivation script.
    ///
    /// Optional as a defensive measure. Every shell hook passes it (tcsh guards
    /// a possibly-unset value with `$?`); when a deactivate action is pending but
    /// none was provided, the hook falls back to `inplace`.
    #[bpaf(long("invocation-type"), argument("INVOCATION_TYPE"), optional)]
    invocation_kind: Option<InvocationKind>,
}

impl HookEnv {
    pub fn handle(self, flox: Flox) -> Result<()> {
        if !flox.features.auto_activate {
            bail!(
                "'hook-env' requires the auto_activate feature flag. Set FLOX_FEATURES_AUTO_ACTIVATE=true."
            );
        }

        // TODO: when we add auto-activation logic, we should probably skip this
        // on the fast path and only add it when we're making a meaningful change.
        // We could also consider counting unique environments or something
        // instead of recording every single run of this command.
        subcommand_metric!("hook-env");

        let mut writer = BufWriter::new(stdout());

        // Consume any actions another flox command (e.g. `flox deactivate`) left
        // for this shell and emit the corresponding script. The common case is
        // no pending actions.
        let actions = take_hook_actions(&flox.runtime_dir, self.shell_pid)
            .context("failed to read prompt-hook actions")?;
        for action in actions {
            match action {
                HookAction::Deactivate {
                    activation_state_dir,
                    flox_env,
                } => {
                    // Default to in-place when the shell didn't pass an
                    // invocation type. Every shell hook passes it today; this is
                    // a defensive fallback, and the prompt hook only ever
                    // deactivates in place.
                    let invocation_kind = self.invocation_kind.unwrap_or(InvocationKind::InPlace);
                    emit_deactivate_script(
                        ShellWithPath::from(self.shell),
                        invocation_kind,
                        &activation_state_dir,
                        &flox_env,
                        flox_activate_tracelevel(),
                        &mut writer,
                    )?;
                },
            }
        }

        // Temporary: set _FLOX_HOOK_FIRED so we can verify the hook fires.
        // This will be replaced by real environment activation logic.
        let cwd = std::env::current_dir()?.to_string_lossy().to_string();
        let escaped_cwd = shell_escape::escape(Cow::Borrowed(&cwd));
        match self.shell {
            Shell::Bash | Shell::Zsh => writeln!(writer, "export _FLOX_HOOK_FIRED={escaped_cwd};")?,
            Shell::Fish => writeln!(writer, "set -gx _FLOX_HOOK_FIRED {escaped_cwd};")?,
            Shell::Tcsh => writeln!(writer, "setenv _FLOX_HOOK_FIRED {escaped_cwd};")?,
        }

        writer.flush()?;
        Ok(())
    }
}
