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

/// Detection result shared by subshell activation and metrics, cached to
/// spend at most one parent-process probe per process. In-place activation
/// resolves parent-before-`SHELL` and stays uncached.
///
/// Sound only while nothing mutates `FLOX_SHELL`/`SHELL` in-process (true
/// today): the cache initializes earlier when metrics are on, so anything
/// that mutates them must bypass this cache or metrics-on and metrics-off
/// runs would resolve different shells.
static SHELL_CHAIN_DETECTION: LazyLock<Option<ShellWithPath>> =
    LazyLock::new(|| detect_shell_for_metrics_with(ShellWithPath::detect_from_parent_process));

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
    subshell_shell_from_chain(SHELL_CHAIN_DETECTION.clone())
}

/// Utility method for testing implementing the logic of shell detection
/// for subshells, generically over a parent shell detection function.
fn detect_shell_for_subshell_with(
    parent_shell_fn: impl Fn() -> Result<ShellWithPath>,
) -> ShellWithPath {
    subshell_shell_from_chain(detect_shell_for_metrics_with(parent_shell_fn))
}

/// Shared by the cached production path and the test seam so the fallback
/// cannot drift between them.
fn subshell_shell_from_chain(detected: Option<ShellWithPath>) -> ShellWithPath {
    detected.unwrap_or_else(|| {
        warn!("Failed to detect shell from environment or parent process. Defaulting to bash");
        ShellWithPath::Bash(INTERACTIVE_BASH_BIN.clone())
    })
}

/// Normalized name of the detected shell for metrics context: no
/// bundled-bash fallback — an undetected shell (e.g. `SHELL=/bin/sh` with
/// an unreadable parent) is `None`, never `bash`. Returns only the static
/// name so the shell's path (and the username inside it) cannot reach a
/// telemetry caller.
pub(crate) fn detect_shell_name_for_metrics() -> Option<&'static str> {
    SHELL_CHAIN_DETECTION.as_ref().map(|shell| shell.name())
}

/// Utility method implementing the logic of shell detection for metrics,
/// generically over a parent shell detection function.
fn detect_shell_for_metrics_with(
    parent_shell_fn: impl Fn() -> Result<ShellWithPath>,
) -> Option<ShellWithPath> {
    ShellWithPath::detect_from_env("FLOX_SHELL")
        .or_else(|err| {
            debug!("Failed to detect shell from FLOX_SHELL: {err}");
            ShellWithPath::detect_from_env("SHELL")
        })
        .or_else(|err| {
            debug!("Failed to detect shell from SHELL: {err}");
            parent_shell_fn()
        })
        .inspect_err(|err| debug!("Failed to detect shell from parent process: {err}"))
        .ok()
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
    fn detect_shell_for_metrics_uses_env_chain() {
        temp_env::with_vars([FLOX_SHELL_SET, SHELL_SET], || {
            let shell = detect_shell_for_metrics_with(|| unreachable!());
            assert_eq!(shell, Some(ShellWithPath::Bash("/flox_shell/bash".into())));
        });

        temp_env::with_vars([FLOX_SHELL_UNSET, SHELL_SET], || {
            let shell = detect_shell_for_metrics_with(|| unreachable!());
            assert_eq!(shell, Some(ShellWithPath::Bash("/shell/bash".into())));
        });

        temp_env::with_vars([FLOX_SHELL_UNSET, SHELL_UNSET], || {
            let shell = detect_shell_for_metrics_with(PARENT_DETECTED);
            assert_eq!(shell, Some(ShellWithPath::Bash("/parent/bash".into())));
        });
    }

    #[test]
    fn detect_shell_for_metrics_reports_nothing_when_uncertain() {
        // Unlike the activation chain, total failure must surface as `None`,
        // never as the bundled-bash fallback — reporting bash for a user
        // whose shell we could not identify would fabricate the dimension.
        temp_env::with_vars([FLOX_SHELL_UNSET, SHELL_UNSET], || {
            let shell = detect_shell_for_metrics_with(PARENT_UNDETECTED);
            assert_eq!(shell, None);
        });
    }

    #[test]
    fn detect_shell_for_metrics_unsupported_shell_is_not_bash() {
        // `/bin/sh` is not a supported shell. The chain may still identify
        // the invoking shell via the parent process, but when it cannot,
        // the answer is `None` — not the bundled bash.
        const SH_SHELL: (&str, Option<&str>) = ("SHELL", Some("/bin/sh"));
        temp_env::with_vars([FLOX_SHELL_UNSET, SH_SHELL], || {
            let shell = detect_shell_for_metrics_with(PARENT_UNDETECTED);
            assert_eq!(shell, None);
        });

        temp_env::with_vars([FLOX_SHELL_UNSET, SH_SHELL], || {
            let shell =
                detect_shell_for_metrics_with(|| Ok(ShellWithPath::Zsh("/parent/zsh".into())));
            assert_eq!(shell, Some(ShellWithPath::Zsh("/parent/zsh".into())));
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
