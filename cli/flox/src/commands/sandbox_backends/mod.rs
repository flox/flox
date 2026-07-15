//! Per-backend activation modules for the sandboxed activation prototype.
//!
//! Each backend that wraps the activation under an OS or container
//! enforcement boundary is self-contained in a module here and implements
//! [`ActivationSandbox`]. The dispatch in `activate.rs` reduces to:
//!
//! ```text
//! resolve backend
//!   → ensure_advisory_mode_supported
//!   → for_backend(backend)           // None → libsandbox in-process path
//!   → preflight()                    // host prerequisite checks
//!   → wrap_activation()              // execs; never returns on success
//! ```
//!
//! # Adding a new backend
//!
//! 1. Add a variant to `SandboxBackend` and a row in `BackendCapabilities`
//!    in `cli/flox-core/src/activate/sandbox_backend.rs`.
//! 2. Add a module in this directory implementing `ActivationSandbox`.
//! 3. Add a `for_backend` arm returning `Some(Box::new(YourBackend::new(ctx)))`.

pub mod host_native;
pub mod oci;
pub mod srt;

use std::convert::Infallible;
use std::path::PathBuf;

use anyhow::Result;
use flox_core::activate::context::InvocationType;
use flox_core::activate::sandbox_backend::SandboxBackend;
use flox_manifest::lockfile::Lockfile;

use flox_config::Config;

/// Environment variable set on the re-exec'd inner activation so all backends
/// can detect that wrapping already occurred.
///
/// Both `host-native` (via `sandbox-exec`) and `srt` set this variable on
/// their re-exec; `activate.rs` reads it to short-circuit a second wrap.
/// Living here rather than in a single backend module makes the cross-backend
/// protocol visible in one place.
pub(crate) const WRAPPED_MARKER_VAR: &str = "_FLOX_SANDBOX_WRAPPED";

/// Context bundling everything the sandbox wrap functions need.
///
/// Passed from `activate.rs` to [`ActivationSandbox::wrap_activation`] so
/// each backend can destructure only what it uses.
#[derive(Debug)]
pub struct SandboxLaunchCtx<'a> {
    /// Absolute path to the `.flox` directory of the environment being
    /// activated.
    pub dot_flox_path: PathBuf,
    /// Short environment name used as the OCI image tag prefix and in
    /// error messages.
    pub env_name: String,
    /// How the user invoked `flox activate` (interactive shell, exec, shell
    /// command, or in-place script).
    pub invocation_type: &'a InvocationType,
    /// Flox handle for API access (OCI bake uses it for the container
    /// builder pipeline).
    pub flox: &'a flox_rust_sdk::flox::Flox,
    /// Resolved lockfile for the environment (OCI uses it for hash-tag
    /// derivation and builder-pin selection).
    pub lockfile: &'a Lockfile,
    /// User and manifest config (OCI reads `sandbox_oci_autobake`).
    pub config: &'a Config,
}

/// A sandbox backend that wraps `flox activate` under an enforcement
/// boundary.
///
/// Implementing types are constructed from a [`SandboxLaunchCtx`] and live
/// only for the duration of a single activation. The trait is object-safe
/// so the factory can return `Box<dyn ActivationSandbox>`.
pub trait ActivationSandbox {
    /// The `SandboxBackend` variant this implementation handles.
    fn backend(&self) -> SandboxBackend;

    /// Run host-side prerequisite checks with actionable error messages.
    ///
    /// Called before `wrap_activation`. Should verify that any required
    /// binary is present on `PATH`, that the platform is supported, and
    /// that any needed state is available. Must not mutate the environment.
    fn preflight(&self) -> Result<()>;

    /// Re-enter the activation under the enforcement boundary, then never
    /// return.
    ///
    /// On success this execs the sandboxed process and the Rust process is
    /// replaced — `Infallible` makes the "never returns on success"
    /// contract visible in the type. Returns `Err` only when the launch
    /// itself fails.
    fn wrap_activation(self: Box<Self>) -> Result<Infallible>;
}

/// Return the backend implementation for `backend`, or `None` when the
/// backend uses the default in-process libsandbox path (or is a planned
/// backend that deliberately keeps its "not yet wired" error at the
/// dispatch site).
///
/// `None` variants:
/// - `Libsandbox` — in-process env-var injection, handled in `activate.rs`.
/// - Any `other` arm — falls through to the "not yet wired" bail in
///   `activate.rs`, preserving the loud failure for scaffolded backends.
pub fn for_backend(
    backend: SandboxBackend,
    ctx: SandboxLaunchCtx<'_>,
) -> Option<Box<dyn ActivationSandbox + '_>> {
    match backend {
        SandboxBackend::HostNative => Some(Box::new(host_native::HostNativeBackend::new(ctx))),
        SandboxBackend::Srt => Some(Box::new(srt::SrtBackend::new(ctx))),
        SandboxBackend::Oci => Some(Box::new(oci::OciBackend::new(ctx))),
        // Libsandbox is the default in-process path; no wrapper object.
        // All other variants keep the "not yet wired" bail in activate.rs.
        _ => None,
    }
}
