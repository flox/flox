//! The `docker-sbx` sandbox backend: run the environment in a local Docker
//! Sandboxes (`sbx`) microVM.
//!
//! Docker Sandboxes is a local-runtime provider that launches each sandbox as a
//! Linux microVM with a private in-VM dockerd, its own filesystem, and its own
//! network. Like `openshell` and `modal`, flox bakes the environment's OCI image
//! via the shared lockfile-hash-tagged Docker bake (`super::bake`), under this
//! backend's own `<env>-docker-sbx:<hash12>` tag namespace. The launch adapter
//! is closer to `openshell` than plain `oci`: the image is handed to `sbx` as a
//! `kind: sandbox` *kit* (its `sandbox.image`), and the network policy is
//! declared in the kit before the run rather than adjudicated live.
//!
//! # Why this backend does not complete the launch on any host today
//!
//! Two external prerequisites gate the microVM launch, and neither can be
//! satisfied from a bare checkout:
//!
//! - **The `sbx` CLI.** Docker Sandboxes ships as a standalone `sbx` binary
//!   (installed via `brew install docker/tap/sbx`, `winget install Docker.sbx`,
//!   or the Ubuntu `docker-sbx` package) that drives its own hypervisor. It is
//!   not the same surface as the classic `docker` daemon; `preflight`
//!   distinguishes *docker-missing* / *daemon-down* / *`sbx`-absent* /
//!   *`sbx`-too-old* so the failure names the actual gap. On hosts where `sbx`
//!   is only reachable as the `docker sbx` subcommand, that path requires Docker
//!   Desktop 4.60 or newer.
//! - **A base image that satisfies sbx's kit contract.** A `kind: sandbox` kit's
//!   base image must provide a non-root `agent` user at uid 1000 with
//!   passwordless sudo, a `/home/agent` home directory, and the HTTP proxy
//!   environment variables (`HTTP_PROXY`/`HTTPS_PROXY`/`NO_PROXY`) preserved
//!   across sudo. The flox bake (the OpenShell compat layer) adds a `sandbox`
//!   user, not sbx's `agent` uid-1000 user, so the baked image is not yet a
//!   drop-in sbx base image.
//!
//! Rather than fake success, this backend implements the deepest honest slice:
//! it runs the real preflight, bakes the real image, compiles the manifest
//! network policy into sbx's kit egress vocabulary, and *generates the kit
//! manifest* (`spec.yaml`) that references the baked image and carries the
//! compiled allow rules. It then fails at the launch boundary with a clear
//! "requires ..." error that points at the generated artifact and names the two
//! missing prerequisites.
//!
//! # Network-policy compilation (the load-bearing lossiness)
//!
//! sbx governs egress through a host-side HTTP/HTTPS proxy. A `kind: sandbox`
//! kit expresses network policy as:
//! - `network.allowedDomains: [domain, …]` — domains the microVM can reach over
//!   HTTP/HTTPS (wildcards like `*.github.com` supported).
//! - `network.deniedDomains: [domain, …]` — domains blocked outright (deny wins
//!   over allow).
//!
//! Only HTTP/HTTPS is proxy-governed: non-HTTP TCP is reachable solely via
//! IP:port rules added through `sbx policy`, and UDP/ICMP are blocked at the
//! network layer and cannot be unblocked. So a `[[options.sandbox.network]]`
//! grant compiles as follows: a `<host>:443` or `<host>:80` endpoint becomes an
//! `allowedDomains` entry (native, faithful); any other port cannot be expressed
//! as a domain rule and is declined with a clear error rather than silently
//! widened to all-ports or dropped. sbx's allowlist carries no read/write method
//! distinction and no per-binary attribution, so the `access`, `protocol`, and
//! `binary` fields of a grant are recorded in the generated kit as comments but
//! do not constrain traffic — declared lossiness, per the backend contract.
//!
//! # Credential injection
//!
//! sbx injects credentials as *sentinel values*: the host-side proxy overwrites
//! the auth header on outbound requests and the microVM sees only a placeholder
//! (e.g. `proxy-managed`). Agent secrets therefore go through `sbx secret set`
//! and the kit's `serviceAuth`/`proxyManaged` fields, never a local `.env` baked
//! into the image. The generated kit leaves those fields for the operator to
//! fill in.
//!
//! # Env knobs (prototype)
//!
//! The bake reuses the openshell/oci knobs:
//! - `FLOX_SANDBOX_OCI_IMAGE` — explicit image ref override (skips bake).
//! - `FLOX_SANDBOX_OCI_ALLOW_STALE` — run the newest existing image when the
//!   expected hash-tag is absent.
//! - `FLOX_SANDBOX_OCI_AUTOBAKE` — bake without prompting.

