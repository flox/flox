//! The verdict socket: protocol, decision logic, and accept loop.
//!
//! libsandbox connects to this Unix socket for an allow/deny verdict on an
//! out-of-policy access. The wire format is newline-delimited JSON, exactly
//! one request line and one response line per connection, matching the
//! hand-rolled emitter/scanner in `package-builder/sandbox.c`.
//!
//! Decision logic in this batch (no human approver yet):
//!   - a request whose path matches any session grant glob -> `allow`, with
//!     `scope` = the matched glob and `cache` = `scope` (the engine caches the
//!     whole subtree, so further accesses under it never RPC again);
//!   - otherwise -> record a pending entry and reply `deny`, with `scope` =
//!     the exact path and `cache` = `ttl` (the engine caches the denial for a
//!     short TTL so a retry after a future grant is picked up).
//!
//! The accept loop is blocking `std::os::unix::net` (flox-activations has no
//! tokio); each connection is handled inline. Connections are cheap and
//! short-lived (one exchange), so a per-connection thread is unnecessary.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::control::{GrantView, PendingView, StatusView};
use super::pending::PendingQueue;
use crate::sandbox::grants::{self, Grant, GrantsFile, JournalRecord};

/// A parsed verdict request from libsandbox.
///
/// Only `fs` requests are served in this batch; `net` rides the same wire in
/// a later increment but is not routed through the broker yet (the engine
/// applies enforce semantics for the network under `ask`).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct VerdictRequest {
    #[serde(default = "default_version")]
    pub v: u32,
    /// `fs` or `net`; only `fs` is handled here.
    pub kind: String,
    /// `read` or `write`.
    pub op: String,
    /// Resolved request path (a realpath from the engine).
    pub path: String,
    /// The path as originally opened (for display); may equal `path`.
    #[serde(default)]
    pub raw: String,
    /// Requesting process id.
    #[serde(default)]
    pub pid: i64,
    /// Requesting executable realpath, or empty.
    #[serde(default)]
    pub exe: String,
}

fn default_version() -> u32 {
    1
}

/// The verdict the broker returns to libsandbox.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerdictResponse {
    pub v: u32,
    /// `allow` or `deny`.
    pub verdict: String,
    /// The glob (on allow) or exact path (on deny) the engine should cache.
    pub scope: String,
    /// How the engine should cache: `scope` (process-lifetime glob), `ttl`
    /// (short-lived exact-path denial), or `none`.
    pub cache: String,
    /// The pending request id, so a receipt can name it. 0 when not queued.
    pub req: u64,
}

impl VerdictResponse {
    fn allow(scope: String) -> Self {
        Self {
            v: 1,
            verdict: "allow".to_string(),
            scope,
            cache: "scope".to_string(),
            req: 0,
        }
    }

    fn deny(path: String, req: u64) -> Self {
        Self {
            v: 1,
            verdict: "deny".to_string(),
            scope: path,
            cache: "ttl".to_string(),
            req,
        }
    }
}

/// One live grant in the broker's session set.
///
/// `persisted` distinguishes a session-only grant (gone at activation exit)
/// from one also written to grants.toml (silences future sessions). The
/// `flox sandbox list` surface shows both, so the control view carries it.
#[derive(Debug, Clone)]
struct SessionGrant {
    /// The glob matched against request paths.
    pattern: String,
    /// Provenance: review / allow / watch.
    source: Option<String>,
    /// True when also written to grants.toml.
    persisted: bool,
}

/// Mutable broker state shared across connections: the session grant set, the
/// pending queue, the grants dir (for persisting an `allow`), and the start
/// time (for the status uptime). Behind a single mutex because verdicts are
/// infrequent (one per out-of-policy access, most of which the engine caches)
/// and the critical section is tiny. The control socket shares this same
/// state, so an `allow` on the control socket is visible to the next verdict.
#[derive(Debug)]
pub struct BrokerState {
    /// Session grant set: seeded from grants.toml, extended by control `allow`.
    grants: Vec<SessionGrant>,
    /// Pending requests with hit counts and request ids.
    pending: PendingQueue,
    /// Where grants.toml and the journal live; `None` disables persistence
    /// (a persisting `allow` then errors rather than silently dropping).
    grants_dir: Option<PathBuf>,
    /// When the broker started, for the status uptime readout.
    started: Instant,
}

impl BrokerState {
    /// Build broker state from the initial set of grant globs, without a grants
    /// dir. A persisting `allow` is unavailable; used by tests that exercise
    /// decision logic without persistence.
    #[cfg(test)]
    pub fn new(grants: Vec<String>) -> Self {
        Self::with_grants(grants, None)
    }

