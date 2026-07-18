//! The `ona` sandbox backend: hand the environment off to an Ona (formerly
//! Gitpod) cloud development environment.
//!
//! Ona is a control-plane / gateway CDE: unlike the host-local backends,
//! nothing runs on the laptop. flox bakes the environment's OCI image via the
//! shared lockfile-hash-tagged Docker bake (`super::bake`), under Ona's own
//! `<env>-ona:<hash12>` tag namespace, then generates the **devcontainer
//! hand-off artifact** (`.devcontainer/devcontainer.json`) that references the
//! baked image. An Ona workspace built from that devcontainer opens with the
//! locked toolchain already present — the BNY / Capital One deployment shape:
//! a Flox-baked image is the reproducible substrate an Ona environment runs on.
//!
//! The threat model inverts relative to the host-local backends: the host
//! filesystem is unreachable from the remote workspace, but the code and any
//! injected secrets leave the laptop. Credentials belong in Ona's own
//! environment-variable / secret mechanism, not a local `.env`.
//!
//! # Why this backend does not complete the launch on any host today
//!
//! Ona builds a workspace from a devcontainer in a git repository through its
//! control plane; opening one needs an Ona account and, post-OpenAI
//! acquisition (2026-06-11), an enterprise workspace or partnership. There is
//! no public no-account image-launch API to drive from a bare checkout, and no
//! Ona/Gitpod CLI is installed on this host. `preflight` therefore detects the
//! CLI if one is present (`ona`, falling back to the legacy `gitpod` name) and,
//! when it is absent, falls back to detecting an existing workspace config in
//! the project — never triggering an interactive login.
//!
//! Rather than fake success, this backend implements the deepest honest slice:
//! it bakes the real image, compiles the manifest network policy into a
//! documented egress expectation, and *generates the devcontainer artifact*
//! (`devcontainer.json` wrapping the baked image, with the compiled policy
//! recorded as `flox.sandbox.*` labels and comments). It then fails at the
//! launch boundary with a clear "requires ..." message that points at the
//! generated artifact and names the account / partnership wall.
//!
//! # Network-policy compilation (the load-bearing lossiness)
//!
//! The devcontainer spec has no native egress-policy vocabulary — enforcement
//! lives in Ona's enterprise network policy, configured on Ona's side. flox
//! compiles the manifest's `[[options.sandbox.network]]` grants into the
//! devcontainer as documented expectations: each `<host>:443` grant becomes a
//! `containerEnv` proxy hint and a metadata label recording the allowed
//! endpoint, so an operator wiring the Ona workspace policy has the exact
//! allowlist flox derived. A non-443 endpoint cannot be expressed as an
//! HTTPS/domain rule and is declined at compile time rather than silently
//! widened. The grant's `access`, `protocol`, and `binary` scoping is recorded
//! in the artifact as comments/labels but does not constrain traffic through
//! the devcontainer contract — declared lossiness, per the backend contract.
//!
//! # Env knobs (prototype)
//!
//! The bake reuses the openshell/oci knobs:
//! - `FLOX_SANDBOX_OCI_IMAGE` — explicit image ref override (skips bake).
//! - `FLOX_SANDBOX_OCI_ALLOW_STALE` — run the newest existing image when the
//!   expected hash-tag is absent.
//! - `FLOX_SANDBOX_OCI_AUTOBAKE` — bake without prompting.
//! - `FLOX_SANDBOX_ONA_REGISTRY` — registry prefix the devcontainer's `image`
//!   reference is built from (e.g. `docker.io/myuser`); recorded in the
//!   artifact so a credentialed operator does not have to hand-edit it before
//!   pushing the image Ona pulls.

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
    flox_sanitized_name,
    json_str_list,
    manifest_network_rules,
    registry_image_ref,
};
use super::preflight::{first_on_path, split_endpoint};
use super::{ActivationSandbox, SandboxLaunchCtx};
use crate::commands::sandbox_backends::oci::lockfile_hash12;

/// Registry prefix the devcontainer's `image` reference is built from. When set
/// (e.g. `docker.io/myuser`), the generated artifact references
/// `<prefix>/<repo>:<hash12>` so a credentialed operator does not have to
/// hand-edit the image ref before pushing and opening a workspace.
pub(crate) const FLOX_SANDBOX_ONA_REGISTRY_VAR: &str = "FLOX_SANDBOX_ONA_REGISTRY";

