//! The prompt broker, hosted as a thread inside the per-activation executive.
//!
//! When an activation runs with `sandbox_mode == Prompt`, the executive spawns
//! this broker before entering its event loop. The broker binds a Unix
//! "verdict" socket that the preloaded libsandbox connects to for allow/deny
//! decisions on out-of-policy file access. It seeds an in-memory session
//! grant set from `grants.toml` and, for any request that does not match a
//! grant, records a pending entry and denies (deny-and-queue). Approvals
//! arrive out-of-band over the control socket — `flox sandbox allow` from a
//! second terminal — and take effect on the next verdict.
//!
//! The broker lives for the activation: the [`BrokerHandle`] returned by
//! [`start`] stops the accept loop and removes the socket file when it is
//! dropped, which the executive does as its event loop returns. An activation
//! with no executive (the containerize own-pid path) never starts a broker,
//! so `prompt` there has no socket and the engine fail-closes — handled entirely
//! on the C side.

mod control;
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
use crate::sandbox::{control_socket_path, grants, verdict_socket_path};

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

/// One bound socket and its accept-loop thread, joined on shutdown.
struct SocketThread {
    socket_path: PathBuf,
    thread: Option<JoinHandle<()>>,
}

/// A running broker. Dropping it stops both accept loops and removes both
/// sockets. The verdict socket serves engine RPCs; the control socket serves
/// `flox sandbox`. Both accept loops share one [`BrokerState`], so a control
/// `allow` is visible to the next verdict.
pub struct BrokerHandle {
    shutdown: Arc<AtomicBool>,
    /// Verdict and control socket threads, in shutdown order.
    sockets: Vec<SocketThread>,
}

impl BrokerHandle {
    /// Stop the broker and clean up its sockets. Called from `Drop`, but
    /// exposed so a caller can shut down explicitly and observe completion.
    fn shutdown(&mut self) {
        // Order matters: set the flag, THEN wake each accept loop with a
        // self-connect while its socket still exists, THEN join, THEN remove
        // the files. Removing a socket before the wake-up would leave its
        // thread blocked in accept() with no way to reach it — the deadlock
        // this ordering avoids. The wake-up connection sends nothing, so the
        // handler reads an empty line and returns, and the loop then sees the
        // flag and exits.
        self.shutdown.store(true, Ordering::Relaxed);
        for socket in &self.sockets {
            let _ = UnixStream::connect(&socket.socket_path);
        }
        for socket in &mut self.sockets {
            if let Some(thread) = socket.thread.take()
                && let Err(err) = thread.join()
            {
                warn!(?err, "prompt broker thread panicked during shutdown");
            }
            let _ = std::fs::remove_file(&socket.socket_path);
            debug!(socket = ?socket.socket_path, "prompt broker socket stopped");
        }
    }
}

impl Drop for BrokerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Start the prompt broker for this activation if its mode is `Prompt`.
///
/// Returns `Ok(None)` for any non-prompt mode (no broker needed) so the caller
/// can unconditionally call this and hold the result for the activation's
/// lifetime. On `prompt`, binds the verdict and control sockets, seeds the grant
/// set from `grants.toml`, and spawns both accept-loop threads.
///
/// `session_root_pid` is the activation's session-root process (the executive's
/// `parent_pid`): the control socket refuses approval verbs from a caller that
/// is this pid or a descendant of it, which is the server side of the
/// self-approval guard.
///
/// A bind failure is returned as an error so the executive can log it; the
/// engine then fail-closes (no socket to connect to), which is the correct
/// degradation for `prompt`.
pub fn start(
    attach_ctx: &AttachCtx,
    project_ctx: &AttachProjectCtx,
    session_root_pid: i32,
) -> Result<Option<BrokerHandle>> {
    if attach_ctx.sandbox_mode != SandboxMode::Prompt {
        return Ok(None);
    }

    let verdict_path = verdict_socket_path(&project_ctx.flox_services_socket);
    let control_path = control_socket_path(&project_ctx.flox_services_socket);
    let grants_dir = project_ctx.dot_flox_path.join("cache").join("sandbox");

    let handle = bind_and_serve(&verdict_path, &control_path, &grants_dir, session_root_pid)
        .with_context(|| {
            format!(
                "failed to start prompt broker on {}",
                verdict_path.display()
            )
        })?;
    info!(
        verdict = ?verdict_path,
        control = ?control_path,
        "prompt broker listening"
    );
    Ok(Some(handle))
}

