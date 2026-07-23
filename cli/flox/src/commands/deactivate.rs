use std::io::{BufWriter, Write, stdout};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use bpaf::Bpaf;
use flox_config::Config;
use flox_core::activate::context::{InvocationKind, InvocationTypes};
use flox_core::activate::vars::{FLOX_ACTIVATIONS_BIN, FLOX_INVOCATION_TYPES_WIRE_VAR};
use flox_core::activations::activation_state_dir_path;
use flox_core::canonical_path::CanonicalPath;
use flox_core::hook_actions::{
    HookAction,
    PROMPT_HOOK_VERSION_ENV,
    prompt_hook_version_mismatched,
    write_hook_actions,
};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
use flox_rust_sdk::models::environment::{
    DOT_FLOX,
    DotFlox,
    GCROOTS_DIR_NAME,
    RenderedEnvironmentLinks,
    UninitializedEnvironment,
};
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

        let target = deactivation_target(&flox, &active);

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

        let target = deactivation_target(&flox, &active);
        // TODO: if disable_hook = true, we'll error later on when we
        // find PROMPT_HOOK_VERSION is not set
        // We'd hit this case if someone has an environment in their RC files, upgrades, and then runs `flox deactivate --in-place ...`
        // We could maybe figure out a way to handle that case, but because that's hidden, ignore for now
        let prompt_hook_version = std::env::var(PROMPT_HOOK_VERSION_ENV).ok();

        let mut writer = BufWriter::new(stdout());
        emit_deactivate_script(
            shell,
            invocation_kind,
            invocation_types_update,
            &target.activation_state_dir,
            &target.flox_env,
            flox_activate_tracelevel(),
            prompt_hook_version.as_deref(),
            None,
            &mut writer,
        )?;
        writer.flush()?;
        Ok(())
    }
}