use std::convert::Infallible;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flox_core::activate::context::InvocationType;
use flox_core::activate::sandbox_backend::SandboxBackend;
use flox_core::activate::sandbox_policy::SandboxNetworkRule;
use flox_manifest::lockfile::Lockfile;
use flox_rust_sdk::providers::container_builder::ContainerBuilderParams;
use semver::Version;
use tracing::debug;

use super::handoff::{ensure_local_image, manifest_network_rules};
use super::preflight::{
    CliVersionCheck,
    binary_on_path,
    check_cli_version,
    first_on_path,
    split_endpoint,
};
use super::{ActivationSandbox, SandboxLaunchCtx};
use crate::commands::sandbox_backends::oci::lockfile_hash12;

/// Repository suffix for the Docker Sandboxes backend's image tags. The image is
/// baked under `<env>-docker-sbx:<hash12>` (with the shared compat layer) so the
/// artifact is recognizable as this backend's and never collides with the other
/// Docker-ingesting backends' tags on a shared store.
const DOCKER_SBX_REPO_SUFFIX: &str = "-docker-sbx";

/// Minimum supported `sbx` CLI version.
///
/// The `kind: sandbox` kit shape this backend generates (the `sandbox:` block
/// with `image`, and the renamed `agentContext`/`sandbox` fields) landed in the
/// 0.32.0 kit-spec revision; an older CLI rejects the manifest. Pinned to that
/// floor; the prototype targets the current release line.
const DOCKER_SBX_MIN_VERSION: Version = Version::new(0, 32, 0);

/// Ports the host-side HTTP/HTTPS proxy governs by domain. A grant on any other
/// port cannot be expressed as an `allowedDomains` rule and is declined.
const PROXY_HTTP_PORTS: [u16; 2] = [80, 443];

pub struct DockerSbxBackend<'a> {
    dot_flox_path: PathBuf,
    env_name: String,
    invocation_type: &'a InvocationType,
    lockfile: &'a Lockfile,
    sandbox_oci_autobake: bool,
    container_builder_params: ContainerBuilderParams,
}

impl<'a> DockerSbxBackend<'a> {
    pub fn new(ctx: SandboxLaunchCtx<'a>) -> Self {
        Self {
            dot_flox_path: ctx.dot_flox_path,
            env_name: ctx.env_name,
            invocation_type: ctx.invocation_type,
            lockfile: ctx.lockfile,
            sandbox_oci_autobake: ctx.sandbox_oci_autobake,
            container_builder_params: ctx.container_builder_params,
        }
    }
}

impl ActivationSandbox for DockerSbxBackend<'_> {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::DockerSbx
    }

    fn preflight(&self) -> Result<()> {
        // The `sbx` CLI drives its own hypervisor; the classic `docker` daemon
        // is a separate surface. Distinguish the failure modes so the message
        // names the actual gap rather than a generic "not found".
        let Some(sbx_path) = first_on_path("sbx") else {
            // `sbx` absent: is docker even installed? If not, that is the first
            // thing to fix; if docker is present but its daemon is down, say so;
            // otherwise point at the `sbx` install (and the `docker sbx`
            // subcommand's Desktop >= 4.60 requirement).
            if !binary_on_path("docker") {
                bail!(
                    "The 'docker-sbx' sandbox backend requires the Docker Sandboxes 'sbx' CLI, and Docker itself was not found on PATH.\n\
                     Install Docker, then install the 'sbx' CLI with 'brew install docker/tap/sbx' (macOS), 'winget install Docker.sbx' (Windows), or the 'docker-sbx' package (Ubuntu), and run 'sbx login'."
                );
            }
            if !docker_daemon_up() {
                bail!(
                    "The 'docker-sbx' sandbox backend requires the Docker Sandboxes 'sbx' CLI, which was not found on PATH, and the Docker daemon is not reachable ('docker info' failed).\n\
                     Start Docker (open Docker Desktop or start the Docker service), install the 'sbx' CLI with 'brew install docker/tap/sbx', then run 'sbx login'."
                );
            }
            bail!(
                "The 'docker-sbx' sandbox backend requires the Docker Sandboxes 'sbx' CLI, which was not found on PATH.\n\
                 Install it with 'brew install docker/tap/sbx' (macOS), 'winget install Docker.sbx' (Windows), or the 'docker-sbx' package (Ubuntu), then run 'sbx login'.\n\
                 If you use the bundled 'docker sbx' subcommand instead, it requires Docker Desktop 4.60 or newer."
            );
        };
        check_docker_sbx_version(&sbx_path)?;
        // A running Docker daemon is still needed to bake and manage the image;
        // the microVM launch is via `sbx`, but the bake loads into Docker's store.
        if !docker_daemon_up() {
            bail!(
                "The 'docker-sbx' sandbox backend needs a running Docker daemon to bake and load the environment image, but 'docker info' failed.\n\
                 Start Docker (open Docker Desktop or start the Docker service), then re-run."
            );
        }
        Ok(())
    }

    fn wrap_activation(self: Box<Self>) -> Result<Infallible> {
        wrap_docker_sbx(
            &self.dot_flox_path,
            &self.env_name,
            self.invocation_type,
            self.lockfile,
            self.sandbox_oci_autobake,
            &self.container_builder_params,
        )
    }
}