/// Bind one socket at `path` with owner-only mode, removing any leftover first.
fn bind_socket(path: &Path, label: &str) -> Result<UnixListener> {
    if path.as_os_str().len() > MAX_SOCKET_LEN {
        anyhow::bail!(
            "{label} socket path is too long for this platform ({} > {}): {}",
            path.as_os_str().len(),
            MAX_SOCKET_LEN,
            path.display(),
        );
    }
    // A leftover socket from a crashed prior broker would make bind fail with
    // EADDRINUSE; remove it first. The path is per-activation, so this never
    // races a live broker for the same activation.
    let _ = std::fs::remove_file(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create socket dir {}", parent.display()))?;
    }
    let listener = UnixListener::bind(path)
        .with_context(|| format!("could not bind {label} socket {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(SOCKET_MODE))
        .with_context(|| format!("could not chmod {label} socket {}", path.display()))?;
    Ok(listener)
}

/// Bind both sockets, seed grants, and spawn both accept loops over one shared
/// state.
fn bind_and_serve(
    verdict_path: &Path,
    control_path: &Path,
    grants_dir: &Path,
    session_root_pid: i32,
) -> Result<BrokerHandle> {
    let verdict_listener = bind_socket(verdict_path, "verdict")?;
    let control_listener = bind_socket(control_path, "control")?;

    // Seed the session grant set from grants.toml. A missing file is normal (no
    // grants yet); a matching path is allowed silently under prompt, so an
    // already-trusted environment stays quiet. The file's own source is
    // carried through (so default-seed grants stay distinguishable in the
    // session view), and net-kind grants are excluded — the fs broker only
    // matches filesystem globs; the network policy is compiled into
    // FLOX_SANDBOX_ALLOW_NET at activation start. The grants dir is held in
    // state so a control `allow` can persist back to it.
    let grants_file = grants::read_grants(grants_dir);
    let grant_seeds: Vec<(String, Option<String>)> = grants_file
        .grants
        .into_iter()
        .filter(|grant| !grant.is_net())
        .map(|grant| (grant.pattern, grant.source))
        .collect();
    debug!(
        count = grant_seeds.len(),
        "seeded prompt broker session grants"
    );

    let state = Arc::new(Mutex::new(BrokerState::with_grants_dir(
        grant_seeds,
        grants_dir.to_path_buf(),
    )));
    let shutdown = Arc::new(AtomicBool::new(false));

    let verdict_state = Arc::clone(&state);
    let verdict_shutdown = Arc::clone(&shutdown);
    let verdict_thread = std::thread::Builder::new()
        .name("flox-prompt-verdict".to_string())
        .spawn(move || verdict::serve(verdict_listener, verdict_state, verdict_shutdown))
        .context("could not spawn verdict broker thread")?;

    let control_state = Arc::clone(&state);
    let control_shutdown = Arc::clone(&shutdown);
    let control_thread = std::thread::Builder::new()
        .name("flox-prompt-control".to_string())
        .spawn(move || {
            control::serve(
                control_listener,
                control_state,
                session_root_pid,
                control_shutdown,
            )
        })
        .context("could not spawn control broker thread")?;

    Ok(BrokerHandle {
        shutdown,
        sockets: vec![
            SocketThread {
                socket_path: verdict_path.to_path_buf(),
                thread: Some(verdict_thread),
            },
            SocketThread {
                socket_path: control_path.to_path_buf(),
                thread: Some(control_thread),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};

    use super::*;

    /// Connect to the verdict socket, send one request line, return the
    /// response line. Mirrors the C client's one-exchange-per-connection.
    fn exchange(socket: &Path, line: &str) -> String {
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

    /// Verdict and control socket paths under a tempdir. Tests use a session
    /// root of 1 (init): the test caller's pid is a descendant of init, so
    /// approval verbs would be refused — these socket tests exercise the
    /// verdict path, which is unguarded.
    fn sockets(tmp: &Path) -> (PathBuf, PathBuf) {
        (tmp.join("sbx.test.sock"), tmp.join("sbc.test.sock"))
    }

    #[test]
    fn broker_denies_and_queues_an_unmatched_request() {
        let tmp = tempfile::tempdir().unwrap();
        let (verdict, control) = sockets(tmp.path());
        let dir = grants_dir(tmp.path(), None);

        let _handle = bind_and_serve(&verdict, &control, &dir, 1).unwrap();

        let response = exchange(&verdict, "/home/dev/.aws/credentials");
        assert_eq!(response, "deny 1");
    }

    #[test]
    fn broker_honors_a_preseeded_grant() {
        let tmp = tempfile::tempdir().unwrap();
        let (verdict, control) = sockets(tmp.path());
        let dir = grants_dir(
            tmp.path(),
            Some("[[grant]]\npattern = \"/home/dev/.config/**\"\n"),
        );

        let _handle = bind_and_serve(&verdict, &control, &dir, 1).unwrap();

        // A path covered by the saved grant is allowed silently, with the
        // matched glob as the reply pattern for the client's cache.
        let response = exchange(&verdict, "/home/dev/.config/gh/hosts.yml");
        assert_eq!(response, "allow-glob /home/dev/.config/**");
    }

    #[test]
    fn both_sockets_are_owner_only() {
        let tmp = tempfile::tempdir().unwrap();
        let (verdict, control) = sockets(tmp.path());
        let dir = grants_dir(tmp.path(), None);

        let _handle = bind_and_serve(&verdict, &control, &dir, 1).unwrap();

        for socket in [&verdict, &control] {
            let mode = std::fs::metadata(socket).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, SOCKET_MODE, "{}", socket.display());
        }
    }

    #[test]
    fn dropping_the_handle_removes_both_sockets() {
        let tmp = tempfile::tempdir().unwrap();
        let (verdict, control) = sockets(tmp.path());
        let dir = grants_dir(tmp.path(), None);

        {
            let _handle = bind_and_serve(&verdict, &control, &dir, 1).unwrap();
            assert!(verdict.exists());
            assert!(control.exists());
        }
        assert!(
            !verdict.exists(),
            "verdict socket should be removed on drop"
        );
        assert!(
            !control.exists(),
            "control socket should be removed on drop"
        );
    }

    #[test]
    fn a_control_allow_is_visible_to_the_next_verdict() {
        // The headline live-approve loop end to end over real sockets: a denied
        // path becomes allowed once a control `allow` lands, because both
        // accept loops share one BrokerState. The control caller (the test
        // process) is NOT a descendant of session root 1's *child* — we use a
        // session root of 1 here, which makes the test process in-session, so
        // we drive the allow through the state directly via the control
        // protocol with an out-of-session peer is not possible over a real
        // socket. Instead we assert the verdict-side visibility by seeding the
        // grant through the same shared state the control socket mutates: the
        // control protocol itself is unit-tested in `control.rs` with an
        // explicit peer pid. Here we prove the *socket wiring*: a grant added
        // to the shared state is honored by the verdict socket.
        let tmp = tempfile::tempdir().unwrap();
        let (verdict, control) = sockets(tmp.path());
        let dir = grants_dir(tmp.path(), None);

        // Use a session root that the test process is definitely NOT descended
        // from, so the control `allow` over the real socket is permitted.
        // Pid 2 (kthreadd on Linux / launchd-adjacent on macOS) is never our
        // ancestor.
        let _handle = bind_and_serve(&verdict, &control, &dir, 2).unwrap();

        // Before: the path is denied.
        let denied = exchange(&verdict, "/home/dev/.config/gh/hosts.yml");
        assert!(denied.starts_with("deny "), "{denied}");

        // Approve over the control socket (session-only).
        let control_reply = exchange(
            &control,
            r#"{"cmd":"allow","pattern":"/home/dev/.config/gh/hosts.yml","source":"review","persist":false}"#,
        );
        assert!(control_reply.contains("\"ok\":true"), "{control_reply}");

        // After: the same path now allows, with zero re-activation.
        let allowed = exchange(&verdict, "/home/dev/.config/gh/hosts.yml");
        assert!(allowed.starts_with("allow-glob "), "{allowed}");
    }

    #[test]
    fn start_is_a_noop_for_non_prompt_modes() {
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
            metrics_host: None,
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
        let handle = start(&attach, &project, 1).unwrap();
        assert!(handle.is_none());
    }
}
