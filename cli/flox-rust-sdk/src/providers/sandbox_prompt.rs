//! Interactive prompt broker for the build-time virtual sandbox (Phase 2).
//!
//! When a manifest build runs under `sandbox = "warn"`/`"enforce"`, libsandbox
//! (the `LD_PRELOAD`/`DYLD` shim) refers each out-of-closure file access that is
//! not already permitted to a broker over an `AF_UNIX` socket, instead of just
//! warning or blocking. This module is that broker: the `flox` side of the
//! control channel.
//!
//! Responsibilities handled here (increment 2):
//!   - own the socket and hand its path to the build (via
//!     `FLOX_SANDBOX_PROMPT_SOCKET`),
//!   - **serialize** requests — connections are accepted and answered one at a
//!     time, so prompts from parallel build processes (`make -j`, `npm`, …)
//!     never interleave,
//!   - **remember** accepted patterns and auto-answer any later request already
//!     covered by one, so the user is asked at most once per pattern even across
//!     many build processes,
//!   - delegate the actual allow/deny decision for a *new* access to a
//!     [`PromptResolver`] — the seam where the interactive UI (increment 3)
//!     plugs in.
//!
//! The wire protocol mirrors what libsandbox speaks: one request and one reply
//! per connection, newline-terminated text.
//!
//!   -> `<realpath>\n`
//!   <- `allow\n`                 allow this access (once)
//!   <- `allow-glob <pattern>\n`  allow, and remember `<pattern>`
//!   <- `deny\n`                  deny this access

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use tempfile::TempDir;
use tracing::{debug, warn};

/// Environment variable libsandbox reads to find the broker socket. Shared
/// with the activation-side broker through flox-core so the two servers
/// cannot drift.
pub const PROMPT_SOCKET_ENV: &str = flox_core::activate::prompt_protocol::PROMPT_SOCKET_ENV;

/// What to do about an out-of-closure access that is not yet covered by an
/// accepted pattern. Returned by a [`PromptResolver`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptDecision {
    /// Allow this exact path (remembered so the same path is not asked again).
    Allow,
    /// Allow and remember a glob pattern (e.g. `~/.npm/**`); future accesses
    /// matching it are auto-allowed, and the pattern can be written back to the
    /// manifest's `sandbox-allow`.
    AllowGlob(String),
    /// Deny this access (libsandbox turns it into `EACCES`).
    Deny,
}

/// Decides what to do about a *new* out-of-closure access (one not already
/// covered by a previously accepted pattern). The broker owns the resolver and
/// calls it on its serving thread, so it may block (e.g. to prompt the user);
/// the single-threaded accept loop guarantees one prompt at a time.
///
/// This is the extension point for increment 3's interactive UI. Implementations
/// must be `Send` because the resolver moves onto the broker thread.
pub trait PromptResolver: Send {
    fn resolve(&mut self, path: &Path) -> PromptDecision;
}

/// A [`PromptResolver`] that denies everything. Useful as a safe default and in
/// tests; with it the broker behaves like plain `enforce`.
#[derive(Debug, Default, Clone)]
pub struct DenyAll;

impl PromptResolver for DenyAll {
    fn resolve(&mut self, _path: &Path) -> PromptDecision {
        PromptDecision::Deny
    }
}

/// The set of exceptions accepted so far during a build. Shared between the
/// serving thread (which appends as the user accepts) and the owner (which reads
/// `globs` afterwards to offer a `sandbox-allow` write-back).
#[derive(Debug, Default)]
struct Accepted {
    /// Glob patterns the user accepted (`AllowGlob`).
    globs: Vec<String>,
    /// Exact realpaths the user accepted (`Allow`).
    exacts: Vec<String>,
    /// `$HOME`, captured once, for expanding a leading `~/` in patterns the same
    /// way libsandbox does.
    home: Option<String>,
}

impl Accepted {
    /// If `path` is already covered by an accepted glob, return that glob.
    fn matching_glob(&self, path: &str) -> Option<&str> {
        self.globs
            .iter()
            .find(|g| glob_matches(g, path, self.home.as_deref()))
            .map(String::as_str)
    }

    fn covered_exactly(&self, path: &str) -> bool {
        self.exacts.iter().any(|p| p == path)
    }
}