/// Verify the resolved `sbx` CLI meets [`DOCKER_SBX_MIN_VERSION`].
///
/// Unlike the openshell/modal CLIs, `sbx` has no `--version` flag: the version
/// is exposed via the `version` subcommand (output shape
/// `sbx version: v0.34.0 <sha>`). The shared version gate takes the query
/// arguments as a field, so `version_args: &["version"]` drives it through the
/// subcommand while reusing the shared parser (which strips the leading `v` and
/// the trailing sha) and the tolerate-unparseable-at-debug semantics. A too-old
/// CLI would reject the generated `kind: sandbox` kit's `sandbox:` block with a
/// schema error.
fn check_docker_sbx_version(sbx_path: &Path) -> Result<()> {
    check_cli_version(sbx_path, &CliVersionCheck {
        tool_name: "Docker Sandboxes (sbx)",
        backend_id: "docker-sbx",
        min_version: DOCKER_SBX_MIN_VERSION,
        upgrade_hint: "Upgrade with 'brew upgrade sbx' (macOS) or your platform's package manager, then re-run.",
        version_args: &["version"],
    })
}

/// Probe whether the Docker daemon is reachable via `docker info`.
///
/// Distinct from `sbx`: baking and loading the image still go through the
/// classic Docker daemon, so a reachable daemon is a separate prerequisite from
/// the `sbx` CLI.
fn docker_daemon_up() -> bool {
    std::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Return the `<env>-docker-sbx` repository name used for Docker image tagging.
fn docker_sbx_repo(env_name: &str) -> String {
    format!("{env_name}{DOCKER_SBX_REPO_SUFFIX}")
}

/// Generate an `sbx` kit name from the environment name.
///
/// Kit names must be lowercase, alphanumeric, and hyphens only. This lowercases
/// the env name, replaces any other character with a hyphen, collapses runs of
/// hyphens, trims leading/trailing hyphens, and prefixes `flox-` so the kit is
/// recognizable. A fully-stripped name falls back to `flox-env`.
pub(crate) fn docker_sbx_kit_name(env_name: &str) -> String {
    let mut out = String::with_capacity(env_name.len() + 5);
    out.push_str("flox-");
    let mut prev_dash = false;
    for c in env_name.to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_end_matches('-');
    if trimmed == "flox" {
        "flox-env".to_string()
    } else {
        trimmed.to_string()
    }
}

// ── Network policy compilation ─────────────────────────────────────────────────

/// The manifest network policy compiled into sbx's kit egress vocabulary.
///
/// sbx governs egress through a host-side HTTP/HTTPS proxy, expressed in a
/// `kind: sandbox` kit as `network.allowedDomains` (and `deniedDomains`, which
/// this backend leaves empty — deny-by-default is the absence of an allow rule,
/// not an explicit deny list). `grants.toml`-style endpoints compile onto the
/// allow list when they target an HTTP/HTTPS port; everything else is declined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DockerSbxNetworkPolicy {
    /// Hosts granted HTTP/HTTPS egress, in declaration order (deduplicated).
    /// Empty means no allow rules, i.e. deny-by-default egress.
    pub allowed_domains: Vec<String>,
}

