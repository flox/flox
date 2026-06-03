//! Deactivation script generation for restoring environment variables.
//!
//! This module provides functionality to generate shell scripts that restore
//! the environment to its pre-activation state by decoding and applying the
//! `_FLOX_HOOK_DIFF` variable captured during activation.

use std::env;
use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
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
/// After the per-shell env-var restoration, emits a `flox-activations detach`
/// command so that state.json is updated when the caller eval's the script.
/// The shell-specific self-PID variable expands to the caller's PID at
/// eval time.
pub fn generate_deactivate_script(
    shell: ShellWithPath,
    writer: &mut impl Write,
    interpreter_path: impl AsRef<Path>,
    flox_activations_bin: &Path,
    activation_state_dir: &Path,
    flox_env: &Path,
) -> Result<()> {
    let activate_d = interpreter_path.as_ref().join("activate.d");
    let encoded_diff = env::var(FLOX_HOOK_DIFF_VAR)
        .context(format!("{} not set in environment", FLOX_HOOK_DIFF_VAR))?;
    let restore_diff = DiffSerializer::decode(&encoded_diff)
        .context(format!("Failed to decode {}", FLOX_HOOK_DIFF_VAR))?;
    let ctx = DeactivateCtx {
        activate_d,
        flox_env: flox_env.to_path_buf(),
        restore_diff,
        flox_activations: flox_activations_bin.to_path_buf(),
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
    let pid_var = shell_variant.self_pid_var();
    writeln!(
        writer,
        r#""{}" detach --activation-state-dir "{}" --pid {pid_var};"#,
        flox_activations_bin.display(),
        activation_state_dir.display(),
    )?;

    Ok(())
}
