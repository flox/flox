use std::io::{BufWriter, Write, stdout};

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_core::activate::context::InvocationType;
use flox_core::activate::mode::ActivateMode;
use flox_core::activations::{
    ActivationState,
    activation_state_dir_path,
    read_activations_json,
    state_json_path,
    write_activations_json,
};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::utils::FLOX_INTERPRETER;
use fslock::LockFile;
use indoc::{formatdoc, indoc};

use super::{activated_environments, uninitialized_environment_description};
use crate::commands::activate::ActivateOptions;
use crate::subcommand_metric;
use crate::utils::message;

/// Environment variable set by `flox activate` to the `.flox` directory path.
const FLOX_ENV_PROJECT_VAR: &str = "FLOX_ENV_PROJECT";

/// The action `flox deactivate --print-script` should take for the current
/// shell.
#[derive(Debug, PartialEq)]
pub(crate) enum DetachAction {
    /// The shell is an interactive subshell spawned by `flox activate`.
    /// Emit `exit` so the shell exits and the environment is deactivated.
    Exit,
    /// The shell is not an interactive subshell (e.g., in-place activation or
    /// a user subshell such as `flox activate && bash && flox deactivate`).
    /// Emit in-place deactivation commands and remove the attachment from
    /// `state.json`.
    Inplace,
}

/// Determine the detach action for the calling shell.
///
/// Reads the invocation type for `shell_pid` from `activation_state` and
/// returns:
/// - `DetachAction::Exit` when the attachment has invocation type
///   `Interactive` — the shell is a subshell, so `exit` will deactivate it.
/// - `DetachAction::Inplace` in all other cases: in-place activations, user
///   subshells, or a missing/ambiguous attachment.
///
/// # In-place PID asymmetry
///
/// For in-place activations, the calling shell's PID is never recorded as an
/// attachment in `state.json` (the executive and `flox-activations` record the
/// PID of the `flox activate` process itself, which differs from the user
/// shell in the in-place flow). Therefore in-place deactivation always takes
/// the `Inplace` path regardless of the PID lookup result. This is correct
/// behavior — the shell continues running, we just unset environment
/// variables — but it means the PID lookup miss for in-place activations is
/// expected and not an error.
pub(crate) fn decide_detach_action(
    activation_state: &ActivationState,
    shell_pid: i32,
) -> DetachAction {
    match activation_state.invocation_type_for_pid(shell_pid) {
        // Attached with Interactive invocation type → exit the subshell.
        Some(Some(InvocationType::Interactive)) => DetachAction::Exit,
        // PID not found at all — could be an in-place activation (expected)
        // or a stale/race-condition miss.
        None => {
            tracing::warn!(
                shell_pid,
                "shell PID not found in state.json during deactivate; \
                 treating as in-place"
            );
            DetachAction::Inplace
        },
        // invocation_type field absent (pre-DEV-78 state.json), InPlace,
        // ShellCommand, or ExecCommand.
        Some(_) => DetachAction::Inplace,
    }
}

/// Emit the deactivation script to `writer`.
///
/// For `DetachAction::Exit`: writes `exit;`.
///
/// For `DetachAction::Inplace`: removes the shell PID attachment from
/// `state.json` (under the provided `lock`).
///
/// Env-var restoration for the `Inplace` path is emitted separately by
/// the caller before invoking this function (via `generate_deactivate_script`).
pub(crate) fn emit_detach_script(
    action: DetachAction,
    writer: &mut impl Write,
    state_json_path: &std::path::Path,
    state: ActivationState,
    lock: LockFile,
    shell_pid: i32,
) -> Result<()> {
    match action {
        DetachAction::Exit => {
            // Release the lock — we don't need to touch state.json.
            // The executive monitors the shell PID and cleans up when it exits.
            drop(lock);
            write!(writer, "exit;")?;
        },
        DetachAction::Inplace => {
            // Detach the shell PID under the lock we already hold so the
            // read-then-write is atomic with respect to other processes.
            let mut updated_state = state;
            updated_state.detach(shell_pid);
            // Per ActivationState::detach doc contract: update_ready_after_detach
            // must be called after detach, but only when there are still
            // attached PIDs (the function panics on an empty map).
            if !updated_state.attached_pids_is_empty() {
                updated_state.update_ready_after_detach();
            }
            // Write back only when there are still attached PIDs or an
            // executive; otherwise the executive will clean up naturally.
            if !updated_state.attached_pids_is_empty() || updated_state.executive_started() {
                write_activations_json(&updated_state, state_json_path, lock)
                    .context("failed to write state.json after detach")?;
            }
        },
    }
    Ok(())
}

