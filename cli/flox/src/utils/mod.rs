use std::env;
use std::ffi::{OsStr, OsString};
use std::io::Stderr;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use flox_activations::hook::tcsh_alias_can_embed;
use flox_core::util::default_nix_env_vars;
use shell_gen::ShellWithPath;

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
/// user reached flox instead: a path names the installer- or profile-managed
/// location, which an upgrade replaces in place.
///
/// A bare name is resolved against PATH here, once, rather than left for the
/// shell to resolve on every prompt. The hook keeps running after activation
/// has put the environment's own `bin` first on PATH, so a `flox` installed in
/// the environment would otherwise take over `hook-env` for the rest of the
/// session, with no guarantee it can read the state this flox wrote. The
/// resolved path is not canonicalized: `/usr/local/bin/flox` or
/// `~/.nix-profile/bin/flox` is what an upgrade replaces, while the store path
/// behind it is not.
pub static FLOX_BIN_FOR_SHELL: LazyLock<String> = LazyLock::new(|| {
    flox_bin_for_shell(
        env::args_os().next(),
        env::current_dir().ok(),
        env::var_os("PATH"),
        env::current_exe().ok(),
    )
});

/// See [`FLOX_BIN_FOR_SHELL`]. A relative `argv0` is anchored to `cwd` and a
/// bare one is looked up on `path_env` (relative PATH entries anchored the
/// same way); neither is canonicalized.
fn flox_bin_for_shell(
    argv0: Option<OsString>,
    cwd: Option<PathBuf>,
    path_env: Option<OsString>,
    exe: Option<PathBuf>,
) -> String {
    let lossy = |path: &Path| path.to_string_lossy().into_owned();
    let running_binary = || exe.as_deref().map_or_else(|| "flox".to_string(), lossy);
    let Some(argv0) = argv0.filter(|argv0| !argv0.is_empty()) else {
        return running_binary();
    };
    let argv0 = PathBuf::from(argv0);
    if argv0.is_absolute() {
        return lossy(&argv0);
    }
    if argv0.components().count() == 1 {
        // The shell searched this same PATH, so a miss means the name was
        // set by a wrapper and would not work from the hook either.
        return find_on_path(&argv0, path_env.as_deref(), cwd.as_deref())
            .map_or_else(running_binary, |p| lossy(&p));
    }
    cwd.map_or_else(running_binary, |cwd| lossy(&cwd.join(&argv0)))
}

/// The command the prompt hook embeds for `shell`: [`FLOX_BIN_FOR_SHELL`],
/// unless the shell is tcsh and its alias body cannot carry that path (see
/// [`flox_activations::hook::tcsh_hook`]). The hook then pins this process's binary, as it did before
/// argv[0] was used, and that shell needs a restart after an upgrade.
pub fn flox_bin_for_hook(shell: &ShellWithPath) -> String {
    flox_bin_for_hook_with(
        matches!(shell, ShellWithPath::Tcsh(_)),
        &FLOX_BIN_FOR_SHELL,
        env::current_exe().ok(),
    )
}

/// See [`flox_bin_for_hook`].
fn flox_bin_for_hook_with(is_tcsh: bool, for_shell: &str, exe: Option<PathBuf>) -> String {
    if !is_tcsh || tcsh_alias_can_embed(for_shell) {
        return for_shell.to_string();
    }
    exe.map_or_else(
        || for_shell.to_string(),
        |exe| exe.to_string_lossy().into_owned(),
    )
}

/// The first executable file named `name` in the directories of `path_env`,
/// as the shell would find it. A relative entry (`.`, or an empty entry
/// meaning the current directory) is anchored to `cwd`, since the hook runs
/// from other directories later; without a `cwd` such entries are skipped.
fn find_on_path(name: &Path, path_env: Option<&OsStr>, cwd: Option<&Path>) -> Option<PathBuf> {
    env::split_paths(path_env?)
        .filter_map(|dir| match (dir.is_absolute(), cwd) {
            (true, _) => Some(dir),
            (false, Some(cwd)) => Some(cwd.join(dir)),
            (false, None) => None,
        })
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable_file(candidate))
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|md| md.is_file() && md.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
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
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn exe() -> Option<PathBuf> {
        Some(PathBuf::from("/nix/store/abc-flox/bin/.flox-wrapped"))
    }

    /// A PATH with two entries: `first` holds a non-executable `flox`, `second`
    /// holds a symlink named `flox` to an executable, so the lookup has to
    /// skip the first and must return the symlink rather than its target.
    fn path_with_flox(tmp: &Path) -> (OsString, PathBuf) {
        let first = tmp.join("first");
        let second = tmp.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("flox"), "").unwrap();
        let target = tmp.join("store-flox");
        std::fs::write(&target, "#!/bin/sh\n").unwrap();
        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&target, perms).unwrap();
        std::os::unix::fs::symlink(&target, second.join("flox")).unwrap();
        (
            env::join_paths([first, second.clone()]).unwrap(),
            second.join("flox"),
        )
    }

    #[test]
    fn bare_name_resolves_through_path_without_following_symlinks() {
        let tmp = tempfile::tempdir().unwrap();
        let (path_env, expected) = path_with_flox(tmp.path());
        assert_eq!(
            flox_bin_for_shell(
                Some("flox".into()),
                Some("/work".into()),
                Some(path_env),
                exe()
            ),
            expected.to_string_lossy()
        );
    }

    #[test]
    fn relative_path_entry_is_anchored_to_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let (_, target) = path_with_flox(tmp.path());
        // `.` as a PATH entry, with cwd set to the directory holding `flox`.
        let dir = target.parent().unwrap();
        let path_env = env::join_paths([Path::new(".")]).unwrap();
        assert_eq!(
            flox_bin_for_shell(
                Some("flox".into()),
                Some(dir.to_path_buf()),
                Some(path_env),
                exe()
            ),
            dir.join(".").join("flox").to_string_lossy()
        );
    }

    #[test]
    fn bare_name_not_on_path_falls_back_to_the_running_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let path_env = env::join_paths([tmp.path()]).unwrap();
        assert_eq!(
            flox_bin_for_shell(
                Some("flox".into()),
                Some("/work".into()),
                Some(path_env),
                exe()
            ),
            "/nix/store/abc-flox/bin/.flox-wrapped"
        );
    }

    #[test]
    fn tcsh_falls_back_to_the_running_binary_for_a_path_it_cannot_embed() {
        let for_shell = "/opt/it's/flox";
        assert_eq!(
            flox_bin_for_hook_with(true, for_shell, exe()),
            "/nix/store/abc-flox/bin/.flox-wrapped"
        );
        assert_eq!(flox_bin_for_hook_with(false, for_shell, exe()), for_shell);
        assert_eq!(
            flox_bin_for_hook_with(true, "/opt/with space/flox", exe()),
            "/opt/with space/flox"
        );
    }

    #[test]
    fn relative_path_is_anchored_to_cwd() {
        assert_eq!(
            flox_bin_for_shell(
                Some("./target/debug/flox".into()),
                Some("/work".into()),
                None,
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
                None,
                exe()
            ),
            "/usr/local/bin/flox"
        );
    }

    #[test]
    fn missing_argv0_falls_back_to_the_running_binary() {
        assert_eq!(
            flox_bin_for_shell(None, Some("/work".into()), None, exe()),
            "/nix/store/abc-flox/bin/.flox-wrapped"
        );
    }
}
