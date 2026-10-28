use std::env;
use std::ffi::OsString;
use std::io::Stderr;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use flox_core::util::default_nix_env_vars;

pub mod active_environments;
pub mod catalog_lock;
pub mod colors;
pub mod credential_store;
pub mod detached;
pub mod detect_shell;
pub mod dialog;
pub mod didyoumean;
pub mod errors;
pub mod events;
pub mod init;
pub mod local_environment_id;
pub mod markdown;
pub mod message;
pub mod metrics;
pub mod openers;
pub mod search;
pub mod table;
pub mod tracing;
pub mod update_notifications;
pub mod upgrade_output;

pub static TERMINAL_STDERR: LazyLock<Mutex<Stderr>> =
    LazyLock::new(|| Mutex::new(std::io::stderr()));

/// Path of the `flox-activations` binary this CLI spawns.
///
/// Compiled in, since the binary is a separate package. `FLOX_ACTIVATIONS_BIN`
/// overrides it, which the dev shell uses to point at the cargo-built binary.
pub static FLOX_ACTIVATIONS_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    PathBuf::from(
        env::var("FLOX_ACTIVATIONS_BIN").unwrap_or(env!("FLOX_ACTIVATIONS_BIN").to_string()),
    )
});

/// The command a shell can use to reach this flox again.
///
/// Embedded in code the shell keeps running after Flox may have been upgraded:
/// the prompt hook `flox activate` registers and the activation commands
/// `flox hook-env` emits for it. `current_exe()` would pin this process's store
/// path, which keeps an open shell on the old binary after an upgrade (and
/// breaks it once that store path is garbage-collected). argv[0] is how the
/// user reached flox instead: a bare name resolves through PATH every time the
/// hook runs, and a path names the installer- or profile-managed location, both
/// of which an upgrade replaces.
pub static FLOX_BIN_FOR_SHELL: LazyLock<String> = LazyLock::new(|| {
    flox_bin_for_shell(
        env::args_os().next(),
        env::current_dir().ok(),
        env::current_exe().ok(),
    )
});

/// See [`FLOX_BIN_FOR_SHELL`]. A relative `argv0` is anchored to `cwd` but never
/// canonicalized, since resolving symlinks would pin the store path again.
fn flox_bin_for_shell(
    argv0: Option<OsString>,
    cwd: Option<PathBuf>,
    exe: Option<PathBuf>,
) -> String {
    let fallback = || {
        exe.as_deref()
            .map(Path::to_string_lossy)
            .map_or_else(|| "flox".to_string(), |exe| exe.into_owned())
    };
    let Some(argv0) = argv0.filter(|argv0| !argv0.is_empty()) else {
        return fallback();
    };
    let argv0 = PathBuf::from(argv0);
    // A bare name (one component) is left to the shell's PATH lookup.
    if argv0.is_absolute() || argv0.components().count() == 1 {
        return argv0.to_string_lossy().into_owned();
    }
    match cwd {
        Some(cwd) => cwd.join(argv0).to_string_lossy().into_owned(),
        None => fallback(),
    }
}

/// Timeout used for network operations that run after the main flox command has
/// completed.
///
/// This is used for metrics submission and checking for updates.
pub const TRAILING_NETWORK_CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// Set the default nix environment variables for the current process
/// (see `default_nix_env_vars` for the rule).
///
/// SAFETY: called once, first thing in main, while the process is still
/// single-threaded.
pub fn populate_default_nix_env_vars() {
    let env_map = default_nix_env_vars();
    for (key, value) in env_map {
        unsafe { env::set_var(key, value) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe() -> Option<PathBuf> {
        Some(PathBuf::from("/nix/store/abc-flox/bin/.flox-wrapped"))
    }

    #[test]
    fn bare_name_is_left_for_path_lookup() {
        assert_eq!(
            flox_bin_for_shell(Some("flox".into()), Some("/work".into()), exe()),
            "flox"
        );
    }

    #[test]
    fn relative_path_is_anchored_to_cwd() {
        assert_eq!(
            flox_bin_for_shell(
                Some("./target/debug/flox".into()),
                Some("/work".into()),
                exe()
            ),
            "/work/./target/debug/flox"
        );
    }

    #[test]
    fn absolute_path_is_kept_unresolved() {
        assert_eq!(
            flox_bin_for_shell(
                Some("/usr/local/bin/flox".into()),
                Some("/work".into()),
                exe()
            ),
            "/usr/local/bin/flox"
        );
    }

    #[test]
    fn missing_argv0_falls_back_to_the_running_binary() {
        assert_eq!(
            flox_bin_for_shell(None, Some("/work".into()), exe()),
            "/nix/store/abc-flox/bin/.flox-wrapped"
        );
    }
}
