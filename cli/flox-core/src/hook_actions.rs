//! A small file that lets a `flox` command ask the prompt hook to act.
//!
//! `flox hook-env` runs on every shell prompt (and on directory changes). A
//! command such as `flox deactivate` runs in a subprocess and so cannot modify
//! its parent shell directly; instead it writes a [`HookActionsFile`] describing
//! what the prompt hook should do, and `flox hook-env` consumes it on the next
//! prompt.
//!
//! The file deliberately serializes a closed [`HookAction`] enum carrying
//! only structured data — never shell code — so a non-`flox` writer cannot use
//! the prompt hook to inject arbitrary commands or environment variables into
//! the shell. It lives under the runtime dir (user-only), like activation
//! `state.json`, and is keyed by the shell's PID.
//!
//! Note the runtime dir's backing store isn't uniform. On systemd Linux it is
//! `XDG_RUNTIME_DIR` (`/run/user/<uid>`), a tmpfs, so this file is RAM-backed;
//! on macOS (and on Linux when `XDG_RUNTIME_DIR` is unset) flox falls back to a
//! disk-backed cache dir, so reads and writes here are real filesystem
//! operations rather than memory-speed ones.

use std::fs::DirBuilder;
use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::{Version, WriteError, traceable_path, write_atomically};

/// An action for `flox hook-env` to perform on the next prompt.
///
/// Intentionally a closed enum that carries only structured data, so the prompt
/// hook cannot be tricked by a non-`flox` writer into injecting arbitrary
/// commands or environment variables into the shell.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookAction {
    /// Emit an in-place deactivation script. The environment-variable diff to
    /// restore is read from the shell's `_FLOX_HOOK_DIFF` at hook time; this
    /// carries the activation state dir needed for the `flox-activations detach`
    /// call and the rendered env link (`flox_env`) the generated script restores
    /// `$FLOX_ENV` from.
    Deactivate {
        activation_state_dir: PathBuf,
        flox_env: PathBuf,
    },
}

/// The current prompt-hook protocol version.
///
/// This is the single source of truth shared by both sides of the protocol, so
/// they can't drift: it is the `version` of the on-disk [`HookActionsFile`] *and*
/// the value the shell hook exports as [`PROMPT_HOOK_VERSION_ENV`]. Before
/// writing a file the hook must read, `flox deactivate` checks the exported
/// value against this. Bump it (and follow the version-compatibility note on
/// [`HookActionsFile`]) on any shape change to the file or the protocol.
pub const PROMPT_HOOK_VERSION: u8 = 1;

/// Environment variable the shell hook exports to advertise that a prompt hook
/// speaking version [`PROMPT_HOOK_VERSION`] is registered in this shell.
///
/// It is exported (unlike `_FLOX_INVOCATION_TYPES`) precisely so a subprocess
/// like `flox deactivate` can read it to confirm the hook is set up and
/// compatible before writing an action file the hook would otherwise never
/// consume.
pub const PROMPT_HOOK_VERSION_ENV: &str = "_FLOX_PROMPT_HOOK_VERSION";

/// Versioned on-disk form of the prompt-hook action file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct HookActionsFile {
    version: Version<PROMPT_HOOK_VERSION>,
    pub actions: Vec<HookAction>,
}

