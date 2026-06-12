//! The ask-broker, hosted as a thread inside the per-activation executive.
//!
//! When an activation runs with `sandbox_mode == Ask`, the executive spawns
//! this broker before entering its event loop. The broker binds a Unix
//! "verdict" socket that the preloaded libsandbox connects to for allow/deny
//! decisions on out-of-policy file access. It seeds an in-memory session
//! grant set from `grants.toml` (read-only this batch) and, for any request
//! that does not match a grant, records a pending entry and denies
//! (auto-deny-and-queue — there is no human approver yet).
//!
//! The broker lives for the activation: the [`BrokerHandle`] returned by
//! [`start`] stops the accept loop and removes the socket file when it is
//! dropped, which the executive does as its event loop returns. An activation
//! with no executive (the containerize own-pid path) never starts a broker,
//! so `ask` there has no socket and the engine fail-closes — handled entirely
//! on the C side.

mod grants;
mod pending;
mod verdict;

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::{Context, Result};
use flox_core::activate::context::{AttachCtx, AttachProjectCtx, SandboxMode};
use tracing::{debug, info, warn};

use self::verdict::BrokerState;
use crate::sandbox::verdict_socket_path;

/// Socket file mode: owner read/write only. The verdict socket is the engine's
/// only contact point and must not be writable by other users on the host.
const SOCKET_MODE: u32 = 0o600;

/// macOS caps `sun_path` at 104 bytes; Linux at 108 (107 usable). The services
/// socket already cleared the stricter limit and the verdict socket is the
/// same length minus the `flox`/`sbx` prefix swap, but re-check defensively so
/// a bind failure surfaces as an actionable error rather than a truncated
/// path.
#[cfg(target_os = "macos")]
const MAX_SOCKET_LEN: usize = 104;
#[cfg(not(target_os = "macos"))]
const MAX_SOCKET_LEN: usize = 107;