/// A running prompt broker. Dropping it shuts the serving thread down and
/// removes the socket.
pub struct SandboxPromptBroker {
    socket_path: PathBuf,
    // Kept so the temp dir (and the socket inside it) lives as long as the
    // broker; dropped last.
    _tempdir: TempDir,
    accepted: Arc<Mutex<Accepted>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SandboxPromptBroker {
    /// Create a broker listening on a fresh socket, serving requests with
    /// `resolver`. The serving thread starts immediately.
    pub fn new(resolver: Box<dyn PromptResolver>) -> std::io::Result<Self> {
        let tempdir = tempfile::Builder::new()
            .prefix("flox-sandbox-prompt")
            .tempdir()?;
        let socket_path = tempdir.path().join("prompt.sock");
        let listener = UnixListener::bind(&socket_path)?;

        let accepted = Arc::new(Mutex::new(Accepted {
            home: std::env::var("HOME").ok(),
            ..Default::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));

        let handle = {
            let accepted = Arc::clone(&accepted);
            let stop = Arc::clone(&stop);
            std::thread::Builder::new()
                .name("flox-sandbox-prompt-broker".into())
                .spawn(move || serve(listener, resolver, accepted, stop))?
        };

        Ok(Self {
            socket_path,
            _tempdir: tempdir,
            accepted,
            stop,
            handle: Some(handle),
        })
    }

    /// Path of the broker socket; set as `FLOX_SANDBOX_PROMPT_SOCKET` in the
    /// build's environment.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Glob patterns the user accepted so far. Increment 4 offers to write these
    /// back to the manifest's `sandbox-allow`.
    pub fn accepted_globs(&self) -> Vec<String> {
        self.accepted
            .lock()
            .expect("broker mutex poisoned")
            .globs
            .clone()
    }

    /// Stop the serving thread and wait for it. Idempotent; also run by `Drop`.
    pub fn shutdown(&mut self) {
        if self.stop.swap(true, Ordering::SeqCst) {
            return; // already stopping
        }
        // Wake the blocking accept() by making a throwaway connection; the loop
        // observes `stop` and exits without serving it.
        let _ = UnixStream::connect(&self.socket_path);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for SandboxPromptBroker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The serving thread: accept one connection at a time and answer it. Because
/// the loop is single-threaded and handles each connection fully before
/// accepting the next, prompts are inherently serialized.
fn serve(
    listener: UnixListener,
    mut resolver: Box<dyn PromptResolver>,
    accepted: Arc<Mutex<Accepted>>,
    stop: Arc<AtomicBool>,
) {
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        match stream {
            Ok(stream) => {
                if let Err(err) = handle_connection(stream, resolver.as_mut(), &accepted) {
                    debug!(%err, "sandbox prompt: connection error");
                }
            },
            Err(err) => {
                warn!(%err, "sandbox prompt: accept failed");
                break;
            },
        }
    }
}

/// Read one request, decide, and write one reply.
fn handle_connection(
    stream: UnixStream,
    resolver: &mut dyn PromptResolver,
    accepted: &Arc<Mutex<Accepted>>,
) -> std::io::Result<()> {
    // The listener may be non-blocking after a wake; ensure normal blocking I/O.
    stream.set_nonblocking(false)?;
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(()); // peer closed without a request (e.g. shutdown wake)
    }
    let path = line.trim_end_matches(['\n', '\r']);
    if path.is_empty() {
        return Ok(());
    }

    let reply = decide(path, resolver, accepted);
    debug!(path, reply = reply.trim_end(), "sandbox prompt: answered");
    (&stream).write_all(reply.as_bytes())
}

/// Produce the wire reply for `path`: an already-accepted glob auto-answers with
/// that glob (so the client caches it too), an already-accepted exact path
/// auto-answers `allow`, and anything else is referred to `resolver`.
fn decide(
    path: &str,
    resolver: &mut dyn PromptResolver,
    accepted: &Arc<Mutex<Accepted>>,
) -> String {
    {
        let acc = accepted.lock().expect("broker mutex poisoned");
        if let Some(glob) = acc.matching_glob(path) {
            return format!("allow-glob {glob}\n");
        }
        if acc.covered_exactly(path) {
            return "allow\n".to_string();
        }
    }
    // Not yet covered: ask the resolver (this may block to prompt the user). The
    // lock is released across the call so reads of `accepted` don't stall.
    match resolver.resolve(Path::new(path)) {
        PromptDecision::AllowGlob(glob) => {
            accepted
                .lock()
                .expect("broker mutex poisoned")
                .globs
                .push(glob.clone());
            format!("allow-glob {glob}\n")
        },
        PromptDecision::Allow => {
            accepted
                .lock()
                .expect("broker mutex poisoned")
                .exacts
                .push(path.to_string());
            "allow\n".to_string()
        },
        PromptDecision::Deny => "deny\n".to_string(),
    }
}

/// Match `path` against `pattern` the way libsandbox does: `fnmatch` with no
/// `FNM_PATHNAME`, so `*`/`**` cross `/`, plus a leading `~/` expanded to
/// `$HOME/`. Used only to decide whether to re-prompt; the actual enforcement is
/// libsandbox's, so a mismatch merely costs an extra prompt.
fn glob_matches(pattern: &str, path: &str, home: Option<&str>) -> bool {
    let expanded;
    let pattern = match (home, pattern.strip_prefix("~/")) {
        (Some(home), Some(rest)) => {
            expanded = format!("{home}/{rest}");
            expanded.as_str()
        },
        _ => pattern,
    };
    wildcard_match(pattern, path)
}

/// Classic backtracking wildcard match supporting `*` (any run, including `/`)
/// and `?` (any single character). Bracket expressions are not handled — accepted
/// `sandbox-allow` patterns are plain path globs, and an unhandled pattern just
/// means we prompt again rather than auto-allowing.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    // Position to backtrack to on a `*`, and the text index it last consumed up
    // to.
    let mut star: Option<usize> = None;
    let mut star_ti = 0usize;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(star_pi) = star {
            // Last `*` absorbs one more character of text and we retry.
            pi = star_pi + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};

    use super::*;

    /// A resolver scripted with a fixed sequence of decisions; records how many
    /// times it was asked so tests can assert auto-answers did NOT call it.
    struct ScriptedResolver {
        decisions: Vec<PromptDecision>,
        next: usize,
        asked: Arc<Mutex<Vec<String>>>,
    }

    impl PromptResolver for ScriptedResolver {
        fn resolve(&mut self, path: &Path) -> PromptDecision {
            self.asked
                .lock()
                .unwrap()
                .push(path.to_string_lossy().into_owned());
            let d = self.decisions[self.next.min(self.decisions.len() - 1)].clone();
            self.next += 1;
            d
        }
    }

    /// Send one request over a fresh connection and return the broker's reply,
    /// mimicking libsandbox's one-shot-per-connection protocol.
    fn query(socket: &Path, path: &str) -> String {
        let mut stream = UnixStream::connect(socket).unwrap();
        stream.write_all(format!("{path}\n").as_bytes()).unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).unwrap();
        reply
    }

