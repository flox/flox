//! Shared constants for the sandbox prompt-broker wire protocol.
//!
//! Two brokers speak this protocol to the libsandbox client in
//! `package-builder/sandbox.c`: the per-build `SandboxPromptBroker` in
//! flox-rust-sdk (interactive `/dev/tty` prompting) and the per-activation
//! broker hosted in the flox-activations executive (headless grant table +
//! review queue). Centralizing the socket variable and reply vocabulary here
//! keeps the two servers and any Rust-side tests from drifting apart; the C
//! client mirrors these strings by hand.
//!
//! The protocol is one AF_UNIX connection per query, newline-terminated text:
//!
//! ```text
//! -> "<realpath>\n"
//! <- "allow\n"                 allow (client caches the exact path)
//! <- "allow-glob <pattern>\n"  allow (client caches the pattern)
//! <- "deny\n"                  deny (errno=EACCES at the interceptor)
//! <- "deny <req>\n"            deny, queued for out-of-band review as
//!                              request <req> (activation broker only)
//! ```
//!
//! No reply (the server closing the connection) is a broker error; the client
//! falls back to its default policy — plain enforce for a build, a graceful
//! fail-closed deny for an activation.

/// Environment variable libsandbox reads to find the broker socket.
pub const PROMPT_SOCKET_ENV: &str = "FLOX_SANDBOX_PROMPT_SOCKET";

/// Reply allowing the exact requested path.
pub const REPLY_ALLOW: &str = "allow";

/// Prefix of a reply allowing a glob pattern; the pattern follows the space.
pub const REPLY_ALLOW_GLOB_PREFIX: &str = "allow-glob ";

/// Reply denying the requested path (the bare, build-broker form).
pub const REPLY_DENY: &str = "deny";
