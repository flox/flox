//! The control socket: the `flox sandbox` protocol and its self-approval guard.
//!
//! The verdict socket (see [`super::verdict`]) is the engine's contact point;
//! the control socket is the operator's. The `flox sandbox` CLI connects here
//! to list the pending queue, list and mutate grants, and read a status
//! summary. Like the verdict socket the wire format is newline-delimited JSON,
//! one request and one response per connection.
//!
//! Two properties separate this socket from the verdict socket:
//!
//! - **It is never exported into the session env.** The CLI rediscovers the
//!   path from the services socket (the sibling rule in [`crate::sandbox`]),
//!   not from an environment variable an in-session process could read.
//! - **Approval verbs are peer-guarded.** `allow` and `revoke` change policy,
//!   so the broker reads the connecting process's pid (`UnixStream::peer_cred`)
//!   and refuses if that pid is the activation session root or a descendant of
//!   it. A coding agent running *inside* the sandbox therefore cannot approve
//!   its own pending requests, even though it can connect to ask. `list` and
//!   `status` are read-only and allowed from anywhere.
//!
//! This is friction, not a wall (ADR-004, cooperative principal): an agent
//! that unsets its preload vars, drives a PTY, or writes grants.toml directly
//! is out of scope. The guard closes the easy self-approval path and every
//! grant is journaled for the tamper diff.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use flox_core::proc_status::is_pid_descendant_of;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::verdict::BrokerState;

/// A control-socket request from `flox sandbox`.
///
/// `cmd` selects the operation; the remaining fields are command-specific and
/// default to absent so a minimal `{"cmd":"status"}` parses.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ControlRequest {
    /// One of `list-pending`, `list-grants`, `allow`, `revoke`, `status`.
    pub cmd: String,
    /// The glob to allow or revoke.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Provenance to record on an `allow` (review / allow / watch).
    #[serde(default)]
    pub source: Option<String>,
    /// Whether an `allow` persists to grants.toml (vs. session-only).
    #[serde(default)]
    pub persist: bool,
    /// Timestamp to stamp on a persisted grant (the CLI stamps it, so the
    /// broker stays clock-free and the value is testable).
    #[serde(default)]
    pub created: Option<String>,
    /// Evidence (file count) recorded on a persisted directory grant.
    #[serde(default)]
    pub evidence: Option<u64>,
}

/// A control-socket response.
///
/// `ok` is the success flag; `error` carries the refusal/why on failure. The
/// payload fields are populated per command and otherwise empty.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ControlResponse {
    /// True when the command succeeded.
    pub ok: bool,
    /// Human-readable refusal reason, when `ok` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Pending entries, for `list-pending`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<PendingView>,
    /// Session grants, for `list-grants`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<GrantView>,
    /// How many pending entries an `allow` cleared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub satisfied: Option<usize>,
    /// Status summary, for `status`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusView>,
}

/// A pending entry as the CLI renders it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingView {
    pub req: u64,
    pub op: String,
    pub path: String,
    pub hits: u64,
}

/// A session grant as the CLI renders it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrantView {
    pub pattern: String,
    #[serde(default)]
    pub source: Option<String>,
    /// True when the grant is also persisted to grants.toml (vs. session-only).
    pub persisted: bool,
}

/// The `status` summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusView {
    pub mode: String,
    pub granted: usize,
    pub pending: usize,
    /// Whole seconds since the broker started.
    pub uptime_secs: u64,
}

/// Serve the control protocol on `listener` until `shutdown` is set.
///
/// Mirrors the verdict accept loop: blocking `accept`, each connection handled
/// inline (the exchange is one request/response). The session-root pid gates
/// approval verbs — every `allow`/`revoke` checks the peer against it.
pub fn serve(
    listener: UnixListener,
    state: Arc<Mutex<BrokerState>>,
    session_root_pid: i32,
    shutdown: Arc<AtomicBool>,
) {
    for stream in listener.incoming() {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        match stream {
            Ok(stream) => handle_connection(stream, &state, session_root_pid),
            Err(err) => {
                debug!(%err, "control listener accept ended");
                break;
            },
        }
    }
}

