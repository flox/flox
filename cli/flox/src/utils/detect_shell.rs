use std::env;
use std::path::PathBuf;
use std::sync::LazyLock;

use anyhow::Result;
use shell_gen::ShellWithPath;
use tracing::{debug, warn};

use crate::utils::openers::CliShellExt;

pub static INTERACTIVE_BASH_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    PathBuf::from(
        env::var("INTERACTIVE_BASH_BIN").unwrap_or(env!("INTERACTIVE_BASH_BIN").to_string()),
    )
});

/// Detect the shell to use for activation
///
/// Used to determine shell for
/// `flox activate` and `flox activate -- CMD`
///
/// Returns the first shell found in the following order:
/// 1. FLOX_SHELL environment variable
/// 2. SHELL environment variable
/// 3. Parent process shell
/// 4. Default to bash bundled with flox
pub(crate) fn detect_shell_for_subshell() -> ShellWithPath {
    detect_shell_for_subshell_with(ShellWithPath::detect_from_parent_process)
}

/// Utility method for testing implementing the logic of shell detection
/// for subshells, generically over a parent shell detection function.
fn detect_shell_for_subshell_with(
    parent_shell_fn: impl Fn() -> Result<ShellWithPath>,
) -> ShellWithPath {
    ShellWithPath::detect_from_env("FLOX_SHELL")
        .or_else(|err| {
            debug!("Failed to detect shell from FLOX_SHELL: {err}");
            ShellWithPath::detect_from_env("SHELL")
        })
        .or_else(|err| {
            debug!("Failed to detect shell from SHELL: {err}");
            parent_shell_fn()
        })
        .unwrap_or_else(|err| {
            debug!("Failed to detect shell from parent process: {err}");
            warn!("Failed to detect shell from environment or parent process. Defaulting to bash");
            ShellWithPath::Bash(INTERACTIVE_BASH_BIN.clone())
        })
}

/// Detect the shell to use for in-place activation
///
/// Used to determine shell for `eval "$(flox activate)"`,
/// `flox activate --print-script`, and
/// when adding activation of a default environment to RC files.
pub(crate) fn detect_shell_for_in_place() -> Result<ShellWithPath> {
    detect_shell_for_in_place_with(ShellWithPath::detect_from_parent_process)
}

/// Utility method for testing implementing the logic of shell detection
/// for in-place activation, generically over a parent shell detection function.
fn detect_shell_for_in_place_with(
    parent_shell_fn: impl Fn() -> Result<ShellWithPath>,
) -> Result<ShellWithPath> {
    ShellWithPath::detect_from_env("FLOX_SHELL")
        .or_else(|_| parent_shell_fn())
        .or_else(|err| {
            warn!("Failed to detect shell from environment: {err}");
            ShellWithPath::detect_from_env("SHELL")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHELL_SET: (&'_ str, Option<&'_ str>) = ("SHELL", Some("/shell/bash"));
    const FLOX_SHELL_SET: (&'_ str, Option<&'_ str>) = ("FLOX_SHELL", Some("/flox_shell/bash"));
    const SHELL_UNSET: (&'_ str, Option<&'_ str>) = ("SHELL", None);
    const FLOX_SHELL_UNSET: (&'_ str, Option<&'_ str>) = ("FLOX_SHELL", None);
    const PARENT_DETECTED: &dyn Fn() -> Result<ShellWithPath> =
        &|| Ok(ShellWithPath::Bash("/parent/bash".into()));
    const PARENT_UNDETECTED: &dyn Fn() -> Result<ShellWithPath> =
        &|| Err(anyhow::anyhow!("parent shell detection failed"));

    #[test]
    fn detect_shell_for_subshell() {
        temp_env::with_vars([FLOX_SHELL_UNSET, SHELL_SET], || {
            let shell = detect_shell_for_subshell_with(|| unreachable!());
            assert_eq!(shell, ShellWithPath::Bash("/shell/bash".into()));
        });

        temp_env::with_vars([FLOX_SHELL_SET, SHELL_SET], || {
            let shell = detect_shell_for_subshell_with(|| unreachable!());
            assert_eq!(shell, ShellWithPath::Bash("/flox_shell/bash".into()));
        });

        temp_env::with_vars([FLOX_SHELL_UNSET, SHELL_UNSET], || {
            let shell = detect_shell_for_subshell_with(PARENT_DETECTED);
            assert_eq!(shell, ShellWithPath::Bash("/parent/bash".into()));
        });

        temp_env::with_vars([FLOX_SHELL_UNSET, SHELL_UNSET], || {
            let shell = detect_shell_for_subshell_with(PARENT_UNDETECTED);
            assert_eq!(shell, ShellWithPath::Bash(INTERACTIVE_BASH_BIN.clone()));
        });
    }

    #[test]
    fn detect_shell_for_in_place() {
        // $SHELL is used as a fallback only if parent detection fails
        temp_env::with_vars([FLOX_SHELL_UNSET, SHELL_SET], || {
            let shell = detect_shell_for_in_place_with(PARENT_DETECTED).unwrap();
            assert_eq!(shell, ShellWithPath::Bash("/parent/bash".into()));

            // fall back to $SHELL if parent detection fails
            let shell = detect_shell_for_in_place_with(PARENT_UNDETECTED).unwrap();
            assert_eq!(shell, ShellWithPath::Bash("/shell/bash".into()));
        });

        // $FLOX_SHELL takes precedence over $SHELL and detected parent shell
        temp_env::with_vars([FLOX_SHELL_SET, SHELL_SET], || {
            let shell = detect_shell_for_in_place_with(PARENT_DETECTED).unwrap();
            assert_eq!(shell, ShellWithPath::Bash("/flox_shell/bash".into()));

            let shell = detect_shell_for_in_place_with(PARENT_UNDETECTED).unwrap();
            assert_eq!(shell, ShellWithPath::Bash("/flox_shell/bash".into()));
        });

        // if both $FLOX_SHELL and $SHELL are unset, we should fail iff parent detection fails
        temp_env::with_vars([FLOX_SHELL_UNSET, SHELL_UNSET], || {
            let shell = detect_shell_for_in_place_with(PARENT_DETECTED).unwrap();
            assert_eq!(shell, ShellWithPath::Bash("/parent/bash".into()));

            let shell = detect_shell_for_in_place_with(PARENT_UNDETECTED);
            assert!(shell.is_err());
        });
    }
}