/// A running broker. Dropping it stops the accept loop and removes the socket.
pub struct BrokerHandle {
    socket_path: PathBuf,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl BrokerHandle {
    /// Stop the broker and clean up its socket. Called from `Drop`, but
    /// exposed so a caller can shut down explicitly and observe completion.
    fn shutdown(&mut self) {
        // Order matters: set the flag, THEN wake the accept loop with a
        // self-connect while the socket still exists, THEN join, THEN remove
        // the file. Removing the socket before the wake-up would leave the
        // thread blocked in accept() with no way to reach it — the deadlock
        // this ordering avoids. The wake-up connection sends nothing, so the
        // handler reads an empty line and returns, and the loop then sees the
        // flag and exits.
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = UnixStream::connect(&self.socket_path);
        if let Some(thread) = self.thread.take()
            && let Err(err) = thread.join()
        {
            warn!(?err, "ask broker thread panicked during shutdown");
        }
        let _ = std::fs::remove_file(&self.socket_path);
        debug!(socket = ?self.socket_path, "ask broker stopped");
    }
}

impl Drop for BrokerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Start the ask broker for this activation if its mode is `Ask`.
///
/// Returns `Ok(None)` for any non-ask mode (no broker needed) so the caller
/// can unconditionally call this and hold the result for the activation's
/// lifetime. On `ask`, binds the verdict socket, seeds the grant set from
/// `grants.toml`, and spawns the accept-loop thread.
///
/// A bind failure is returned as an error so the executive can log it; the
/// engine then fail-closes (no socket to connect to), which is the correct
/// degradation for `ask`.
pub fn start(
    attach_ctx: &AttachCtx,
    project_ctx: &AttachProjectCtx,
) -> Result<Option<BrokerHandle>> {
    if attach_ctx.sandbox_mode != SandboxMode::Ask {
        return Ok(None);
    }

    let socket_path = verdict_socket_path(&project_ctx.flox_services_socket);
    let grants_dir = project_ctx.dot_flox_path.join("cache").join("sandbox");

    let handle = bind_and_serve(&socket_path, &grants_dir)
        .with_context(|| format!("failed to start ask broker on {}", socket_path.display()))?;
    info!(socket = ?socket_path, "ask broker listening");
    Ok(Some(handle))
}

/// Bind the verdict socket, seed grants, and spawn the accept loop.
fn bind_and_serve(socket_path: &Path, grants_dir: &Path) -> Result<BrokerHandle> {
    if socket_path.as_os_str().len() > MAX_SOCKET_LEN {
        anyhow::bail!(
            "verdict socket path is too long for this platform ({} > {}): {}",
            socket_path.as_os_str().len(),
            MAX_SOCKET_LEN,
            socket_path.display(),
        );
    }

    // A leftover socket from a crashed prior broker would make bind fail with
    // EADDRINUSE; remove it first. The path is per-activation, so this never
    // races a live broker for the same activation.
    let _ = std::fs::remove_file(socket_path);
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create socket dir {}", parent.display()))?;
    }

    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("could not bind verdict socket {}", socket_path.display()))?;
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(SOCKET_MODE))
        .with_context(|| format!("could not chmod verdict socket {}", socket_path.display()))?;

    // Seed the session grant set from grants.toml (read-only this batch). A
    // missing file is normal (no grants yet); a matching path is allowed
    // silently under ask, so an already-trusted environment stays quiet.
    let grants_file = grants::read_grants(grants_dir);
    let grant_globs: Vec<String> = grants_file
        .grants
        .into_iter()
        .map(|grant| grant.pattern)
        .collect();
    debug!(count = grant_globs.len(), "seeded ask broker session grants");

    let state = Arc::new(Mutex::new(BrokerState::new(grant_globs)));
    let shutdown = Arc::new(AtomicBool::new(false));
    let serve_shutdown = Arc::clone(&shutdown);
    let thread = std::thread::Builder::new()
        .name("flox-ask-broker".to_string())
        .spawn(move || verdict::serve(listener, state, serve_shutdown))
        .context("could not spawn ask broker thread")?;

    Ok(BrokerHandle {
        socket_path: socket_path.to_path_buf(),
        shutdown,
        thread: Some(thread),
    })
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};

    use super::*;

    /// Connect to the verdict socket, send one request line, return the
    /// response line. Mirrors the C client's one-exchange-per-connection.
    fn ask(socket: &Path, line: &str) -> String {
        let mut stream = UnixStream::connect(socket).unwrap();
        stream.write_all(line.as_bytes()).unwrap();
        stream.write_all(b"\n").unwrap();
        stream.flush().unwrap();
        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        reader.read_line(&mut response).unwrap();
        response.trim().to_string()
    }

    /// A grants dir under a tempdir, optionally containing a grants.toml body.
    fn grants_dir(tmp: &Path, body: Option<&str>) -> PathBuf {
        let dir = tmp.join("cache").join("sandbox");
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(body) = body {
            std::fs::write(dir.join(grants::GRANTS_FILE_NAME), body).unwrap();
        }
        dir
    }

    #[test]
    fn broker_denies_and_queues_an_unmatched_request() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("sbx.test.sock");
        let dir = grants_dir(tmp.path(), None);

        let _handle = bind_and_serve(&socket, &dir).unwrap();

        let response = ask(
            &socket,
            r#"{"v":1,"kind":"fs","op":"read","path":"/home/dev/.aws/credentials","raw":"~/.aws/credentials","pid":1,"exe":""}"#,
        );
        assert!(response.contains("\"verdict\":\"deny\""), "{response}");
        assert!(response.contains("\"cache\":\"ttl\""), "{response}");
        assert!(response.contains("\"req\":1"), "{response}");
        assert!(
            response.contains("\"scope\":\"/home/dev/.aws/credentials\""),
            "{response}"
        );
    }

    #[test]
    fn broker_honors_a_preseeded_grant() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("sbx.test.sock");
        let dir = grants_dir(
            tmp.path(),
            Some("[[grant]]\npattern = \"/home/dev/.config/**\"\n"),
        );

        let _handle = bind_and_serve(&socket, &dir).unwrap();

        // A path covered by the saved grant is allowed silently under ask,
        // with the matched glob as the cache scope.
        let response = ask(
            &socket,
            r#"{"v":1,"kind":"fs","op":"read","path":"/home/dev/.config/gh/hosts.yml","raw":"~/.config/gh/hosts.yml","pid":1,"exe":""}"#,
        );
        assert!(response.contains("\"verdict\":\"allow\""), "{response}");
        assert!(
            response.contains("\"scope\":\"/home/dev/.config/**\""),
            "{response}"
        );
        assert!(response.contains("\"cache\":\"scope\""), "{response}");
    }

    #[test]
    fn the_socket_is_owner_only() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("sbx.test.sock");
        let dir = grants_dir(tmp.path(), None);

        let _handle = bind_and_serve(&socket, &dir).unwrap();

        let mode = std::fs::metadata(&socket).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, SOCKET_MODE);
    }

    #[test]
    fn dropping_the_handle_removes_the_socket() {
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("sbx.test.sock");
        let dir = grants_dir(tmp.path(), None);

        {
            let _handle = bind_and_serve(&socket, &dir).unwrap();
            assert!(socket.exists());
        }
        assert!(!socket.exists(), "socket should be removed on drop");
    }

    #[test]
    fn start_is_a_noop_for_non_ask_modes() {
        let tmp = tempfile::tempdir().unwrap();
        let dot_flox = tmp.path().join(".flox");
        std::fs::create_dir_all(&dot_flox).unwrap();

        let attach = AttachCtx {
            env: "/flox_env".to_string(),
            env_cache: tmp.path().join("cache"),
            env_description: "test".to_string(),
            flox_active_environments: "[]".to_string(),
            prompt_color_1: "".to_string(),
            prompt_color_2: "".to_string(),
            flox_prompt_environments: "".to_string(),
            set_prompt: false,
            flox_env_cuda_detection: "".to_string(),
            interpreter_path: PathBuf::from("/nix/store/fake"),
            sandbox_mode: SandboxMode::Enforce,
        };
        let project = AttachProjectCtx {
            env_project: tmp.path().to_path_buf(),
            dot_flox_path: dot_flox,
            flox_env_log_dir: tmp.path().join("log"),
            flox_services_socket: tmp.path().join("flox.id.sock"),
            process_compose_bin: PathBuf::from("/nix/store/fake-pc"),
            services_to_start: Vec::new(),
        };

        // Enforce (and warn/off) never start a broker.
        let handle = start(&attach, &project).unwrap();
        assert!(handle.is_none());
    }
}