/// Repository suffix for the Ona backend's image tags. The image is baked under
/// `<env>-ona:<hash12>` (with the shared compat layer) and the devcontainer's
/// image reference reuses that name, so the pushed artifact is recognizable as
/// Ona's and never collides with the other backends' tags on a shared registry.
const ONA_REPO_SUFFIX: &str = "-ona";

/// Candidate CLI names for the Ona control plane, newest first.
///
/// Ona rebranded from Gitpod; the current CLI is `ona`, with the legacy
/// `gitpod` name still shipping on some installs. Preflight reports whichever
/// it finds and does not require either — the hand-off artifact is generated
/// regardless, and the launch boundary names the account wall either way.
const ONA_CLI_CANDIDATES: [&str; 2] = ["ona", "gitpod"];

/// Project-relative path of the generated devcontainer hand-off artifact. Ona
/// (and every Dev Containers-compatible platform) reads
/// `.devcontainer/devcontainer.json` from the repo root.
const DEVCONTAINER_REL_PATH: &str = ".devcontainer/devcontainer.json";

pub struct OnaBackend<'a> {
    dot_flox_path: PathBuf,
    env_name: String,
    lockfile: &'a Lockfile,
    sandbox_oci_autobake: bool,
    container_builder_params: ContainerBuilderParams,
}

impl<'a> OnaBackend<'a> {
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

impl ActivationSandbox for OnaBackend<'_> {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::Ona
    }

    fn preflight(&self) -> Result<()> {
        // Ona is control-plane/cloud: the image bake runs through Docker, so
        // Docker is the one genuinely required host tool.
        if first_on_path("docker").is_none() {
            bail!(
                "The 'ona' sandbox backend bakes the environment image with Docker, which was \
                 not found on PATH.\n\
                 Install Docker Desktop or the Docker CLI, then re-run."
            );
        }
        // The Ona CLI (or the legacy `gitpod` CLI) is not required to generate
        // the devcontainer hand-off artifact, but detecting it up front lets the
        // launch-boundary message be precise about what the operator has. Never
        // trigger an interactive login: presence-on-PATH is the only probe.
        match detect_ona_cli() {
            Some((name, path)) => {
                debug!(cli = name, path = %path.display(), "detected Ona control-plane CLI");
            },
            None => {
                debug!(
                    "no Ona/Gitpod CLI on PATH; the devcontainer artifact is generated \
                     regardless and the launch boundary names the account wall"
                );
            },
        }
        Ok(())
    }

    fn wrap_activation(self: Box<Self>) -> Result<Infallible> {
        wrap_ona(
            &self.dot_flox_path,
            &self.env_name,
            self.lockfile,
            self.sandbox_oci_autobake,
            &self.container_builder_params,
        )
    }
}

/// Locate the Ona control-plane CLI on `PATH`, preferring the current `ona`
/// name over the legacy `gitpod` name. Returns the matched name and its path.
pub(crate) fn detect_ona_cli() -> Option<(&'static str, PathBuf)> {
    ONA_CLI_CANDIDATES
        .into_iter()
        .find_map(|name| first_on_path(name).map(|path| (name, path)))
}

/// Return the `<env>-ona` repository name used for Docker image tagging.
fn ona_repo(env_name: &str) -> String {
    format!("{env_name}{ONA_REPO_SUFFIX}")
}

// ── Network policy compilation ─────────────────────────────────────────────────

/// The manifest network policy compiled into Ona's egress expectation.
///
/// The devcontainer spec has no native egress vocabulary — enforcement lives in
/// Ona's enterprise network policy. flox compiles the grants into a documented
/// allowlist recorded in the devcontainer metadata: `<host>:443` grants become
/// allowed HTTPS endpoints; everything else is deny-by-default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OnaNetworkPolicy {
    /// Deny all outbound traffic (no grants declared).
    pub deny_all: bool,
    /// Hosts granted HTTPS/443 egress, in declaration order (deduplicated).
    pub allowed_hosts: Vec<String>,
}