/// Compile the manifest's `[[options.sandbox.network]]` rules into sbx's kit
/// egress vocabulary.
///
/// - No rules → an empty allow list (deny-by-default; the microVM reaches
///   nothing outbound).
/// - A `<host>:80` or `<host>:443` rule → an `allowedDomains` entry (native,
///   faithful; wildcards preserved).
/// - Any other port → a hard error: sbx's proxy governs egress by domain over
///   HTTP/HTTPS only, and silently promoting the grant to an all-ports rule (or
///   dropping it) would violate the "never silently widen or narrow grants"
///   contract. A non-HTTP endpoint must be granted via `sbx policy allow network
///   "<ip>:<port>"` instead.
pub(crate) fn compile_docker_sbx_network_policy(
    rules: &[SandboxNetworkRule],
) -> Result<DockerSbxNetworkPolicy> {
    let mut domains: Vec<String> = Vec::with_capacity(rules.len());
    for rule in rules {
        let (host, port) = split_endpoint(&rule.endpoint)?;
        if !PROXY_HTTP_PORTS.contains(&port) {
            bail!(
                "The 'docker-sbx' sandbox backend can only grant HTTP/HTTPS endpoints via its domain allowlist, but rule '{endpoint}' targets port {port}.\n\
                 Docker Sandboxes governs egress by domain over ports 80 and 443 only; rewrite the endpoint as '{host}:443', or grant the non-HTTP endpoint after launch with 'sbx policy allow network \"<ip>:{port}\"'.",
                endpoint = rule.endpoint
            );
        }
        if !domains.contains(&host) {
            domains.push(host);
        }
    }
    Ok(DockerSbxNetworkPolicy {
        allowed_domains: domains,
    })
}

// ── Kit manifest generation ────────────────────────────────────────────────────

/// Inputs to [`render_docker_sbx_kit`].
///
/// Grouping the fields keeps the pure renderer's signature self-documenting.
pub(crate) struct KitParams<'a> {
    /// Kit name (`flox-<sanitized-env>`).
    pub kit_name: &'a str,
    /// Base image reference sbx pulls as the kit's `sandbox.image`.
    pub image_ref: &'a str,
    /// Compiled egress policy.
    pub network: &'a DockerSbxNetworkPolicy,
}

/// Render the `sbx` kit manifest (`spec.yaml`, a pure function, no I/O).
///
/// The emitted YAML declares a `kind: sandbox` kit that uses the baked image as
/// its base and carries the compiled `network.allowedDomains`. An operator whose
/// image satisfies sbx's base-image contract loads it with `sbx kit load` and
/// launches it with `sbx run --kit <name>`.
pub(crate) fn render_docker_sbx_kit(params: &KitParams<'_>) -> String {
    let network_block = if params.network.allowed_domains.is_empty() {
        // Deny-by-default: no allow rules. An explicit empty list documents the
        // posture and keeps the section present for the operator to extend.
        "network:\n  allowedDomains: []\n".to_string()
    } else {
        let mut block = String::from("network:\n  allowedDomains:\n");
        for domain in &params.network.allowed_domains {
            // Single-quoted: a wildcard host like `*.github.com` is an alias
            // token to a YAML parser when unquoted. The endpoint charset check
            // in `split_endpoint` guarantees no quote or newline can appear.
            block.push_str(&format!("    - '{domain}'\n"));
        }
        block
    };
    // kit_name and image_ref come from validated sources (sanitized name, repo +
    // hash12 tag), so single-quoted literals are injection-safe.
    indoc::formatdoc! {r#"
        # Generated by `flox activate --sandbox --sandbox-backend docker-sbx`.
        # This is a Docker Sandboxes (sbx) kit manifest. flox baked the
        # environment image locally and references it below as the kit's base
        # image. Load it with `sbx kit load <this dir>` and launch it with
        # `sbx run --kit {kit_name}`.
        #
        # Egress is deny-by-default. `network.allowedDomains` below is the
        # manifest's [[options.sandbox.network]] rules compiled to sbx's kit
        # vocabulary (domain allowlist over HTTP/HTTPS). access/protocol/binary
        # scoping from the manifest is NOT enforceable on sbx and is dropped here.
        #
        # Credentials are injected as sentinel values by the host-side proxy:
        # store agent secrets with `sbx secret set` and wire them via the
        # `credentials`/`network.serviceAuth`/`environment.proxyManaged` fields
        # rather than baking a .env into the image.
        #
        # NOTE: sbx's base-image contract requires a non-root `agent` user at
        # uid 1000 with passwordless sudo, a /home/agent home, and preserved
        # HTTP proxy env. The flox bake does not yet produce that shape; build on
        # `docker/sandbox-templates:shell-docker` or adapt the image before use.
        schemaVersion: "1"
        kind: sandbox
        name: {kit_name}
        displayName: {kit_name}
        description: Flox environment sandbox baked by flox activate --sandbox.
        {network_block}sandbox:
          image: '{image_ref}'
        "#,
        kit_name = params.kit_name,
        image_ref = params.image_ref,
        network_block = network_block,
    }
}

