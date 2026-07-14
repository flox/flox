//! The `srt` sandbox backend: re-exec under Anthropic's sandbox-runtime.
//!
//! `srt` drives `sandbox-exec` on macOS and `bubblewrap` on Linux, and adds
//! proxy-based network egress control, so it brings the same kernel boundary as
//! `host-native` packaged as an installable tool — and, unlike host-native's
//! bespoke profile, it is wired on both platforms.
//!
//! Generates an srt settings file mirroring the host-native deny-`$HOME` policy
//! (srt reads are deny-then-allow, so deny `$HOME` and re-allow only the project
//! and Flox's state) and runs `srt -s <settings> <flox> activate <args>` with
//! `_FLOX_SANDBOX_WRAPPED=1` (srt preserves the environment). Network egress is
//! default-deny — srt's secure-by-default — which `host-native` does not
//! enforce.

use std::convert::Infallible;
use std::path::{Path, PathBuf};

use anyhow::Result;
use flox_core::activate::sandbox_backend::SandboxBackend;

use super::{ActivationSandbox, SandboxLaunchCtx, WRAPPED_MARKER_VAR};

pub struct SrtBackend {
    dot_flox_path: PathBuf,
}

impl SrtBackend {
    pub fn new(ctx: SandboxLaunchCtx<'_>) -> Self {
        Self {
            dot_flox_path: ctx.dot_flox_path,
        }
    }
}

impl ActivationSandbox for SrtBackend {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::Srt
    }

    fn preflight(&self) -> Result<()> {
        if !binary_on_path("srt") {
            anyhow::bail!(
                "The 'srt' sandbox backend requires Anthropic's sandbox-runtime, which was not \
                 found on PATH.\nInstall it (e.g. 'flox install sandbox-runtime' or \
                 'npm install -g @anthropic-ai/sandbox-runtime'), or use \
                 '--sandbox-backend host-native'."
            );
        }
        Ok(())
    }

    fn wrap_activation(self: Box<Self>) -> Result<Infallible> {
        wrap_srt(&self.dot_flox_path)
    }
}

/// Re-exec the current `flox activate` under Anthropic's sandbox-runtime
/// (`srt`), then never return.
fn wrap_srt(dot_flox_path: &Path) -> Result<Infallible> {
    use std::os::unix::process::CommandExt;

    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        anyhow::anyhow!("HOME is not set; cannot build the srt sandbox settings.")
    })?;
    let home = std::fs::canonicalize(&home).unwrap_or(home);
    let dot_flox =
        std::fs::canonicalize(dot_flox_path).unwrap_or_else(|_| dot_flox_path.to_path_buf());
    let project = dot_flox.parent().unwrap_or(&dot_flox).to_path_buf();

    let settings = srt_settings_json(&home, &project);
    // The settings file must outlive this process (exec replaces us), so it is
    // written to a temp file that is intentionally not cleaned up.
    let settings_path =
        std::env::temp_dir().join(format!("flox-srt-{}.json", std::process::id()));
    std::fs::write(&settings_path, settings)
        .map_err(|err| anyhow::anyhow!("Failed to write the srt settings file: {err}"))?;

    let flox_exe = std::env::current_exe()
        .map_err(|err| anyhow::anyhow!("Cannot locate the flox binary to re-exec: {err}"))?;
    let inner_args: Vec<String> = std::env::args().skip(1).collect();

    // `exec` only returns on failure.
    let err = std::process::Command::new("srt")
        .arg("-s")
        .arg(&settings_path)
        .arg(&flox_exe)
        .args(&inner_args)
        .env(WRAPPED_MARKER_VAR, "1")
        .exec();
    Err(anyhow::anyhow!("Failed to launch srt: {err}."))
}

/// The srt settings JSON for a host-native-equivalent activation policy.
///
/// srt reads are deny-then-allow (`allowRead` wins), so denying `$HOME` and
/// re-allowing only the project and Flox's dirs leaves arbitrary home files
/// (e.g. `~/.ssh`) unreadable. Writes are allow-only. Network is default-deny
/// (empty allowlist) — the egress control `host-native` lacks.
fn srt_settings_json(home: &Path, project: &Path) -> String {
    let h = home.display().to_string();
    let p = project.display().to_string();
    let cache = format!("{h}/.cache");
    let config = format!("{h}/.config");
    let local = format!("{h}/.local");
    let settings = serde_json::json!({
        "filesystem": {
            "denyRead": [&h],
            "allowRead": [&p, &cache, &config, &local],
            "allowWrite": [&p, &cache, &config, &local, "/tmp", "/private/tmp"],
            "denyWrite": [],
        },
        "network": {
            "allowedDomains": [],
            "deniedDomains": [],
            // The activation talks to the Nix daemon and binds Flox's own run
            // sockets; allow those unix-socket dirs (subpath-matched) while
            // keeping TCP egress default-deny.
            "allowUnixSockets": ["/nix/var/nix/daemon-socket", &cache],
        },
    });
    serde_json::to_string_pretty(&settings).expect("serializing a literal JSON value cannot fail")
}

/// `true` if an executable named `name` is on `PATH`.
fn binary_on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}
