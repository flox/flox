//! Selection and capability declaration for the activation sandbox backend.
//!
//! The *mode* ([`SandboxMode`](super::sandbox_mode::SandboxMode)) says how
//! strictly to apply policy (off/warn/enforce/prompt). The *backend* selected
//! here says which enforcement mechanism applies it. `libsandbox` is the
//! default and is the advisory loader-interposition engine that ships today;
//! the other backends are alternative enforcement mechanisms being benchmarked
//! to choose a default sandbox provider.
//!
//! Each backend carries a [`BackendCapabilities`] declaration — the
//! "per-provider lossiness declaration" from the provider contract. It is pure
//! data: it drives the `flox sandbox backends` listing, the benchmark's
//! "expected-contained" red-team mapping, and the DX/lossiness report. It does
//! not by itself change enforcement behavior.
//!
//! See `forge:slices/2026/06-sandboxed-activation-prototype/artifacts/backend-contract.md`.

use std::fmt::Display;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Environment variable that selects the activation sandbox backend.
///
/// Unset or `libsandbox` reproduces today's behavior. Other values select an
/// alternative enforcement mechanism; see [`SandboxBackend`].
pub const FLOX_SANDBOX_BACKEND_VAR: &str = "FLOX_SANDBOX_BACKEND";

/// The enforcement mechanism that applies the sandbox policy for an activation.
#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, Hash, PartialEq, Eq, Default, schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
#[cfg_attr(any(test, feature = "tests"), derive(proptest_derive::Arbitrary))]
pub enum SandboxBackend {
    /// Advisory libc interposition (`LD_PRELOAD` / `DYLD_INSERT_LIBRARIES`).
    /// The engine that ships today, and the default.
    #[default]
    Libsandbox,
    /// The Nix build sandbox (sandbox-exec on macOS, namespaces + seccomp on
    /// Linux) adapted as an activation backend.
    Nix,
    /// Host-native OS sandbox: Seatbelt / `sandbox-exec` on macOS, bubblewrap +
    /// Landlock on Linux.
    HostNative,
    /// Anthropic sandbox-runtime (`srt`): a host-native OS sandbox plus a
    /// bundled egress proxy, driven by a policy close to `grants.toml`.
    Srt,
    /// OCI container: Apple Container on macOS, Podman on Linux.
    Oci,
    /// Embeddable micro-VM (libkrun): hypervisor isolation with a guest kernel
    /// via libkrunfw (Hypervisor.framework on macOS, KVM on Linux).
    Libkrun,
}

/// The class of isolation boundary a backend enforces, ordered weakest to
/// strongest. Backends that share a class tend to cluster on the isolation and
/// raw-performance axes; they are differentiated by integration cost, policy
/// expressiveness, and ask-flow fit.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Hash, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Enforcement {
    /// Cooperative libc interposition; bypassed by static binaries, raw
    /// syscalls, and env scrubbing. Friction and audit, not containment.
    Advisory,
    /// Kernel-enforced on the host with native filesystem I/O (Seatbelt,
    /// bubblewrap + Landlock + seccomp, the Nix build sandbox).
    HostKernel,
    /// OS-level container; a Linux guest VM on macOS. Shares the host kernel on
    /// Linux.
    Container,
    /// Hypervisor-isolated micro-VM; the strongest boundary, at an I/O cost.
    Hypervisor,
}

/// How a backend is available on a given platform.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Hash, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PlatformSupport {
    /// Runs natively; the workload runs on the host OS.
    Native,
    /// Available, but the workload runs inside a Linux guest VM (a DX
    /// divergence: macOS users land in Linux).
    ViaLinuxVm,
    /// Not available on this platform.
    Unsupported,
}

/// How far a backend's integration has progressed in this effort.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Hash, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum IntegrationStatus {
    /// Wired into activation and exercised.
    Implemented,
    /// Selector, capabilities, and launch plan exist; the launch path is not
    /// yet wired. Selecting it errors with a clear message.
    Scaffolded,
    /// Roster member with a launch plan but no implementation yet.
    Planned,
}