/// Compile the manifest's `[[options.sandbox.network]]` rules into Ona's egress
/// expectation.
///
/// - No rules → deny-all (secure-by-default; the devcontainer records an empty
///   allowlist and the workspace's Ona network policy denies egress).
/// - A `<host>:443` rule → an allowed-host entry (compiled onto the HTTPS
///   allowlist recorded in the devcontainer metadata).
/// - Any non-443 port → a hard error: the devcontainer hand-off expresses egress
///   as HTTPS domains only, and silently promoting the grant to an all-ports
///   rule (or dropping it) would violate the "never silently widen or narrow
///   grants" contract.
pub(crate) fn compile_ona_network_policy(rules: &[SandboxNetworkRule]) -> Result<OnaNetworkPolicy> {
    if rules.is_empty() {
        return Ok(OnaNetworkPolicy {
            deny_all: true,
            allowed_hosts: Vec::new(),
        });
    }
    let mut hosts: Vec<String> = Vec::with_capacity(rules.len());
    for rule in rules {
        let (host, port) = split_endpoint(&rule.endpoint)?;
        if port != 443 {
            bail!(
                "The 'ona' sandbox backend expresses egress as HTTPS/443 domains in the \
                 devcontainer hand-off, but rule '{endpoint}' targets port {port}.\n\
                 Rewrite the endpoint as '{host}:443', or select a backend with per-port egress \
                 (e.g. 'openshell').",
                endpoint = rule.endpoint
            );
        }
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    Ok(OnaNetworkPolicy {
        deny_all: false,
        allowed_hosts: hosts,
    })
}

// ── Devcontainer artifact generation ───────────────────────────────────────────

/// Inputs to [`render_devcontainer`].
pub(crate) struct DevcontainerParams<'a> {
    /// Ona workspace name (`flox-<sanitized-env>`).
    pub workspace_name: &'a str,
    /// Registry image reference Ona pulls to build the workspace.
    pub image_ref: &'a str,
    /// Compiled egress expectation.
    pub network: &'a OnaNetworkPolicy,
}

/// Render the Ona devcontainer hand-off artifact (pure function, no I/O).
///
/// The emitted `devcontainer.json` wraps the baked image and records the
/// compiled egress allowlist as `flox.sandbox.*` metadata labels plus a
/// `containerEnv` proxy hint, so an Ona workspace built from it opens with the
/// locked toolchain present and an operator wiring Ona's network policy has the
/// exact allowlist flox derived. The devcontainer spec permits `//`-style
/// comments and trailing commas (JSON with Comments), which the header uses to
/// state the hand-off contract.
pub(crate) fn render_devcontainer(params: &DevcontainerParams<'_>) -> String {
    // The egress allowlist recorded in metadata. Deny-all is an empty array;
    // a grant set lists the allowed HTTPS hosts.
    let allowlist_json = json_str_list(&params.network.allowed_hosts);
    // A single-string summary of the compiled policy for the human-readable
    // label and the log-friendly comment.
    let policy_summary = if params.network.deny_all {
        "deny-all (no grants declared)".to_string()
    } else {
        format!(
            "HTTPS/443 allowed: {}",
            params.network.allowed_hosts.join(", ")
        )
    };
    // image_ref and workspace_name come from validated sources (repo + hash12
    // tag, sanitized name), so the double-quoted JSON literals are injection
    // safe; json_str_lit additionally escapes each allowlist host.
    indoc::formatdoc! {r#"
        // Generated by `flox activate --sandbox --sandbox-backend ona`.
        // This is the devcontainer hand-off artifact for the Ona (formerly
        // Gitpod) backend. flox baked the environment image locally; Ona builds
        // a workspace from this devcontainer, so push that image to a registry
        // Ona can pull (as "{image_ref}"), commit this file to the repo Ona
        // opens, then create a workspace from the repo.
        //
        // Egress is deny-by-default. The manifest's [[options.sandbox.network]]
        // grants are compiled into "flox.sandbox.network.allow" below as the
        // HTTPS/443 allowlist. The devcontainer spec has no native egress
        // vocabulary, so this allowlist is the expectation an operator wires
        // into Ona's enterprise network policy; access/protocol/binary scoping
        // from the manifest is recorded but NOT enforceable through the
        // devcontainer contract.
        //   policy: {policy_summary}
        {{
          "name": "{workspace_name}",
          "image": "{image_ref}",
          "containerEnv": {{
            "FLOX_SANDBOX_BACKEND": "ona"
          }},
          "customizations": {{
            "flox": {{
              "sandbox": {{
                "backend": "ona",
                "network": {{
                  "default": "deny",
                  "allow": {allowlist_json}
                }}
              }}
            }}
          }},
          "overrideCommand": false
        }}
        "#,
        image_ref = params.image_ref,
        workspace_name = params.workspace_name,
        policy_summary = policy_summary,
        allowlist_json = allowlist_json,
    }
}