    fn broker_with(
        decisions: Vec<PromptDecision>,
    ) -> (SandboxPromptBroker, Arc<Mutex<Vec<String>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let resolver = ScriptedResolver {
            decisions,
            next: 0,
            asked: Arc::clone(&asked),
        };
        let broker = SandboxPromptBroker::new(Box::new(resolver)).unwrap();
        (broker, asked)
    }

    #[test]
    fn deny_is_relayed() {
        let (broker, asked) = broker_with(vec![PromptDecision::Deny]);
        assert_eq!(query(broker.socket_path(), "/etc/hosts"), "deny\n");
        assert_eq!(*asked.lock().unwrap(), vec!["/etc/hosts".to_string()]);
    }

    #[test]
    fn allow_is_relayed_and_caches_exact_path() {
        let (broker, asked) = broker_with(vec![PromptDecision::Allow]);
        assert_eq!(query(broker.socket_path(), "/etc/hosts"), "allow\n");
        // Second request for the same path is auto-allowed without re-asking.
        assert_eq!(query(broker.socket_path(), "/etc/hosts"), "allow\n");
        assert_eq!(*asked.lock().unwrap(), vec!["/etc/hosts".to_string()]);
    }

    #[test]
    fn allow_glob_caches_and_auto_answers_siblings() {
        let (broker, asked) = broker_with(vec![PromptDecision::AllowGlob(
            "/home/u/.npm/**".to_string(),
        )]);
        assert_eq!(
            query(broker.socket_path(), "/home/u/.npm/foo"),
            "allow-glob /home/u/.npm/**\n"
        );
        // A sibling under the accepted glob auto-answers with the same glob and
        // does not consult the resolver again.
        assert_eq!(
            query(broker.socket_path(), "/home/u/.npm/bar/baz"),
            "allow-glob /home/u/.npm/**\n"
        );
        assert_eq!(broker.accepted_globs(), vec!["/home/u/.npm/**".to_string()]);
        assert_eq!(*asked.lock().unwrap(), vec!["/home/u/.npm/foo".to_string()]);
    }

    #[test]
    fn unrelated_path_after_glob_is_prompted() {
        let (broker, asked) = broker_with(vec![
            PromptDecision::AllowGlob("/home/u/.npm/**".to_string()),
            PromptDecision::Deny,
        ]);
        assert_eq!(
            query(broker.socket_path(), "/home/u/.npm/foo"),
            "allow-glob /home/u/.npm/**\n"
        );
        // /etc/hosts is not under the glob, so the resolver is asked again.
        assert_eq!(query(broker.socket_path(), "/etc/hosts"), "deny\n");
        assert_eq!(asked.lock().unwrap().len(), 2);
    }

    #[test]
    fn wildcard_match_semantics() {
        assert!(wildcard_match("/a/**", "/a/b/c"));
        assert!(wildcard_match("/a/*", "/a/b/c")); // '*' crosses '/'
        assert!(wildcard_match("/a/?", "/a/b"));
        assert!(!wildcard_match("/a/?", "/a/bc"));
        assert!(!wildcard_match("/a/b", "/a/c"));
        assert!(wildcard_match("*", "anything/at/all"));
    }

    #[test]
    fn glob_matches_expands_home() {
        assert!(glob_matches("~/.npm/**", "/home/u/.npm/x", Some("/home/u")));
        assert!(!glob_matches(
            "~/.npm/**",
            "/home/u/.cache/x",
            Some("/home/u")
        ));
    }
}
