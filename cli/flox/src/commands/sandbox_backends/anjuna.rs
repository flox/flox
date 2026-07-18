//! The `anjuna` sandbox backend: hand the environment off to Anjuna Security's
//! confidential-computing runtime.
//!
//! Anjuna is a Trusted-Execution-Environment (TEE) partner-handoff backend, and
//! its ingestion contract differs from every other backend on the roster: it
//! does not *run* an OCI image, it *converts* one. Anjuna's `anjuna-nitro-cli`
//! `build-enclave` step takes a container image (via `--docker-uri`) plus an
//! enclave-config YAML (`--enclave-config-file`) and produces an *enclave image*
//! (`.eif`, an Enclave Image File) that runs hardware-isolated inside a
//! confidential-computing TEE — AWS Nitro Enclaves, AMD SEV-SNP, or Intel SGX.
//! So the integration is OCI-adjacent: the flox bake is the converter's input,
//! and Anjuna's runtime enforces from inside the enclave.
//!
//! flox bakes the environment's OCI image via the shared lockfile-hash-tagged
//! Docker bake (`super::bake`), under Anjuna's own `<env>-anjuna:<hash12>` tag
//! namespace, then generates the Anjuna hand-off — an `enclave-config.yaml`
//! converter config plus the `anjuna-nitro-cli build-enclave` invocation that
//! feeds the baked image through the converter.
//!
//! # The attestation axis (new to the seam)
//!
//! Anjuna introduces an artifact class the seam has not met before: an
//! *attestation binding*. A TEE enclave's identity is its *measurement* — the
//! Nitro attestation report's PCR values (PCR0/PCR1/PCR2), or the SEV-SNP /
//! SGX equivalent — computed over the enclave image at build time. A relying
//! party verifies that measurement before trusting the enclave. flox cannot
//! compute the measurement (it needs the licensed converter and TEE hardware),
//! but it *can* bind the expected measurement to the reproducible closure: the
//! generated hand-off records the lockfile hash alongside a placeholder for the
//! measurement `build-enclave` will emit, so an operator can assert
//! "enclave measurement X corresponds to flox lockfile hash Y" as part of their
//! attestation policy. That binding is the load-bearing story: the enclave that
//! runs is the enclave flox's reproducible environment produced, provable to a
//! third party.
//!
//! The threat model inverts relative to the host-local backends, and further
//! than the other cloud backends: the host filesystem is unreachable from the
//! enclave, and the enclave's memory is encrypted even against the host it runs
//! on — confidentiality against the infrastructure operator is the whole point.
//! But the code and any injected secrets leave the laptop, and credentials
//! belong in the enclave's own attested secret-provisioning mechanism (Anjuna's
//! KMS / secret-injection flow keyed to the attestation report), not a local
//! `.env`.
//!
//! # Why this backend does not complete the launch on any host today
//!
//! Two walls, both named at the launch boundary:
//!
//! 1. **License.** `anjuna-nitro-cli` and the Anjuna runtime are commercially
//!    licensed and not open source; they are distributed through Anjuna's
//!    private package repository, not a public download or the Flox catalog.
//!    This host has no Anjuna tooling.
//! 2. **Hardware.** A TEE requires SGX/SEV-SNP silicon or an AWS Nitro parent
//!    instance with enclave support — a Linux cloud instance. macOS arm64 has
//!    no such capability, so even with the CLI the enclave build + run cannot
//!    complete here.
//!
//! Rather than fake success, this backend implements the deepest honest slice:
//! it bakes the real image, compiles the manifest network policy into the
//! enclave config's egress allowlist, generates the enclave-config YAML + the
//! `build-enclave` invocation + the attestation-binding note, and then fails at
//! the launch boundary with a clear message naming both walls and pointing at
//! the generated artifacts.
//!
//! # Network-policy compilation (the declared lossiness)
//!
//! Egress from a Nitro enclave traverses the parent instance's
//! `anjuna-nitro-netd` vsock↔network proxy, which allowlists egress by host.
//! flox compiles the manifest's `[[options.sandbox.network]]` grants into the
//! enclave config's egress allowlist: each `<host>:443` grant becomes an
//! allowed host. A non-443 endpoint is declined at compile time rather than
//! silently widened — the converter config expresses host allowlisting, not a
//! port matrix, so the grant's port is dropped for the hosts it does accept
//! (a declared lossiness). No grants compiles to an empty allowlist plus a
//! `deny_all_egress: true` marker so the enclave defaults closed rather than
//! open. The grant's `access`, `protocol`, and `binary` scoping is recorded in
//! the artifact as comments but does not constrain traffic through the
//! converter-config contract — declared lossiness, per the backend contract.
//! The *enforcing* proxy config lives on Anjuna's side; the generated config is
//! flox's faithful statement of intent.
//!
//! # Env knobs (prototype)
//!
//! The bake reuses the openshell/oci knobs:
//! - `FLOX_SANDBOX_OCI_IMAGE` — explicit image ref override (skips bake).
//! - `FLOX_SANDBOX_OCI_ALLOW_STALE` — run the newest existing image when the
//!   expected hash-tag is absent.
//! - `FLOX_SANDBOX_OCI_AUTOBAKE` — bake without prompting.
//! - `FLOX_SANDBOX_ANJUNA_REGISTRY` — registry prefix the `build-enclave`
//!   `--docker-uri` is built from (e.g. `docker.io/myuser`); recorded in the
//!   artifact so a credentialed operator does not have to hand-edit it before
//!   pushing the image the converter pulls.

