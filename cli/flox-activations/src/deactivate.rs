//! Deactivation script generation for restoring environment variables.
//!
//! This module provides functionality to generate shell scripts that restore
//! the environment to its pre-activation state by decoding and applying the
//! `_FLOX_HOOK_DIFF` variable captured during activation.

use std::io::Write;
use std::path::Path;

use anyhow::Result;
use shell_gen::ShellWithPath;

use crate::gen_rc::Action;
use crate::gen_rc::bash::{BashStartupArgs, generate_bash_profile_commands};
use crate::gen_rc::fish::{FishStartupArgs, generate_fish_profile_commands};
use crate::gen_rc::tcsh::{TcshStartupArgs, generate_tcsh_profile_commands};
use crate::gen_rc::zsh::{ZshStartupArgs, generate_zsh_profile_commands};

/// Generate a deactivation script for the specified shell.
///
/// This reads the `_FLOX_HOOK_DIFF` environment variable (if present),
/// decodes it, and generates shell commands to:
/// - Unset variables that were added during activation
/// - Restore variables that were modified during activation
/// - Restore variables that were removed during activation
/// - Unset `_FLOX_HOOK_DIFF` itself
///
/// Returns an error if `_FLOX_HOOK_DIFF` is not set in the environment or
/// cannot be decoded.
pub fn generate_deactivate_script(
    shell: ShellWithPath,
    writer: &mut impl Write,
    interpreter_path: impl AsRef<Path>,
) -> Result<()> {
    let activate_d = interpreter_path.as_ref().join("activate.d");

    match shell {
        ShellWithPath::Bash(_) => {
            let action: Action<BashStartupArgs> = Action::Deactivate { activate_d };
            generate_bash_profile_commands(&action, writer)
        },
        ShellWithPath::Zsh(_) => {
            let action: Action<ZshStartupArgs> = Action::Deactivate { activate_d };
            generate_zsh_profile_commands(&action, writer)
        },
        ShellWithPath::Fish(_) => {
            let action: Action<FishStartupArgs> = Action::Deactivate { activate_d };
            generate_fish_profile_commands(&action, writer)
        },
        ShellWithPath::Tcsh(_) => {
            let action: Action<TcshStartupArgs> = Action::Deactivate { activate_d };
            generate_tcsh_profile_commands(&action, writer)
        },
    }
}
