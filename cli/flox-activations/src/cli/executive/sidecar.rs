//! Supervision of `[plugin-hooks].sidecar` processes.
//!
//! A sidecar is a plugin-supplied long-running process with the
//! activation's lifetime, spawned by the executive before its readiness
//! handshake — so a spawn failure fails the activation — and terminated
//! and reaped during terminal teardown, before plugin `on-deactivate.d`
//! scripts run. A crash mid-activation is logged and non-fatal, with no
//! automatic restart: plugins must fail closed on a dead peer. On Linux
//! the child gets `PR_SET_PDEATHSIG` so an unclean executive death takes
//! it down too; on macOS the contract obliges the sidecar to watch the
//! executive pid it receives in its ctx. Design:
//! docs/plugin-lifecycle-hooks.md §3.5.

use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use flox_core::activate::context::{AttachProjectCtx, PluginHookExec};
use flox_core::activate::hooks::{
    FLOX_HOOK_CTX_VAR,
    FLOX_HOOK_JQ_VAR,
    FLOX_HOOK_VAR,
    FLOX_PLUGIN_NAME_VAR,
    JQ_BIN,
};
use flox_core::proc_status::pid_is_running;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use serde::Serialize;
use tracing::{debug, info, warn};

/// How long a sidecar gets between SIGTERM and SIGKILL at teardown.
const SIDECAR_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

/// The executive's sidecars, recorded once at spawn. Process-global
/// because teardown runs in free functions reached from several event
/// arms; the executive is a single-purpose daemon, and this is its
/// supervision state.
static SIDECARS: std::sync::OnceLock<Vec<Sidecar>> = std::sync::OnceLock::new();

/// Record the spawned sidecars for teardown. Call once.
pub fn record_sidecars(sidecars: Vec<Sidecar>) {
    let _ = SIDECARS.set(sidecars);
}

/// Terminate and reap every recorded sidecar. Idempotent — safe to call
/// from more than one teardown path.
pub fn terminate_recorded_sidecars() {
    if let Some(sidecars) = SIDECARS.get() {
        terminate_sidecars(sidecars);
    }
}

/// The versioned context a sidecar receives, written into its private
/// runtime dir (both are removed by the executive at teardown).
#[derive(Debug, Serialize)]
struct SidecarCtx<'a> {
    ctx_version: u32,
    hook: &'a str,
    dot_flox_path: &'a Path,
    /// Private `0700` runtime dir for the sidecar's sockets, sibling to
    /// the services socket. Kept short: macOS caps `sun_path` at 104
    /// bytes for anything bound inside it.
    runtime_dir: &'a Path,
    services_socket: &'a Path,
    session_root_pid: i32,
    /// The supervising executive. On macOS the sidecar must exit when
    /// this pid dies (kqueue `NOTE_EXIT` or polling); on Linux
    /// `PR_SET_PDEATHSIG` already enforces it.
    executive_pid: i32,
    plugin_table: &'a serde_json::Value,
}

/// A spawned sidecar under executive supervision.
#[derive(Debug)]
pub struct Sidecar {
    pub plugin_name: String,
    pub pid: i32,
    pub runtime_dir: PathBuf,
}