    /// Build broker state with a grants dir, so a control `allow` can persist.
    /// The seeded grants are marked persisted (they came from grants.toml).
    pub fn with_grants_dir(seed_patterns: Vec<String>, grants_dir: PathBuf) -> Self {
        Self::with_grants(seed_patterns, Some(grants_dir))
    }

    fn with_grants(seed_patterns: Vec<String>, grants_dir: Option<PathBuf>) -> Self {
        let grants = seed_patterns
            .into_iter()
            .map(|pattern| SessionGrant {
                pattern,
                source: Some("saved".to_string()),
                persisted: true,
            })
            .collect();
        Self {
            grants,
            pending: PendingQueue::new(),
            grants_dir,
            started: Instant::now(),
        }
    }

    /// Decide a verdict for one request, mutating the pending queue on a deny.
    ///
    /// Pure with respect to the response shape given the same grants and
    /// pending state, so it is unit-testable without a socket.
    pub fn decide(&mut self, request: &VerdictRequest) -> VerdictResponse {
        let patterns: Vec<String> = self.grants.iter().map(|g| g.pattern.clone()).collect();
        if let Some(glob) = first_matching_grant(&patterns, &request.path) {
            return VerdictResponse::allow(glob);
        }
        let req = self.pending.record(&request.path, &request.op);
        VerdictResponse::deny(request.path.clone(), req)
    }

    /// Number of distinct pending requests, for tests. The CLI reads the
    /// count through [`Self::status_view`] instead.
    #[cfg(test)]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// The pending queue as control-protocol views, sorted by req id so the
    /// CLI renders a stable order.
    pub fn pending_view(&self) -> Vec<PendingView> {
        let mut entries: Vec<PendingView> = self
            .pending
            .entries()
            .into_iter()
            .map(|entry| PendingView {
                req: entry.req,
                op: entry.op,
                path: entry.path,
                hits: entry.hits,
            })
            .collect();
        entries.sort_by_key(|entry| entry.req);
        entries
    }

    /// The session grant set as control-protocol views.
    pub fn grants_view(&self) -> Vec<GrantView> {
        self.grants
            .iter()
            .map(|grant| GrantView {
                pattern: grant.pattern.clone(),
                source: grant.source.clone(),
                persisted: grant.persisted,
            })
            .collect()
    }

    /// The status summary.
    pub fn status_view(&self) -> StatusView {
        StatusView {
            mode: "ask".to_string(),
            granted: self.grants.len(),
            pending: self.pending.len(),
            uptime_secs: self.started.elapsed().as_secs(),
        }
    }

    /// Add `pattern` to the live session grant set (so the next verdict matches
    /// it) and, when `persist`, append it to grants.toml and journal it. Clears
    /// every pending entry the new grant now covers and returns how many.
    ///
    /// The grant takes effect in the current session immediately — this is the
    /// half that makes "approve, then retry" succeed without re-activating.
    pub fn allow_pattern(
        &mut self,
        pattern: &str,
        source: Option<&str>,
        persist: bool,
        created: Option<&str>,
        evidence: Option<u64>,
    ) -> anyhow::Result<usize> {
        // Persist first: if writing grants.toml fails, the session set is left
        // unchanged so the caller sees a clean error rather than a live grant
        // that silently failed to save.
        if persist {
            let grants_dir = self.grants_dir.as_ref().ok_or_else(|| {
                anyhow::anyhow!("no grants directory configured; cannot persist this grant")
            })?;
            persist_grant(grants_dir, pattern, source, created, evidence)?;
        }

        // Add or upgrade the session grant. A re-allow that persists upgrades a
        // session-only grant to persisted rather than duplicating it.
        if let Some(existing) = self.grants.iter_mut().find(|g| g.pattern == pattern) {
            existing.persisted = existing.persisted || persist;
            if existing.source.is_none() {
                existing.source = source.map(str::to_string);
            }
        } else {
            self.grants.push(SessionGrant {
                pattern: pattern.to_string(),
                source: source.map(str::to_string),
                persisted: persist,
            });
        }

        // Clear pending entries the new grant now covers.
        Ok(self.clear_pending_matching(pattern))
    }

    /// Remove `pattern` from the session set and, if a grants dir is set, from
    /// grants.toml. Revoking a pattern not present is a no-op success.
    pub fn revoke_pattern(&mut self, pattern: &str) -> anyhow::Result<()> {
        self.grants.retain(|grant| grant.pattern != pattern);
        if let Some(grants_dir) = self.grants_dir.as_ref() {
            let mut file = grants::read_grants(grants_dir);
            file.grants.retain(|grant| grant.pattern != pattern);
            grants::write_grants(grants_dir, &file)?;
        }
        Ok(())
    }

