//! The `cognition-devin` sandbox backend: hand the environment off to
//! Cognition's Devin runtime.
//!
//! Devin is a partner-handoff backend, and its ingestion contract differs from
//! every other cloud backend on the roster. Devin does **not** consume an OCI
//! image directly: a Devin session boots from a *snapshot* that Devin's own
//! builder produces from a YAML *blueprint*. Devin's docs map the concepts
//! explicitly — blueprint ≈ Dockerfile, build ≈ `docker build`, snapshot ≈
//! image. So the integration is inverted relative to the OCI-ingesting cloud
//! backends: Flox supplies the *environment definition* Devin runs inside, and
//! Devin's runtime enforces. This is the co-sell shape — Flox is the
//! reproducible environment layer beneath Devin's runtime + sandbox layer.
//!
//! flox still bakes the environment's OCI image via the shared
//! lockfile-hash-tagged Docker bake (`super::bake`), under Devin's own
//! `<env>-cognition-devin:<hash12>` tag namespace: it is the reproducible
//! substrate a credentialed operator can `docker load` and reference, and
//! baking is the content-addressed step every OCI-handoff backend shares. But
//! the hand-off artifact flox generates is a **git-backed blueprint**
//! (`.devin/blueprint.yaml`), not a devcontainer — its `initialize` step
//! installs Flox and activates the locked environment, so Devin's build
//! reproduces the closure inside the snapshot every session boots from.
//!
//! The threat model inverts relative to the host-local backends: the host
//! filesystem is unreachable from Devin's runtime, but the code and any injected
//! secrets run in Devin's cloud. Credentials belong in Devin's own secret
//! mechanism (the blueprint's secrets/env-var material), not a local `.env`.
//!
//! # Why this backend does not complete the launch on any host today
//!
//! Devin builds a snapshot from a blueprint through its own builder and boots
//! sessions from it; there is no public no-account API that ingests an arbitrary
//! image or launches an arbitrary runtime. Driving the whole flow needs a Devin
//! subscription and — per the co-sell note — a partnership with Cognition's
//! Sandbox/Infra team. There is a public Sessions REST API (`api.devin.ai`,
//! Bearer `DEVIN_API_KEY`) that drives Devin *the agent* and accepts a
//! `snapshot_id`, plus a v3 snapshot-setup / blueprint API, but none of that is
//! reachable from a bare checkout without an account. No `devin` CLI is
//! installed on this host. `preflight` therefore detects the CLI if present
//! (never triggering an interactive login) and requires only Docker.
//!
//! Rather than fake success, this backend implements the deepest honest slice:
//! it bakes the real image, compiles the manifest network policy into Devin's
//! CLI-sandbox egress vocabulary, and *generates the blueprint artifact*
//! (`.devin/blueprint.yaml` with the Flox-install `initialize` step and the
//! compiled `sandbox.allowed_domains` allowlist). It then fails at the launch
//! boundary with a clear "requires ..." message that points at the generated
//! artifact and names the subscription / partnership wall.
//!
//! # Network-policy compilation (the load-bearing lossiness)
//!
//! Devin's CLI sandbox filters egress by domain through a loopback proxy:
//! `allowed_domains` is an allowlist (when non-empty, only matching domains
//! pass) and `denied_domains` always blocks (deny takes precedence). flox
//! compiles the manifest's `[[options.sandbox.network]]` grants into
//! `allowed_domains`: each `<host>:443` grant becomes an allowed domain. A
//! non-443 endpoint cannot be expressed as an HTTPS/domain rule and is declined
//! at compile time rather than silently widened — Devin filters per-domain, not
//! per-port, so the grant's port is dropped for the allowed hosts it does
//! accept (a declared lossiness). No grants compiles to an empty allowlist,
//! which Devin's own semantics leave as "no allowlist restriction"; flox records
//! deny-by-default intent explicitly as a comment plus a `denied_domains: ["*"]`
//! catch-all so the generated policy is deny-by-default rather than open. The
//! grant's `access`, `protocol`, and `binary` scoping is recorded in the
//! artifact as comments but does not constrain traffic through the blueprint
//! contract — declared lossiness, per the backend contract.
//!
//! # Env knobs (prototype)
//!
//! The bake reuses the openshell/oci knobs:
//! - `FLOX_SANDBOX_OCI_IMAGE` — explicit image ref override (skips bake).
//! - `FLOX_SANDBOX_OCI_ALLOW_STALE` — run the newest existing image when the
//!   expected hash-tag is absent.
//! - `FLOX_SANDBOX_OCI_AUTOBAKE` — bake without prompting.
//! - `FLOX_SANDBOX_COGNITION_DEVIN_REGISTRY` — registry prefix the blueprint's
//!   image reference is built from (e.g. `docker.io/myuser`); recorded in the
//!   artifact so a credentialed operator does not have to hand-edit it before
//!   pushing the image Devin's build can pull.

