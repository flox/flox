use std::io::{BufWriter, Write, stdout};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use bpaf::Bpaf;
use flox_config::Config;
use flox_core::activate::context::{InvocationKind, InvocationTypes};
use flox_core::activate::vars::{FLOX_ACTIVATIONS_BIN, FLOX_INVOCATION_TYPES_WIRE_VAR};
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
use tracing::debug;

use super::activated_environments;
use crate::subcommand_metric;
use crate::utils::active_environments::ActiveEnvironment;
use crate::utils::detect_shell::detect_shell_for_in_place;
use crate::utils::message;

#[derive(Bpaf, Clone)]
pub struct Deactivate {
    /// The calling shell's `$_FLOX_INVOCATION_TYPES` map, for print-script
    /// mode (hidden, for shell hook use).
    ///
    /// When provided, emits a deactivation script to stdout. The entry for
    /// the layer being popped (the most recently activated environment)
    /// determines the exit strategy:
    /// - `"interactive"` → emit an exit (subshell will exit and clean up);
    ///   see [`emit_deactivate_script`] for the tcsh-specific form
    /// - anything else → emit in-place env-var restoration, plus a detach
    ///   command unless the layer has no entry (the shell never activated
    ///   the layer being popped, so its PID was never attached)
    #[bpaf(long("print-script"), argument("INVOCATION_TYPES"), optional, hide)]
    pub print_script: Option<String>,

    /// Like `--print-script`, but reading the map from
    /// `_FLOX_INVOCATION_TYPES_WIRE`: tcsh cannot pass JSON values on a
    /// backtick command line, so callers there export the short-lived
    /// variable around this call (hidden, for shell hook use).
    #[bpaf(long("print-script-from-env"), switch, hide)]
    pub print_script_from_env: bool,
}