/// Read the connecting peer's process id from a Unix stream.
///
/// `SO_PEERCRED` on Linux (via nix's `PeerCredentials`, which carries the pid)
/// and `LOCAL_PEERPID` on macOS (via nix's `LocalPeerPid`). `None` when the
/// lookup fails — the caller treats an unreadable peer as in-session for
/// approval verbs, failing toward refusing.
#[cfg(target_os = "linux")]
fn peer_pid(stream: &UnixStream) -> Option<i32> {
    use std::os::fd::AsFd;

    use nix::sys::socket::getsockopt;
    use nix::sys::socket::sockopt::PeerCredentials;

    getsockopt(&stream.as_fd(), PeerCredentials)
        .ok()
        .map(|cred| cred.pid())
}

#[cfg(target_os = "macos")]
fn peer_pid(stream: &UnixStream) -> Option<i32> {
    use std::os::fd::AsFd;

    use nix::sys::socket::getsockopt;
    use nix::sys::socket::sockopt::LocalPeerPid;

    getsockopt(&stream.as_fd(), LocalPeerPid).ok()
}

/// Handle one control exchange.
fn handle_connection(stream: UnixStream, state: &Arc<Mutex<BrokerState>>, session_root_pid: i32) {
    // Read the peer pid before anything else: an approval verb is refused if
    // the caller is in-session. A pid we cannot read is treated as untrusted
    // for approval verbs (fail toward refusing), but read-only verbs still work.
    let peer_pid = peer_pid(&stream);

    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(err) => {
            warn!(%err, "could not clone control stream");
            return;
        },
    });
    let mut writer = stream;

    let mut line = String::new();
    if let Err(err) = reader.read_line(&mut line) {
        warn!(%err, "could not read control request");
        return;
    }
    let line = line.trim();
    if line.is_empty() {
        return;
    }

    let response = dispatch(line, state, session_root_pid, peer_pid);
    write_response(&mut writer, &response);
}

/// Parse and dispatch one request line. Factored out so the whole protocol is
/// unit-testable without a socket (peer pid is passed explicitly).
fn dispatch(
    line: &str,
    state: &Arc<Mutex<BrokerState>>,
    session_root_pid: i32,
    peer_pid: Option<i32>,
) -> ControlResponse {
    let request: ControlRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            warn!(%err, line, "unparseable control request");
            return ControlResponse {
                ok: false,
                error: Some("could not parse request".to_string()),
                ..Default::default()
            };
        },
    };

    let mut guard = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match request.cmd.as_str() {
        "list-pending" => ControlResponse {
            ok: true,
            pending: guard.pending_view(),
            ..Default::default()
        },
        "list-grants" => ControlResponse {
            ok: true,
            grants: guard.grants_view(),
            ..Default::default()
        },
        "status" => ControlResponse {
            ok: true,
            status: Some(guard.status_view()),
            ..Default::default()
        },
        "allow" => {
            if let Some(refusal) = approval_refusal(session_root_pid, peer_pid) {
                return refusal;
            }
            let Some(pattern) = request.pattern.as_deref() else {
                return missing_pattern();
            };
            match guard.allow_pattern(
                pattern,
                request.source.as_deref(),
                request.persist,
                request.created.as_deref(),
                request.evidence,
            ) {
                Ok(satisfied) => ControlResponse {
                    ok: true,
                    satisfied: Some(satisfied),
                    ..Default::default()
                },
                Err(err) => ControlResponse {
                    ok: false,
                    error: Some(format!("could not save grant: {err:#}")),
                    ..Default::default()
                },
            }
        },
        "revoke" => {
            if let Some(refusal) = approval_refusal(session_root_pid, peer_pid) {
                return refusal;
            }
            let Some(pattern) = request.pattern.as_deref() else {
                return missing_pattern();
            };
            match guard.revoke_pattern(pattern) {
                Ok(()) => ControlResponse {
                    ok: true,
                    ..Default::default()
                },
                Err(err) => ControlResponse {
                    ok: false,
                    error: Some(format!("could not revoke grant: {err:#}")),
                    ..Default::default()
                },
            }
        },
        other => ControlResponse {
            ok: false,
            error: Some(format!("unknown command '{other}'")),
            ..Default::default()
        },
    }
}