use std::convert::Infallible;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flox_core::activate::sandbox_backend::SandboxBackend;
use flox_core::activate::sandbox_policy::SandboxNetworkRule;
use flox_manifest::lockfile::Lockfile;
use flox_rust_sdk::providers::container_builder::ContainerBuilderParams;
use tracing::debug;

use super::handoff::{
    ensure_local_image,
    manifest_network_rules,
    registry_image_ref,
    yaml_str_list,
};
use super::preflight::{first_on_path, split_endpoint};
use super::{ActivationSandbox, SandboxLaunchCtx};
use crate::commands::sandbox_backends::oci::lockfile_hash12;

/// Registry prefix the blueprint's image reference is built from. When set
/// (e.g. `docker.io/myuser`), the generated artifact references
/// `<prefix>/<repo>:<hash12>` so a credentialed operator does not have to
/// hand-edit the image ref before pushing and building the snapshot.
pub(crate) const FLOX_SANDBOX_COGNITION_DEVIN_REGISTRY_VAR: &str =
    "FLOX_SANDBOX_COGNITION_DEVIN_REGISTRY";

/// Repository suffix for the Devin backend's image tags. The image is baked
/// under `<env>-cognition-devin:<hash12>` (with the shared compat layer) and the
/// blueprint's image reference reuses that name, so the pushed artifact is
/// recognizable as Devin's and never collides with the other backends' tags on
/// a shared registry.
const DEVIN_REPO_SUFFIX: &str = "-cognition-devin";

/// The Devin CLI name. Devin ships a `devin` CLI that drives sessions and the
/// local sandbox; it is presence-detected, not required — the blueprint hand-off
/// is generated regardless, and the launch boundary names the subscription wall
/// either way.
const DEVIN_CLI: &str = "devin";

/// Project-relative path of the generated git-backed blueprint hand-off. Devin
/// reads `.devin/blueprint.yaml` from the repository's default branch and builds
/// a snapshot from it.
const BLUEPRINT_REL_PATH: &str = ".devin/blueprint.yaml";

pub struct CognitionDevinBackend<'a> {
    dot_flox_path: PathBuf,
    env_name: String,
    lockfile: &'a Lockfile,
    sandbox_oci_autobake: bool,
    container_builder_params: ContainerBuilderParams,
}

impl<'a> CognitionDevinBackend<'a> {
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

impl ActivationSandbox for CognitionDevinBackend<'_> {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::CognitionDevin
    }

    fn preflight(&self) -> Result<()> {
        // Devin is partner-handoff/cloud: the image bake runs through Docker, so
        // Docker is the one genuinely required host tool.
        if first_on_path("docker").is_none() {
            bail!(
                "The 'cognition-devin' sandbox backend bakes the environment image with Docker, \
                 which was not found on PATH.\n\
                 Install Docker Desktop or the Docker CLI, then re-run."
            );
        }
        // The Devin CLI is not required to generate the blueprint hand-off, but
        // detecting it up front lets the launch-boundary message be precise
        // about what the operator has. Never trigger an interactive login:
        // presence-on-PATH is the only probe.
        match first_on_path(DEVIN_CLI) {
            Some(path) => {
                debug!(cli = DEVIN_CLI, path = %path.display(), "detected Devin CLI");
            },
            None => {
                debug!(
                    "no Devin CLI on PATH; the blueprint artifact is generated regardless and \
                     the launch boundary names the subscription/partnership wall"
                );
            },
        }
        Ok(())
    }

    fn wrap_activation(self: Box<Self>) -> Result<Infallible> {
        wrap_cognition_devin(
            &self.dot_flox_path,
            &self.env_name,
            self.lockfile,
            self.sandbox_oci_autobake,
            &self.container_builder_params,
        )
    }
}