use std::convert::Infallible;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flox_core::activate::sandbox_backend::SandboxBackend;
use flox_core::activate::sandbox_policy::SandboxNetworkRule;
use flox_manifest::lockfile::Lockfile;
use flox_rust_sdk::providers::container_builder::ContainerBuilderParams;
use tracing::debug;

use super::handoff::{ensure_local_image, manifest_network_rules};
use super::preflight::{first_on_path, split_endpoint};
use super::{ActivationSandbox, SandboxLaunchCtx};
use crate::commands::sandbox_backends::oci::lockfile_hash12;

/// Registry prefix the `build-enclave --docker-uri` reference is built from.
/// When set (e.g. `docker.io/myuser`), the generated invocation references
/// `<prefix>/<repo>:<hash12>` so a credentialed operator does not have to
/// hand-edit the image ref before pushing and converting the enclave image.
pub(crate) const FLOX_SANDBOX_ANJUNA_REGISTRY_VAR: &str = "FLOX_SANDBOX_ANJUNA_REGISTRY";

/// Repository suffix for the Anjuna backend's image tags. The image is baked
/// under `<env>-anjuna:<hash12>` (with the shared compat layer) and the
/// `build-enclave` invocation reuses that name, so the pushed artifact is
/// recognizable as Anjuna's and never collides with the other backends' tags on
/// a shared registry.
const ANJUNA_REPO_SUFFIX: &str = "-anjuna";

/// The Anjuna Nitro CLI name. Anjuna ships `anjuna-nitro-cli` to convert a
/// container image into an enclave image and run it; it is commercially
/// licensed and distributed through Anjuna's private repository. It is
/// presence-detected, not required — the converter-config hand-off is generated
/// regardless, and the launch boundary names the license + hardware walls
/// either way.
const ANJUNA_CLI: &str = "anjuna-nitro-cli";

/// Project-relative directory the generated Anjuna hand-off is written under.
/// Unlike the ona/devin committed-repo artifacts, the enclave-converter config
/// and build script are build inputs a credentialed operator regenerates, so
/// they live under `.flox/cache/` like the modal launcher rather than at the
/// repo root.
const ANJUNA_CACHE_SUBDIR: &str = ".flox/cache/anjuna";

/// Filename of the generated enclave-converter config (`--enclave-config-file`).
const ENCLAVE_CONFIG_FILENAME: &str = "enclave-config.yaml";

/// Filename of the generated `build-enclave` invocation + attestation-binding
/// note.
const BUILD_SCRIPT_FILENAME: &str = "build-enclave.sh";

pub struct AnjunaBackend<'a> {
    dot_flox_path: PathBuf,
    env_name: String,
    lockfile: &'a Lockfile,
    sandbox_oci_autobake: bool,
    container_builder_params: ContainerBuilderParams,
}

impl<'a> AnjunaBackend<'a> {
    pub fn new(ctx: SandboxLaunchCtx<'a>) -> Self {
        Self {
            dot_flox_path: ctx.dot_flox_path,
            env_name: ctx.env_name,
            lockfile: ctx.lockfile,
            sandbox_oci_autobake: ctx.sandbox_oci_autobake,
            container_builder_params: ctx.container_builder_params,
        }
    }
}