/// The per-provider capability and lossiness declaration for a backend.
///
/// Compared whole in tests so a new field is caught everywhere it is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackendCapabilities {
    pub backend: SandboxBackend,
    pub enforcement: Enforcement,
    /// Provides adversarial containment, vs. advisory friction only.
    pub enforces: bool,
    /// Can adjudicate an out-of-policy access mid-session (live "prompt"),
    /// vs. only redeeming a new grant at the next spawn/activation.
    pub live_ask: bool,
    /// Enforces domain-level network egress, natively or via a bundled proxy.
    /// When false, only IP/CIDR:port rules are expressible without flox adding
    /// its own proxy.
    pub domain_egress: bool,
    /// Distinguishes read / write / exec per grant, vs. being op-blind.
    pub per_op: bool,
    /// Virtualizes the filesystem and so pays a virtio-fs-style I/O penalty,
    /// worst on Flox's small-file/symlink-heavy Nix-store and node_modules
    /// traversal.
    pub fs_virtualized: bool,
    pub macos: PlatformSupport,
    pub linux: PlatformSupport,
    pub status: IntegrationStatus,
}

impl SandboxBackend {
    /// Every backend on the roster, in spectrum order (weakest boundary first).
    pub const ALL: [SandboxBackend; 6] = [
        SandboxBackend::Libsandbox,
        SandboxBackend::Nix,
        SandboxBackend::HostNative,
        SandboxBackend::Srt,
        SandboxBackend::Oci,
        SandboxBackend::Libkrun,
    ];

    /// The backend selected by [`FLOX_SANDBOX_BACKEND_VAR`], or the default
    /// (`libsandbox`) when unset. Returns a parse error for an unknown value.
    pub fn from_env() -> Result<SandboxBackend, SandboxBackendParseError> {
        match std::env::var(FLOX_SANDBOX_BACKEND_VAR) {
            Ok(value) if !value.is_empty() => value.parse(),
            _ => Ok(SandboxBackend::default()),
        }
    }

    /// The capability and lossiness declaration for this backend.
    pub fn capabilities(self) -> BackendCapabilities {
        use Enforcement::*;
        use IntegrationStatus::*;
        use PlatformSupport::*;

        match self {
            SandboxBackend::Libsandbox => BackendCapabilities {
                backend: self,
                enforcement: Advisory,
                enforces: false,
                live_ask: true,
                domain_egress: false,
                per_op: false,
                fs_virtualized: false,
                macos: Native,
                linux: Native,
                status: Implemented,
            },
            SandboxBackend::Nix => BackendCapabilities {
                backend: self,
                enforcement: HostKernel,
                enforces: true,
                live_ask: false,
                domain_egress: false,
                per_op: true,
                fs_virtualized: false,
                macos: Native,
                linux: Native,
                status: Scaffolded,
            },
            SandboxBackend::HostNative => BackendCapabilities {
                backend: self,
                enforcement: HostKernel,
                enforces: true,
                live_ask: false,
                domain_egress: false,
                per_op: true,
                fs_virtualized: false,
                macos: Native,
                linux: Native,
                // Wired via `sandbox-exec` on macOS; the Linux (bubblewrap)
                // launch path errors with a clear message until wired.
                status: Implemented,
            },
            SandboxBackend::Srt => BackendCapabilities {
                backend: self,
                enforcement: HostKernel,
                enforces: true,
                live_ask: true,
                domain_egress: true,
                per_op: true,
                fs_virtualized: false,
                // Wired on both platforms (srt drives sandbox-exec on macOS,
                // bubblewrap on Linux); requires the `srt` tool on PATH.
                macos: Native,
                linux: Native,
                status: Implemented,
            },
            SandboxBackend::Oci => BackendCapabilities {
                backend: self,
                enforcement: Container,
                enforces: true,
                live_ask: false,
                domain_egress: false,
                per_op: true,
                fs_virtualized: true,
                macos: ViaLinuxVm,
                linux: Native,
                status: Scaffolded,
            },
            SandboxBackend::Libkrun => BackendCapabilities {
                backend: self,
                enforcement: Hypervisor,
                enforces: true,
                live_ask: false,
                domain_egress: false,
                per_op: true,
                fs_virtualized: true,
                macos: ViaLinuxVm,
                linux: Native,
                status: Planned,
            },
        }
    }
}