/// Return the `<env>-cognition-devin` repository name used for Docker image
/// tagging.
fn devin_repo(env_name: &str) -> String {
    format!("{env_name}{DEVIN_REPO_SUFFIX}")
}

// ── Network policy compilation ─────────────────────────────────────────────────

/// The manifest network policy compiled into Devin's CLI-sandbox egress
/// vocabulary.
///
/// Devin filters egress by domain through a loopback proxy: `allowed_domains` is
/// an allowlist (only matching domains pass when non-empty) and `denied_domains`
/// always blocks. flox compiles the manifest grants into `allowed_domains`;
/// everything else is deny-by-default, which flox makes explicit with a
/// `denied_domains: ["*"]` catch-all so an empty allowlist does not read as
/// "open".
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DevinNetworkPolicy {
    /// Deny all outbound traffic (no grants declared).
    pub deny_all: bool,
    /// Hosts granted egress, in declaration order (deduplicated). Compiled onto
    /// Devin's `allowed_domains` allowlist.
    pub allowed_domains: Vec<String>,
}

/// Compile the manifest's `[[options.sandbox.network]]` rules into Devin's
/// egress vocabulary.
///
/// - No rules → deny-all (secure-by-default; the blueprint records an empty
///   allowlist plus a `denied_domains: ["*"]` catch-all so Devin's sandbox
///   denies egress rather than defaulting open).
/// - A `<host>:443` rule → an allowed-domain entry (native; Devin filters
///   per-domain, so the port is dropped — a declared lossiness).
/// - Any non-443 port → a hard error: Devin's domain filter cannot express a
///   port, and silently promoting the grant to all-ports (or dropping it) would
///   violate the "never silently widen or narrow grants" contract.
pub(crate) fn compile_devin_network_policy(
    rules: &[SandboxNetworkRule],
) -> Result<DevinNetworkPolicy> {
    if rules.is_empty() {
        return Ok(DevinNetworkPolicy {
            deny_all: true,
            allowed_domains: Vec::new(),
        });
    }
    let mut domains: Vec<String> = Vec::with_capacity(rules.len());
    for rule in rules {
        let (host, port) = split_endpoint(&rule.endpoint)?;
        if port != 443 {
            bail!(
                "The 'cognition-devin' sandbox backend expresses egress as allowed domains in the \
                 Devin blueprint, but rule '{endpoint}' targets port {port}.\n\
                 Devin's sandbox filters per-domain, not per-port; rewrite the endpoint as \
                 '{host}:443', or select a backend with per-port egress (e.g. 'openshell').",
                endpoint = rule.endpoint
            );
        }
        if !domains.contains(&host) {
            domains.push(host);
        }
    }
    Ok(DevinNetworkPolicy {
        deny_all: false,
        allowed_domains: domains,
    })
}

// ── Blueprint artifact generation ──────────────────────────────────────────────

/// Inputs to [`render_blueprint`].
pub(crate) struct BlueprintParams<'a> {
    /// Registry image reference Devin's build can pull / an operator can
    /// `docker load`.
    pub image_ref: &'a str,
    /// Compiled egress policy.
    pub network: &'a DevinNetworkPolicy,
}