#[derive(Bpaf, Clone)]
pub struct Deactivate {
    /// Print a deactivation script to stdout instead of showing instructions
    #[bpaf(long("print-script"), hide)]
    pub print_script: bool,
}

impl Deactivate {
    pub fn handle(self, flox: Flox) -> Result<()> {
        if !flox.features.auto_activate {
            return self.old_exit(flox);
        }

        subcommand_metric!("deactivate");

        if self.print_script {
            return self.handle_print_script(flox);
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

    /// Handle `flox deactivate --print-script`.
    ///
    /// Reads `state.json` for the environment identified by
    /// `FLOX_ENV_PROJECT` and emits a script that either exits the calling
    /// shell (for interactive subshell activations) or performs an in-place
    /// deactivation (env-var restoration + state.json attachment removal).
    fn handle_print_script(self, flox: Flox) -> Result<()> {
        // TODO: might make sense to move detect_shell_for_in_place
        // off ActivateOptions
        let shell = ActivateOptions::detect_shell_for_in_place()?;

        let raw_env_project = std::env::var(FLOX_ENV_PROJECT_VAR).with_context(|| {
            format!("No Flox environment is active ({FLOX_ENV_PROJECT_VAR} is not set).")
        })?;
        let canonical_env_project = std::fs::canonicalize(&raw_env_project).with_context(|| {
            format!(
                "Failed to canonicalize {FLOX_ENV_PROJECT_VAR} path '{}': \
                     the path may not exist or may contain invalid components.",
                raw_env_project
            )
        })?;
        let dot_flox_path = canonical_env_project.join(".flox");

        let activation_state_dir = activation_state_dir_path(&flox.runtime_dir, &dot_flox_path);
        let state_path = state_json_path(&activation_state_dir);

        // Acquire the lock once; hold it through the write for Inplace.
        let (state_opt, lock) = read_activations_json(&state_path)
            .with_context(|| format!("failed to read state.json at '{}'", state_path.display()))?;

        // Parent PID of `flox deactivate` is the calling shell.
        let shell_pid = std::os::unix::process::parent_id() as i32;

        let (action, state) = match state_opt {
            Some(state) => {
                let action = decide_detach_action(&state, shell_pid);
                (action, state)
            },
            // No state.json — treat as in-place no-op.
            None => {
                let empty_state = ActivationState::new(
                    &ActivateMode::default(),
                    Some(&dot_flox_path),
                    dot_flox_path.join("run/default"),
                );
                (DetachAction::Inplace, empty_state)
            },
        };

        let mut writer = BufWriter::new(stdout());

        match action {
            DetachAction::Exit => {
                // For interactive subshells, just emit `exit;`.
                // The exitrc script handles env-var restoration as the shell exits.
                emit_detach_script(
                    DetachAction::Exit,
                    &mut writer,
                    &state_path,
                    state,
                    lock,
                    shell_pid,
                )?;
            },
            DetachAction::Inplace => {
                // For in-place activations, emit env-var restoration first, then
                // remove the PID attachment from state.json.
                flox_activations::deactivate::generate_deactivate_script(
                    shell,
                    &mut writer,
                    &*FLOX_INTERPRETER,
                )?;
                emit_detach_script(
                    DetachAction::Inplace,
                    &mut writer,
                    &state_path,
                    state,
                    lock,
                    shell_pid,
                )?;
            },
        }

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
    use std::path::PathBuf;

    use flox_core::activate::context::InvocationType;
    use flox_core::activate::mode::ActivateMode;
    use flox_core::activations::test_helpers::{read_activation_state, write_activation_state};
    use flox_core::activations::{
        ActivationState,
        StartIdentifier,
        activation_state_dir_path,
        read_activations_json,
        state_json_path,
    };
    use tempfile::TempDir;

    use super::*;

    /// Build a minimal `ActivationState` with a single attached PID and the
    /// given invocation type.
    ///
    /// Sets `executive_pid = 1` so the state can be written to disk (the
    /// write helper requires a non-zero executive PID).
    fn state_with_pid(
        dot_flox_path: &std::path::Path,
        pid: i32,
        invocation_type: InvocationType,
    ) -> ActivationState {
        let mut state = ActivationState::new(
            &ActivateMode::default(),
            Some(dot_flox_path),
            dot_flox_path.join("run/default"),
        );
        state.set_executive_pid(1);
        let start_id = StartIdentifier::new("/nix/store/test");
        state.start_or_attach(pid, &start_id.store_path, invocation_type);
        state
    }

    fn write_state(
        runtime_dir: &std::path::Path,
        dot_flox_path: &std::path::Path,
        pid: i32,
        invocation_type: InvocationType,
    ) {
        let state = state_with_pid(dot_flox_path, pid, invocation_type);
        write_activation_state(runtime_dir, dot_flox_path, state);
    }

    fn read_state_from_disk(
        runtime_dir: &std::path::Path,
        dot_flox_path: &std::path::Path,
    ) -> (ActivationState, fslock::LockFile) {
        let activation_state_dir = activation_state_dir_path(runtime_dir, dot_flox_path);
        let path = state_json_path(&activation_state_dir);
        let (state_opt, lock) = read_activations_json(&path).unwrap();
        (state_opt.unwrap(), lock)
    }

    // --- Unit tests for decide_detach_action ---

    #[test]
    fn interactive_invocation_type_returns_exit() {
        let dot_flox_path = PathBuf::from("/tmp/test/.flox");
        let pid = 12345_i32;
        let state = state_with_pid(&dot_flox_path, pid, InvocationType::Interactive);
        assert_eq!(
            decide_detach_action(&state, pid),
            DetachAction::Exit,
            "should return Exit for interactive subshell"
        );
    }

    #[test]
    fn inplace_invocation_type_returns_inplace() {
        let dot_flox_path = PathBuf::from("/tmp/test/.flox");
        let pid = 12345_i32;
        let state = state_with_pid(&dot_flox_path, pid, InvocationType::InPlace);
        assert_eq!(
            decide_detach_action(&state, pid),
            DetachAction::Inplace,
            "should return Inplace for in-place activation"
        );
    }

    #[test]
    fn pid_not_attached_returns_inplace() {
        let dot_flox_path = PathBuf::from("/tmp/test/.flox");
        let attached_pid = 12345_i32;
        let unattached_pid = 99999_i32;
        let state = state_with_pid(&dot_flox_path, attached_pid, InvocationType::Interactive);
        assert_eq!(
            decide_detach_action(&state, unattached_pid),
            DetachAction::Inplace,
            "should return Inplace when PID is not in state.json"
        );
    }

    #[test]
    fn shell_command_invocation_type_returns_inplace() {
        let dot_flox_path = PathBuf::from("/tmp/test/.flox");
        let pid = 12345_i32;
        let state = state_with_pid(
            &dot_flox_path,
            pid,
            InvocationType::ShellCommand("ls".to_string()),
        );
        assert_eq!(
            decide_detach_action(&state, pid),
            DetachAction::Inplace,
            "should return Inplace for shell command invocation"
        );
    }

    #[test]
    fn exec_command_invocation_type_returns_inplace() {
        let dot_flox_path = PathBuf::from("/tmp/test/.flox");
        let pid = 12345_i32;
        let state = state_with_pid(
            &dot_flox_path,
            pid,
            InvocationType::ExecCommand(vec!["ls".to_string(), "-la".to_string()]),
        );
        assert_eq!(
            decide_detach_action(&state, pid),
            DetachAction::Inplace,
            "should return Inplace for exec command invocation"
        );
    }

    /// Pre-DEV-78 backward compatibility: state.json written before the
    /// `invocation_type` field was added deserializes to `None` for that
    /// field. The attachment still exists in the map (outer `Some`), but
    /// `invocation_type` is absent (`Some(None)`). This should be treated
    /// as Inplace, not Exit.
    #[test]
    fn missing_invocation_type_returns_inplace() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");
        let pid = 12345_i32;

        // Write a normal Interactive state to disk, then read it back as raw
        // JSON, remove the `invocation_type` field from the attachment, and
        // write that modified JSON back. This simulates a state.json produced
        // before DEV-82 added the field.
        write_state(tmp.path(), &dot_flox_path, pid, InvocationType::Interactive);

        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);
        let state_path = state_json_path(&activation_state_dir);
        let raw_json_str = std::fs::read_to_string(&state_path).unwrap();
        let mut json_val: serde_json::Value = serde_json::from_str(&raw_json_str).unwrap();

        // Strip `invocation_type` from the attachment for this PID.
        json_val["attached_pids"][pid.to_string()]
            .as_object_mut()
            .unwrap()
            .remove("invocation_type");

        std::fs::write(
            &state_path,
            serde_json::to_string_pretty(&json_val).unwrap(),
        )
        .unwrap();

        let (state_opt, _lock) = read_activations_json(&state_path).unwrap();
        let state = state_opt.expect("state should be present");

        // Confirm we read back Some(None) — attached but no invocation_type.
        assert_eq!(
            state.invocation_type_for_pid(pid),
            Some(None),
            "pre-DEV-78 attachment should have invocation_type = None"
        );
        assert_eq!(
            decide_detach_action(&state, pid),
            DetachAction::Inplace,
            "missing invocation_type should be treated as Inplace"
        );
    }

    // --- Unit tests for emit_detach_script ---

    #[test]
    fn exit_action_writes_exit_statement() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");
        // We still need a lock file path even though Exit doesn't write.
        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);
        let state_path = state_json_path(&activation_state_dir);
        // Create the directory so lock acquisition succeeds.
        std::fs::create_dir_all(&activation_state_dir).unwrap();
        let lock = flox_core::activations::acquire_activations_json_lock(&state_path).unwrap();

