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
    /// NVIDIA OpenShell: bakes the environment into a Docker-resident OCI image
    /// and launches it via `openshell sandbox create`, which connects through a
    /// local OpenShell gateway. Provides native L7 domain-egress policy via
    /// OpenShell's egress proxy. Linux-native; macOS requires a Linux VM
    /// (provided by the gateway's Docker compute driver).
    Openshell,
    /// Modal Sandboxes: bakes the environment into a lockfile-hash-tagged OCI
    /// image and launches a remote `modal.Sandbox` from it via the Modal Python
    /// SDK. Cloud-remote — nothing runs on the host, so host-filesystem
    /// assertions are preflight-only and the threat model inverts (host fs is
    /// unreachable, but code and secrets leave the laptop). Egress is
    /// deny-by-default with a native domain allowlist (TLS/443 only) plus
    /// CIDR ranges; policy is fixed at sandbox creation, so redemption is
    /// recreation. Reachable identically from macOS and Linux via the API.
    Modal,
    /// Docker Sandboxes (`sbx`): bakes the environment into a lockfile-hash-tagged
    /// OCI image and hands it to Docker's `sbx` CLI as a `kind: sandbox` kit's
    /// base image, launched into a local microVM (private in-VM dockerd, own
    /// filesystem and network). Egress is deny-by-default through the host-side
    /// HTTP/HTTPS proxy: manifest grants compile into the kit's `allowedDomains`
    /// (domain/wildcard, HTTP/HTTPS only), a declared lossiness — non-HTTP or
    /// non-standard-port endpoints cannot be expressed as domain rules and are
    /// declined rather than silently widened. Credentials are injected as
    /// sentinel values by the proxy (the real value never enters the microVM).
    /// Policy is declared before the run and applied at creation, so there is no
    /// per-request ask API — redemption is a restart. The microVM runs Linux, so
    /// macOS drives it via the sbx hypervisor rather than running the workload on
    /// the host.
    DockerSbx,
    /// E2B: a cloud-API code-execution sandbox provider. flox bakes the
    /// environment into a lockfile-hash-tagged OCI image, then generates the
    /// E2B template hand-off — an `e2b.Dockerfile` whose `FROM` is the baked
    /// image plus an `e2b.toml` template config — that `e2b template build`
    /// turns into a sandbox template; a sandbox launched from that template
    /// (SDK/CLI) runs the locked toolchain. Cloud-remote: nothing runs on the
    /// host, so host assertions are preflight-only and the threat model inverts
    /// (host fs unreachable, code and secrets leave the laptop). Egress is
    /// default-OPEN on E2B (`allowInternetAccess=true`), so flox's policy
    /// compile ALWAYS emits an explicit deny posture and only then adds the
    /// manifest's `:443`/`:80` host allowlist; E2B's host/SNI filtering covers
    /// ports 80/443 only and does not filter QUIC/UDP, a declared lossiness.
    /// Policy is applied at sandbox creation, but E2B exposes a live
    /// `updateNetwork` (replace-not-merge) on a running sandbox — the one true
    /// live network-grant redemption in the cloud tier — which is an
    /// operator-initiated policy replacement, not a per-request ask, so
    /// `live_ask` stays false. Reached over the API identically from macOS and
    /// Linux.
    E2b,
    /// Ona (formerly Gitpod): a control-plane / gateway CDE. flox bakes the
    /// environment into a lockfile-hash-tagged OCI image and generates the
    /// devcontainer hand-off artifact (`.devcontainer/devcontainer.json`) that
    /// wraps the baked image; an Ona workspace built from that image opens with
    /// the locked toolchain already present. Enforcement (workspace isolation,
    /// network policy) stays entirely on Ona's side — no local enforcement ever
    /// runs on the laptop, so host assertions are preflight-only and the threat
    /// model inverts (host fs is unreachable, code and secrets leave the
    /// laptop). Manifest network grants compile into the devcontainer as
    /// `containerEnv` proxy hints plus documented policy expectations; the
    /// grant's per-binary / read-write scoping is recorded but not enforceable
    /// through the devcontainer contract, a declared lossiness. Ona is an
    /// enterprise product reached over its API from any host, so `Native` here
    /// means "the host can drive it", not "the workload runs on this OS".
    Ona,
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
    pub const ALL: [SandboxBackend; 11] = [
        SandboxBackend::Libsandbox,
        SandboxBackend::Nix,
        SandboxBackend::HostNative,
        SandboxBackend::Srt,
        SandboxBackend::Oci,
        SandboxBackend::Openshell,
        SandboxBackend::Modal,
        SandboxBackend::DockerSbx,
        SandboxBackend::Ona,
        SandboxBackend::E2b,
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
                status: Implemented,
            },
            SandboxBackend::Openshell => BackendCapabilities {
                backend: self,
                enforcement: Container,
                enforces: true,
                live_ask: false,
                // OpenShell's egress proxy provides native L7 domain-egress
                // policy; no additional flox-side proxy is required.
                domain_egress: true,
                per_op: true,
                fs_virtualized: true,
                // macOS users land in a Linux guest via the gateway's Docker
                // compute driver (same DX as the `oci` backend).
                macos: ViaLinuxVm,
                linux: Native,
                status: Implemented,
            },
            SandboxBackend::Modal => BackendCapabilities {
                backend: self,
                enforcement: Container,
                enforces: true,
                // Policy (network allowlists, mounts) is fixed at
                // `Sandbox.create`; an out-of-policy access is redeemed only by
                // recreating the sandbox with a wider policy, never live.
                live_ask: false,
                // `outbound_domain_allowlist` is a native domain allowlist, but
                // it governs TLS/443 only — non-443 and non-TLS egress falls to
                // the CIDR allowlist. That 443-only ceiling is a declared
                // lossiness, not a reason to claim no domain egress.
                domain_egress: true,
                // The allowlists are endpoint-scoped (domain / CIDR); they carry
                // no read/write method distinction and no per-binary attribution,
                // and the remote filesystem is the baked image plus mounts rather
                // than per-path r/w/x grants. Op-blind by the contract's meaning.
                per_op: false,
                // The workload runs in a remote Modal VM, so it pays the
                // virtualized-filesystem cost class rather than native host I/O.
                fs_virtualized: true,
                // Cloud-remote: the sandbox is reached over the Modal API, so the
                // launch path is identical from macOS and Linux — nothing runs on
                // the host. `Native` here means "the host can drive it", not
                // "the workload runs on this OS".
                macos: Native,
                linux: Native,
                // The launch boundary needs a Modal account/token and an OCI
                // image pushed to a registry Modal can pull (the SDK ingests
                // images by registry ref, not from a local Docker daemon).
                // Preflight, bake, policy compilation, and launcher-artifact
                // generation are wired; the final remote launch is not, so this
                // is honestly Scaffolded, not Implemented.
                status: Scaffolded,
            },
            SandboxBackend::DockerSbx => BackendCapabilities {
                backend: self,
                // A per-sandbox microVM with a private in-VM dockerd. The
                // boundary is hypervisor-class, but the policy surface (a
                // host-side egress proxy plus a workspace bind mount) matches
                // the other container backends, so it clusters with them here.
                enforcement: Container,
                enforces: true,
                // Network and filesystem policy is chosen before the run and
                // applied at sandbox creation; an out-of-policy access is
                // redeemed only by editing the policy and restarting, never
                // adjudicated live. No per-request ask API exists.
                live_ask: false,
                // The host-side proxy governs egress by domain (`allowedDomains`
                // supports wildcards), but only for HTTP/HTTPS. Non-HTTP TCP is
                // reachable solely by IP:port rules and UDP/ICMP are blocked
                // outright — that HTTP-only ceiling is a declared lossiness, not
                // a reason to claim no domain egress.
                domain_egress: true,
                // The allowlist is endpoint-scoped (domain / IP:port); it carries
                // no read/write method distinction and no per-binary attribution,
                // and the guest filesystem is the baked image plus the workspace
                // mount rather than per-path r/w/x grants. Op-blind by the
                // contract's meaning.
                per_op: false,
                // The workload runs inside a Linux microVM, so it pays the
                // virtualized-filesystem cost class rather than native host I/O.
                fs_virtualized: true,
                // Local microVM reached through the `sbx` CLI: the launch path is
                // identical from macOS and Linux (the CLI drives its own
                // hypervisor). `Native` here means "the host can drive it", not
                // "the workload runs on this OS".
                macos: Native,
                linux: Native,
                // Preflight, bake, policy compilation, and kit-manifest
                // generation are wired, but the final `sbx run` needs the `sbx`
                // CLI (absent on this host) and a baked image that satisfies sbx's
                // base-image contract (non-root `agent` user at uid 1000,
                // passwordless sudo, preserved proxy env) — which the flox bake
                // does not yet produce. Honestly Scaffolded, not Implemented.
                status: Scaffolded,
            },
            SandboxBackend::Ona => BackendCapabilities {
                backend: self,
                // Ona runs the workload inside a control-plane-managed CDE
                // (dev-container/microVM class); the boundary is Ona's, the
                // policy surface (a devcontainer image plus a workspace network
                // policy) clusters with the other container-class backends.
                enforcement: Container,
                enforces: true,
                // Workspace policy is fixed when the environment is created from
                // the devcontainer/image; an out-of-policy access is redeemed by
                // recreating the workspace with a wider policy, never live.
                live_ask: false,
                // Ona governs egress by domain at its gateway/control plane, but
                // the devcontainer hand-off flox produces expresses grants only
                // as documented policy expectations and proxy hints — the
                // enterprise network policy that actually enforces them is
                // configured on Ona's side. Declared as domain-capable with that
                // lossiness noted in the module docs.
                domain_egress: true,
                // The devcontainer/workspace policy is endpoint-scoped; it
                // carries no read/write method distinction and no per-binary
                // attribution, and the workspace filesystem is the baked image
                // plus the cloned repo rather than per-path r/w/x grants.
                // Op-blind by the contract's meaning.
                per_op: false,
                // The workload runs in a remote Ona CDE, so it pays the
                // virtualized-filesystem cost class rather than native host I/O.
                fs_virtualized: true,
                // Control-plane/cloud: the workspace is reached over Ona's API,
                // so the hand-off path is identical from macOS and Linux —
                // nothing runs on the host. `Native` here means "the host can
                // drive it", not "the workload runs on this OS".
                macos: Native,
                linux: Native,
                // The launch boundary needs an Ona account and (post-OpenAI
                // acquisition) an enterprise workspace/partnership: Ona builds
                // the workspace from a devcontainer in a git repo, and there is
                // no public no-account image-launch API on this host. Preflight,
                // bake, policy compilation, and devcontainer-artifact generation
                // are wired; the final workspace open is not, so this is
                // honestly Scaffolded, not Implemented.
                status: Scaffolded,
            },
            SandboxBackend::E2b => BackendCapabilities {
                backend: self,
                // E2B runs the workload in a cloud microVM (Firecracker-class);
                // the boundary is E2B's, and the policy surface (a template
                // image plus a sandbox network config) clusters with the other
                // container-class cloud backends.
                enforcement: Container,
                enforces: true,
                // Network policy is applied at sandbox creation. E2B does
                // expose a live `updateNetwork` (replace-not-merge) on a
                // running sandbox — a genuine live network-grant redemption —
                // but that is an operator-initiated policy replacement, not a
                // per-request adjudication of a specific out-of-policy access.
                // The contract's `live_ask` means the latter, so it stays
                // false; the live-update capability is documented in the module
                // docs as the one true live network redemption in the cloud
                // tier.
                live_ask: false,
                // E2B filters egress by host/SNI, but only on ports 80 and 443;
                // QUIC/UDP is not filtered. That HTTP(S)-only ceiling is a
                // declared lossiness, not a reason to claim no domain egress.
                // Critically, E2B's default is `allowInternetAccess=true`, so
                // flox always compiles an explicit deny posture first and adds
                // only the manifest's allowlist on top.
                domain_egress: true,
                // The sandbox network config is endpoint-scoped (host
                // allowlist); it carries no read/write method distinction and
                // no per-binary attribution, and the guest filesystem is the
                // baked template image rather than per-path r/w/x grants.
                // Op-blind by the contract's meaning.
                per_op: false,
                // The workload runs in a remote E2B microVM, so it pays the
                // virtualized-filesystem cost class rather than native host I/O.
                fs_virtualized: true,
                // Cloud-remote: the sandbox is reached over the E2B API, so the
                // launch path is identical from macOS and Linux — nothing runs
                // on the host. `Native` here means "the host can drive it", not
                // "the workload runs on this OS".
                macos: Native,
                linux: Native,
                // The launch boundary needs an E2B account/API key and a
                // template built from the baked image (E2B ingests images as
                // the `FROM` base of an `e2b.Dockerfile`, built via
                // `e2b template build` against E2B's builder). Preflight, bake,
                // policy compilation, and template-artifact generation are
                // wired; the final template build and remote launch are not, so
                // this is honestly Scaffolded, not Implemented.
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
            SandboxBackend::Openshell => "openshell",
            SandboxBackend::Modal => "modal",
            SandboxBackend::DockerSbx => "docker-sbx",
            SandboxBackend::Ona => "ona",
            SandboxBackend::E2b => "e2b",
            SandboxBackend::Libkrun => "libkrun",
        };
        write!(f, "{name}")
    }
}

