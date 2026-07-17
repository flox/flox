//! Deactivation script generation for restoring environment variables.
//!
//! This module provides functionality to generate shell scripts that restore
//! the environment to its pre-activation state by decoding and applying the
//! `_FLOX_HOOK_DIFF` variable captured during activation.

use std::env;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use flox_core::activate::context::InvocationTypes;
use shell_gen::{Shell, ShellWithPath};

use crate::attach_diff::diff_serializer::{DiffSerializer, FLOX_HOOK_DIFF_VAR};
use crate::gen_rc::bash::{BashStartupArgs, generate_bash_profile_commands};
use crate::gen_rc::fish::{FishStartupArgs, generate_fish_profile_commands};
use crate::gen_rc::tcsh::{TcshStartupArgs, generate_tcsh_profile_commands};
use crate::gen_rc::zsh::{ZshStartupArgs, generate_zsh_profile_commands};
use crate::gen_rc::{Action, DeactivateCtx};

/// Generate a deactivation script for the specified shell.
///
/// This reads the `_FLOX_HOOK_DIFF` environment variable, decodes it,
/// and generates shell commands to:
/// - Unset variables that were added during activation
/// - Restore variables that were modified during activation
/// - Restore variables that were removed during activation
/// - Unset `_FLOX_HOOK_DIFF` itself
///
/// Returns an error if `_FLOX_HOOK_DIFF` is not set in the environment or
/// cannot be decoded.
///
/// With `emit_detach`, a `flox-activations detach` command follows the
/// per-shell env-var restoration so that state.json is updated when the
/// caller eval's the script; the shell-specific self-PID variable expands to
/// the caller's PID at eval time. Pass `false` when the eval'ing shell never
/// attached to the activation (its `_FLOX_INVOCATION_TYPES` map has no entry
/// for this layer, e.g. a subshell that inherited the activation's
/// environment): there is nothing to detach, and the attached shell detaches
/// itself when it deactivates.
///
/// `invocation_types` is the consumed remainder of that map to write back to
/// the eval'ing shell, or `None` to leave the variable alone — see
/// [`DeactivateCtx::invocation_types`].
#[allow(clippy::too_many_arguments)]
pub fn generate_deactivate_script(
    shell: ShellWithPath,
    writer: &mut impl Write,
    interpreter_path: impl AsRef<Path>,
    flox_activations_bin: &Path,
    activation_state_dir: &Path,
    flox_env: &Path,
    flox_activate_tracelevel: u32,
    emit_detach: bool,
    invocation_types: Option<InvocationTypes>,
) -> Result<()> {
    let encoded_diff = env::var(FLOX_HOOK_DIFF_VAR)
        .context(format!("{} not set in environment", FLOX_HOOK_DIFF_VAR))?;
    generate_deactivate_script_with_diff(
        shell,
        writer,
        interpreter_path,
        flox_activations_bin,
        activation_state_dir,
        flox_env,
        flox_activate_tracelevel,
        &encoded_diff,
        emit_detach,
        invocation_types,
    )
}

/// Like [`generate_deactivate_script`], but for an explicitly provided encoded
/// diff rather than the caller's `_FLOX_HOOK_DIFF`.
///
/// Used by the prompt hook to pop several stacked in-place activations in one
/// run: the shell's `_FLOX_HOOK_DIFF` only ever holds the front layer's diff,
/// and deeper layers' diffs are obtained by walking the chain with
/// [`embedded_hook_diff`].
#[allow(clippy::too_many_arguments)]
pub fn generate_deactivate_script_with_diff(
    shell: ShellWithPath,
    writer: &mut impl Write,
    interpreter_path: impl AsRef<Path>,
    flox_activations_bin: &Path,
    activation_state_dir: &Path,
    flox_env: &Path,
    flox_activate_tracelevel: u32,
    encoded_diff: &str,
    emit_detach: bool,
    invocation_types: Option<InvocationTypes>,
) -> Result<()> {
    let activate_d = interpreter_path.as_ref().join("activate.d");
    let restore_diff = DiffSerializer::decode(encoded_diff)
        .context(format!("Failed to decode {}", FLOX_HOOK_DIFF_VAR))?;
    let ctx = DeactivateCtx {
        activate_d,
        flox_env: flox_env.to_path_buf(),
        flox_activate_tracelevel,
        restore_diff,
        flox_activations: flox_activations_bin.to_path_buf(),
        invocation_types,
    };

    // Capture the shell variant before consuming `shell` in the match below.
    let shell_variant = Shell::from(shell.clone());

    match shell {
        ShellWithPath::Bash(_) => {
            let action: Action<BashStartupArgs> = Action::Deactivate(ctx);
            generate_bash_profile_commands(&action, writer)
        },
        ShellWithPath::Zsh(_) => {
            let action: Action<ZshStartupArgs> = Action::Deactivate(ctx);
            generate_zsh_profile_commands(&action, writer)
        },
        ShellWithPath::Fish(_) => {
            let action: Action<FishStartupArgs> = Action::Deactivate(ctx);
            generate_fish_profile_commands(&action, writer)
        },
        ShellWithPath::Tcsh(_) => {
            let action: Action<TcshStartupArgs> = Action::Deactivate(ctx);
            generate_tcsh_profile_commands(&action, writer)
        },
    }?;

    // Emit the detach command using the shell-appropriate self-PID variable
    // so that the expression expands correctly when the caller eval's the
    // output.  `$$` is correct for bash/zsh/tcsh; fish uses `$fish_pid`.
    // Skipped when the eval'ing shell never attached to the activation
    if emit_detach {
        let pid_var = shell_variant.self_pid_var();
        writeln!(
            writer,
            r#""{}" detach --activation-state-dir "{}" --pid {pid_var};"#,
            flox_activations_bin.display(),
            activation_state_dir.display(),
        )?;
    }

    Ok(())
}

/// The next (outer) layer's encoded `_FLOX_HOOK_DIFF`, as captured inside
/// `encoded_diff`.
///
/// In-place activations track `_FLOX_HOOK_DIFF` in their diff (see
/// `AttachDiff`), so a nested layer's diff records the previous layer's
/// encoded value under `modified`. Decoding one layer's diff therefore yields
/// the diff of the layer beneath it, forming a chain from the front of the
/// activation stack to the outermost in-place layer. Returns `None` for the
/// outermost layer (the variable had no previous value, so it is in `added`)
/// and for non-in-place activations (which don't track the variable at all).
pub fn embedded_hook_diff(encoded_diff: &str) -> Result<Option<String>> {
    let diff = DiffSerializer::decode(encoded_diff)
        .context(format!("Failed to decode {}", FLOX_HOOK_DIFF_VAR))?;
    Ok(diff.modified.get(FLOX_HOOK_DIFF_VAR).cloned())
}