/// Verify this shell has a prompt hook able to service a plain
/// `flox deactivate`, returning a descriptive error if not.
///
/// `flox deactivate` records its request in a file the prompt hook reads on the
/// next prompt; with no compatible hook, that file is never consumed and the
/// command is a silent no-op. Two ways it can be missing: `disable_hook = true`
/// turns the hook off, or [`PROMPT_HOOK_VERSION_ENV`] is unset (no hook set up in
/// this shell) or set to an incompatible version.
fn ensure_prompt_hook_available(config: &Config) -> Result<()> {
    if config.flox.disable_hook.unwrap_or(false) {
        bail!(formatdoc! {"
            The Flox prompt hook is disabled ('disable_hook = true'), so 'flox deactivate' cannot take effect on the next prompt.
            Re-enable it with 'flox config --delete disable_hook', then deactivate again.
        "});
    }

    let prompt_hook_version = std::env::var(PROMPT_HOOK_VERSION_ENV).ok();

    if prompt_hook_version_mismatched(prompt_hook_version.as_deref()) {
        bail!(formatdoc! {"
            This shell's Flox prompt hook was set up by an incompatible version of Flox.
            Restart your shell to pick up the current version, then deactivate again.
        "});
    }

    if prompt_hook_version.is_none() {
        bail!(formatdoc! {"
            The Flox prompt hook is not set up in this shell, so 'flox deactivate' cannot take effect on the next prompt.
            Restart your shell to activate with the prompt hook, then deactivate again.
        "});
    }

    Ok(())
}

/// Verify `prompt_hook_version` matches the compiled [`PROMPT_HOOK_VERSION`]
/// before [`emit_deactivate_script`] decodes any diff the hook produced.
///
/// Checked once at the sole chokepoint for diff decoding (see
/// `emit_deactivate_script`'s docs), so everything downstream — the
/// nested-layer decode in `flox-activations` and the `DiffSerializer` payload
/// it decodes — can assume the payload is current. An absent marker (no hook)
/// and a mismatched one both mean the diff could be stale or a shape the binary
/// no longer understands, so both fail the same way.
fn ensure_prompt_hook_version_current(prompt_hook_version: Option<&str>) -> Result<()> {
    if prompt_hook_version_mismatched(prompt_hook_version) || prompt_hook_version.is_none() {
        bail!(formatdoc! {"
            This shell's Flox prompt hook is out of sync with the running Flox.
            Restart your shell to pick up the current version, then try again.
        "});
    }

    Ok(())
}

/// The data needed to deactivate the front-of-stack active environment.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DeactivationTarget {
    /// Activation state dir, for the `flox-activations detach` call.
    pub(crate) activation_state_dir: PathBuf,
    /// Rendered env link (`$FLOX_ENV`) the activate path used, needed to restore
    /// the prompt and PATH on an in-place deactivation.
    pub(crate) flox_env: PathBuf,
}

/// Resolve an active-stack entry into the data needed to deactivate it.
///
/// Everything derives from the entry itself, recorded in the shell's
/// environment when the layer was activated; the environment is deliberately
/// never opened. Opening re-derives data the stack already records, and can
/// rebuild the environment, contact FloxHub, or fail outright (deleted
/// directory, broken manifest, expired auth) — none of which should be able
/// to stop or slow down a teardown. Deriving the rendered env link from the
/// entry rather than reading `$FLOX_ENV` at runtime is what lets an env that
/// isn't the most recently activated one be torn down. This is also where a
/// future `flox deactivate <ENV>` argument would resolve a specific
/// environment rather than the last-active one.
///
/// Both fields reproduce the activation's exact bytes:
/// - The activation state dir is keyed by a hash of the `.flox` path the
///   activation used. The stack entry records that canonicalized path, so it
///   is used verbatim rather than re-canonicalized (symlink resolution may
///   have changed since, and fails once the directory is deleted).
/// - `flox_env` is built with the same link constructors activation used,
///   from the same recorded inputs, so it byte-matches the `$FLOX_ENV` the
///   activation exported — the deactivation script matches it against the
///   shell's script-tracking state by string equality. The per-shell entry
///   is the only trustworthy source for this: `state.json` is shared by
///   every activation of the same `.flox` path and keeps the values of
///   whichever activation created it, which can describe a different
///   generation than the layer being torn down. If the link no longer exists
///   on disk (deleted environment or link), env-var restoration and detach
///   are unaffected; only the env's own `profile.deactivate` scripts are
///   skipped, where opening would have rebuilt the link first.
pub(crate) fn deactivation_target(flox: &Flox, active: &ActiveEnvironment) -> DeactivationTarget {
    let dot_flox_path = recorded_dot_flox_path(flox, &active.environment);
    let activation_state_dir = activation_state_dir_path(&flox.runtime_dir, &dot_flox_path);

    // Previously canonicalized path that may no longer exist.
    let run_dir = CanonicalPath::new_unchecked(dot_flox_path.join(GCROOTS_DIR_NAME));
    let name = active.environment.name();
    let links = match active.generation {
        Some(generation) => {
            RenderedEnvironmentLinks::new_in_base_dir_with_name_system_and_generation(
                &run_dir,
                name.as_ref(),
                &flox.system,
                generation,
            )
        },
        None => RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system(
            &run_dir,
            name.as_ref(),
            &flox.system,
        ),
    };
    let flox_env = links.for_mode(&active.mode).to_path_buf();

    DeactivationTarget {
        activation_state_dir,
        flox_env,
    }
}

/// The `.flox` path this entry's activation used, without opening the
/// environment.
///
/// Local environments record their canonicalized `.flox` path in the active
/// stack, used verbatim — already the exact bytes activation hashed, and
/// re-canonicalizing could only diverge from them (a retargeted symlink) or
/// fail (a deleted directory). Remote environments record only a pointer, so
/// the path is re-derived: joining the pointer onto the cache dir names the
/// checkout, but activation canonicalized that join
/// ([`RemoteEnvironment::new_in`]) before hashing it into the state-dir name
/// and building `$FLOX_ENV`. The cache dir is used as configured, never
/// canonicalized, so whenever it sits behind a symlink the raw join differs
/// from the path activation used — canonicalizing the same way here
/// reproduces the activation's exact bytes. If the checkout was deleted
/// mid-session, canonicalization fails and the raw join stands in: the state
/// dir selected from it may then miss, in which case the emitted detach is a
/// no-op, but the teardown still proceeds.
fn recorded_dot_flox_path(flox: &Flox, environment: &UninitializedEnvironment) -> PathBuf {
    match environment {
        UninitializedEnvironment::DotFlox(DotFlox { path, .. }) => path.clone(),
        UninitializedEnvironment::Remote(pointer) => {
            let dot_flox_path = RemoteEnvironment::checkout_path(flox, pointer).join(DOT_FLOX);
            match CanonicalPath::new(&dot_flox_path) {
                Ok(path) => path.into_inner(),
                Err(err) => {
                    debug!(%err, "could not canonicalize remote checkout path");
                    dot_flox_path
                },
            }
        },
    }
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
/// Shared by `flox deactivate --print-script` and `flox hook-env`, and the
/// sole chokepoint for diff decoding: the only caller of
/// `generate_deactivate_script[_with_diff]` in `flox-activations`. It first
/// checks `prompt_hook_version` via [`ensure_prompt_hook_version_current`], so
/// decoding below — the nested-layer decode and the `DiffSerializer` payload
/// itself — can assume the diff is current.
///
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
///
/// `prompt_hook_version` is the caller's exported [`PROMPT_HOOK_VERSION_ENV`],
/// read once at the boundary and threaded in (like `flox_activate_tracelevel`)
/// so this function never reads the process environment.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_deactivate_script(
    shell: ShellWithPath,
    invocation_kind: Option<InvocationKind>,
    invocation_types: Option<InvocationTypes>,
    activation_state_dir: &Path,
    flox_env: &Path,
    flox_activate_tracelevel: u32,
    prompt_hook_version: Option<&str>,
    encoded_diff: Option<&str>,
    writer: &mut impl Write,
) -> Result<()> {
    // TODO: we error high up in handle() for hook-env and activate, should we
    // do the same in deactivate for consistency?
    ensure_prompt_hook_version_current(prompt_hook_version)?;

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
    use flox_core::activate::mode::ActivateMode;
    use flox_core::activations::{
        ActivationState,
        acquire_activations_json_lock,
        state_json_path,
        write_activations_json,
    };
    use flox_core::hook_actions::PROMPT_HOOK_VERSION;
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::generations::GenerationId;
    use flox_rust_sdk::models::environment::{EnvironmentPointer, ManagedPointer, PathPointer};

    use super::*;

    /// The current [`PROMPT_HOOK_VERSION`] as the shell hook would export it,
    /// passed to [`emit_deactivate_script`] so its version guard sees a
    /// compatible hook.
    fn current_prompt_hook_version() -> String {
        PROMPT_HOOK_VERSION.to_string()
    }

    fn interactive_deactivate_script(shell: ShellWithPath) -> String {
        let mut buf = Vec::new();
        emit_deactivate_script(
            shell,
            Some(InvocationKind::Interactive),
            Some(InvocationTypes::default()),
            Path::new("/activation_state_dir"),
            Path::new("/flox_env"),
            0,
            Some(&current_prompt_hook_version()),
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
            Some(&current_prompt_hook_version()),
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

    /// An active-stack entry for a path environment named `proj` at
    /// `dot_flox_path`.
    fn active_path_environment(dot_flox_path: &Path, mode: ActivateMode) -> ActiveEnvironment {
        ActiveEnvironment {
            environment: UninitializedEnvironment::DotFlox(DotFlox {
                path: dot_flox_path.to_path_buf(),
                pointer: EnvironmentPointer::Path(PathPointer::new("proj".parse().unwrap())),
            }),
            generation: None,
            mode,
        }
    }

    /// Write a state.json recording `flox_env` for an activation of
    /// `dot_flox_path` in `mode`, returning the activation state dir.
    fn write_activation_state(
        runtime_dir: &Path,
        dot_flox_path: &Path,
        mode: &ActivateMode,
        flox_env: &Path,
    ) -> PathBuf {
        let activation_state_dir = activation_state_dir_path(runtime_dir, dot_flox_path);
        std::fs::create_dir_all(&activation_state_dir).unwrap();
        let state_json = state_json_path(&activation_state_dir);
        let mut state = ActivationState::new(mode, Some(dot_flox_path), flox_env);
        state.set_executive_pid(1);
        let lock = acquire_activations_json_lock(&state_json).unwrap();
        write_activations_json(&state, &state_json, lock).unwrap();
        activation_state_dir
    }

    #[test]
    fn deactivation_target_for_deleted_directory() {
        let (flox, tempdir) = flox_instance();
        // Never created: stands in for a project directory deleted while its
        // environment was active.
        let dot_flox_path = tempdir.path().join("proj/.flox");

        let target = deactivation_target(
            &flox,
            &active_path_environment(&dot_flox_path, ActivateMode::Run),
        );
        assert_eq!(target, DeactivationTarget {
            activation_state_dir: activation_state_dir_path(&flox.runtime_dir, &dot_flox_path),
            flox_env: dot_flox_path.join(format!("run/{}.proj-run", flox.system)),
        });
    }

    #[test]
    fn deactivation_target_ignores_recorded_activation_state() {
        // Every activation of the same `.flox` path shares one state dir, and
        // state.json's flox_env is written once, by whichever activation
        // created the file — later layers can export a different link. Plant
        // a state.json from a generation-pinned creator, then tear down an
        // environment without a defined generation at the same path: the
        // target must derive the environment's own link (no generation
        // suffix) from its stack entry, not take the recorded gen4 link,
        // while still resolving the shared state dir.
        let (flox, tempdir) = flox_instance();
        let dot_flox_path = tempdir.path().join("proj/.flox");
        let activation_state_dir = write_activation_state(
            &flox.runtime_dir,
            &dot_flox_path,
            &ActivateMode::Dev,
            &dot_flox_path.join(format!("run/{}.proj.gen4-dev", flox.system)),
        );

        let target = deactivation_target(
            &flox,
            &active_path_environment(&dot_flox_path, ActivateMode::Dev),
        );
        assert_eq!(target, DeactivationTarget {
            activation_state_dir,
            flox_env: dot_flox_path.join(format!("run/{}.proj-dev", flox.system)),
        });
    }

    #[test]
    fn deactivation_target_includes_generation() {
        let (flox, tempdir) = flox_instance();
        let dot_flox_path = tempdir.path().join("proj/.flox");

        let target = deactivation_target(&flox, &ActiveEnvironment {
            environment: UninitializedEnvironment::DotFlox(DotFlox {
                path: dot_flox_path.clone(),
                pointer: EnvironmentPointer::Path(PathPointer::new("proj".parse().unwrap())),
            }),
            generation: Some(GenerationId::from(4usize)),
            mode: ActivateMode::Run,
        });
        assert_eq!(target, DeactivationTarget {
            activation_state_dir: activation_state_dir_path(&flox.runtime_dir, &dot_flox_path),
            flox_env: dot_flox_path.join(format!("run/{}.proj.gen4-run", flox.system)),
        });
    }

    #[test]
    fn deactivation_target_for_remote_environment_uses_canonicalized_checkout() {
        let (mut flox, tempdir) = flox_instance();
        // Route the cache dir through a symlink so the raw `checkout_path`
        // join and its canonicalized form provably differ on every platform.
        let cache_link = tempdir.path().join("cache_link");
        std::os::unix::fs::symlink(&flox.cache_dir, &cache_link).unwrap();
        flox.cache_dir = cache_link;
        let pointer = ManagedPointer::new(
            "owner".parse().unwrap(),
            "proj".parse().unwrap(),
            &flox.floxhub,
        );

        // Build the expectation the way activation does: create the checkout,
        // then canonicalize its path (as `RemoteEnvironment::new_in` does
        // before the path is hashed into the state-dir name).
        let raw_dot_flox_path = RemoteEnvironment::checkout_path(&flox, &pointer).join(DOT_FLOX);
        std::fs::create_dir_all(&raw_dot_flox_path).unwrap();
        let dot_flox_path = std::fs::canonicalize(&raw_dot_flox_path).unwrap();
        // Guards the test against becoming a no-op: were the paths equal, the
        // assertion below would hold even if `deactivation_target` skipped
        // canonicalization.
        assert_ne!(dot_flox_path, raw_dot_flox_path);

        let target = deactivation_target(&flox, &ActiveEnvironment {
            environment: UninitializedEnvironment::Remote(pointer),
            generation: None,
            mode: ActivateMode::Dev,
        });
        assert_eq!(target, DeactivationTarget {
            activation_state_dir: activation_state_dir_path(&flox.runtime_dir, &dot_flox_path),
            flox_env: dot_flox_path.join(format!("run/{}.proj-dev", flox.system)),
        });
    }

    #[test]
    fn deactivation_target_for_remote_environment_with_missing_checkout() {
        // The checkout was deleted mid-session: canonicalization is
        // impossible, so the uncanonicalized derivation stands in and the
        // teardown still resolves.
        let (flox, _tempdir) = flox_instance();
        let pointer = ManagedPointer::new(
            "owner".parse().unwrap(),
            "proj".parse().unwrap(),
            &flox.floxhub,
        );

        let target = deactivation_target(&flox, &ActiveEnvironment {
            environment: UninitializedEnvironment::Remote(pointer.clone()),
            generation: None,
            mode: ActivateMode::Run,
        });

        let dot_flox_path = RemoteEnvironment::checkout_path(&flox, &pointer).join(DOT_FLOX);
        assert_eq!(target, DeactivationTarget {
            activation_state_dir: activation_state_dir_path(&flox.runtime_dir, &dot_flox_path),
            flox_env: dot_flox_path.join(format!("run/{}.proj-run", flox.system)),
        });
    }
}