/// Render the Devin blueprint hand-off artifact (pure function, no I/O).
///
/// The emitted `.devin/blueprint.yaml` has an `initialize` step that installs
/// Flox and activates the locked environment (so Devin's build reproduces the
/// closure in the snapshot), and records the compiled egress allowlist in a
/// `sandbox` block. The comment header states the hand-off contract and the
/// declared lossiness. Devin's blueprint is YAML with `#` comments.
pub(crate) fn render_blueprint(params: &BlueprintParams<'_>) -> String {
    let allowlist_yaml = yaml_str_list(&params.network.allowed_domains);
    // deny-by-default: an empty allowlist reads as "no restriction" to Devin, so
    // a `["*"]` denylist catch-all makes the intent explicit and deny-by-default.
    // When a grant set is present the allowlist itself is authoritative (only
    // matching domains pass), so no catch-all denylist is needed.
    let denied_yaml = if params.network.deny_all {
        "[\"*\"]".to_string()
    } else {
        "[]".to_string()
    };
    let policy_summary = if params.network.deny_all {
        "deny-all (no grants declared)".to_string()
    } else {
        format!("allowed: {}", params.network.allowed_domains.join(", "))
    };
    // image_ref comes from validated sources (repo + hash12 tag), so the
    // double-quoted YAML scalar is injection safe; yaml_str_list additionally
    // escapes each allowlist host.
    indoc::formatdoc! {r#"
        # Generated by `flox activate --sandbox --sandbox-backend cognition-devin`.
        # This is the git-backed blueprint hand-off for the Cognition (Devin
        # runtime) backend. Devin does not ingest an OCI image directly: its
        # builder runs this blueprint's `initialize` step to produce the snapshot
        # every session boots from (blueprint ~= Dockerfile, build ~= docker
        # build, snapshot ~= image). flox baked the environment image locally as
        # the reproducible substrate ("{image_ref}"); the initialize step below
        # installs Flox and activates the locked environment so Devin's build
        # reproduces the same closure.
        #
        # Commit this file as `.devin/blueprint.yaml` on the repo's default
        # branch, then sync + build it through Devin (API or UI).
        #
        # Egress is deny-by-default. The manifest's [[options.sandbox.network]]
        # grants are compiled into `sandbox.allowed_domains` below (Devin's
        # CLI-sandbox allowlist: only matching domains pass through the loopback
        # proxy). Devin filters per-domain, not per-port, so the grant's port is
        # dropped; access/protocol/binary scoping from the manifest is recorded
        # here but is NOT enforceable through the blueprint contract.
        #   policy: {policy_summary}
        initialize: |
          # Install Flox and activate the locked environment so the snapshot
          # boots with the same closure flox baked into "{image_ref}".
          curl -fsSL https://downloads.flox.dev/by-env/stable/deb/flox.x86_64-linux.deb -o /tmp/flox.deb
          sudo dpkg -i /tmp/flox.deb || sudo apt-get -f install -y
          flox activate -- true

        # Devin CLI-sandbox network policy. `allowed_domains` is an allowlist
        # (only matching domains pass); `denied_domains` always blocks and takes
        # precedence. flox authored this from the environment's declared grants.
        sandbox:
          allowed_domains: {allowlist_yaml}
          denied_domains: {denied_yaml}
          network_mode: "full"
        "#,
        image_ref = params.image_ref,
        policy_summary = policy_summary,
        allowlist_yaml = allowlist_yaml,
        denied_yaml = denied_yaml,
    }
}

// ── Launch path ────────────────────────────────────────────────────────────────

/// Bake the image, compile the policy, generate the blueprint artifact, then
/// fail at the launch boundary — never fake the remote snapshot build.
fn wrap_cognition_devin(
    dot_flox_path: &Path,
    env_name: &str,
    lockfile: &Lockfile,
    autobake: bool,
    container_builder_params: &ContainerBuilderParams,
) -> Result<Infallible> {
    let dot_flox =
        std::fs::canonicalize(dot_flox_path).unwrap_or_else(|_| dot_flox_path.to_path_buf());
    let project = dot_flox.parent().unwrap_or(&dot_flox).to_path_buf();

    // Bake and tag the image under Devin's own namespace
    // (`<env>-cognition-devin`), with the shared compat layer. The baked image
    // is the reproducible substrate; the blueprint's initialize step reproduces
    // the same closure inside Devin's snapshot.
    let repo = devin_repo(env_name);
    let hash12 = lockfile_hash12(lockfile);

    ensure_local_image(
        &repo,
        env_name,
        dot_flox_path,
        lockfile,
        autobake,
        container_builder_params,
        "Devin image",
    )?;

    // Compile the manifest network policy into Devin's egress vocabulary.
    let rules = manifest_network_rules(lockfile)?;
    let network = compile_devin_network_policy(&rules)?;

    // Build the blueprint artifact.
    let registry_prefix = std::env::var(FLOX_SANDBOX_COGNITION_DEVIN_REGISTRY_VAR)
        .ok()
        .filter(|v| !v.is_empty());
    let image_ref = registry_image_ref(&repo, &hash12, registry_prefix.as_deref());
    let blueprint = render_blueprint(&BlueprintParams {
        image_ref: &image_ref,
        network: &network,
    });
    let artifact_path = write_blueprint(&project, &blueprint)?;

    // Fail at the launch boundary with the concrete prerequisites. Name whether
    // a Devin CLI was detected so the message is precise about the subscription
    // wall.
    let cli_note = match first_on_path(DEVIN_CLI) {
        Some(_) => "the 'devin' CLI is installed, but ".to_string(),
        None => "no 'devin' CLI was found on PATH, and ".to_string(),
    };
    let registry_hint = match &registry_prefix {
        Some(prefix) => {
            let prefix = prefix.trim_end_matches('/');
            format!("tag and push it as '{prefix}/{repo}:{hash12}'")
        },
        None => format!(
            "set {FLOX_SANDBOX_COGNITION_DEVIN_REGISTRY_VAR}=<registry-prefix> and re-run, then push '<prefix>/{repo}:{hash12}'"
        ),
    };
    bail!(
        "The 'cognition-devin' sandbox backend hands the baked environment off to Cognition's \
         Devin runtime, which requires prerequisites this host cannot satisfy automatically:\n  \
         1. Push the baked image '{repo}:{hash12}' to a registry Devin's build can pull \
         ({registry_hint}).\n  \
         2. A Devin subscription and a partnership: {cli_note}Devin builds a snapshot from the \
         committed blueprint through its own builder — there is no public image-launch API. \
         A co-sell with Cognition's Sandbox/Infra team is the path to a backend-grade \
         integration.\n\
         flox generated the Devin blueprint hand-off at:\n  {artifact}\n\
         Commit it as '.devin/blueprint.yaml' on the repo's default branch, push the image, then \
         sync + build the snapshot through Devin.",
        artifact = artifact_path.display()
    )
}