/// Spawn every recorded sidecar hook. Any failure is returned — the
/// caller runs before the readiness handshake, so the error fails the
/// activation.
pub fn spawn_sidecars(
    hooks: &[PluginHookExec],
    project: &AttachProjectCtx,
    session_root_pid: i32,
) -> Result<Vec<Sidecar>> {
    let executive_pid = std::process::id() as i32;
    let runtime_base = project
        .flox_services_socket
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);

    let mut sidecars = Vec::new();
    for (index, hook) in hooks.iter().enumerate() {
        let runtime_dir = runtime_base.join(format!("sc.{executive_pid}.{index}"));
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&runtime_dir)
            .with_context(|| {
                format!(
                    "could not create the sidecar runtime dir for plugin '{}'",
                    hook.plugin_name
                )
            })?;

        let ctx = SidecarCtx {
            ctx_version: 1,
            hook: "sidecar",
            dot_flox_path: &project.dot_flox_path,
            runtime_dir: &runtime_dir,
            services_socket: &project.flox_services_socket,
            session_root_pid,
            executive_pid,
            plugin_table: &hook.plugin_table,
        };
        let ctx_path = runtime_dir.join("ctx.json");
        std::fs::write(&ctx_path, serde_json::to_vec_pretty(&ctx)?).with_context(|| {
            format!(
                "could not write the sidecar ctx for plugin '{}'",
                hook.plugin_name
            )
        })?;

        let mut command = Command::new(&hook.hook_path);
        command
            .env(FLOX_HOOK_CTX_VAR, &ctx_path)
            .env(FLOX_HOOK_VAR, "sidecar")
            .env(FLOX_PLUGIN_NAME_VAR, &hook.plugin_name)
            .env(FLOX_HOOK_JQ_VAR, &*JQ_BIN)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        // SAFETY: the pre_exec closure runs in the forked child; it is
        // limited to marking inherited descriptors close-on-exec (the
        // spawn_executive precedent) and, on Linux, binding the child's
        // lifetime to the executive with PR_SET_PDEATHSIG.
        unsafe {
            use std::os::unix::process::CommandExt as _;
            command.pre_exec(|| {
                close_fds::CloseFdsBuilder::new().cloexecfrom(3);
                #[cfg(target_os = "linux")]
                nix::sys::prctl::set_pdeathsig(Signal::SIGTERM).map_err(std::io::Error::from)?;
                Ok(())
            });
        }

        let child = command.spawn().with_context(|| {
            format!(
                "Failed to spawn the sidecar for plugin '{}' at {}.",
                hook.plugin_name,
                hook.hook_path.display()
            )
        })?;
        let pid = child.id() as i32;
        info!(plugin = hook.plugin_name, pid, "spawned sidecar");
        // The child is never waited on directly: the signal-handler thread
        // reaps every child. A watcher thread logs the (non-fatal) exit —
        // no restart, per the contract.
        spawn_exit_logger(hook.plugin_name.clone(), pid);

        sidecars.push(Sidecar {
            plugin_name: hook.plugin_name.clone(),
            pid,
            runtime_dir,
        });
    }
    Ok(sidecars)
}

/// Log a sidecar exiting mid-activation. Crash is non-fatal and there is
/// no automatic restart; plugins fail closed on a dead peer.
fn spawn_exit_logger(plugin_name: String, pid: i32) {
    std::thread::spawn(move || {
        match waitpid_any::WaitHandle::open(pid) {
            Ok(mut handle) => {
                let _ = handle.wait();
            },
            Err(err) => {
                debug!(%err, pid, "sidecar exit watch unavailable; polling");
                while pid_is_running(pid) {
                    std::thread::sleep(Duration::from_millis(500));
                }
            },
        }
        warn!(
            plugin = plugin_name,
            pid, "sidecar exited; not restarting (plugins fail closed on a dead peer)"
        );
    });
}

/// Terminate and reap every sidecar: SIGTERM, a grace period, then
/// SIGKILL; the private runtime dir (including the ctx file) is removed.
/// Runs during terminal teardown, after services shut down and before
/// plugin `on-deactivate.d` scripts.
pub fn terminate_sidecars(sidecars: &[Sidecar]) {
    for sidecar in sidecars {
        let pid = Pid::from_raw(sidecar.pid);
        if pid_is_running(sidecar.pid) {
            debug!(
                plugin = sidecar.plugin_name,
                pid = sidecar.pid,
                "terminating sidecar"
            );
            let _ = kill(pid, Signal::SIGTERM);
            let deadline = Instant::now() + SIDECAR_SHUTDOWN_GRACE;
            while pid_is_running(sidecar.pid) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(100));
            }
            if pid_is_running(sidecar.pid) {
                warn!(
                    plugin = sidecar.plugin_name,
                    pid = sidecar.pid,
                    "sidecar ignored SIGTERM; killing"
                );
                let _ = kill(pid, Signal::SIGKILL);
            }
        }
        if let Err(err) = std::fs::remove_dir_all(&sidecar.runtime_dir)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            warn!(%err, dir = %sidecar.runtime_dir.display(), "could not remove sidecar runtime dir");
        }
    }
}