// ── Launch path ────────────────────────────────────────────────────────────────

/// Bake the image, compile the policy, generate the kit manifest, then fail at
/// the launch boundary — never fake the microVM launch.
fn wrap_docker_sbx(
    dot_flox_path: &Path,
    env_name: &str,
    _invocation: &InvocationType,
    lockfile: &Lockfile,
    autobake: bool,
    container_builder_params: &ContainerBuilderParams,
) -> Result<Infallible> {
    let repo = docker_sbx_repo(env_name);
    let hash12 = lockfile_hash12(lockfile);

    // Ensure the local hash-tagged image exists (baking with the shared compat
    // layer if absent). The sbx launch uses this image as the kit's base image;
    // baking it locally first is the same content-addressed step every
    // OCI-ingesting provider shares.
    ensure_local_image(
        &repo,
        env_name,
        dot_flox_path,
        lockfile,
        autobake,
        container_builder_params,
        "Docker Sandboxes image",
    )?;

    // Compile the manifest network policy into sbx's kit egress vocabulary.
    let rules = manifest_network_rules(lockfile)?;
    let network = compile_docker_sbx_network_policy(&rules)?;

    // Generate the kit manifest referencing the baked image.
    let image_ref = format!("{repo}:{hash12}");
    let kit_name = docker_sbx_kit_name(env_name);
    let kit = render_docker_sbx_kit(&KitParams {
        kit_name: &kit_name,
        image_ref: &image_ref,
        network: &network,
    });
    let (kit_dir, spec_path) = write_docker_sbx_kit(dot_flox_path, &kit)?;

    // Fail at the launch boundary with the two concrete prerequisites.
    bail!(
        "The 'docker-sbx' sandbox backend launches a local Docker Sandboxes microVM, which requires two \
         prerequisites this host cannot satisfy automatically:\n  \
         1. The 'sbx' CLI (install with 'brew install docker/tap/sbx' and run 'sbx login'); the \
         bundled 'docker sbx' subcommand instead needs Docker Desktop 4.60 or newer.\n  \
         2. A base image that satisfies sbx's kit contract (a non-root 'agent' user at uid 1000 with \
         passwordless sudo, a /home/agent home, and preserved HTTP proxy env). The flox bake adds a \
         'sandbox' user, not sbx's 'agent' user, so the baked image '{image_ref}' must be adapted \
         first (build on 'docker/sandbox-templates:shell-docker').\n\
         flox generated the kit manifest at:\n  {spec}\n\
         With 'sbx' installed and the image adapted, load it with 'sbx kit load {kit_dir}' and run it \
         with 'sbx run --kit {kit_name}'.",
        spec = spec_path.display(),
        kit_dir = kit_dir.display(),
    )
}

