//! Shared helper for spawning detached background side-effect processes.
//!
//! Both `check-for-upgrades` and `send-telemetry` run as detached children
//! that outlive the parent CLI process. This module holds the common spawn
//! mechanism (process-group detach, fd close, stdio redirect, env propagation)
//! and the CI escape hatch that prevents background children from being
//! spawned during integration tests.

use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::SystemTime;

use anyhow::{Context, Result, bail};
use flox_core::vars::{FLOX_VERSION_STRING, FLOX_VERSION_VAR};
use flox_rust_sdk::utils::CommandExt as _;
use tracing::debug;

use crate::utils::events::{FLOX_INVOCATION_ID_VAR, current_invocation_id};

/// Returns `true` when `_FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS` is set to a
/// truthy value (`"1"`, `"true"`, or `"yes"`).
///
/// Both detached-spawn sites check this before spawning so integration tests
/// get a deterministic, no-background-process environment.
pub fn bg_side_effects_disabled() -> bool {
    matches!(
        std::env::var("_FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS")
            .unwrap_or_default()
            .to_lowercase()
            .as_str(),
        "1" | "true" | "yes"
    )
}

/// Configuration for a detached background side-effect process.
pub struct DetachedCommand<'a> {
    /// Subcommand name and arguments to pass to the current flox binary.
    pub args: &'a [&'a str],
    /// Log-file stem used to construct the output path: the log file will be
    /// created as `log_dir/<stem>-<unix_timestamp>.log`.
    pub log_stem: &'a str,
    /// Directory where the log file will be written.
    pub log_dir: &'a Path,
}

impl DetachedCommand<'_> {
    /// Spawn a detached background `flox` child process.
    ///
    /// The child:
    /// - inherits the parent's `FLOX_VERSION` and invocation-id env vars so its
    ///   v2 events join the parent's stream;
    /// - has stdin/stdout redirected to `/dev/null` (stdout hygiene is critical
    ///   for `hook-env`, whose stdout is the shell's command-substitution buffer);
    /// - has stderr redirected to a log file under `log_dir`;
    /// - is detached from the parent's process group (`setsid`) so signals
    ///   delivered to the parent do not kill the background child;
    /// - has excess file descriptors closed before exec.
    ///
    /// Returns immediately — the parent does not wait for the child.
    ///
    /// ## SAFETY
    ///
    /// [`pre_exec`](std::os::unix::process::CommandExt::pre_exec) runs in an
    /// environment atypical for Rust where many ownership guarantees do not
    /// hold. The scope here is intentionally minimal: close excess fds and call
    /// `setsid`. Closing fds *before* exec is considerably safer than doing so
    /// in the child after it has opened its own descriptors.
    pub fn spawn(self, self_executable: Option<std::path::PathBuf>) -> Result<()> {
        if bg_side_effects_disabled() {
            debug!("Skipping background job for tests (_FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS)");
            return Ok(());
        }

        let self_executable = match self_executable {
            Some(path) => path,
            None if cfg!(test) => {
                bail!("self_executable must be provided in tests")
            },
            // SECURITY: The flox executable path is at an immutable nix store
            // path, so reading it here is safe.
            None => std::env::current_exe()?,
        };

        let mut command = Command::new(&self_executable);

        // Propagate the version which the wrapper script sets and the CLI then
        // unsets — the child needs it to emit its own version telemetry.
        command.env(FLOX_VERSION_VAR, &*FLOX_VERSION_STRING);

        // Propagate the parent's invocation_id so the child's v2 events join
        // the parent's stream rather than appearing as a separate top-level
        // invocation. Written only onto this Command, not into the parent
        // process env, so it does not leak forward into the user's shell.
        if let Some(parent_invocation_id) = current_invocation_id() {
            command.env(FLOX_INVOCATION_ID_VAR, parent_invocation_id.to_string());
        }

        for arg in self.args {
            command.arg(arg);
        }

        // Redirect logs to a timestamped file.
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("now is after UNIX EPOCH")
            .as_secs();
        let log_file_path = self
            .log_dir
            .join(format!("{}-{}.log", self.log_stem, timestamp));

        debug!(
            log_file = ?log_file_path,
            "Logging detached child output to file, redirecting stdin/stdout to /dev/null"
        );

        std::fs::create_dir_all(self.log_dir)?;
        let log_file = File::create(&log_file_path)
            .with_context(|| format!("Failed to create log file {}", log_file_path.display()))?;
        let log_file_fd = log_file.as_raw_fd();

        command.stderr(log_file);
        command.stdout(Stdio::null());
        command.stdin(Stdio::null());

        let keep_fds = [log_file_fd];

        // Close extra fds and detach from the parent process group.
        // See the SAFETY section above.
        unsafe {
            use std::os::unix::process::CommandExt as _;
            command.pre_exec(move || {
                close_fds::CloseFdsBuilder::new()
                    .keep_fds(&keep_fds)
                    .cloexecfrom(3);

                nix::unistd::setsid()?;
                Ok(())
            });
        }

        debug!(cmd = %command.display(), "Spawning detached background process");

        // Fire and forget — the parent does not wait.
        command
            .spawn()
            .with_context(|| format!("Failed to spawn background process {:?}", self_executable))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bg_side_effects_disabled_reads_env_var() {
        // Default: env var unset → not disabled
        temp_env::with_var(
            "_FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS",
            None::<&str>,
            || {
                assert!(!bg_side_effects_disabled());
            },
        );

        // Explicitly set to "1" → disabled
        temp_env::with_var("_FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS", Some("1"), || {
            assert!(bg_side_effects_disabled());
        });

        // Explicitly set to "true" → disabled
        temp_env::with_var(
            "_FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS",
            Some("true"),
            || {
                assert!(bg_side_effects_disabled());
            },
        );
    }

    #[test]
    fn detached_command_skips_spawn_when_bg_disabled() {
        // With the CI escape hatch set, spawn returns Ok(()) without spawning.
        temp_env::with_var("_FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS", Some("1"), || {
            let dir = tempfile::tempdir().unwrap();
            let result = DetachedCommand {
                args: &["send-telemetry", "--force"],
                log_stem: "send-telemetry",
                log_dir: dir.path(),
            }
            .spawn(Some(std::env::current_exe().unwrap()));
            assert!(result.is_ok(), "skipped spawn must return Ok");
        });
    }
}