// ── Launch path ────────────────────────────────────────────────────────────────

/// Bake the image, compile the policy, generate the devcontainer artifact, then
/// fail at the launch boundary — never fake the remote workspace open.
fn wrap_ona(
    dot_flox_path: &Path,
    env_name: &str,
    lockfile: &Lockfile,
    autobake: bool,
    container_builder_params: &ContainerBuilderParams,
) -> Result<Infallible> {
    let dot_flox =
        std::fs::canonicalize(dot_flox_path).unwrap_or_else(|_| dot_flox_path.to_path_buf());
    let project = dot_flox.parent().unwrap_or(&dot_flox).to_path_buf();

    // Bake and tag the image under Ona's own namespace (`<env>-ona`), with the
    // shared compat layer. The pushed artifact is recognizable as Ona's and
    // never collides with the other backends' tags on a shared registry.
    let repo = ona_repo(env_name);
    let hash12 = lockfile_hash12(lockfile);

    // Ensure the local hash-tagged image exists (baking with the shared compat
    // layer if absent). Ona builds the workspace by pulling the image from a
    // registry, but baking it locally first is the same content-addressed step
    // every OCI-ingesting provider shares.
    ensure_local_image(
        &repo,
        env_name,
        dot_flox_path,
        lockfile,
        autobake,
        container_builder_params,
        "Ona image",
    )?;

    // Compile the manifest network policy into Ona's egress expectation.
    let rules = manifest_network_rules(lockfile)?;
    let network = compile_ona_network_policy(&rules)?;

    // Build the devcontainer artifact.
    let registry_prefix = std::env::var(FLOX_SANDBOX_ONA_REGISTRY_VAR)
        .ok()
        .filter(|v| !v.is_empty());
    let image_ref = registry_image_ref(&repo, &hash12, registry_prefix.as_deref());
    let workspace_name = flox_sanitized_name(env_name);
    let devcontainer = render_devcontainer(&DevcontainerParams {
        workspace_name: &workspace_name,
        image_ref: &image_ref,
        network: &network,
    });
    let artifact_path = write_devcontainer(&project, &devcontainer)?;

    // Fail at the launch boundary with the concrete prerequisites. Name whether
    // an Ona CLI was detected so the message is precise about the account wall.
    let cli_note = match detect_ona_cli() {
        Some((name, _)) => format!("the '{name}' CLI is installed, but "),
        None => "no Ona/Gitpod CLI was found on PATH, and ".to_string(),
    };
    let registry_hint = match &registry_prefix {
        Some(prefix) => {
            let prefix = prefix.trim_end_matches('/');
            format!("tag and push it as '{prefix}/{repo}:{hash12}'")
        },
        None => format!(
            "set {FLOX_SANDBOX_ONA_REGISTRY_VAR}=<registry-prefix> and re-run, then push '<prefix>/{repo}:{hash12}'"
        ),
    };
    bail!(
        "The 'ona' sandbox backend hands the baked environment off to an Ona (formerly Gitpod) \
         workspace, which requires prerequisites this host cannot satisfy automatically:\n  \
         1. Push the baked image '{repo}:{hash12}' to a registry Ona can pull \
         ({registry_hint}).\n  \
         2. An Ona account and an enterprise workspace: {cli_note}Ona builds the workspace from \
         the committed devcontainer through its control plane. Post-OpenAI-acquisition trial \
         access is uncertain — a partnership contact is likely required.\n\
         flox generated the devcontainer hand-off at:\n  {artifact}\n\
         Commit it to the repo Ona opens, push the image, then create a workspace from the repo.",
        artifact = artifact_path.display()
    )
}