    /// Drain pending entries whose path matches `pattern`, returning the count.
    fn clear_pending_matching(&mut self, pattern: &str) -> usize {
        let Ok(compiled) = glob::Pattern::new(pattern) else {
            return 0;
        };
        self.pending.drain_matching(|path| compiled.matches(path))
    }
}

/// Append `pattern` to grants.toml and journal it.
///
/// The journal entry is what keeps a broker-written grant from being flagged
/// as self-approved at the next activation: every grant the broker writes is
/// journaled, so only out-of-band edits show up in the tamper diff.
fn persist_grant(
    grants_dir: &std::path::Path,
    pattern: &str,
    source: Option<&str>,
    created: Option<&str>,
    evidence: Option<u64>,
) -> anyhow::Result<()> {
    let mut file = grants::read_grants(grants_dir);
    // Replace an existing entry for the same pattern rather than duplicating,
    // so re-approving a path does not grow the file unbounded.
    file.grants.retain(|grant| grant.pattern != pattern);
    file.grants.push(Grant {
        pattern: pattern.to_string(),
        ops: Vec::new(),
        source: source.map(str::to_string),
        created: created.map(str::to_string),
        evidence,
    });
    let GrantsFile { version, grants } = file;
    grants::write_grants(grants_dir, &GrantsFile { version, grants })?;
    grants::append_journal(grants_dir, &JournalRecord {
        event: "grant".to_string(),
        pattern: Some(pattern.to_string()),
        source: source.map(str::to_string),
        created: created.map(str::to_string),
    });
    Ok(())
}

/// Return the first grant glob that matches `path`, if any.
///
/// Uses fnmatch semantics via the `glob::Pattern` matcher so the broker and
/// the engine's `fnmatch`-based scope cache agree on what a glob covers.
fn first_matching_grant(grants: &[String], path: &str) -> Option<String> {
    grants.iter().find_map(|glob| {
        glob::Pattern::new(glob)
            .ok()
            .filter(|pattern| pattern.matches(path))
            .map(|_| glob.clone())
    })
}

/// Serve verdicts on `listener` until `shutdown` is set or the listener errors.
///
/// Blocks on `accept`. Each accepted connection is handled inline: read one
/// request line, decide, write one response line. A bad line (unparseable, or
/// a `net` request that is out of scope this batch) is answered with a `deny`
/// + `cache:none` so the engine fails closed rather than hanging.
///
/// Shutdown is cooperative: the host sets `shutdown` and then connects to the
/// socket once to wake this thread out of its blocking `accept`. The loop
/// checks the flag after each accept and returns when it is set, so the
/// wake-up connection (which sends nothing) simply ends the loop. Checking the
/// flag via a wake-up connection — rather than removing the socket first —
/// avoids the deadlock where unlinking the path leaves `accept` blocked with
/// no way to reach it.
pub fn serve(listener: UnixListener, state: Arc<Mutex<BrokerState>>, shutdown: Arc<AtomicBool>) {
    for stream in listener.incoming() {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }
        match stream {
            Ok(stream) => handle_connection(stream, &state),
            // A closed/errored listener ends the loop; the broker thread then
            // returns and the socket file is cleaned up by the host.
            Err(err) => {
                debug!(%err, "verdict listener accept ended");
                break;
            },
        }
    }
}

/// Handle one verdict exchange on `stream`.
fn handle_connection(stream: UnixStream, state: &Arc<Mutex<BrokerState>>) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(err) => {
            warn!(%err, "could not clone verdict stream");
            return;
        },
    });
    let mut writer = stream;

    let mut line = String::new();
    if let Err(err) = reader.read_line(&mut line) {
        warn!(%err, "could not read verdict request");
        return;
    }
    let line = line.trim();
    if line.is_empty() {
        return;
    }

    let response = decide_line(line, state);
    write_response(&mut writer, &response);
}

/// Parse one request line and decide the verdict.
///
/// Factored out so the parse-plus-decide path is unit-testable without a
/// socket. An unparseable line or a non-`fs` request yields a fail-closed
/// `deny` with `cache:none` (do not cache a malformed exchange).
fn decide_line(line: &str, state: &Arc<Mutex<BrokerState>>) -> VerdictResponse {
    let request: VerdictRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(err) => {
            warn!(%err, line, "unparseable verdict request");
            return fail_closed();
        },
    };
    if request.kind != "fs" {
        debug!(kind = %request.kind, "non-fs verdict request denied (out of scope)");
        return fail_closed();
    }
    let mut guard = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.decide(&request)
}