/// Write the generated blueprint artifact to `<project>/.devin/blueprint.yaml`
/// and return its path.
///
/// The blueprint lives at the repo root (not under `.flox/cache/`) because Devin
/// reads `.devin/blueprint.yaml` from the repository it syncs — the artifact is
/// meant to be committed, like the Ona devcontainer, unlike the modal launcher
/// which is a local run script.
fn write_blueprint(project: &Path, blueprint: &str) -> Result<PathBuf> {
    let artifact_path = project.join(BLUEPRINT_REL_PATH);
    let dir = artifact_path
        .parent()
        .expect("blueprint path always has a parent");
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create Devin blueprint dir '{}'", dir.display()))?;
    std::fs::write(&artifact_path, blueprint).with_context(|| {
        format!(
            "failed to write Devin blueprint to '{}'",
            artifact_path.display()
        )
    })?;
    debug!(path = %artifact_path.display(), "wrote cognition-devin blueprint artifact");
    Ok(artifact_path)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use flox_core::activate::sandbox_policy::SandboxNetworkAccess;

    use super::*;

    // ── devin_repo ────────────────────────────────────────────────────────────

    #[test]
    fn repo_has_cognition_devin_suffix() {
        assert_eq!(devin_repo("myenv"), "myenv-cognition-devin");
    }

    #[test]
    fn repo_never_collides_with_other_backends() {
        let env = "myenv";
        let hash = "abc123def456";
        let oci = format!("{env}:{hash}");
        let ona = format!("{env}-ona:{hash}");
        let daytona = format!("{env}-daytona:{hash}");
        let devin = format!("{}:{hash}", devin_repo(env));
        assert_ne!(devin, oci);
        assert_ne!(devin, ona);
        assert_ne!(devin, daytona);
    }

    // ── compile_devin_network_policy ──────────────────────────────────────────

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
        let policy = compile_devin_network_policy(&[]).unwrap();
        assert_eq!(policy, DevinNetworkPolicy {
            deny_all: true,
            allowed_domains: Vec::new(),
        });
    }

    #[test]
    fn tls_443_rules_compile_to_allowed_domains() {
        let rules = [rule("api.github.com:443"), rule("api.anthropic.com:443")];
        let policy = compile_devin_network_policy(&rules).unwrap();
        assert_eq!(policy, DevinNetworkPolicy {
            deny_all: false,
            allowed_domains: vec![
                "api.github.com".to_string(),
                "api.anthropic.com".to_string(),
            ],
        });
    }

    #[test]
    fn duplicate_hosts_are_deduplicated() {
        let rules = [rule("api.github.com:443"), rule("api.github.com:443")];
        let policy = compile_devin_network_policy(&rules).unwrap();
        assert_eq!(policy.allowed_domains, vec!["api.github.com".to_string()]);
    }

    #[test]
    fn wildcard_host_is_preserved() {
        let policy = compile_devin_network_policy(&[rule("*.github.com:443")]).unwrap();
        assert_eq!(policy.allowed_domains, vec!["*.github.com".to_string()]);
    }

    #[test]
    fn access_and_binary_do_not_affect_compilation() {
        // The blueprint hand-off carries no method distinction; a scoped grant
        // compiles identically to an unscoped one (declared lossiness).
        let scoped = SandboxNetworkRule {
            endpoint: "api.github.com:443".to_string(),
            access: Some(SandboxNetworkAccess::ReadOnly),
            protocol: None,
            binary: Some("curl".to_string()),
        };
        let policy = compile_devin_network_policy(&[scoped]).unwrap();
        assert_eq!(policy, DevinNetworkPolicy {
            deny_all: false,
            allowed_domains: vec!["api.github.com".to_string()],
        });
    }

    #[test]
    fn non_443_port_is_rejected() {
        let err = compile_devin_network_policy(&[rule("db.example.com:5432")]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("per-domain, not per-port"), "got: {msg}");
        assert!(msg.contains("db.example.com:443"), "got: {msg}");
    }

    #[test]
    fn endpoint_without_port_is_rejected() {
        let err = compile_devin_network_policy(&[rule("example.com")]).unwrap_err();
        assert!(err.to_string().contains("<HOST>:<PORT>"), "got: {err}");
    }

    #[test]
    fn endpoint_with_invalid_host_is_rejected() {
        let err = compile_devin_network_policy(&[rule("bad host\nhost:443")]).unwrap_err();
        assert!(
            err.to_string().contains("Invalid sandbox network endpoint"),
            "got: {err}"
        );
    }

    // ── render_blueprint ──────────────────────────────────────────────────────

    fn deny_all_policy() -> DevinNetworkPolicy {
        DevinNetworkPolicy {
            deny_all: true,
            allowed_domains: Vec::new(),
        }
    }

    #[test]
    fn blueprint_deny_all_has_empty_allowlist_and_catch_all_denylist() {
        let doc = render_blueprint(&BlueprintParams {
            image_ref: "myenv-cognition-devin:abc123",
            network: &deny_all_policy(),
        });
        // Deny-all: empty allowlist plus explicit `["*"]` denylist so Devin does
        // not default open on an empty allowlist.
        assert!(doc.contains("allowed_domains: []"), "got:\n{doc}");
        assert!(doc.contains("denied_domains: [\"*\"]"), "got:\n{doc}");
        assert!(doc.contains("network_mode: \"full\""), "got:\n{doc}");
        assert!(
            doc.contains("deny-all (no grants declared)"),
            "policy summary comment missing:\n{doc}"
        );
        assert!(
            doc.contains("myenv-cognition-devin:abc123"),
            "image ref missing:\n{doc}"
        );
        // The initialize step installs Flox and activates the environment so
        // Devin's build reproduces the closure in the snapshot.
        assert!(
            doc.contains("initialize: |") && doc.contains("flox activate"),
            "initialize must activate the environment:\n{doc}"
        );
    }

    #[test]
    fn blueprint_allowlist_rendered() {
        let net = DevinNetworkPolicy {
            deny_all: false,
            allowed_domains: vec!["api.github.com".to_string(), "*.anthropic.com".to_string()],
        };
        let doc = render_blueprint(&BlueprintParams {
            image_ref: "env-cognition-devin:tag",
            network: &net,
        });
        assert!(
            doc.contains("allowed_domains: [\"api.github.com\", \"*.anthropic.com\"]"),
            "got:\n{doc}"
        );
        // A grant set uses the allowlist as authoritative; the denylist is empty.
        assert!(doc.contains("denied_domains: []"), "got:\n{doc}");
        assert!(
            doc.contains("allowed: api.github.com, *.anthropic.com"),
            "policy summary comment missing:\n{doc}"
        );
    }

    #[test]
    fn blueprint_header_states_handoff_contract() {
        let doc = render_blueprint(&BlueprintParams {
            image_ref: "env-cognition-devin:tag",
            network: &deny_all_policy(),
        });
        assert!(
            doc.starts_with(
                "# Generated by `flox activate --sandbox --sandbox-backend cognition-devin`."
            ),
            "got:\n{doc}"
        );
        // The header must name the blueprint contract: this is not an image
        // hand-off — Devin builds a snapshot from the blueprint.
        assert!(
            doc.contains(".devin/blueprint.yaml"),
            "blueprint contract missing:\n{doc}"
        );
        assert!(
            doc.contains("does not ingest an OCI image directly"),
            "inversion note missing:\n{doc}"
        );
    }
}