impl Display for SandboxBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            SandboxBackend::Libsandbox => "libsandbox",
            SandboxBackend::Nix => "nix",
            SandboxBackend::HostNative => "host-native",
            SandboxBackend::Srt => "srt",
            SandboxBackend::Oci => "oci",
            SandboxBackend::Libkrun => "libkrun",
        };
        write!(f, "{name}")
    }
}

#[derive(Debug, thiserror::Error)]
#[error(
    "'{0}' is not a valid sandbox backend. Expected one of: libsandbox, nix, host-native, srt, oci, libkrun."
)]
pub struct SandboxBackendParseError(String);

impl FromStr for SandboxBackend {
    type Err = SandboxBackendParseError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "libsandbox" => Ok(SandboxBackend::Libsandbox),
            "nix" => Ok(SandboxBackend::Nix),
            "host-native" => Ok(SandboxBackend::HostNative),
            "srt" => Ok(SandboxBackend::Srt),
            "oci" => Ok(SandboxBackend::Oci),
            "libkrun" => Ok(SandboxBackend::Libkrun),
            other => Err(SandboxBackendParseError(other.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_libsandbox() {
        assert_eq!(SandboxBackend::default(), SandboxBackend::Libsandbox);
    }

    #[test]
    fn display_and_from_str_round_trip() {
        for backend in SandboxBackend::ALL {
            assert_eq!(
                backend.to_string().parse::<SandboxBackend>().unwrap(),
                backend
            );
        }
    }

    #[test]
    fn from_str_rejects_unknown_value() {
        let err = "bogus".parse::<SandboxBackend>().unwrap_err();
        assert_eq!(
            err.to_string(),
            "'bogus' is not a valid sandbox backend. Expected one of: libsandbox, nix, host-native, srt, oci, libkrun.",
        );
    }

    #[test]
    fn serde_round_trips_kebab_case() {
        let cases = [
            (SandboxBackend::Libsandbox, "\"libsandbox\""),
            (SandboxBackend::HostNative, "\"host-native\""),
            (SandboxBackend::Libkrun, "\"libkrun\""),
        ];
        for (backend, json) in cases {
            assert_eq!(serde_json::to_string(&backend).unwrap(), json);
            assert_eq!(
                serde_json::from_str::<SandboxBackend>(json).unwrap(),
                backend
            );
        }
    }

    #[test]
    fn implemented_backends_are_libsandbox_host_native_and_srt() {
        let implemented: Vec<SandboxBackend> = SandboxBackend::ALL
            .into_iter()
            .filter(|b| b.capabilities().status == IntegrationStatus::Implemented)
            .collect();
        assert_eq!(implemented, vec![
            SandboxBackend::Libsandbox,
            SandboxBackend::HostNative,
            SandboxBackend::Srt,
        ]);
    }

    #[test]
    fn libsandbox_is_advisory_and_does_not_enforce() {
        let caps = SandboxBackend::Libsandbox.capabilities();
        assert_eq!(caps, BackendCapabilities {
            backend: SandboxBackend::Libsandbox,
            enforcement: Enforcement::Advisory,
            enforces: false,
            live_ask: true,
            domain_egress: false,
            per_op: false,
            fs_virtualized: false,
            macos: PlatformSupport::Native,
            linux: PlatformSupport::Native,
            status: IntegrationStatus::Implemented,
        });
    }

    #[test]
    fn every_backend_declares_its_own_capabilities() {
        // The capability row always reports the backend it belongs to.
        for backend in SandboxBackend::ALL {
            assert_eq!(backend.capabilities().backend, backend);
        }
    }

    #[test]
    fn only_advisory_backend_fails_to_enforce() {
        for backend in SandboxBackend::ALL {
            let caps = backend.capabilities();
            assert_eq!(
                caps.enforces,
                caps.enforcement != Enforcement::Advisory,
                "{backend} enforces/advisory mismatch",
            );
        }
    }
}