        let state = ActivationState::new(
            &ActivateMode::default(),
            Some(&dot_flox_path),
            dot_flox_path.join("run/default"),
        );

        let mut buf = Vec::new();
        emit_detach_script(DetachAction::Exit, &mut buf, &state_path, state, lock, 42).unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "exit;",
            "exit action should emit exactly 'exit;'"
        );
    }

    #[test]
    fn inplace_action_removes_attachment_from_state() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");
        let pid = 12345_i32;

        write_state(tmp.path(), &dot_flox_path, pid, InvocationType::InPlace);

        let (state, lock) = read_state_from_disk(tmp.path(), &dot_flox_path);

        let mut buf = Vec::new();
        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);
        let state_path = state_json_path(&activation_state_dir);
        emit_detach_script(
            DetachAction::Inplace,
            &mut buf,
            &state_path,
            state,
            lock,
            pid,
        )
        .unwrap();

        // Verify the attachment was removed from state.json on disk.
        let updated = read_activation_state(tmp.path(), &dot_flox_path);
        assert!(
            updated.invocation_type_for_pid(pid).is_none(),
            "PID attachment should be removed from state.json"
        );
    }

    /// When there is no state.json (e.g., a bare in-place activation that
    /// never wrote state), the empty `ActivationState` should pass through
    /// without creating a file or writing any output.
    #[test]
    fn inplace_action_with_no_state_writes_nothing() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");
        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);
        let state_path = state_json_path(&activation_state_dir);
        // Create the lock directory but do NOT create state.json itself.
        std::fs::create_dir_all(&activation_state_dir).unwrap();
        let lock = flox_core::activations::acquire_activations_json_lock(&state_path).unwrap();

        let empty_state = ActivationState::new(
            &ActivateMode::default(),
            Some(&dot_flox_path),
            dot_flox_path.join("run/default"),
        );

        let mut buf = Vec::new();
        emit_detach_script(
            DetachAction::Inplace,
            &mut buf,
            &state_path,
            empty_state,
            lock,
            42,
        )
        .unwrap();

        // No state.json should have been created.
        assert!(
            !state_path.exists(),
            "state.json should not be created when state is empty"
        );
        // No output should have been emitted.
        assert!(
            buf.is_empty(),
            "no output should be emitted for empty in-place state"
        );
    }

    #[test]
    fn inplace_action_does_not_emit_exit() {
        let tmp = TempDir::new().unwrap();
        let dot_flox_path = tmp.path().join(".flox");
        let pid = 12345_i32;

        write_state(tmp.path(), &dot_flox_path, pid, InvocationType::InPlace);

        let (state, lock) = read_state_from_disk(tmp.path(), &dot_flox_path);

        let mut buf = Vec::new();
        let activation_state_dir = activation_state_dir_path(tmp.path(), &dot_flox_path);
        let state_path = state_json_path(&activation_state_dir);
        emit_detach_script(
            DetachAction::Inplace,
            &mut buf,
            &state_path,
            state,
            lock,
            pid,
        )
        .unwrap();

        let output = String::from_utf8(buf).unwrap();
        assert!(
            !output.contains("exit"),
            "in-place action should not contain 'exit' in output"
        );
    }
}
