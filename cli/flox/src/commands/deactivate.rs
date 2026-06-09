use std::io::{BufWriter, Write, stdout};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use bpaf::Bpaf;
use flox_core::activate::context::InvocationKind;
use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
use flox_core::activations::activation_state_dir_path;
use flox_core::hook_actions::{
    HookAction,
    PROMPT_HOOK_VERSION,
    PROMPT_HOOK_VERSION_ENV,
    write_hook_actions,
};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::utils::FLOX_INTERPRETER;
use indoc::{formatdoc, indoc};
use shell_gen::ShellWithPath;

use super::{activated_environments, uninitialized_environment_description};
use crate::config::Config;
use crate::subcommand_metric;
use crate::utils::active_environments::ActiveEnvironment;
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
    pub fn handle(self, config: Config, flox: Flox) -> Result<()> {
        if !flox.features.auto_activate {
            return self.old_exit(flox);
        }

        subcommand_metric!("deactivate");

        if let Some(invocation_type) = self.print_script.clone() {
            return self.handle_print_script(flox, invocation_type);
        }

        self.handle_request_deactivate(config, flox)
    }

    /// Handle a plain `flox deactivate` (no `--print-script`).
    ///
    /// `flox deactivate` runs in a subprocess and can't modify its parent
    /// shell, so it writes a prompt-hook action file that the shell's prompt
    /// hook (`flox hook-env`) reads on the next prompt and turns into an
    /// in-place deactivation. The file is keyed by the shell's PID:
    /// `flox deactivate` is a direct child of the interactive shell, so its
    /// parent PID is the shell that will read the file.
    fn handle_request_deactivate(self, config: Config, flox: Flox) -> Result<()> {
        let Some(active) = activated_environments().last_active_full() else {
            message::info(indoc! {"
                No environment active!
                Exit active environments by typing 'exit' to exit your current shell or close your terminal.
                Environments can be activated using `flox activate`.
            "});

            return Ok(());
        };

        // The action file we are about to write is only useful if this shell has
        // a compatible prompt hook to consume it. Fail loudly otherwise, rather
        // than printing a success message for a deactivation that never happens.
        ensure_prompt_hook_available(&config)?;

        let target = open_deactivation_target(&flox, active)?;

        write_hook_actions(&flox.runtime_dir, nix::unistd::getppid().as_raw(), vec![
            HookAction::Deactivate {
                activation_state_dir: target.activation_state_dir,
                flox_env: target.flox_env,
            },
        ])
        .context("failed to record deactivation request")?;

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

        let active = activated_environments()
            .last_active_full()
            .ok_or_else(|| anyhow!("No environment active."))?;
        let target = open_deactivation_target(&flox, active)?;

        if invocation_type.is_empty() {
            bail!("cannot deactivate for empty INVOCATION_TYPE")
        }

        let invocation_kind = InvocationKind::from_str(&invocation_type)
            .context("could not determine invocation type".to_string())?;

        let mut writer = BufWriter::new(stdout());
        emit_deactivate_script(
            shell,
            invocation_kind,
            &target.activation_state_dir,
            &target.flox_env,
            flox_activate_tracelevel(),
            &mut writer,
        )
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

/// Verify that this shell has a prompt hook able to service a plain
/// `flox deactivate`, returning a descriptive error if not.
///
/// `flox deactivate` records its request in a file the shell's prompt hook reads
/// on the next prompt; with no compatible hook registered, that file is never
/// consumed and the command would be a silent no-op. Two ways it can be missing:
/// - `disable_hook = true` in the config turns the prompt hook off entirely.
/// - The hook exports [`PROMPT_HOOK_VERSION_ENV`] at activation: an unset value
///   means no hook is set up (or it predates this marker), and a non-matching
///   value means it was set up by an incompatible version of Flox.
fn ensure_prompt_hook_available(config: &Config) -> Result<()> {
    if config.flox.disable_hook.unwrap_or(false) {
        bail!(formatdoc! {"
            The Flox prompt hook is disabled ('disable_hook = true'), so 'flox deactivate' cannot take effect on the next prompt.
            Re-enable it with 'flox config --delete disable_hook', then deactivate again.
        "});
    }

    match std::env::var(PROMPT_HOOK_VERSION_ENV)
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
    {
        Some(version) if version == PROMPT_HOOK_VERSION => Ok(()),
        Some(_) => bail!(formatdoc! {"
            This shell's Flox prompt hook was set up by an incompatible version of Flox.
            Restart your shell to pick up the current version, then deactivate again.
        "}),
        None => bail!(formatdoc! {"
            The Flox prompt hook is not set up in this shell, so 'flox deactivate' cannot take effect on the next prompt.
            Restart your shell to activate with the prompt hook, then deactivate again.
        "}),
    }
}

/// The data needed to deactivate the front-of-stack active environment.
struct DeactivationTarget {
    /// Activation state dir, for the `flox-activations detach` call.
    activation_state_dir: PathBuf,
    /// Rendered env link (`$FLOX_ENV`) the activate path used, needed to restore
    /// the prompt and PATH on an in-place deactivation.
    flox_env: PathBuf,
}

/// Resolve an active-stack entry into the data needed to deactivate it.
///
/// Opens the concrete environment so managed and remote environments are also
/// supported (not just local path environments). The rendered env link is
/// resolved via the entry's activation `mode` rather than read from `$FLOX_ENV`
/// at runtime, which is what lets an env that isn't the most recently activated
/// one be torn down. This is also where a future `flox deactivate <ENV>`
/// argument would resolve a specific environment rather than the last-active
/// one.
fn open_deactivation_target(flox: &Flox, active: ActiveEnvironment) -> Result<DeactivationTarget> {
    let mode = active.mode;
    let mut concrete_env = active
        .environment
        .into_concrete_environment(flox, None)
        .context("failed to open active environment for deactivation")?;
    let dot_flox_path = concrete_env.dot_flox_path().to_path_buf();
    let flox_env = concrete_env
        .rendered_env_links(flox)
        .context("failed to read rendered env links for active environment")?
        .for_mode(&mode)
        .to_path_buf();
    let activation_state_dir = activation_state_dir_path(&flox.runtime_dir, &dot_flox_path);

    Ok(DeactivationTarget {
        activation_state_dir,
        flox_env,
    })
}

/// The subsystem verbosity to thread into a generated deactivation script, read
/// from `_FLOX_SUBSYSTEM_VERBOSITY` (0 when unset or unparseable).
pub(crate) fn flox_activate_tracelevel() -> u32 {
    std::env::var("_FLOX_SUBSYSTEM_VERBOSITY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Emit a deactivation script for `invocation_kind` to `writer`.
///
/// Shared by `flox deactivate --print-script` and `flox hook-env`:
/// - `Interactive` → `exit;` (the subshell exits and the executive cleans up
///   state.json when the PID goes away).
/// - `InPlace`/`ShellCommand` → restore env vars, then emit a
///   `flox-activations detach` command so state.json is updated once the caller
///   eval's the script.
/// - `ExecCommand` → unreachable; `_FLOX_INVOCATION_TYPE` is never `exec_command`.
pub(crate) fn emit_deactivate_script(
    shell: ShellWithPath,
    invocation_kind: InvocationKind,
    activation_state_dir: &Path,
    flox_env: &Path,
    flox_activate_tracelevel: u32,
    writer: &mut impl Write,
) -> Result<()> {
    match invocation_kind {
        InvocationKind::Interactive => {
            write!(writer, "exit;")?;
            Ok(())
        },
        InvocationKind::InPlace | InvocationKind::ShellCommand => {
            flox_activations::deactivate::generate_deactivate_script(
                shell,
                writer,
                &*FLOX_INTERPRETER,
                &FLOX_ACTIVATIONS_BIN,
                activation_state_dir,
                flox_env,
                flox_activate_tracelevel,
            )
            .context("failed to generate deactivation script")
        },
        InvocationKind::ExecCommand => {
            bail!("cannot deactivate an exec command activation");
        },
    }
}