#[derive(Debug, thiserror::Error)]
pub enum HookActionsError {
    #[error("failed to create prompt-hook directory")]
    CreateDir(#[source] std::io::Error),
    #[error("failed to serialize prompt-hook actions")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to write prompt-hook actions")]
    Write(#[source] WriteError),
    #[error("failed to read prompt-hook actions")]
    Read(#[source] std::io::Error),
    #[error("failed to remove prompt-hook actions file")]
    Remove(#[source] std::io::Error),
}

/// Path to the prompt-hook action file for a given shell PID.
///
/// A single directory keyed only by shell PID — no environment component — so
/// the hook checks exactly one path per prompt even when environments are
/// layered.
pub fn hook_actions_path(runtime_dir: &Path, shell_pid: i32) -> PathBuf {
    runtime_dir
        .join("prompt-hook-actions")
        .join(format!("{shell_pid}.json"))
}

/// Write `actions` to the prompt-hook action file for `shell_pid`.
///
/// This runs once per writing command (e.g. `flox deactivate`), which is a
/// foreground, user-initiated action — so write-path performance barely matters
/// here, and the atomic write's small extra cost is worth it. Optimize the read
/// path ([`take_hook_actions`]) instead: it runs on every shell prompt.
pub fn write_hook_actions(
    runtime_dir: &Path,
    shell_pid: i32,
    actions: Vec<HookAction>,
) -> Result<(), HookActionsError> {
    let path = hook_actions_path(runtime_dir, shell_pid);
    let dir = path.parent().expect("actions path has a parent");

    // The prompt hook evaluates whatever this file resolves to, so no other
    // user may write it. XDG_RUNTIME_DIR is already user-only, but set 0o700
    // explicitly (mirrors acquire_activations_json_lock).
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(HookActionsError::CreateDir)?;

    let file = HookActionsFile {
        version: Version,
        actions,
    };
    let contents = serde_json::to_vec_pretty(&file).map_err(HookActionsError::Serialize)?;
    write_atomically(&path, contents).map_err(HookActionsError::Write)
}

/// Read and remove the prompt-hook action file for `shell_pid`, returning the
/// actions it contained (or an empty vec if there is no file).
///
/// This is the hot path: it runs on every shell prompt, almost always with no
/// file present, so the miss case is kept to a single failing `open` and the
/// file is removed (not emptied) after a hit to preserve that. Keep it cheap.
///
/// ## Why there is no lock here
///
/// This is called by `flox hook-env` from the shell's prompt / directory-change
/// hook; the file is written by another `flox` command (`flox deactivate`).
/// There is deliberately no file lock, and none is needed:
///
/// - A shell runs its `precmd`/`chpwd` (and fish prompt/PWD/preexec) hooks
///   synchronously and *blocks* on the `$(flox hook-env ...)` subprocess, so two
///   `hook-env` invocations from the same shell never overlap. Even when a
///   single `cd` fires both a directory-change hook and the prompt hook, they
///   run one after the other and read-once handles the second (it finds no
///   file).
/// - The writer (`flox deactivate`) is a foreground command that finishes before
///   any hook fires.
/// - The file is keyed by shell PID, so different shells never share a file.
/// - Async prompt frameworks don't change this: powerlevel10k only asyncs its
///   own `gitstatusd` query (its precmd hook still runs synchronously in the
///   main shell), and zsh-async only runs functions explicitly submitted via
///   `async_job` in a worker — neither relocates our plain hook entry into a
///   worker, and our hook template never submits `hook-env` via `async_job`.
///
/// The no-lock reasoning above is about concurrency, and it is *not* why the
/// writer uses `write_atomically`: with no overlapping reader, a torn read can't
/// happen anyway. The atomic write instead guards against a single, *interrupted*
/// writer — if `flox deactivate` is killed (Ctrl-C / SIGKILL / OOM) between
/// opening the file and finishing the write, the rename-into-place guarantees
/// the target path holds either the previous file or the complete new one, never
/// a half. And as a backstop this reader tolerates a partial or otherwise
/// unparseable file regardless (it consumes and ignores it — see below), so the
/// atomicity is defense-in-depth, not load-bearing. Please don't "fix" the
/// absent lock by adding one.
pub fn take_hook_actions(
    runtime_dir: &Path,
    shell_pid: i32,
) -> Result<Vec<HookAction>, HookActionsError> {
    let path = hook_actions_path(runtime_dir, shell_pid);

    let contents = match std::fs::read(&path) {
        Ok(contents) => contents,
        // The common, every-prompt case: no pending actions.
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(HookActionsError::Read(err)),
    };

    // Remove the file rather than emptying it so the steady state is "no file":
    // every subsequent prompt then hits the cheap miss path above (a single
    // failing `open`) instead of having to open, read, and parse a lingering
    // empty file. We consume it even when it doesn't parse, so a malformed or
    // wrong-version file can't make every prompt fail.
    std::fs::remove_file(&path).map_err(HookActionsError::Remove)?;

    match serde_json::from_slice::<HookActionsFile>(&contents) {
        Ok(file) => Ok(file.actions),
        Err(err) => {
            warn!(
                %err,
                path = traceable_path(&path),
                "ignoring unparseable prompt-hook action file"
            );
            Ok(Vec::new())
        },
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    const PID: i32 = 1234;

    #[test]
    fn hook_actions_file_serializes_with_version_and_tagged_action() {
        let file = HookActionsFile {
            version: Version,
            actions: vec![HookAction::Deactivate {
                activation_state_dir: PathBuf::from("/run/flox/activations/abc-proj"),
                flox_env: PathBuf::from("/run/flox/abc-proj/run"),
            }],
        };

        assert_eq!(
            serde_json::to_value(&file).unwrap(),
            json!({
                "version": 1,
                "actions": [
                    {
                        "type": "deactivate",
                        "activation_state_dir": "/run/flox/activations/abc-proj",
                        "flox_env": "/run/flox/abc-proj/run",
                    }
                ]
            })
        );
    }

    #[test]
    fn hook_actions_file_round_trips() {
        let file = HookActionsFile {
            version: Version,
            actions: vec![HookAction::Deactivate {
                activation_state_dir: PathBuf::from("/run/flox/activations/abc-proj"),
                flox_env: PathBuf::from("/run/flox/abc-proj/run"),
            }],
        };

        let json = serde_json::to_string(&file).unwrap();
        let parsed: HookActionsFile = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed, file);
    }

    #[test]
    fn actions_path_is_pid_keyed_single_dir() {
        let runtime_dir = Path::new("/run/flox");
        assert_eq!(
            hook_actions_path(runtime_dir, PID),
            PathBuf::from("/run/flox/prompt-hook-actions/1234.json")
        );
    }

    #[test]
    fn write_then_take_returns_actions_and_consumes_file() {
        let runtime = tempdir().unwrap();
        let actions = vec![HookAction::Deactivate {
            activation_state_dir: PathBuf::from("/run/flox/activations/abc-proj"),
            flox_env: PathBuf::from("/run/flox/abc-proj/run"),
        }];

        write_hook_actions(runtime.path(), PID, actions.clone()).unwrap();
        assert!(hook_actions_path(runtime.path(), PID).exists());

        assert_eq!(take_hook_actions(runtime.path(), PID).unwrap(), actions);
        // read-once: the file is gone afterwards.
        assert!(!hook_actions_path(runtime.path(), PID).exists());
    }

    #[test]
    fn second_take_after_consuming_returns_empty() {
        // Mirrors a chpwd hook then a prompt hook firing for a single `cd`:
        // the first consumes the action, the second finds nothing.
        let runtime = tempdir().unwrap();
        write_hook_actions(runtime.path(), PID, vec![HookAction::Deactivate {
            activation_state_dir: PathBuf::from("/run/flox/activations/abc-proj"),
            flox_env: PathBuf::from("/run/flox/abc-proj/run"),
        }])
        .unwrap();

        take_hook_actions(runtime.path(), PID).unwrap();
        assert_eq!(take_hook_actions(runtime.path(), PID).unwrap(), Vec::new());
    }

    #[test]
    fn take_with_no_file_returns_empty() {
        let runtime = tempdir().unwrap();
        assert_eq!(take_hook_actions(runtime.path(), PID).unwrap(), Vec::new());
    }

    #[test]
    fn take_ignores_and_consumes_unparseable_file() {
        let runtime = tempdir().unwrap();
        let path = hook_actions_path(runtime.path(), PID);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Wrong version — Version<1> deserialization rejects it.
        std::fs::write(&path, br#"{"version": 99, "actions": []}"#).unwrap();

        assert_eq!(take_hook_actions(runtime.path(), PID).unwrap(), Vec::new());
        // Even an unparseable file is consumed so it can't fail every prompt.
        assert!(!path.exists());
    }
}
