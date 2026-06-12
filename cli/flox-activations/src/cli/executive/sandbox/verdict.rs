//! The verdict socket: protocol, decision logic, and accept loop.
//!
//! libsandbox connects to this Unix socket for an allow/deny verdict on an
//! out-of-policy access. The wire format is the shared prompt-broker line
//! protocol (see `flox_core::activate::prompt_protocol`): one request line
//! holding the realpath, one reply line, per connection — the same protocol
//! the per-build `SandboxPromptBroker` speaks, so the C client in
//! `package-builder/sandbox.c` serves builds and activations identically.
//!
//! Decision logic (no human approver on this socket; approvals arrive over
//! the control socket):
//!   - a request whose path matches any session grant glob ->
//!     `allow-glob <pattern>` (the engine caches the pattern, so further
//!     accesses under it never RPC again);
//!   - otherwise -> record a pending entry and reply `deny <req>` (the engine
//!     caches the denial for a short TTL so a retry after a future grant is
//!     picked up, and the receipt names the review id);
//!   - an empty or unreadable request -> no reply (close), which the client
//!     treats as a broker error and the activation fails closed.
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

use flox_core::activate::prompt_protocol::{REPLY_ALLOW_GLOB_PREFIX, REPLY_DENY};
use tracing::{debug, warn};

use super::control::{GrantView, PendingView, StatusView};
use super::pending::PendingQueue;
use crate::sandbox::grants::{self, Grant, JournalRecord};

/// The verdict the broker returns to libsandbox, one line on the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerdictReply {
    /// Allow, carrying the matched grant glob for the client's pattern cache.
    AllowGlob(String),
    /// Deny, queued for out-of-band review as request `req`.
    Deny { req: u64 },
}

impl VerdictReply {
    /// The newline-terminated wire form of this reply.
    pub fn wire_line(&self) -> String {
        match self {
            VerdictReply::AllowGlob(glob) => format!("{REPLY_ALLOW_GLOB_PREFIX}{glob}\n"),
            VerdictReply::Deny { req } => format!("{REPLY_DENY} {req}\n"),
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
        let seeds = grants
            .into_iter()
            .map(|pattern| (pattern, Some("saved".to_string())))
            .collect();
        Self::with_grants(seeds, None)
    }

    /// Build broker state with a grants dir, so a control `allow` can persist.
    /// `seeds` are `(pattern, source)` pairs read from grants.toml; the file's
    /// own source is carried through so the review surface can tell a
    /// default-seed grant from one the user approved. All seeds are marked
    /// persisted (they came from grants.toml).
    pub fn with_grants_dir(seeds: Vec<(String, Option<String>)>, grants_dir: PathBuf) -> Self {
        Self::with_grants(seeds, Some(grants_dir))
    }

    fn with_grants(seeds: Vec<(String, Option<String>)>, grants_dir: Option<PathBuf>) -> Self {
        let grants = seeds
            .into_iter()
            .map(|(pattern, source)| SessionGrant {
                pattern,
                source: source.or_else(|| Some("saved".to_string())),
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

    /// Decide a verdict for one requested realpath, mutating the pending queue
    /// on a deny.
    ///
    /// Pure with respect to the reply shape given the same grants and pending
    /// state, so it is unit-testable without a socket.
    pub fn decide(&mut self, path: &str) -> VerdictReply {
        let patterns: Vec<String> = self.grants.iter().map(|g| g.pattern.clone()).collect();
        if let Some(glob) = first_matching_grant(&patterns, path) {
            return VerdictReply::AllowGlob(glob);
        }
        let req = self.pending.record(path);
        VerdictReply::Deny { req }
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
            mode: "prompt".to_string(),
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
        kind: None,
        ops: Vec::new(),
        source: source.map(str::to_string),
        created: created.map(str::to_string),
        evidence,
    });
    grants::write_grants(grants_dir, &file)?;
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
/// the engine's `fnmatch`-based caches agree on what a glob covers.
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
/// request line (the realpath), decide, write one reply line. An empty or
/// unreadable line gets no reply — the closed connection reads as a broker
/// error at the client, which fails closed for an activation.
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
    let path = line.trim();
    if !path.starts_with('/') {
        // The protocol carries realpaths only. An empty or non-absolute line
        // is a garbled exchange (or a same-user process poking the socket):
        // give no reply — the client reads EOF as a broker error and fails
        // closed — and never let junk into the pending review queue.
        return;
    }

    let reply = {
        let mut guard = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.decide(path)
    };
    write_reply(&mut writer, &reply);
}

/// Write one newline-terminated reply line.
fn write_reply(writer: &mut UnixStream, reply: &VerdictReply) {
    let line = reply.wire_line();
    if let Err(err) = writer.write_all(line.as_bytes()) {
        warn!(%err, "could not write verdict reply");
        return;
    }
    let _ = writer.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_match_allows_with_the_matched_glob() {
        let mut state = BrokerState::new(vec!["/home/dev/.cargo/**".to_string()]);
        let reply = state.decide("/home/dev/.cargo/registry/x");
        assert_eq!(
            reply,
            VerdictReply::AllowGlob("/home/dev/.cargo/**".to_string())
        );
        assert_eq!(reply.wire_line(), "allow-glob /home/dev/.cargo/**\n");
        // An allow never queues anything.
        assert_eq!(state.pending_len(), 0);
    }

    #[test]
    fn no_match_denies_and_queues_a_req_id() {
        let mut state = BrokerState::new(vec!["/data/**".to_string()]);
        let reply = state.decide("/home/dev/.aws/credentials");
        assert_eq!(reply, VerdictReply::Deny { req: 1 });
        assert_eq!(reply.wire_line(), "deny 1\n");
        assert_eq!(state.pending_len(), 1);
    }

    #[test]
    fn repeat_deny_reuses_the_same_req_id() {
        let mut state = BrokerState::new(vec![]);
        let first = state.decide("/p");
        let second = state.decide("/p");
        assert_eq!(first, VerdictReply::Deny { req: 1 });
        assert_eq!(second, VerdictReply::Deny { req: 1 });
        // Coalesced into one pending entry despite two requests.
        assert_eq!(state.pending_len(), 1);
    }

    #[test]
    fn empty_grant_set_denies_everything() {
        let mut state = BrokerState::new(vec![]);
        let reply = state.decide("/anything");
        assert_eq!(reply, VerdictReply::Deny { req: 1 });
    }
}