impl Deactivate {
    pub fn handle(self, config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("deactivate");

        if self.print_script_from_env {
            if self.print_script.is_some() {
                bail!("--print-script and --print-script-from-env are mutually exclusive");
            }
            // A missing wire variable reads like an empty map: the safe
            // interpretation is that this shell performed no activations.
            let invocation_types =
                std::env::var(FLOX_INVOCATION_TYPES_WIRE_VAR).unwrap_or_default();
            return self.handle_print_script(flox, invocation_types);
        }

        if let Some(invocation_types) = self.print_script.clone() {
            return self.handle_print_script(flox, invocation_types);
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

        debug!(
            flox_env = ?target.flox_env,
            "requesting hook-env performs deactivation"
        );
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
    /// restoration, plus a `flox-activations detach` command when this shell
    /// attached to the activation).
    ///
    /// The `invocation_types` argument is the caller's
    /// `_FLOX_INVOCATION_TYPES` shell variable — the invocation types of the
    /// activations that shell performed, keyed by environment pointer —
    /// removing the need to read state.json inside this binary. Only the
    /// entry for the layer being popped matters here: deactivation pops one
    /// layer. A missing entry means the shell never activated that layer
    /// (e.g. a subshell that inherited the activation's environment), so the
    /// script restores the environment without detaching.
    fn handle_print_script(self, flox: Flox, invocation_types: String) -> Result<()> {
        let shell = detect_shell_for_in_place()?;

        let active = activated_environments()
            .last_active_full()
            .ok_or_else(|| anyhow!("No environment active."))?;

        let mut remaining_invocation_types = InvocationTypes::from_str(&invocation_types)
            .context("could not determine invocation type".to_string())?;
        let key = serde_json::to_value(&active.environment)
            .context("could not serialize the environment pointer")?;
        let invocation_kind = remaining_invocation_types.take(&key);
        // Embed the consumed map in the emitted script only when an entry was
        // taken; no entry (a shell that never activated this layer) leaves
        // the variable alone.
        let invocation_types_update = invocation_kind
            .is_some()
            .then_some(remaining_invocation_types);

        let target = open_deactivation_target(&flox, active)?;

        let mut writer = BufWriter::new(stdout());
        emit_deactivate_script(
            shell,
            invocation_kind,
            invocation_types_update,
            &target.activation_state_dir,
            &target.flox_env,
            flox_activate_tracelevel(),
            None,
            &mut writer,
        )?;
        writer.flush()?;
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
pub(crate) struct DeactivationTarget {
    /// Activation state dir, for the `flox-activations detach` call.
    pub(crate) activation_state_dir: PathBuf,
    /// Rendered env link (`$FLOX_ENV`) the activate path used, needed to restore
    /// the prompt and PATH on an in-place deactivation.
    pub(crate) flox_env: PathBuf,
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
pub(crate) fn open_deactivation_target(
    flox: &Flox,
    active: ActiveEnvironment,
) -> Result<DeactivationTarget> {
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
/// Shared by `flox deactivate --print-script` and `flox hook-env`.
/// `invocation_kind` is the invocation-type stack entry for the layer being
/// popped — the entry the eval'ing shell pushed when it activated that layer:
/// - `Some(Interactive)` → an exit (the subshell exits and the executive
///   cleans up state.json when the PID goes away). For most shells this is a
///   literal `exit;`, but tcsh removes a special alias (`precmd`/`cwdcmd`) —
///   printing "Faulty alias 'precmd' removed." — without exiting the shell
///   when `exit` unwinds out of an `eval` inside it. The tcsh prompt hook
///   evals this script from exactly that position, so for tcsh emit a flag
///   the alias body checks after the eval, exiting at the alias top level
///   instead (see `tcsh_hook` in `flox-activations`).
/// - `Some(InPlace | ShellCommand)` → restore env vars, then emit a
///   `flox-activations detach` command so state.json is updated once the
///   caller eval's the script.
/// - `None` → the eval'ing shell never activated this layer (it inherited the
///   activation's exported environment, e.g. a subshell): restore env vars
///   but emit no detach — this shell's PID was never attached, and the shell
///   that did attach detaches itself when it deactivates.
/// - `Some(ExecCommand)` → unreachable; `_FLOX_INVOCATION_TYPES` never
///   records `execcommand`.
///
/// `invocation_types` is the consumed remainder of the eval'ing shell's
/// `_FLOX_INVOCATION_TYPES` map, written back to the shell variable by the
/// generated script (`None` leaves it alone) — see
/// `DeactivateCtx::invocation_types` in `flox-activations`. The interactive
/// script exits the shell, so it never writes the map back.
///
/// `encoded_diff` selects which layer's diff to restore: `None` restores the
/// front of the stack from this process's `_FLOX_HOOK_DIFF`; `Some` restores
/// an explicitly provided diff, which is how the prompt hook pops several
/// stacked layers in one run (see `embedded_hook_diff`).
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_deactivate_script(
    shell: ShellWithPath,
    invocation_kind: Option<InvocationKind>,
    invocation_types: Option<InvocationTypes>,
    activation_state_dir: &Path,
    flox_env: &Path,
    flox_activate_tracelevel: u32,
    encoded_diff: Option<&str>,
    writer: &mut impl Write,
) -> Result<()> {
    match invocation_kind {
        Some(InvocationKind::Interactive) => {
            debug!("triggering exit for interactive deactivation");
            match shell {
                ShellWithPath::Tcsh(_) => write!(writer, "set _flox_exit=1;")?,
                _ => write!(writer, "exit;")?,
            }
            Ok(())
        },
        Some(InvocationKind::InPlace | InvocationKind::ShellCommand) | None => {
            debug!("emitting deactivation script for environment");
            let emit_detach = invocation_kind.is_some();
            match encoded_diff {
                Some(encoded_diff) => {
                    flox_activations::deactivate::generate_deactivate_script_with_diff(
                        shell,
                        writer,
                        &*FLOX_INTERPRETER,
                        &FLOX_ACTIVATIONS_BIN,
                        activation_state_dir,
                        flox_env,
                        flox_activate_tracelevel,
                        encoded_diff,
                        emit_detach,
                        invocation_types,
                    )
                },
                None => flox_activations::deactivate::generate_deactivate_script(
                    shell,
                    writer,
                    &*FLOX_INTERPRETER,
                    &FLOX_ACTIVATIONS_BIN,
                    activation_state_dir,
                    flox_env,
                    flox_activate_tracelevel,
                    emit_detach,
                    invocation_types,
                ),
            }
            .context("failed to generate deactivation script")
        },
        Some(InvocationKind::ExecCommand) => {
            bail!("cannot deactivate an exec command activation");
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use flox_activations::attach_diff::diff_serializer::DiffSerializer;

    use super::*;

    fn interactive_deactivate_script(shell: ShellWithPath) -> String {
        let mut buf = Vec::new();
        emit_deactivate_script(
            shell,
            Some(InvocationKind::Interactive),
            Some(InvocationTypes::default()),
            Path::new("/activation_state_dir"),
            Path::new("/flox_env"),
            0,
            None,
            &mut buf,
        )
        .unwrap();
        String::from_utf8(buf).unwrap()
    }

    fn in_place_deactivate_script(
        invocation_kind: Option<InvocationKind>,
        invocation_types: Option<InvocationTypes>,
    ) -> String {
        let encoded_diff = DiffSerializer {
            added: HashSet::from(["TEST_VAR".to_string()]),
            modified: HashMap::new(),
            removed: HashMap::new(),
        }
        .encode()
        .unwrap();
        let mut buf = Vec::new();
        emit_deactivate_script(
            ShellWithPath::Bash("bash".into()),
            invocation_kind,
            invocation_types,
            Path::new("/activation_state_dir"),
            Path::new("/flox_env"),
            0,
            Some(&encoded_diff),
            &mut buf,
        )
        .unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn in_place_deactivate_emits_detach_and_map_remainder() {
        let remainder =
            r#"[{"env":{"name":"outer","type":"path"},"invocation_type":"interactive"}]"#;
        let script = in_place_deactivate_script(
            Some(InvocationKind::InPlace),
            Some(remainder.parse().unwrap()),
        );
        assert!(
            script.contains("detach --activation-state-dir"),
            "expected a detach command:\n{script}"
        );
        assert!(
            script.contains("_FLOX_INVOCATION_TYPES=") && script.contains(remainder),
            "expected the consumed map written back:\n{script}"
        );
    }

    #[test]
    fn in_place_deactivate_unsets_emptied_map() {
        // Popping the shell's only own activation empties the map: the
        // script unsets the variable rather than assigning an empty value.
        let script = in_place_deactivate_script(
            Some(InvocationKind::InPlace),
            Some(InvocationTypes::default()),
        );
        assert!(
            script.contains("unset _FLOX_INVOCATION_TYPES;"),
            "expected the emptied map unset:\n{script}"
        );
    }

    #[test]
    fn deactivate_without_invocation_type_omits_detach() {
        // No invocation-type entry means the eval'ing shell never attached
        // to the activation (e.g. a subshell that inherited the activation's
        // environment): restore the environment, but emit no detach — that
        // shell's PID was never attached — and leave the (unset) map
        // variable alone.
        let script = in_place_deactivate_script(None, None);
        assert!(
            !script.contains("detach --activation-state-dir"),
            "expected no detach command:\n{script}"
        );
        assert!(
            !script.contains("_FLOX_INVOCATION_TYPES"),
            "expected the map variable left alone:\n{script}"
        );
    }

    #[test]
    fn interactive_deactivate_emits_exit_for_bash() {
        let script = interactive_deactivate_script(ShellWithPath::Bash("bash".into()));
        assert_eq!(script, "exit;");
    }

    #[test]
    fn interactive_deactivate_emits_exit_flag_for_tcsh() {
        // The tcsh prompt hook evals this script inside the precmd/cwdcmd alias
        // body; an `exit` there makes tcsh remove the alias ("Faulty alias
        // 'precmd' removed.") without exiting, so the script sets a flag the
        // alias checks after the eval instead.
        let script = interactive_deactivate_script(ShellWithPath::Tcsh("tcsh".into()));
        assert_eq!(script, "set _flox_exit=1;");
    }
}