/// The refusal response for an approval verb from an in-session caller, or
/// `None` when the caller is allowed to approve.
///
/// A caller is in-session when its pid is the activation session root or a
/// descendant of it. An unreadable peer pid is also refused for approval verbs:
/// the broker cannot prove the caller is outside the session, so it fails
/// toward refusing (the operator can retry from a terminal where peer creds are
/// readable, which is every supported platform).
fn approval_refusal(session_root_pid: i32, peer_pid: Option<i32>) -> Option<ControlResponse> {
    let in_session = match peer_pid {
        Some(pid) => is_pid_descendant_of(pid, session_root_pid),
        None => true,
    };
    if !in_session {
        return None;
    }
    Some(ControlResponse {
        ok: false,
        error: Some(
            "refusing to approve from inside the sandboxed session. \
             Approve from another terminal: flox sandbox allow '<glob>'"
                .to_string(),
        ),
        ..Default::default()
    })
}

fn missing_pattern() -> ControlResponse {
    ControlResponse {
        ok: false,
        error: Some("a pattern is required".to_string()),
        ..Default::default()
    }
}

/// Write one newline-terminated JSON response line.
fn write_response(writer: &mut UnixStream, response: &ControlResponse) {
    let mut line = match serde_json::to_string(response) {
        Ok(line) => line,
        Err(err) => {
            warn!(%err, "could not serialize control response");
            return;
        },
    };
    line.push('\n');
    if let Err(err) = writer.write_all(line.as_bytes()) {
        warn!(%err, "could not write control response");
        return;
    }
    let _ = writer.flush();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::executive::sandbox::verdict::VerdictRequest;

    /// A broker state with a grants dir under a tempdir, returning both so the
    /// test can inspect grants.toml after an `allow`.
    fn state_in(tmp: &std::path::Path) -> Arc<Mutex<BrokerState>> {
        let grants_dir = tmp.join("cache").join("sandbox");
        std::fs::create_dir_all(&grants_dir).unwrap();
        Arc::new(Mutex::new(BrokerState::with_grants_dir(vec![], grants_dir)))
    }

    fn fs_request(path: &str, op: &str) -> VerdictRequest {
        VerdictRequest {
            v: 1,
            kind: "fs".to_string(),
            op: op.to_string(),
            path: path.to_string(),
            raw: path.to_string(),
            pid: 1,
            exe: String::new(),
        }
    }

    /// An out-of-session peer pid for tests: pid 1 (init) is never a descendant
    /// of a high session-root pid, so approval verbs are allowed.
    const OUT_OF_SESSION_PEER: Option<i32> = Some(1);
    /// A session root far above pid 1 so the out-of-session peer is not a
    /// descendant of it.
    const SESSION_ROOT: i32 = 999_999;

    #[test]
    fn list_pending_returns_queued_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        // Queue one request by deciding it through the verdict path.
        state
            .lock()
            .unwrap()
            .decide(&fs_request("/home/dev/.aws/credentials", "read"));

        let response = dispatch(
            r#"{"cmd":"list-pending"}"#,
            &state,
            SESSION_ROOT,
            OUT_OF_SESSION_PEER,
        );
        assert!(response.ok);
        assert_eq!(response.pending.len(), 1);
        assert_eq!(response.pending[0].path, "/home/dev/.aws/credentials");
        assert_eq!(response.pending[0].op, "read");
    }

    #[test]
    fn allow_adds_a_session_grant_and_clears_satisfied_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        state
            .lock()
            .unwrap()
            .decide(&fs_request("/home/dev/.config/gh/hosts.yml", "read"));

        // Allow the exact path, session-only.
        let response = dispatch(
            r#"{"cmd":"allow","pattern":"/home/dev/.config/gh/hosts.yml","source":"review","persist":false}"#,
            &state,
            SESSION_ROOT,
            OUT_OF_SESSION_PEER,
        );
        assert!(response.ok, "{response:?}");
        // The pending entry it covered is cleared.
        assert_eq!(response.satisfied, Some(1));

        // The grant is now live in the session set: a fresh decide allows it.
        let verdict = state
            .lock()
            .unwrap()
            .decide(&fs_request("/home/dev/.config/gh/hosts.yml", "read"));
        assert_eq!(verdict.verdict, "allow");
        // Session-only: grants.toml was not written.
        let grants_toml = tmp.path().join("cache/sandbox/grants.toml");
        assert!(!grants_toml.exists());
    }

    #[test]
    fn allow_with_persist_writes_grants_toml_and_journals() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());

        let response = dispatch(
            r#"{"cmd":"allow","pattern":"/home/dev/.cargo/registry/**","source":"review","persist":true,"created":"2026-06-11","evidence":214}"#,
            &state,
            SESSION_ROOT,
            OUT_OF_SESSION_PEER,
        );
        assert!(response.ok, "{response:?}");

        let grants_dir = tmp.path().join("cache/sandbox");
        // The grant is persisted...
        let saved = crate::sandbox::grants::read_grants(&grants_dir);
        assert_eq!(saved.grants.len(), 1);
        assert_eq!(saved.grants[0].pattern, "/home/dev/.cargo/registry/**");
        assert_eq!(saved.grants[0].evidence, Some(214));
        // ...and journaled, so it is not flagged as self-approved.
        assert!(
            crate::sandbox::grants::unjournaled_patterns(&grants_dir).is_empty(),
            "a broker-written grant must be journaled"
        );
    }

    #[test]
    fn revoke_removes_a_grant_from_the_session_and_file() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        // Save a persisted grant first.
        dispatch(
            r#"{"cmd":"allow","pattern":"/home/dev/data/**","persist":true}"#,
            &state,
            SESSION_ROOT,
            OUT_OF_SESSION_PEER,
        );
        // Then revoke it.
        let response = dispatch(
            r#"{"cmd":"revoke","pattern":"/home/dev/data/**"}"#,
            &state,
            SESSION_ROOT,
            OUT_OF_SESSION_PEER,
        );
        assert!(response.ok, "{response:?}");

        // Gone from the session set: a new decide no longer allows.
        let verdict = state
            .lock()
            .unwrap()
            .decide(&fs_request("/home/dev/data/x", "read"));
        assert_eq!(verdict.verdict, "deny");
        // Gone from grants.toml too.
        let grants_dir = tmp.path().join("cache/sandbox");
        assert!(
            crate::sandbox::grants::read_grants(&grants_dir)
                .grants
                .is_empty()
        );
    }

    #[test]
    fn an_in_session_peer_is_refused_an_allow() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        // The peer IS the session root: a self-approval attempt.
        let peer_is_session_root = Some(SESSION_ROOT);
        let response = dispatch(
            r#"{"cmd":"allow","pattern":"/home/dev/secret","persist":true}"#,
            &state,
            SESSION_ROOT,
            peer_is_session_root,
        );
        assert!(!response.ok);
        assert!(
            response
                .error
                .as_deref()
                .unwrap()
                .contains("inside the sandboxed session"),
            "{response:?}"
        );
        // Nothing was written.
        let grants_dir = tmp.path().join("cache/sandbox");
        assert!(
            crate::sandbox::grants::read_grants(&grants_dir)
                .grants
                .is_empty()
        );
    }

    #[test]
    fn an_unreadable_peer_is_refused_an_approval_verb() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        // No peer pid: the broker cannot prove the caller is out-of-session.
        let response = dispatch(
            r#"{"cmd":"revoke","pattern":"/x"}"#,
            &state,
            SESSION_ROOT,
            None,
        );
        assert!(!response.ok);
        assert!(
            response
                .error
                .as_deref()
                .unwrap()
                .contains("inside the sandboxed session")
        );
    }

    #[test]
    fn read_only_verbs_work_from_any_peer() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        // Even an in-session peer (and an unreadable one) can list and status.
        for peer in [Some(SESSION_ROOT), None] {
            let response = dispatch(r#"{"cmd":"status"}"#, &state, SESSION_ROOT, peer);
            assert!(response.ok, "status must work from any peer: {response:?}");
            let response = dispatch(r#"{"cmd":"list-pending"}"#, &state, SESSION_ROOT, peer);
            assert!(response.ok, "list-pending must work from any peer");
        }
    }

    #[test]
    fn an_unknown_command_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        let response = dispatch(
            r#"{"cmd":"frobnicate"}"#,
            &state,
            SESSION_ROOT,
            OUT_OF_SESSION_PEER,
        );
        assert!(!response.ok);
        assert!(
            response
                .error
                .as_deref()
                .unwrap()
                .contains("unknown command")
        );
    }

    #[test]
    fn status_reports_mode_and_counts() {
        let tmp = tempfile::tempdir().unwrap();
        let state = state_in(tmp.path());
        state.lock().unwrap().decide(&fs_request("/p", "read"));
        let response = dispatch(
            r#"{"cmd":"status"}"#,
            &state,
            SESSION_ROOT,
            OUT_OF_SESSION_PEER,
        );
        let status = response.status.unwrap();
        assert_eq!(status.mode, "ask");
        assert_eq!(status.pending, 1);
    }
}