impl ActivationSandbox for AnjunaBackend<'_> {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::Anjuna
    }

    fn preflight(&self) -> Result<()> {
        // Anjuna is partner-handoff/TEE: the image bake runs through Docker, so
        // Docker is the one genuinely required host tool.
        if first_on_path("docker").is_none() {
            bail!(
                "The 'anjuna' sandbox backend bakes the environment image with Docker, \
                 which was not found on PATH.\n\
                 Install Docker Desktop or the Docker CLI, then re-run."
            );
        }
        // The Anjuna CLI is not required to generate the converter-config
        // hand-off, but detecting it up front lets the launch-boundary message
        // be precise about what the operator has. Never trigger an interactive
        // login: presence-on-PATH is the only probe.
        match first_on_path(ANJUNA_CLI) {
            Some(path) => {
                debug!(cli = ANJUNA_CLI, path = %path.display(), "detected Anjuna Nitro CLI");
            },
            None => {
                debug!(
                    "no Anjuna Nitro CLI on PATH; it is commercially licensed and absent here. \
                     The converter-config artifact is generated regardless and the launch \
                     boundary names the license + hardware walls"
                );
            },
        }
        Ok(())
    }

    fn wrap_activation(self: Box<Self>) -> Result<Infallible> {
        wrap_anjuna(
            &self.dot_flox_path,
            &self.env_name,
            self.lockfile,
            self.sandbox_oci_autobake,
            &self.container_builder_params,
        )
    }
}

/// Return the `<env>-anjuna` repository name used for Docker image tagging.
fn anjuna_repo(env_name: &str) -> String {
    format!("{env_name}{ANJUNA_REPO_SUFFIX}")
}

/// Build the registry image reference the `build-enclave --docker-uri`
/// references.
///
/// When `FLOX_SANDBOX_ANJUNA_REGISTRY` is set, the ref is
/// `<prefix>/<repo>:<hash12>`; otherwise the bare local `<repo>:<hash12>` tag is
/// used as a placeholder (the operator must retag/push before the conversion).
pub(crate) fn anjuna_image_ref(repo: &str, hash12: &str, registry_prefix: Option<&str>) -> String {
    match registry_prefix {
        Some(prefix) => {
            let prefix = prefix.trim_end_matches('/');
            format!("{prefix}/{repo}:{hash12}")
        },
        None => format!("{repo}:{hash12}"),
    }
}

// ── Network policy compilation ─────────────────────────────────────────────────

/// The manifest network policy compiled into Anjuna's enclave-config egress
/// vocabulary.
///
/// Egress from a Nitro enclave traverses the parent's `anjuna-nitro-netd`
/// vsock↔network proxy, which allowlists egress by host. flox compiles the
/// manifest grants into that allowlist; everything else is deny-by-default,
/// which flox makes explicit with a `deny_all_egress` marker so an empty
/// allowlist does not read as "open".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnjunaNetworkPolicy {
    /// Deny all outbound traffic (no grants declared).
    pub deny_all: bool,
    /// Hosts granted egress, in declaration order (deduplicated). Compiled onto
    /// the enclave config's egress allowlist.
    pub allowed_hosts: Vec<String>,
}