/// Write the generated kit under `.flox/cache/docker-sbx-kit/` and return the
/// kit directory and its `spec.yaml` path.
fn write_docker_sbx_kit(dot_flox_path: &Path, kit: &str) -> Result<(PathBuf, PathBuf)> {
    let kit_dir = dot_flox_path.join("cache").join("docker-sbx-kit");
    std::fs::create_dir_all(&kit_dir)
        .with_context(|| format!("failed to create kit dir '{}'", kit_dir.display()))?;
    let spec_path = kit_dir.join("spec.yaml");
    std::fs::write(&spec_path, kit)
        .with_context(|| format!("failed to write kit spec to '{}'", spec_path.display()))?;
    debug!(path = %spec_path.display(), "wrote docker-sbx kit manifest");
    Ok((kit_dir, spec_path))
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use flox_core::activate::sandbox_policy::SandboxNetworkAccess;

    use super::super::preflight::parse_cli_version;
    use super::*;

    // ── version parsing (sbx `version` subcommand, not `--version`) ───────────

    #[test]
    fn real_sbx_version_output_parses() {
        // Ground truth from `sbx version` on docker-sbx@0.34.0:
        //   "sbx version: v0.34.0 2eae0c4fc3894475da3318615f69783b0e7be747"
        // The shared parser must strip the leading `v`, ignore the label and the
        // trailing commit sha, and pick out the semver.
        let out = "sbx version: v0.34.0 2eae0c4fc3894475da3318615f69783b0e7be747";
        assert_eq!(parse_cli_version(out), Some(Version::new(0, 34, 0)));
    }

    #[test]
    fn real_sbx_version_meets_minimum() {
        // The version this backend was validated against must clear the floor.
        assert!(Version::new(0, 34, 0) >= DOCKER_SBX_MIN_VERSION);
        // A pre-kit-spec CLI must fall below it.
        assert!(Version::new(0, 20, 0) < DOCKER_SBX_MIN_VERSION);
    }

    // ── docker_sbx_kit_name ───────────────────────────────────────────────────

    #[test]
    fn kit_name_prefix_and_sanitization() {
        assert_eq!(docker_sbx_kit_name("MyEnv"), "flox-myenv");
        assert_eq!(docker_sbx_kit_name("my.env-v2 beta"), "flox-my-env-v2-beta");
    }

    #[test]
    fn kit_name_collapses_and_trims_dashes() {
        // Runs of non-alphanumeric characters collapse to a single dash, and a
        // trailing dash is trimmed so the name stays a valid kit identifier.
        assert_eq!(docker_sbx_kit_name("a..b__c--"), "flox-a-b-c");
        assert_eq!(docker_sbx_kit_name("env!!!"), "flox-env");
    }

    #[test]
    fn kit_name_all_special_falls_back() {
        // A name that strips to nothing must not yield the bare `flox-` prefix
        // (which trims to `flox`, an invalid empty-body kit name).
        assert_eq!(docker_sbx_kit_name("!!!"), "flox-env");
        assert_eq!(docker_sbx_kit_name(""), "flox-env");
    }

    // ── docker_sbx_repo ───────────────────────────────────────────────────────

    #[test]
    fn repo_has_docker_sbx_suffix() {
        assert_eq!(docker_sbx_repo("myenv"), "myenv-docker-sbx");
    }

    #[test]
    fn repo_never_collides_with_other_backends() {
        let env = "myenv";
        let hash = "abc123def456";
        let oci = format!("{env}:{hash}");
        let openshell = format!("{env}-openshell:{hash}");
        let modal = format!("{env}-modal:{hash}");
        let docker_sbx = format!("{}:{hash}", docker_sbx_repo(env));
        assert_ne!(docker_sbx, oci);
        assert_ne!(docker_sbx, openshell);
        assert_ne!(docker_sbx, modal);
    }

    // ── compile_docker_sbx_network_policy ──────────────────────────────────────

    fn rule(endpoint: &str) -> SandboxNetworkRule {
        SandboxNetworkRule {
            endpoint: endpoint.to_string(),
            access: None,
            protocol: None,
            binary: None,
        }
    }

    #[test]
    fn no_rules_compiles_to_empty_allowlist() {
        let policy = compile_docker_sbx_network_policy(&[]).unwrap();
        assert_eq!(policy, DockerSbxNetworkPolicy {
            allowed_domains: Vec::new(),
        });
    }

    #[test]
    fn https_rules_compile_to_allowed_domains() {
        let rules = [rule("api.github.com:443"), rule("api.anthropic.com:443")];
        let policy = compile_docker_sbx_network_policy(&rules).unwrap();
        assert_eq!(policy, DockerSbxNetworkPolicy {
            allowed_domains: vec![
                "api.github.com".to_string(),
                "api.anthropic.com".to_string(),
            ],
        });
    }

    #[test]
    fn http_port_80_is_allowed() {
        let policy = compile_docker_sbx_network_policy(&[rule("cache.example.com:80")]).unwrap();
        assert_eq!(policy.allowed_domains, vec![
            "cache.example.com".to_string()
        ]);
    }

    #[test]
    fn duplicate_hosts_are_deduplicated() {
        let rules = [rule("api.github.com:443"), rule("api.github.com:443")];
        let policy = compile_docker_sbx_network_policy(&rules).unwrap();
        assert_eq!(policy.allowed_domains, vec!["api.github.com".to_string()]);
    }

    #[test]
    fn wildcard_host_is_preserved() {
        let policy = compile_docker_sbx_network_policy(&[rule("*.github.com:443")]).unwrap();
        assert_eq!(policy.allowed_domains, vec!["*.github.com".to_string()]);
    }

    #[test]
    fn access_and_protocol_do_not_affect_compilation() {
        // sbx's allowlist carries no method distinction; a scoped grant compiles
        // identically to an unscoped one (declared lossiness).
        let scoped = SandboxNetworkRule {
            endpoint: "api.github.com:443".to_string(),
            access: Some(SandboxNetworkAccess::ReadOnly),
            protocol: None,
            binary: Some("curl".to_string()),
        };
        let policy = compile_docker_sbx_network_policy(&[scoped]).unwrap();
        assert_eq!(policy, DockerSbxNetworkPolicy {
            allowed_domains: vec!["api.github.com".to_string()],
        });
    }

    #[test]
    fn non_http_port_is_rejected() {
        let err = compile_docker_sbx_network_policy(&[rule("db.example.com:5432")]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("HTTP/HTTPS"), "got: {msg}");
        assert!(msg.contains("db.example.com:443"), "got: {msg}");
        assert!(msg.contains("sbx policy allow network"), "got: {msg}");
    }

    #[test]
    fn endpoint_without_port_is_rejected() {
        let err = compile_docker_sbx_network_policy(&[rule("example.com")]).unwrap_err();
        assert!(err.to_string().contains("<HOST>:<PORT>"), "got: {err}");
    }

    #[test]
    fn endpoint_with_invalid_host_is_rejected() {
        let err = compile_docker_sbx_network_policy(&[rule("bad host\nhost:443")]).unwrap_err();
        assert!(
            err.to_string().contains("Invalid sandbox network endpoint"),
            "got: {err}"
        );
    }

    // ── render_docker_sbx_kit ─────────────────────────────────────────────────

    fn empty_policy() -> DockerSbxNetworkPolicy {
        DockerSbxNetworkPolicy {
            allowed_domains: Vec::new(),
        }
    }

    #[test]
    fn kit_deny_all_uses_empty_allowlist() {
        let spec = render_docker_sbx_kit(&KitParams {
            kit_name: "flox-myenv",
            image_ref: "myenv-docker-sbx:abc123",
            network: &empty_policy(),
        });
        assert!(spec.contains("kind: sandbox"), "got:\n{spec}");
        assert!(spec.contains("name: flox-myenv"), "got:\n{spec}");
        assert!(
            spec.contains("image: 'myenv-docker-sbx:abc123'"),
            "got:\n{spec}"
        );
        assert!(spec.contains("allowedDomains: []"), "got:\n{spec}");
        // Deny-all must not emit any per-domain allow entries.
        assert!(
            !spec.contains("    - '"),
            "deny-all must not list domains:\n{spec}"
        );
    }

    #[test]
    fn kit_allowlist_rendered() {
        let net = DockerSbxNetworkPolicy {
            allowed_domains: vec!["api.github.com".to_string(), "*.anthropic.com".to_string()],
        };
        let spec = render_docker_sbx_kit(&KitParams {
            kit_name: "flox-env",
            image_ref: "env-docker-sbx:tag",
            network: &net,
        });
        assert!(spec.contains("allowedDomains:\n"), "got:\n{spec}");
        assert!(spec.contains("    - 'api.github.com'\n"), "got:\n{spec}");
        // Wildcard hosts must be single-quoted so YAML does not read them as
        // alias tokens.
        assert!(spec.contains("    - '*.anthropic.com'\n"), "got:\n{spec}");
        assert!(!spec.contains("allowedDomains: []"), "got:\n{spec}");
    }

    #[test]
    fn kit_declares_sandbox_kind_and_schema() {
        let spec = render_docker_sbx_kit(&KitParams {
            kit_name: "flox-env",
            image_ref: "env-docker-sbx:tag",
            network: &empty_policy(),
        });
        assert!(spec.contains("schemaVersion: \"1\""), "got:\n{spec}");
        assert!(spec.contains("kind: sandbox"), "got:\n{spec}");
        assert!(spec.contains("sandbox:\n  image:"), "got:\n{spec}");
        // The base-image caveat must be documented in the generated manifest.
        assert!(spec.contains("agent"), "got:\n{spec}");
        assert!(spec.contains("uid 1000"), "got:\n{spec}");
    }
}
