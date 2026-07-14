//! The `host-native` sandbox backend: re-exec under the OS-level sandbox.
//!
//! macOS uses `sandbox-exec` with a generated SBPL profile that is permissive
//! at the base (`allow default`, so flox and the user's tools run) but denies
//! reading the contents of, and writing to, the user's entire home directory —
//! the agent's primary secret/data surface — except the project being activated
//! and Flox's own state. Metadata (stat/traverse) stays allowed so paths into
//! the re-allowed subdirs resolve; reads outside `$HOME` (system, Nix) are
//! untouched. Enforcement holds even against SIP-protected system binaries that
//! bypass advisory libsandbox. The Linux (bubblewrap + Landlock) path is not yet
//! wired.

use std::convert::Infallible;
use std::path::{Path, PathBuf};

use anyhow::Result;
use flox_core::activate::sandbox_backend::SandboxBackend;

use super::{ActivationSandbox, SandboxLaunchCtx, WRAPPED_MARKER_VAR};

pub struct HostNativeBackend {
    dot_flox_path: PathBuf,
}

impl HostNativeBackend {
    pub fn new(ctx: SandboxLaunchCtx<'_>) -> Self {
        Self {
            dot_flox_path: ctx.dot_flox_path,
        }
    }
}

impl ActivationSandbox for HostNativeBackend {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::HostNative
    }

    fn preflight(&self) -> Result<()> {
        #[cfg(not(target_os = "macos"))]
        {
            anyhow::bail!(
                "The 'host-native' sandbox backend is only wired on macOS (sandbox-exec).\n\
                 The Linux backend (bubblewrap + Landlock) is not yet implemented. \
                 Unset FLOX_SANDBOX_BACKEND or use '--sandbox-backend libsandbox'."
            );
        }
        #[cfg(target_os = "macos")]
        Ok(())
    }

    fn wrap_activation(self: Box<Self>) -> Result<Infallible> {
        wrap_host_native(&self.dot_flox_path)
    }
}

/// Re-exec the current `flox activate` invocation under the host-native OS
/// sandbox, then never return (the inner activation runs confined). Returns the
/// error on a failure to launch, or on an unsupported platform.
fn wrap_host_native(dot_flox_path: &Path) -> Result<Infallible> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::process::CommandExt;

        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            anyhow::anyhow!("HOME is not set; cannot build the host-native sandbox profile.")
        })?;
        // SBPL matches the realpath, so canonicalize (/var -> /private/var).
        let home = std::fs::canonicalize(&home).unwrap_or(home);
        // Re-allow the project root (the parent of its `.flox`) so the agent
        // can read and write the code it is working on.
        let dot_flox =
            std::fs::canonicalize(dot_flox_path).unwrap_or_else(|_| dot_flox_path.to_path_buf());
        let project = dot_flox.parent().unwrap_or(&dot_flox).to_path_buf();
        let profile = host_native_profile(&home, &project);

        let flox_exe = std::env::current_exe()
            .map_err(|err| anyhow::anyhow!("Cannot locate the flox binary to re-exec: {err}"))?;
        let inner_args: Vec<String> = std::env::args().skip(1).collect();

        // `exec` only returns on failure.
        let err = std::process::Command::new("sandbox-exec")
            .arg("-p")
            .arg(&profile)
            .arg(&flox_exe)
            .args(&inner_args)
            .env(WRAPPED_MARKER_VAR, "1")
            .exec();
        Err(anyhow::anyhow!(
            "Failed to launch the host-native sandbox: {err}. \
             'sandbox-exec' must be available on macOS."
        ))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = dot_flox_path;
        anyhow::bail!(
            "The 'host-native' sandbox backend is only wired on macOS (sandbox-exec).\n\
             The Linux backend (bubblewrap + Landlock) is not yet implemented. \
             Unset FLOX_SANDBOX_BACKEND or use '--sandbox-backend libsandbox'."
        );
    }
}

/// The macOS `sandbox-exec` (SBPL) profile for a host-native activation. Paths
/// are canonical because SBPL matches the realpath.
///
/// Deny-by-default for the user's home: the contents of every file under
/// `$HOME` are unreadable and unwritable except the project and Flox's own
/// state. `file-read-metadata` is left to `allow default` so path resolution
/// into the allowed subdirs works. `.env` files stay secret even inside the
/// project (the last matching rule wins).
#[cfg(target_os = "macos")]
fn host_native_profile(home: &Path, project: &Path) -> String {
    let h = home.display();
    let p = project.display();
    format!(
        r##"(version 1)
(allow default)
; Deny reading the contents of, and writing to, the user's home — the agent's
; primary secret/data surface. Metadata (stat/traverse) stays allowed via
; `allow default`, and reads outside $HOME (system, Nix) are untouched, so flox
; and tools still run.
(deny file-read-data file-write* (subpath "{h}"))
(allow file-read-data file-write*
  (subpath "{p}")
  (subpath "{h}/.cache")
  (subpath "{h}/.config")
  (subpath "{h}/.local")
  (subpath "{h}/Library/Application Support/flox")
  (subpath "{h}/Library/Caches/flox"))
; Keep .env files secret even inside the project.
(deny file-read-data file-write* (regex #"/\.env(\.[^/]*)?$"))
"##
    )
}