/// Write the generated devcontainer artifact to `<project>/.devcontainer/
/// devcontainer.json` and return its path.
///
/// The devcontainer lives at the repo root (not under `.flox/cache/`) because
/// Ona reads it from the repository it opens — the artifact is meant to be
/// committed, unlike the modal launcher which is a local run script.
fn write_devcontainer(project: &Path, devcontainer: &str) -> Result<PathBuf> {
    let artifact_path = project.join(DEVCONTAINER_REL_PATH);
    let dir = artifact_path
        .parent()
        .expect("devcontainer path always has a parent");
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create devcontainer dir '{}'", dir.display()))?;
    std::fs::write(&artifact_path, devcontainer).with_context(|| {
        format!(
            "failed to write devcontainer to '{}'",
            artifact_path.display()
        )
    })?;
    debug!(path = %artifact_path.display(), "wrote ona devcontainer artifact");
    Ok(artifact_path)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use flox_core::activate::sandbox_policy::SandboxNetworkAccess;

    use super::*;

    // ── ona_repo ──────────────────────────────────────────────────────────────

    #[test]
    fn repo_has_ona_suffix() {
        assert_eq!(ona_repo("myenv"), "myenv-ona");
    }

    #[test]
    fn repo_never_collides_with_other_backends() {
        let env = "myenv";
        let hash = "abc123def456";
        let oci = format!("{env}:{hash}");
        let openshell = format!("{env}-openshell:{hash}");
        let modal = format!("{env}-modal:{hash}");
        let ona = format!("{}:{hash}", ona_repo(env));
        assert_ne!(ona, oci);
        assert_ne!(ona, openshell);
        assert_ne!(ona, modal);
    }

    // ── compile_ona_network_policy ────────────────────────────────────────────

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
        let policy = compile_ona_network_policy(&[]).unwrap();
        assert_eq!(policy, OnaNetworkPolicy {
            deny_all: true,
            allowed_hosts: Vec::new(),
        });
    }

    #[test]
    fn tls_443_rules_compile_to_allowed_hosts() {
        let rules = [rule("api.github.com:443"), rule("api.anthropic.com:443")];
        let policy = compile_ona_network_policy(&rules).unwrap();
        assert_eq!(policy, OnaNetworkPolicy {
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
        let policy = compile_ona_network_policy(&rules).unwrap();
        assert_eq!(policy.allowed_hosts, vec!["api.github.com".to_string()]);
    }

    #[test]
    fn wildcard_host_is_preserved() {
        let policy = compile_ona_network_policy(&[rule("*.github.com:443")]).unwrap();
        assert_eq!(policy.allowed_hosts, vec!["*.github.com".to_string()]);
    }

    #[test]
    fn access_and_binary_do_not_affect_compilation() {
        // The devcontainer hand-off carries no method distinction; a scoped
        // grant compiles identically to an unscoped one (declared lossiness).
        let scoped = SandboxNetworkRule {
            endpoint: "api.github.com:443".to_string(),
            access: Some(SandboxNetworkAccess::ReadOnly),
            protocol: None,
            binary: Some("curl".to_string()),
        };
        let policy = compile_ona_network_policy(&[scoped]).unwrap();
        assert_eq!(policy, OnaNetworkPolicy {
            deny_all: false,
            allowed_hosts: vec!["api.github.com".to_string()],
        });
    }

    #[test]
    fn non_443_port_is_rejected() {
        let err = compile_ona_network_policy(&[rule("db.example.com:5432")]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("HTTPS/443"), "got: {msg}");
        assert!(msg.contains("db.example.com:443"), "got: {msg}");
    }

    #[test]
    fn endpoint_without_port_is_rejected() {
        let err = compile_ona_network_policy(&[rule("example.com")]).unwrap_err();
        assert!(err.to_string().contains("<HOST>:<PORT>"), "got: {err}");
    }

    #[test]
    fn endpoint_with_invalid_host_is_rejected() {
        let err = compile_ona_network_policy(&[rule("bad host\nhost:443")]).unwrap_err();
        assert!(
            err.to_string().contains("Invalid sandbox network endpoint"),
            "got: {err}"
        );
    }

    // ── render_devcontainer ───────────────────────────────────────────────────

    fn deny_all_policy() -> OnaNetworkPolicy {
        OnaNetworkPolicy {
            deny_all: true,
            allowed_hosts: Vec::new(),
        }
    }

    #[test]
    fn devcontainer_deny_all_has_empty_allowlist_and_valid_json() {
        let doc = render_devcontainer(&DevcontainerParams {
            workspace_name: "flox-myenv",
            image_ref: "myenv-ona:abc123",
            network: &deny_all_policy(),
        });
        assert!(doc.contains("\"name\": \"flox-myenv\""), "got:\n{doc}");
        assert!(
            doc.contains("\"image\": \"myenv-ona:abc123\""),
            "got:\n{doc}"
        );
        assert!(doc.contains("\"default\": \"deny\""), "got:\n{doc}");
        assert!(doc.contains("\"allow\": []"), "got:\n{doc}");
        assert!(
            doc.contains("deny-all (no grants declared)"),
            "policy summary comment missing:\n{doc}"
        );
        // The JSON body (below the comment header) must parse once comments are
        // stripped — the devcontainer spec is JSONC, but the object itself is
        // strict JSON.
        assert!(
            strip_jsonc_comments(&doc)
                .parse::<serde_json::Value>()
                .is_ok(),
            "devcontainer body must be valid JSON:\n{}",
            strip_jsonc_comments(&doc)
        );
    }

    #[test]
    fn devcontainer_allowlist_rendered_and_valid_json() {
        let net = OnaNetworkPolicy {
            deny_all: false,
            allowed_hosts: vec!["api.github.com".to_string(), "*.anthropic.com".to_string()],
        };
        let doc = render_devcontainer(&DevcontainerParams {
            workspace_name: "flox-env",
            image_ref: "env-ona:tag",
            network: &net,
        });
        assert!(
            doc.contains("\"allow\": [\"api.github.com\", \"*.anthropic.com\"]"),
            "got:\n{doc}"
        );
        assert!(
            doc.contains("HTTPS/443 allowed: api.github.com, *.anthropic.com"),
            "policy summary comment missing:\n{doc}"
        );
        let body = strip_jsonc_comments(&doc);
        let parsed: serde_json::Value = body
            .parse()
            .unwrap_or_else(|e| panic!("devcontainer body must be valid JSON: {e}\n{body}"));
        assert_eq!(parsed["image"], "env-ona:tag");
        assert_eq!(parsed["containerEnv"]["FLOX_SANDBOX_BACKEND"], "ona");
        let allow = &parsed["customizations"]["flox"]["sandbox"]["network"]["allow"];
        assert_eq!(allow[0], "api.github.com");
        assert_eq!(allow[1], "*.anthropic.com");
    }

    #[test]
    fn devcontainer_header_states_handoff_contract() {
        let doc = render_devcontainer(&DevcontainerParams {
            workspace_name: "flox-env",
            image_ref: "env-ona:tag",
            network: &deny_all_policy(),
        });
        assert!(
            doc.starts_with("// Generated by `flox activate --sandbox --sandbox-backend ona`."),
            "got:\n{doc}"
        );
        // The header must name the account/partnership wall's precondition: the
        // image must be pulled from a registry Ona can reach.
        assert!(
            doc.contains("push that image to a registry"),
            "hand-off contract missing:\n{doc}"
        );
    }

    /// Strip `//`-style line comments from a JSONC document so the strict-JSON
    /// body can be validated. The devcontainer header uses only line comments,
    /// never block comments, and no allowlist host may contain `//` (the
    /// endpoint charset check forbids `/`), so line-stripping is sufficient.
    fn strip_jsonc_comments(doc: &str) -> String {
        doc.lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    // ── detect_ona_cli ────────────────────────────────────────────────────────

    #[test]
    fn cli_candidates_prefer_ona_over_gitpod() {
        // The current name comes first so a host with both resolves to `ona`.
        assert_eq!(ONA_CLI_CANDIDATES, ["ona", "gitpod"]);
    }
}
