//! The broker's pending-request queue.
//!
//! When a request does not match any session grant, the broker has no human
//! approver in this batch, so it records the request as *pending* and denies
//! (auto-deny-and-queue). A later batch's `flox sandbox` CLI reads this queue
//! to present the review surface; for now it accumulates entries with hit
//! counts and a monotonically increasing request id.
//!
//! Coalescing is deliberately minimal here: identical `(realpath, op)`
//! requests join one entry and bump its hit count rather than allocating a
//! new `req` id each time, so a storm of repeated opens does not inflate the
//! queue. Debounce, scope suggestion, and the storm fuse from the design are
//! later refinements; the contract this batch must hold is "every distinct
//! `(path, op)` gets a stable `req` id, and repeats increment a counter."

use std::collections::HashMap;

/// A single queued request the broker could not resolve from grants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEntry {
    /// Stable request id assigned the first time this `(path, op)` was seen.
    pub req: u64,
    /// Resolved request path.
    pub path: String,
    /// Access kind (`read` / `write`).
    pub op: String,
    /// How many times this `(path, op)` has been requested.
    pub hits: u64,
}

/// The pending queue. Keyed by `(realpath, op)` so a read and a write of the
/// same path are distinct entries (they carry different review semantics).
#[derive(Debug, Default)]
pub struct PendingQueue {
    /// Next request id to hand out. Monotonic; never reused even if an entry
    /// is later drained, so a `req` number unambiguously names one request
    /// for the lifetime of the broker.
    next_req: u64,
    /// `(path, op)` -> entry.
    entries: HashMap<(String, String), PendingEntry>,
}

impl PendingQueue {
    pub fn new() -> Self {
        Self {
            next_req: 1,
            entries: HashMap::new(),
        }
    }

    /// Record a request for `(path, op)`, returning its request id.
    ///
    /// The first time a `(path, op)` is seen it is assigned the next id and
    /// its hit count starts at 1. Subsequent identical requests reuse that id
    /// and bump the hit count, so the returned id is stable across retries —
    /// which lets the C-side receipt say "queued as req N" consistently and
    /// lets a retry-after-approval line up with the same pending entry.
    pub fn record(&mut self, path: &str, op: &str) -> u64 {
        let key = (path.to_string(), op.to_string());
        if let Some(entry) = self.entries.get_mut(&key) {
            entry.hits += 1;
            return entry.req;
        }
        let req = self.next_req;
        self.next_req += 1;
        self.entries.insert(key, PendingEntry {
            req,
            path: path.to_string(),
            op: op.to_string(),
            hits: 1,
        });
        req
    }

    /// Number of distinct pending `(path, op)` entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    // Read accessors below are exercised by tests and consumed by the next
    // batch's `flox sandbox` review CLI, which reads this queue; allow them to
    // be unused in the current non-test build without a warning.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Snapshot of pending entries, for the review surface and tests.
    pub fn entries(&self) -> Vec<PendingEntry> {
        self.entries.values().cloned().collect()
    }

    /// Remove every entry whose path satisfies `keep_if_matches`, returning how
    /// many were removed.
    ///
    /// Used when a grant is approved: the entries the new grant now covers are
    /// no longer pending, so the broker drains them and reports the count to
    /// the CLI ("approved, cleared N pending"). A predicate (rather than an
    /// exact path) lets a directory grant clear a whole burst at once.
    pub fn drain_matching(&mut self, keep_if_matches: impl Fn(&str) -> bool) -> usize {
        let before = self.entries.len();
        self.entries
            .retain(|(path, _op), _entry| !keep_if_matches(path));
        before - self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_request_gets_req_one_and_one_hit() {
        let mut queue = PendingQueue::new();
        let req = queue.record("/home/dev/.config/x", "read");
        assert_eq!(req, 1);
        assert_eq!(queue.len(), 1);
        let entry = &queue.entries()[0];
        assert_eq!(entry.req, 1);
        assert_eq!(entry.hits, 1);
        assert_eq!(entry.op, "read");
    }

    #[test]
    fn repeat_of_same_path_and_op_coalesces_and_bumps_hits() {
        let mut queue = PendingQueue::new();
        let first = queue.record("/p", "read");
        let second = queue.record("/p", "read");
        let third = queue.record("/p", "read");
        // Same id every time; the queue stays a single entry.
        assert_eq!(first, 1);
        assert_eq!(second, 1);
        assert_eq!(third, 1);
        assert_eq!(queue.len(), 1);
        assert_eq!(queue.entries()[0].hits, 3);
    }

    #[test]
    fn distinct_paths_and_ops_get_distinct_monotonic_ids() {
        let mut queue = PendingQueue::new();
        // A read and a write of the same path are distinct review questions.
        let read_p = queue.record("/p", "read");
        let write_p = queue.record("/p", "write");
        let read_q = queue.record("/q", "read");
        assert_eq!(read_p, 1);
        assert_eq!(write_p, 2);
        assert_eq!(read_q, 3);
        assert_eq!(queue.len(), 3);
    }
}