#[derive(Debug, thiserror::Error)]
#[error(
    "'{0}' is not a valid sandbox backend. Expected one of: libsandbox, nix, host-native, srt, oci, openshell, modal, docker-sbx, ona, e2b, libkrun."
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
            "openshell" => Ok(SandboxBackend::Openshell),
            "modal" => Ok(SandboxBackend::Modal),
            "docker-sbx" => Ok(SandboxBackend::DockerSbx),
            "ona" => Ok(SandboxBackend::Ona),
            "e2b" => Ok(SandboxBackend::E2b),
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
            "'bogus' is not a valid sandbox backend. Expected one of: libsandbox, nix, host-native, srt, oci, openshell, modal, docker-sbx, ona, e2b, libkrun.",
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
    fn implemented_backends_are_libsandbox_host_native_srt_oci_and_openshell() {
        let implemented: Vec<SandboxBackend> = SandboxBackend::ALL
            .into_iter()
            .filter(|b| b.capabilities().status == IntegrationStatus::Implemented)
            .collect();
        assert_eq!(implemented, vec![
            SandboxBackend::Libsandbox,
            SandboxBackend::HostNative,
            SandboxBackend::Srt,
            SandboxBackend::Oci,
            SandboxBackend::Openshell,
        ]);
    }

    #[test]
    fn openshell_capabilities_row() {
        let caps = SandboxBackend::Openshell.capabilities();
        assert_eq!(caps, BackendCapabilities {
            backend: SandboxBackend::Openshell,
            enforcement: Enforcement::Container,
            enforces: true,
            live_ask: false,
            domain_egress: true,
            per_op: true,
            fs_virtualized: true,
            macos: PlatformSupport::ViaLinuxVm,
            linux: PlatformSupport::Native,
            status: IntegrationStatus::Implemented,
        });
    }

    #[test]
    fn openshell_display_and_parse_round_trip() {
        let backend = SandboxBackend::Openshell;
        let s = backend.to_string();
        assert_eq!(s, "openshell");
        assert_eq!(s.parse::<SandboxBackend>().unwrap(), backend);
    }

    #[test]
    fn openshell_serde_round_trip() {
        let json = serde_json::to_string(&SandboxBackend::Openshell).unwrap();
        assert_eq!(json, "\"openshell\"");
        let parsed: SandboxBackend = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SandboxBackend::Openshell);
    }

    #[test]
    fn modal_capabilities_row() {
        let caps = SandboxBackend::Modal.capabilities();
        assert_eq!(caps, BackendCapabilities {
            backend: SandboxBackend::Modal,
            enforcement: Enforcement::Container,
            enforces: true,
            live_ask: false,
            domain_egress: true,
            per_op: false,
            fs_virtualized: true,
            macos: PlatformSupport::Native,
            linux: PlatformSupport::Native,
            status: IntegrationStatus::Scaffolded,
        });
    }

    #[test]
    fn modal_display_and_parse_round_trip() {
        let backend = SandboxBackend::Modal;
        let s = backend.to_string();
        assert_eq!(s, "modal");
        assert_eq!(s.parse::<SandboxBackend>().unwrap(), backend);
    }

    #[test]
    fn modal_serde_round_trip() {
        let json = serde_json::to_string(&SandboxBackend::Modal).unwrap();
        assert_eq!(json, "\"modal\"");
        let parsed: SandboxBackend = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SandboxBackend::Modal);
    }

    #[test]
    fn docker_sbx_capabilities_row() {
        let caps = SandboxBackend::DockerSbx.capabilities();
        assert_eq!(caps, BackendCapabilities {
            backend: SandboxBackend::DockerSbx,
            enforcement: Enforcement::Container,
            enforces: true,
            live_ask: false,
            domain_egress: true,
            per_op: false,
            fs_virtualized: true,
            macos: PlatformSupport::Native,
            linux: PlatformSupport::Native,
            status: IntegrationStatus::Scaffolded,
        });
    }

    #[test]
    fn docker_sbx_display_and_parse_round_trip() {
        let backend = SandboxBackend::DockerSbx;
        let s = backend.to_string();
        assert_eq!(s, "docker-sbx");
        assert_eq!(s.parse::<SandboxBackend>().unwrap(), backend);
    }

    #[test]
    fn docker_sbx_serde_round_trip() {
        let json = serde_json::to_string(&SandboxBackend::DockerSbx).unwrap();
        assert_eq!(json, "\"docker-sbx\"");
        let parsed: SandboxBackend = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SandboxBackend::DockerSbx);
    }

    #[test]
    fn ona_capabilities_row() {
        let caps = SandboxBackend::Ona.capabilities();
        assert_eq!(caps, BackendCapabilities {
            backend: SandboxBackend::Ona,
            enforcement: Enforcement::Container,
            enforces: true,
            live_ask: false,
            domain_egress: true,
            per_op: false,
            fs_virtualized: true,
            macos: PlatformSupport::Native,
            linux: PlatformSupport::Native,
            status: IntegrationStatus::Scaffolded,
        });
    }

    #[test]
    fn ona_display_and_parse_round_trip() {
        let backend = SandboxBackend::Ona;
        let s = backend.to_string();
        assert_eq!(s, "ona");
        assert_eq!(s.parse::<SandboxBackend>().unwrap(), backend);
    }

    #[test]
    fn ona_serde_round_trip() {
        let json = serde_json::to_string(&SandboxBackend::Ona).unwrap();
        assert_eq!(json, "\"ona\"");
        let parsed: SandboxBackend = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SandboxBackend::Ona);
    }

    #[test]
    fn e2b_capabilities_row() {
        let caps = SandboxBackend::E2b.capabilities();
        assert_eq!(caps, BackendCapabilities {
            backend: SandboxBackend::E2b,
            enforcement: Enforcement::Container,
            enforces: true,
            live_ask: false,
            domain_egress: true,
            per_op: false,
            fs_virtualized: true,
            macos: PlatformSupport::Native,
            linux: PlatformSupport::Native,
            status: IntegrationStatus::Scaffolded,
        });
    }

    #[test]
    fn e2b_display_and_parse_round_trip() {
        let backend = SandboxBackend::E2b;
        let s = backend.to_string();
        assert_eq!(s, "e2b");
        assert_eq!(s.parse::<SandboxBackend>().unwrap(), backend);
    }

    #[test]
    fn e2b_serde_round_trip() {
        let json = serde_json::to_string(&SandboxBackend::E2b).unwrap();
        assert_eq!(json, "\"e2b\"");
        let parsed: SandboxBackend = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SandboxBackend::E2b);
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
