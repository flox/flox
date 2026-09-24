//! Shared helper for spawning detached background side-effect processes.
//!
//! Both `check-for-upgrades` and `send-telemetry` run as detached children
//! that outlive the parent CLI process. This module holds the common spawn
//! mechanism (process-group detach, fd close, stdio redirect, env propagation)
//! and the CI escape hatch that prevents background children from being
//! spawned during integration tests.

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Command, Stdio};

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

/// How the detached child's stderr log file is named and opened.
pub enum LogFile {
    /// A fixed filename, truncated on each spawn. Bounds disk growth for a
    /// child spawned on every invocation, where a per-invocation file would
    /// accumulate without cleanup.
    Rolling(String),
    /// A caller-supplied filename, created fresh each spawn. The caller is
    /// responsible for including a sortable timestamp so an external GC (the
    /// activations executive's `gc_logs_per_process`) can keep the last N.
    PerInvocation(String),
}

/// Configuration for a detached background side-effect process.
pub struct DetachedCommand<'a> {
    /// Subcommand name and arguments to pass to the current flox binary.
    pub args: &'a [String],
    /// Names and opens the child's stderr log file within `log_dir`.
    pub log_file: LogFile,
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

        std::fs::create_dir_all(self.log_dir)?;
        let log_file = self.open_log_file()?;
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

    /// Open the child's stderr log file per the configured [`LogFile`] strategy.
    ///
    /// `Rolling` truncates so a child spawned on every invocation reuses one
    /// bounded file; `PerInvocation` creates the caller-named file fresh.
    fn open_log_file(&self) -> Result<File> {
        let (path, file) = match &self.log_file {
            LogFile::Rolling(name) => {
                let path = self.log_dir.join(name);
                let file = OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&path);
                (path, file)
            },
            LogFile::PerInvocation(name) => {
                let path = self.log_dir.join(name);
                (path.clone(), File::create(&path))
            },
        };

        debug!(
            log_file = ?path,
            "Logging detached child output to file, redirecting stdin/stdout to /dev/null"
        );

        file.with_context(|| format!("Failed to create log file {}", path.display()))
    }
}

/// The rolling log filename for the send-telemetry child.
pub const SEND_TELEMETRY_LOG_NAME: &str = "send-telemetry.log";

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
            let args = vec!["send-telemetry".to_string(), "--force".to_string()];
            let result = DetachedCommand {
                args: &args,
                log_file: LogFile::Rolling(SEND_TELEMETRY_LOG_NAME.to_string()),
                log_dir: dir.path(),
            }
            .spawn(Some(std::env::current_exe().unwrap()));
            assert!(result.is_ok(), "skipped spawn must return Ok");
        });
    }

    #[test]
    fn rolling_log_file_is_truncated_on_reopen() {
        // A Rolling log reuses one file and truncates it, so a child spawned on
        // every invocation cannot grow the log dir without bound.
        let dir = tempfile::tempdir().unwrap();
        let cmd = DetachedCommand {
            args: &[],
            log_file: LogFile::Rolling(SEND_TELEMETRY_LOG_NAME.to_string()),
            log_dir: dir.path(),
        };

        use std::io::Write as _;
        let mut first = cmd.open_log_file().unwrap();
        first
            .write_all(b"stale contents from a previous run")
            .unwrap();
        drop(first);

        // Reopening truncates: the previous run's bytes are gone and only one
        // file exists.
        let _second = cmd.open_log_file().unwrap();
        let path = dir.path().join(SEND_TELEMETRY_LOG_NAME);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "",
            "rolling log must be truncated on reopen"
        );
        let log_count = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(log_count, 1, "rolling log must not accumulate files");
    }

    #[test]
    fn per_invocation_log_file_uses_the_given_name() {
        // A PerInvocation log is created under the caller's name so the caller
        // can embed a sortable timestamp for the executive GC to prune.
        let dir = tempfile::tempdir().unwrap();
        let cmd = DetachedCommand {
            args: &[],
            log_file: LogFile::PerInvocation("upgrade-check.42.log".to_string()),
            log_dir: dir.path(),
        };
        let _file = cmd.open_log_file().unwrap();
        assert!(dir.path().join("upgrade-check.42.log").is_file());
    }
}