/// Compile the manifest's `[[options.sandbox.network]]` rules into Anjuna's
/// enclave-config egress vocabulary.
///
/// - No rules → deny-all (secure-by-default; the config records an empty
///   allowlist plus `deny_all_egress: true` so the enclave defaults closed).
/// - A `<host>:443` rule → an allowed-host entry (the netd proxy allowlists by
///   host, so the port is dropped — a declared lossiness).
/// - Any non-443 port → a hard error: the converter config expresses host
///   allowlisting, not a port matrix, and silently promoting the grant to
///   all-ports (or dropping it) would violate the "never silently widen or
///   narrow grants" contract.
pub(crate) fn compile_anjuna_network_policy(
    rules: &[SandboxNetworkRule],
) -> Result<AnjunaNetworkPolicy> {
    if rules.is_empty() {
        return Ok(AnjunaNetworkPolicy {
            deny_all: true,
            allowed_hosts: Vec::new(),
        });
    }
    let mut hosts: Vec<String> = Vec::with_capacity(rules.len());
    for rule in rules {
        let (host, port) = split_endpoint(&rule.endpoint)?;
        if port != 443 {
            bail!(
                "The 'anjuna' sandbox backend expresses egress as allowed hosts in the enclave \
                 converter config, but rule '{endpoint}' targets port {port}.\n\
                 Anjuna's enclave egress proxy allowlists per-host, not per-port; rewrite the \
                 endpoint as '{host}:443', or select a backend with per-port egress (e.g. \
                 'openshell').",
                endpoint = rule.endpoint
            );
        }
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    Ok(AnjunaNetworkPolicy {
        deny_all: false,
        allowed_hosts: hosts,
    })
}

// ── Enclave-config artifact generation ─────────────────────────────────────────

/// Render a YAML flow-sequence of double-quoted scalars, e.g.
/// `["a.com", "b.com"]`.
///
/// YAML double-quoted scalars escape backslash and double-quote; the
/// `split_endpoint` charset check already forbids both in hosts, but the
/// escaping is the belt-and-suspenders guard the artifact depends on. Kept local
/// to this module — the enclave config is YAML, a per-provider serialization
/// shape the shared `toml_str_*` / `json_str_*` helpers do not cover.
pub(crate) fn yaml_str_list(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|s| {
            let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{escaped}\"")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

/// Inputs to [`render_enclave_config`] and [`render_build_script`].
pub(crate) struct AnjunaHandoffParams<'a> {
    /// Registry image reference the converter pulls / an operator can
    /// `docker load`.
    pub image_ref: &'a str,
    /// The lockfile hash the attestation binding ties the enclave measurement
    /// to.
    pub lockfile_hash: &'a str,
    /// Compiled egress policy.
    pub network: &'a AnjunaNetworkPolicy,
}

/// Render the Anjuna enclave-converter config (pure function, no I/O).
///
/// The emitted `enclave-config.yaml` is the `--enclave-config-file` input to
/// `anjuna-nitro-cli build-enclave`. It records the egress allowlist flox
/// compiled from the manifest grants and states the attestation-binding intent
/// and the declared lossiness in comments. Anjuna's config is YAML with `#`
/// comments.
pub(crate) fn render_enclave_config(params: &AnjunaHandoffParams<'_>) -> String {
    let allowlist_yaml = yaml_str_list(&params.network.allowed_hosts);
    let policy_summary = if params.network.deny_all {
        "deny-all (no grants declared)".to_string()
    } else {
        format!("allowed: {}", params.network.allowed_hosts.join(", "))
    };
    // image_ref comes from validated sources (repo + hash12 tag) and lockfile
    // hash from the lockfile; yaml_str_list additionally escapes each allowlist
    // host.
    indoc::formatdoc! {r#"
        # Generated by `flox activate --sandbox --sandbox-backend anjuna`.
        # This is the enclave-converter config for the Anjuna Security (TEE)
        # backend. Anjuna does not run an OCI image directly: its
        # `anjuna-nitro-cli build-enclave` step CONVERTS a container image
        # (--docker-uri) plus this config into an enclave image (.eif) that runs
        # hardware-isolated inside a confidential-computing TEE (AWS Nitro
        # Enclaves / AMD SEV-SNP / Intel SGX). flox baked the environment image
        # as the converter's input ("{image_ref}").
        #
        # ATTESTATION BINDING. The enclave's identity is its measurement (the
        # Nitro attestation report's PCR0/PCR1/PCR2, or the SEV-SNP/SGX
        # equivalent), computed by `build-enclave` over the enclave image. That
        # measurement corresponds to this flox lockfile hash:
        #   flox lockfile hash: {lockfile_hash}
        # Record the measurement `build-enclave` emits alongside this hash so a
        # relying party can assert "enclave measurement X == flox lockfile hash
        # Y" in their attestation policy. See build-enclave.sh for the invocation.
        #
        # Egress is deny-by-default. The manifest's [[options.sandbox.network]]
        # grants are compiled into `egress.allowed_hosts` below; enforcement is
        # by the parent instance's anjuna-nitro-netd vsock<->network proxy. The
        # proxy allowlists per-host, not per-port, so the grant's port is
        # dropped; access/protocol/binary scoping from the manifest is recorded
        # here but is NOT enforceable through the converter-config contract.
        #   policy: {policy_summary}
        egress:
          deny_all_egress: {deny_all}
          allowed_hosts: {allowlist_yaml}
        "#,
        image_ref = params.image_ref,
        lockfile_hash = params.lockfile_hash,
        policy_summary = policy_summary,
        deny_all = params.network.deny_all,
        allowlist_yaml = allowlist_yaml,
    }
}

/// Render the `anjuna-nitro-cli build-enclave` invocation + attestation-binding
/// note (pure function, no I/O).
///
/// This is the shell script a credentialed operator runs on a TEE-capable Linux
/// instance: it converts the baked image into an enclave image and prints the
/// attestation measurement to bind to the lockfile hash.
pub(crate) fn render_build_script(params: &AnjunaHandoffParams<'_>) -> String {
    // image_ref and lockfile_hash come from validated sources; the script embeds
    // them in a single-quoted shell context, safe for the hostname/tag charset.
    indoc::formatdoc! {r#"
        #!/usr/bin/env bash
        # Generated by `flox activate --sandbox --sandbox-backend anjuna`.
        #
        # Run this on a TEE-capable Linux instance (AWS Nitro parent with enclave
        # support, or an SEV-SNP / SGX host) with a licensed anjuna-nitro-cli on
        # PATH. It converts the flox-baked image into an enclave image (.eif) and
        # prints the attestation measurement to bind to the lockfile hash.
        #
        # ATTESTATION BINDING
        #   flox lockfile hash: {lockfile_hash}
        # After build-enclave completes, record its PCR measurement alongside
        # this hash: the enclave that runs is the one flox's reproducible
        # environment produced, provable to a relying party.
        set -euo pipefail

        IMAGE_URI='{image_ref}'
        CONFIG_FILE="$(dirname "$0")/{config_file}"
        OUTPUT_EIF="flox-anjuna-enclave.eif"

        # Convert the baked container image into an enclave image. --docker-uri
        # feeds the flox bake; --enclave-config-file feeds the egress allowlist
        # flox compiled from the manifest grants.
        anjuna-nitro-cli build-enclave \
          --docker-uri "$IMAGE_URI" \
          --enclave-config-file "$CONFIG_FILE" \
          --output-file "$OUTPUT_EIF"

        # build-enclave prints the enclave measurement (PCR0/PCR1/PCR2). Record
        # it against the flox lockfile hash above. Then launch:
        #   anjuna-nitro-cli run-enclave --eif-path "$OUTPUT_EIF"
        "#,
        lockfile_hash = params.lockfile_hash,
        image_ref = params.image_ref,
        config_file = ENCLAVE_CONFIG_FILENAME,
    }
}

// ── Launch path ────────────────────────────────────────────────────────────────

/// Bake the image, compile the policy, generate the enclave-converter config +
/// build invocation + attestation-binding note, then fail at the launch
/// boundary — never fake the enclave build.
fn wrap_anjuna(
    dot_flox_path: &Path,
    env_name: &str,
    lockfile: &Lockfile,
    autobake: bool,
    container_builder_params: &ContainerBuilderParams,
) -> Result<Infallible> {
    let dot_flox =
        std::fs::canonicalize(dot_flox_path).unwrap_or_else(|_| dot_flox_path.to_path_buf());
    let project = dot_flox.parent().unwrap_or(&dot_flox).to_path_buf();

    // Bake and tag the image under Anjuna's own namespace (`<env>-anjuna`), with
    // the shared compat layer. The baked image is the converter's input.
    let repo = anjuna_repo(env_name);
    let hash12 = lockfile_hash12(lockfile);

    ensure_local_image(
        &repo,
        env_name,
        dot_flox_path,
        lockfile,
        autobake,
        container_builder_params,
        "Anjuna image",
    )?;

    // Compile the manifest network policy into Anjuna's egress vocabulary.
    let rules = manifest_network_rules(lockfile)?;
    let network = compile_anjuna_network_policy(&rules)?;

    // Build the converter-config + build-invocation artifacts.
    let registry_prefix = std::env::var(FLOX_SANDBOX_ANJUNA_REGISTRY_VAR)
        .ok()
        .filter(|v| !v.is_empty());
    let image_ref = anjuna_image_ref(&repo, &hash12, registry_prefix.as_deref());
    let params = AnjunaHandoffParams {
        image_ref: &image_ref,
        lockfile_hash: &hash12,
        network: &network,
    };
    let config = render_enclave_config(&params);
    let build_script = render_build_script(&params);
    let (config_path, script_path) = write_anjuna_handoff(&project, &config, &build_script)?;

    // Fail at the launch boundary with the concrete prerequisites. Name whether
    // an Anjuna CLI was detected so the message is precise about the license
    // wall.
    let cli_note = match first_on_path(ANJUNA_CLI) {
        Some(_) => "the 'anjuna-nitro-cli' is installed, but ".to_string(),
        None => "no 'anjuna-nitro-cli' was found on PATH (it is commercially licensed), and "
            .to_string(),
    };
    let registry_hint = match &registry_prefix {
        Some(prefix) => {
            let prefix = prefix.trim_end_matches('/');
            format!("tag and push it as '{prefix}/{repo}:{hash12}'")
        },
        None => format!(
            "set {FLOX_SANDBOX_ANJUNA_REGISTRY_VAR}=<registry-prefix> and re-run, then push '<prefix>/{repo}:{hash12}'"
        ),
    };
    bail!(
        "The 'anjuna' sandbox backend converts the baked environment into an Anjuna enclave \
         image, which requires prerequisites this host cannot satisfy automatically:\n  \
         1. Push the baked image '{repo}:{hash12}' to a registry the Anjuna converter can pull \
         ({registry_hint}).\n  \
         2. An Anjuna commercial license: {cli_note}the anjuna-nitro-cli and runtime are not \
         open source — obtain them through Anjuna (a warm partnership contact exists; see the \
         demo notes).\n  \
         3. TEE hardware: the enclave needs SGX/SEV-SNP silicon or an AWS Nitro parent \
         instance. macOS arm64 has no such capability, so the build + run must happen on a cloud \
         Linux instance.\n\
         flox generated the Anjuna converter config + build invocation at:\n  {config}\n  {script}\n\
         On a licensed, TEE-capable instance, push the image, then run the build script to \
         convert the enclave image and record its attestation measurement against the lockfile \
         hash '{hash12}'.",
        config = config_path.display(),
        script = script_path.display(),
    )
}

/// Write the generated enclave-config + build script under
/// `<project>/.flox/cache/anjuna/` and return their paths.
///
/// The artifacts live under `.flox/cache/` (like the modal launcher, unlike the
/// committed ona devcontainer / devin blueprint) because they are build inputs a
/// credentialed operator regenerates, not files meant to be committed to the
/// repo.
fn write_anjuna_handoff(
    project: &Path,
    config: &str,
    build_script: &str,
) -> Result<(PathBuf, PathBuf)> {
    let dir = project.join(ANJUNA_CACHE_SUBDIR);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create Anjuna hand-off dir '{}'", dir.display()))?;

    let config_path = dir.join(ENCLAVE_CONFIG_FILENAME);
    std::fs::write(&config_path, config).with_context(|| {
        format!(
            "failed to write Anjuna enclave config to '{}'",
            config_path.display()
        )
    })?;

    let script_path = dir.join(BUILD_SCRIPT_FILENAME);
    std::fs::write(&script_path, build_script).with_context(|| {
        format!(
            "failed to write Anjuna build script to '{}'",
            script_path.display()
        )
    })?;

    debug!(
        config = %config_path.display(),
        script = %script_path.display(),
        "wrote anjuna hand-off artifacts"
    );
    Ok((config_path, script_path))
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use flox_core::activate::sandbox_policy::SandboxNetworkAccess;

    use super::*;

    // ── anjuna_repo ───────────────────────────────────────────────────────────

    #[test]
    fn repo_has_anjuna_suffix() {
        assert_eq!(anjuna_repo("myenv"), "myenv-anjuna");
    }

    #[test]
    fn repo_never_collides_with_other_backends() {
        let env = "myenv";
        let hash = "abc123def456";
        let oci = format!("{env}:{hash}");
        let ona = format!("{env}-ona:{hash}");
        let daytona = format!("{env}-daytona:{hash}");
        let devin = format!("{env}-cognition-devin:{hash}");
        let anjuna = format!("{}:{hash}", anjuna_repo(env));
        assert_ne!(anjuna, oci);
        assert_ne!(anjuna, ona);
        assert_ne!(anjuna, daytona);
        assert_ne!(anjuna, devin);
    }

    // ── anjuna_image_ref ──────────────────────────────────────────────────────

    #[test]
    fn image_ref_without_registry_is_bare_tag() {
        assert_eq!(
            anjuna_image_ref("myenv-anjuna", "abc123", None),
            "myenv-anjuna:abc123"
        );
    }

    #[test]
    fn image_ref_with_registry_prefixes_and_trims_slash() {
        assert_eq!(
            anjuna_image_ref("myenv-anjuna", "abc123", Some("docker.io/user")),
            "docker.io/user/myenv-anjuna:abc123"
        );
        assert_eq!(
            anjuna_image_ref("myenv-anjuna", "abc123", Some("docker.io/user/")),
            "docker.io/user/myenv-anjuna:abc123"
        );
    }

    // ── yaml_str_list ─────────────────────────────────────────────────────────

    #[test]
    fn yaml_str_list_quotes_and_escapes() {
        assert_eq!(yaml_str_list(&[]), "[]");
        assert_eq!(
            yaml_str_list(&["api.github.com".to_string(), "*.anthropic.com".to_string()]),
            "[\"api.github.com\", \"*.anthropic.com\"]"
        );
        assert_eq!(yaml_str_list(&["a\"b".to_string()]), "[\"a\\\"b\"]");
    }

    // ── compile_anjuna_network_policy ─────────────────────────────────────────

    fn rule(endpoint: &str) -> SandboxNetworkRule {
        SandboxNetworkRule {
            endpoint: endpoint.to_string(),
            access: None,
            protocol: None,
            binary: None,
        }
    }

    #[test]
    fn no_rules_compiles_to_deny_all() {
        let policy = compile_anjuna_network_policy(&[]).unwrap();
        assert_eq!(policy, AnjunaNetworkPolicy {
            deny_all: true,
            allowed_hosts: Vec::new(),
        });
    }

    #[test]
    fn tls_443_rules_compile_to_allowed_hosts() {
        let rules = [rule("api.github.com:443"), rule("api.anthropic.com:443")];
        let policy = compile_anjuna_network_policy(&rules).unwrap();
        assert_eq!(policy, AnjunaNetworkPolicy {
            deny_all: false,
            allowed_hosts: vec![
                "api.github.com".to_string(),
                "api.anthropic.com".to_string(),
            ],
        });
    }

    #[test]
    fn duplicate_hosts_are_deduplicated() {
        let rules = [rule("api.github.com:443"), rule("api.github.com:443")];
        let policy = compile_anjuna_network_policy(&rules).unwrap();
        assert_eq!(policy.allowed_hosts, vec!["api.github.com".to_string()]);
    }

    #[test]
    fn wildcard_host_is_preserved() {
        let policy = compile_anjuna_network_policy(&[rule("*.github.com:443")]).unwrap();
        assert_eq!(policy.allowed_hosts, vec!["*.github.com".to_string()]);
    }

    #[test]
    fn access_and_binary_do_not_affect_compilation() {
        // The converter-config hand-off carries no method distinction; a scoped
        // grant compiles identically to an unscoped one (declared lossiness).
        let scoped = SandboxNetworkRule {
            endpoint: "api.github.com:443".to_string(),
            access: Some(SandboxNetworkAccess::ReadOnly),
            protocol: None,
            binary: Some("curl".to_string()),
        };
        let policy = compile_anjuna_network_policy(&[scoped]).unwrap();
        assert_eq!(policy, AnjunaNetworkPolicy {
            deny_all: false,
            allowed_hosts: vec!["api.github.com".to_string()],
        });
    }

    #[test]
    fn non_443_port_is_rejected() {
        let err = compile_anjuna_network_policy(&[rule("db.example.com:5432")]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("per-host, not per-port"), "got: {msg}");
        assert!(msg.contains("db.example.com:443"), "got: {msg}");
    }

    #[test]
    fn endpoint_without_port_is_rejected() {
        let err = compile_anjuna_network_policy(&[rule("example.com")]).unwrap_err();
        assert!(err.to_string().contains("<HOST>:<PORT>"), "got: {err}");
    }

    #[test]
    fn endpoint_with_invalid_host_is_rejected() {
        let err = compile_anjuna_network_policy(&[rule("bad host\nhost:443")]).unwrap_err();
        assert!(
            err.to_string().contains("Invalid sandbox network endpoint"),
            "got: {err}"
        );
    }

    // ── render_enclave_config ─────────────────────────────────────────────────

    fn deny_all_policy() -> AnjunaNetworkPolicy {
        AnjunaNetworkPolicy {
            deny_all: true,
            allowed_hosts: Vec::new(),
        }
    }

    fn params<'a>(
        image_ref: &'a str,
        lockfile_hash: &'a str,
        network: &'a AnjunaNetworkPolicy,
    ) -> AnjunaHandoffParams<'a> {
        AnjunaHandoffParams {
            image_ref,
            lockfile_hash,
            network,
        }
    }

    #[test]
    fn enclave_config_deny_all_has_empty_allowlist_and_deny_marker() {
        let net = deny_all_policy();
        let doc = render_enclave_config(&params("myenv-anjuna:abc123", "abc123def456", &net));
        assert!(doc.contains("allowed_hosts: []"), "got:\n{doc}");
        assert!(doc.contains("deny_all_egress: true"), "got:\n{doc}");
        assert!(
            doc.contains("deny-all (no grants declared)"),
            "policy summary comment missing:\n{doc}"
        );
        assert!(
            doc.contains("myenv-anjuna:abc123"),
            "image ref missing:\n{doc}"
        );
        // The attestation binding must record the lockfile hash.
        assert!(
            doc.contains("flox lockfile hash: abc123def456"),
            "attestation binding missing:\n{doc}"
        );
    }

    #[test]
    fn enclave_config_allowlist_rendered() {
        let net = AnjunaNetworkPolicy {
            deny_all: false,
            allowed_hosts: vec!["api.github.com".to_string(), "*.anthropic.com".to_string()],
        };
        let doc = render_enclave_config(&params("env-anjuna:tag", "deadbeef", &net));
        assert!(
            doc.contains("allowed_hosts: [\"api.github.com\", \"*.anthropic.com\"]"),
            "got:\n{doc}"
        );
        assert!(doc.contains("deny_all_egress: false"), "got:\n{doc}");
        assert!(
            doc.contains("allowed: api.github.com, *.anthropic.com"),
            "policy summary comment missing:\n{doc}"
        );
    }

    #[test]
    fn enclave_config_header_states_converter_contract() {
        let net = deny_all_policy();
        let doc = render_enclave_config(&params("env-anjuna:tag", "hash", &net));
        assert!(
            doc.starts_with("# Generated by `flox activate --sandbox --sandbox-backend anjuna`."),
            "got:\n{doc}"
        );
        // The header must name the converter contract: this is not an image
        // hand-off — Anjuna converts the image into an enclave image.
        assert!(
            doc.contains("build-enclave"),
            "converter contract missing:\n{doc}"
        );
        assert!(
            doc.contains("does not run an OCI image directly"),
            "inversion note missing:\n{doc}"
        );
        // The attestation axis must be named.
        assert!(
            doc.contains("ATTESTATION BINDING"),
            "attestation axis missing:\n{doc}"
        );
    }

    #[test]
    fn enclave_config_egress_block_is_well_formed() {
        // The peer backends assert on rendered structure with string checks
        // rather than pulling in a YAML parser as a dev-dependency; do the same,
        // pinning the two-key egress block shape the converter reads.
        let net = AnjunaNetworkPolicy {
            deny_all: false,
            allowed_hosts: vec!["api.github.com".to_string()],
        };
        let doc = render_enclave_config(&params("env-anjuna:tag", "hash", &net));
        assert!(doc.contains("egress:"), "egress key missing:\n{doc}");
        assert!(
            doc.contains("  deny_all_egress: false"),
            "deny marker missing or misindented:\n{doc}"
        );
        assert!(
            doc.contains("  allowed_hosts: [\"api.github.com\"]"),
            "allowlist missing or misindented:\n{doc}"
        );
    }

    // ── render_build_script ───────────────────────────────────────────────────

    #[test]
    fn build_script_invokes_build_enclave_with_docker_uri() {
        let net = deny_all_policy();
        let doc = render_build_script(&params("docker.io/user/env-anjuna:tag", "hash99", &net));
        assert!(doc.starts_with("#!/usr/bin/env bash"), "got:\n{doc}");
        assert!(
            doc.contains("anjuna-nitro-cli build-enclave"),
            "got:\n{doc}"
        );
        assert!(
            doc.contains("--docker-uri \"$IMAGE_URI\""),
            "docker-uri flag missing:\n{doc}"
        );
        assert!(
            doc.contains("IMAGE_URI='docker.io/user/env-anjuna:tag'"),
            "image ref missing:\n{doc}"
        );
        assert!(
            doc.contains("--enclave-config-file \"$CONFIG_FILE\""),
            "enclave-config flag missing:\n{doc}"
        );
        // The attestation binding must tie the measurement to the lockfile hash.
        assert!(
            doc.contains("flox lockfile hash: hash99"),
            "attestation binding missing:\n{doc}"
        );
        assert!(
            doc.contains("run-enclave --eif-path"),
            "run hint missing:\n{doc}"
        );
    }
}