/// A fail-closed verdict for a malformed or unsupported request: deny, and do
/// not cache (so a corrected retry is re-evaluated rather than stuck on a
/// cached denial).
fn fail_closed() -> VerdictResponse {
    VerdictResponse {
        v: 1,
        verdict: "deny".to_string(),
        scope: String::new(),
        cache: "none".to_string(),
        req: 0,
    }
}

/// Write one newline-terminated JSON response line.
fn write_response(writer: &mut UnixStream, response: &VerdictResponse) {
    let mut line = match serde_json::to_string(response) {
        Ok(line) => line,
        Err(err) => {
            warn!(%err, "could not serialize verdict response");
            return;
        },
    };
    line.push('\n');
    if let Err(err) = writer.write_all(line.as_bytes()) {
        warn!(%err, "could not write verdict response");
        return;
    }
    let _ = writer.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(path: &str, op: &str) -> VerdictRequest {
        VerdictRequest {
            v: 1,
            kind: "fs".to_string(),
            op: op.to_string(),
            path: path.to_string(),
            raw: path.to_string(),
            pid: 4242,
            exe: "/usr/bin/cat".to_string(),
        }
    }

    #[test]
    fn grant_match_allows_with_scope_glob() {
        let mut state = BrokerState::new(vec!["/home/dev/.cargo/**".to_string()]);
        let response = state.decide(&request("/home/dev/.cargo/registry/x", "read"));
        assert_eq!(response, VerdictResponse {
            v: 1,
            verdict: "allow".to_string(),
            scope: "/home/dev/.cargo/**".to_string(),
            cache: "scope".to_string(),
            req: 0,
        });
        // An allow never queues anything.
        assert_eq!(state.pending_len(), 0);
    }

    #[test]
    fn no_match_denies_with_ttl_and_queues_a_req_id() {
        let mut state = BrokerState::new(vec!["/data/**".to_string()]);
        let response = state.decide(&request("/home/dev/.aws/credentials", "read"));
        assert_eq!(response, VerdictResponse {
            v: 1,
            verdict: "deny".to_string(),
            scope: "/home/dev/.aws/credentials".to_string(),
            cache: "ttl".to_string(),
            req: 1,
        });
        assert_eq!(state.pending_len(), 1);
    }

    #[test]
    fn repeat_deny_reuses_the_same_req_id() {
        let mut state = BrokerState::new(vec![]);
        let first = state.decide(&request("/p", "read"));
        let second = state.decide(&request("/p", "read"));
        assert_eq!(first.req, 1);
        assert_eq!(second.req, 1);
        // Coalesced into one pending entry despite two requests.
        assert_eq!(state.pending_len(), 1);
    }

    #[test]
    fn empty_grant_set_denies_everything() {
        let mut state = BrokerState::new(vec![]);
        let response = state.decide(&request("/anything", "write"));
        assert_eq!(response.verdict, "deny");
        assert_eq!(response.cache, "ttl");
    }

    #[test]
    fn unparseable_line_fails_closed_without_caching() {
        let state = Arc::new(Mutex::new(BrokerState::new(vec![])));
        let response = decide_line("not json", &state);
        assert_eq!(response.verdict, "deny");
        assert_eq!(response.cache, "none");
        // A malformed exchange must not create a pending entry.
        assert_eq!(state.lock().unwrap().pending_len(), 0);
    }

    #[test]
    fn net_request_is_out_of_scope_and_fails_closed() {
        let state = Arc::new(Mutex::new(BrokerState::new(vec![
            "/should/not/matter".to_string(),
        ])));
        let line = r#"{"v":1,"kind":"net","op":"connect","path":"1.2.3.4:443"}"#;
        let response = decide_line(line, &state);
        assert_eq!(response.verdict, "deny");
        assert_eq!(response.cache, "none");
    }

    #[test]
    fn fs_request_line_round_trips_through_decide_line() {
        let state = Arc::new(Mutex::new(BrokerState::new(vec!["/home/**".to_string()])));
        let line =
            r#"{"v":1,"kind":"fs","op":"read","path":"/home/dev/x","raw":"~/x","pid":7,"exe":""}"#;
        let response = decide_line(line, &state);
        assert_eq!(response.verdict, "allow");
        assert_eq!(response.scope, "/home/**");
    }
}
