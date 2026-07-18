//! The `e2b` sandbox backend: run the environment in a remote E2B sandbox.
//!
//! E2B is a cloud-API sandbox provider. Like the other cloud backends, nothing
//! runs on the host: flox bakes the environment's OCI image via the shared
//! lockfile-hash-tagged Docker bake (`super::bake`), under E2B's own
//! `<env>-e2b:<hash12>` tag namespace, then generates the **E2B template
//! hand-off** — an `e2b.Dockerfile` whose `FROM` is the baked image plus an
//! `e2b.toml` template config. `e2b template build` turns that into a sandbox
//! template, and a sandbox launched from the template (SDK/CLI) runs the locked
//! toolchain. The threat model inverts relative to the host-local backends: the
//! host filesystem is unreachable from the remote sandbox, but the code and any
//! injected secrets leave the laptop. Credentials belong in E2B's own secret
//! mechanism, not a local `.env`.
//!
//! # Why this backend does not complete the launch on any host today
//!
//! Two external prerequisites gate the remote launch, and neither can be
//! satisfied from a bare checkout:
//!
//! - **An E2B account and API key.** The E2B SDK authenticates with
//!   `E2B_API_KEY` and the CLI with `E2B_ACCESS_TOKEN` / `e2b auth login`;
//!   without either, every API call fails. `preflight` distinguishes
//!   *CLI-missing* from *CLI-present-but-unauthenticated* cheaply and
//!   non-interactively (`e2b auth info` reports the signed-in state and never
//!   opens a browser; env-key presence is accepted as a fallback signal).
//! - **A built template.** E2B ingests images as the `FROM` base of an
//!   `e2b.Dockerfile`, built through E2B's builder with `e2b template build`.
//!   There is no local-Docker-daemon or tarball ingestion path, so the locally
//!   baked `<env>-e2b:<hash12>` image must be referenced from the generated
//!   Dockerfile and built into a template before a sandbox can be created.
//!
//! Rather than fake success, this backend implements the deepest honest slice:
//! it runs the real preflight, bakes the real image, compiles the manifest
//! network policy into E2B's egress vocabulary, and *generates the template
//! artifacts* (`e2b.Dockerfile` + `e2b.toml`). It then fails at the launch
//! boundary with a clear "requires ..." error that points at the generated
//! artifacts and names the two missing prerequisites.
//!
//! # Network-policy compilation (the load-bearing lossiness)
//!
//! E2B's network default is the opposite of every other backend here:
//! `allowInternetAccess=true` — a *default-open* egress posture. So the compile
//! must ALWAYS emit an explicit deny posture first and add only the manifest's
//! allowlist on top; the absence of grants is NOT sufficient to close egress on
//! E2B the way an empty allowlist closes it on the container backends.
//!
//! E2B's host/SNI filtering covers ports 80 and 443 only, and does not filter
//! QUIC/UDP. A `<host>:443` (or `<host>:80`) grant compiles onto the host
//! allowlist (native, faithful); any other port cannot be expressed as a
//! host/SNI rule and is declined with a clear error rather than silently
//! widened. The 80/443-only ceiling and the unfiltered-QUIC gap are declared
//! lossiness, per the backend contract. The grant's `access`, `protocol`, and
//! `binary` scoping is recorded in the template artifact as comments but does
//! not constrain traffic.
//!
//! # Live network redemption (`updateNetwork`)
//!
//! E2B exposes `updateNetwork` on a *running* sandbox — a replace-not-merge
//! update of the network policy without recreating the sandbox. That is the one
//! true live network-grant redemption in the cloud tier: unlike Modal/Ona,
//! where widening egress requires recreating the sandbox/workspace, an E2B
//! sandbox can have its allowlist replaced live. The capabilities row still
//! declares `live_ask = false`, because `updateNetwork` is an
//! operator-initiated policy *replacement*, not a per-request adjudication of a
//! specific out-of-policy access (which is what the contract's `live_ask`
//! means). The generated `e2b.toml` records the deny-by-default posture and the
//! allowlist an operator would apply live via `updateNetwork`.
//!
//! # Env knobs (prototype)
//!
//! The bake reuses the openshell/oci knobs:
//! - `FLOX_SANDBOX_OCI_IMAGE` — explicit image ref override (skips bake).
//! - `FLOX_SANDBOX_OCI_ALLOW_STALE` — run the newest existing image when the
//!   expected hash-tag is absent.
//! - `FLOX_SANDBOX_OCI_AUTOBAKE` — bake without prompting.
//! - `FLOX_SANDBOX_E2B_REGISTRY` — registry prefix the generated
//!   `e2b.Dockerfile`'s `FROM` reference is built from (e.g.
//!   `docker.io/myuser`); recorded in the artifact so a credentialed operator
//!   does not have to hand-edit it before pushing the image E2B's builder pulls.

use std::convert::Infallible;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flox_core::activate::sandbox_backend::SandboxBackend;
use flox_core::activate::sandbox_policy::SandboxNetworkRule;
use flox_manifest::lockfile::Lockfile;
use flox_rust_sdk::providers::container_builder::ContainerBuilderParams;
use semver::Version;
use tracing::debug;

use super::handoff::{
    ensure_local_image,
    flox_sanitized_name,
    manifest_network_rules,
    registry_image_ref,
    toml_str_list,
    toml_str_lit,
};
use super::preflight::{
    CliVersionCheck,
    DEFAULT_VERSION_ARGS,
    check_cli_version,
    first_on_path,
    split_endpoint,
};
use super::{ActivationSandbox, SandboxLaunchCtx};
use crate::commands::sandbox_backends::oci::lockfile_hash12;

/// Registry prefix the generated `e2b.Dockerfile`'s `FROM` reference is built
/// from. When set (e.g. `docker.io/myuser`), the artifact references
/// `<prefix>/<repo>:<hash12>` so a credentialed operator does not have to
/// hand-edit the image ref before pushing it and building the template.
pub(crate) const FLOX_SANDBOX_E2B_REGISTRY_VAR: &str = "FLOX_SANDBOX_E2B_REGISTRY";

/// SDK API-key environment variable E2B authenticates with.
const E2B_API_KEY_VAR: &str = "E2B_API_KEY";

/// CLI access-token environment variable E2B authenticates with.
const E2B_ACCESS_TOKEN_VAR: &str = "E2B_ACCESS_TOKEN";

/// Repository suffix for the E2B backend's image tags. The image is baked under
/// `<env>-e2b:<hash12>` (with the shared compat layer) and the generated
/// Dockerfile's `FROM` reuses that name, so the pushed artifact is recognizable
/// as E2B's and never collides with the other backends' tags on a shared
/// registry.
const E2B_REPO_SUFFIX: &str = "-e2b";

/// Minimum supported E2B CLI version.
///
/// The Dockerfile-based template flow (`e2b template build` reading an
/// `e2b.Dockerfile`) is the surface the generated artifact targets. Pinned
/// conservatively to a 1.0 floor; the shared version gate tolerates an
/// unparseable/failed `--version` so an unusual build never blocks.
const E2B_MIN_VERSION: Version = Version::new(1, 0, 0);

/// Project-relative filename of the generated E2B Dockerfile. `e2b template
/// build` looks for `e2b.Dockerfile` in the build context by default.
const E2B_DOCKERFILE_NAME: &str = "e2b.Dockerfile";

/// Project-relative filename of the generated E2B template config.
const E2B_TOML_NAME: &str = "e2b.toml";

/// Install guidance for the E2B CLI when it is absent from `PATH`.
///
/// The `@e2b/cli` package is not in the Flox Catalog (only the Python SDK,
/// `python3xxPackages.e2b`, is), so the only supported path is npm — with
/// `nodejs` from the catalog supplying `npm`.
const E2B_CLI_INSTALL_HINT: &str = "The '@e2b/cli' package is not in the Flox Catalog (only the Python SDK is), so \
     install it with npm: 'flox install nodejs' to provide npm, then \
     'npm install -g @e2b/cli'. Run 'e2b auth login' to authenticate.";

/// Upgrade guidance for a too-old E2B CLI. npm-only for the same reason as
/// [`E2B_CLI_INSTALL_HINT`].
const E2B_CLI_UPGRADE_HINT: &str = "Upgrade with npm: 'npm install -g @e2b/cli@latest' (the '@e2b/cli' package is not \
     in the Flox Catalog), then re-run.";

pub struct E2bBackend<'a> {
    dot_flox_path: PathBuf,
    env_name: String,
    lockfile: &'a Lockfile,
    sandbox_oci_autobake: bool,
    container_builder_params: ContainerBuilderParams,
}

impl<'a> E2bBackend<'a> {
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

impl ActivationSandbox for E2bBackend<'_> {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::E2b
    }

    fn preflight(&self) -> Result<()> {
        // E2B is cloud-remote: the image bake runs through Docker, so Docker is
        // one genuinely required host tool. The E2B CLI is the other — it
        // builds the template from the generated Dockerfile.
        if first_on_path("docker").is_none() {
            bail!(
                "The 'e2b' sandbox backend bakes the environment image with Docker, which was \
                 not found on PATH.\n\
                 Install Docker Desktop or the Docker CLI, then re-run."
            );
        }
        let Some(e2b_path) = first_on_path("e2b") else {
            bail!(
                "The 'e2b' sandbox backend requires the E2B CLI, which was not found on PATH.\n{}",
                E2B_CLI_INSTALL_HINT
            );
        };
        check_e2b_version(&e2b_path)?;
        // Distinguish CLI-present-but-unauthenticated from CLI-present-and-ready
        // without triggering the interactive `e2b auth login` web flow.
        // `e2b auth info` reports the signed-in identity and fails (non-zero, no
        // prompt) when no credentials are configured. As a fallback, an API key
        // or access token in the environment is treated as authenticated (the
        // SDK path uses `E2B_API_KEY` directly).
        if !e2b_authenticated(&e2b_path) {
            bail!(
                "The E2B CLI is installed but not authenticated (no signed-in session and no \
                 {E2B_API_KEY_VAR}/{E2B_ACCESS_TOKEN_VAR} in the environment).\n\
                 Run 'e2b auth login' to sign in (opens a browser; requires an E2B account — \
                 the free tier suffices), or export {E2B_API_KEY_VAR}=<key> from the E2B \
                 dashboard."
            );
        }
        Ok(())
    }

    fn wrap_activation(self: Box<Self>) -> Result<Infallible> {
        wrap_e2b(
            &self.dot_flox_path,
            &self.env_name,
            self.lockfile,
            self.sandbox_oci_autobake,
            &self.container_builder_params,
        )
    }
}

/// Verify the resolved E2B CLI meets [`E2B_MIN_VERSION`].
///
/// The shared gate runs `e2b --version`, parses the output, and turns a
/// too-old client into an actionable message while tolerating a failed or
/// unparseable `--version` (logged at debug). The hint carries the
/// E2B-specific upgrade instructions.
fn check_e2b_version(e2b_path: &Path) -> Result<()> {
    check_cli_version(e2b_path, &CliVersionCheck {
        tool_name: "E2B",
        backend_id: "e2b",
        min_version: E2B_MIN_VERSION,
        upgrade_hint: E2B_CLI_UPGRADE_HINT,
        version_args: DEFAULT_VERSION_ARGS,
    })
}

/// Return `true` when the E2B CLI is authenticated, or an API key / access
/// token is present in the environment.
///
/// `e2b auth info` is a cheap, non-interactive probe: it reports the signed-in
/// identity and exits non-zero (without opening a browser) when no session
/// exists. Env-key presence is accepted as a fallback because the SDK launch
/// path authenticates with `E2B_API_KEY` directly and does not require a CLI
/// session.
fn e2b_authenticated(e2b_path: &Path) -> bool {
    let env_key_present = std::env::var_os(E2B_API_KEY_VAR)
        .filter(|v| !v.is_empty())
        .is_some()
        || std::env::var_os(E2B_ACCESS_TOKEN_VAR)
            .filter(|v| !v.is_empty())
            .is_some();
    if env_key_present {
        return true;
    }
    std::process::Command::new(e2b_path)
        .args(["auth", "info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Return the `<env>-e2b` repository name used for Docker image tagging.
fn e2b_repo(env_name: &str) -> String {
    format!("{env_name}{E2B_REPO_SUFFIX}")
}

// ── Network policy compilation ─────────────────────────────────────────────────

/// The manifest network policy compiled into E2B's egress vocabulary.
///
/// E2B's default is `allowInternetAccess=true` (default-open), so the compiled
/// policy ALWAYS carries an explicit deny posture: `allow_internet_access` is
/// forced false and only the manifest's `:80`/`:443` hosts are allowed on top.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct E2bNetworkPolicy {
    /// Explicit deny posture. Always `false` — E2B defaults to open, so flox
    /// overrides it to closed and layers the allowlist on top. Kept as a field
    /// (rather than implied) so the generated artifact states the override
    /// literally.
    pub allow_internet_access: bool,
    /// Hosts granted 80/443 SNI egress, in declaration order (deduplicated).
    pub allowed_hosts: Vec<String>,
}

/// Compile the manifest's `[[options.sandbox.network]]` rules into E2B's egress
/// vocabulary.
///
/// - Any rule set (including empty) → `allow_internet_access = false`: E2B is
///   default-open, so flox always emits the explicit deny posture.
/// - A `<host>:443` or `<host>:80` rule → an allowed-host entry (E2B filters by
///   host/SNI on ports 80 and 443).
/// - Any other port → a hard error: E2B's host/SNI filtering covers 80/443
///   only, and silently promoting the grant (or dropping it) would violate the
///   "never silently widen or narrow grants" contract.
pub(crate) fn compile_e2b_network_policy(rules: &[SandboxNetworkRule]) -> Result<E2bNetworkPolicy> {
    let mut hosts: Vec<String> = Vec::with_capacity(rules.len());
    for rule in rules {
        let (host, port) = split_endpoint(&rule.endpoint)?;
        if port != 443 && port != 80 {
            bail!(
                "The 'e2b' sandbox backend filters egress by host/SNI on ports 80 and 443 only, \
                 but rule '{endpoint}' targets port {port}.\n\
                 Rewrite the endpoint as '{host}:443' (or '{host}:80'), or select a backend with \
                 per-port egress (e.g. 'openshell'). Note E2B does not filter QUIC/UDP at all.",
                endpoint = rule.endpoint
            );
        }
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    Ok(E2bNetworkPolicy {
        // ALWAYS closed: E2B's default is open, so the deny posture is explicit.
        allow_internet_access: false,
        allowed_hosts: hosts,
    })
}

// ── Template artifact generation ───────────────────────────────────────────────

/// Inputs to [`render_e2b_dockerfile`] and [`render_e2b_toml`].
pub(crate) struct TemplateParams<'a> {
    /// E2B template name (`flox-<sanitized-env>`).
    pub template_name: &'a str,
    /// Image reference the Dockerfile's `FROM` pulls.
    pub image_ref: &'a str,
    /// Compiled egress policy.
    pub network: &'a E2bNetworkPolicy,
    /// The activation entrypoint the template's start command runs.
    pub start_cmd: &'a str,
}

/// Render the E2B `e2b.Dockerfile` (pure function, no I/O).
///
/// The emitted Dockerfile is a thin wrapper whose `FROM` is the baked image;
/// `e2b template build` reads it and builds a sandbox template from it. The base
/// image already carries the locked closure, so the wrapper adds nothing but a
/// provenance comment — E2B's contract is "template = Dockerfile FROM <base>".
pub(crate) fn render_e2b_dockerfile(params: &TemplateParams<'_>) -> String {
    // image_ref comes from a validated source (repo + hash12 tag, optional
    // sanitized registry prefix), so embedding it unquoted in the FROM line is
    // injection-safe (Dockerfile FROM takes a bare image ref).
    indoc::formatdoc! {r#"
        # syntax=docker/dockerfile:1
        # Generated by `flox activate --sandbox --sandbox-backend e2b`.
        # This is the E2B template Dockerfile for the '{template_name}' template.
        # flox baked the environment image locally; E2B builds a sandbox template
        # from this Dockerfile, so push that image to a registry E2B's builder can
        # pull (as '{image_ref}'), then run `e2b template build` in this directory.
        #
        # The base image already carries the locked flox closure — this wrapper
        # only records provenance. Egress policy lives in e2b.toml, not here.
        FROM {image_ref}
        "#,
        template_name = params.template_name,
        image_ref = params.image_ref,
    }
}

/// Render the E2B `e2b.toml` template config (pure function, no I/O).
///
/// The config names the template, points at the generated Dockerfile, records
/// the start command, and states the deny-by-default egress posture plus the
/// compiled allowlist. E2B's network default is open, so the header calls out
/// that flox forces `allow_internet_access = false` and only the listed hosts
/// are reachable (on 80/443; QUIC is unfiltered — a declared lossiness). The
/// allowlist doubles as the argument an operator applies live via
/// `updateNetwork`.
pub(crate) fn render_e2b_toml(params: &TemplateParams<'_>) -> String {
    let allowlist_toml = toml_str_list(&params.network.allowed_hosts);
    // The start command can contain double quotes (the default wrapper does),
    // so emit it as an escaped TOML basic-string literal rather than a raw
    // `"{start_cmd}"`, which would break TOML parsing.
    let start_cmd_lit = toml_str_lit(params.start_cmd);
    let policy_summary = if params.network.allowed_hosts.is_empty() {
        "deny-all (no grants declared; E2B default-open overridden to closed)".to_string()
    } else {
        format!(
            "deny-by-default; 80/443 SNI allowed: {}",
            params.network.allowed_hosts.join(", ")
        )
    };
    // template_name and start_cmd come from validated/canonical sources;
    // single-quoted TOML basic strings are used for embedded scalars, and
    // toml_str_lit escapes each allowlist host.
    indoc::formatdoc! {r#"
        # Generated by `flox activate --sandbox --sandbox-backend e2b`.
        # E2B template config for '{template_name}'. Build it with:
        #   e2b template build
        # (from this directory; it reads {dockerfile} as the template base).
        #
        # Egress is deny-by-default. E2B's own default is allow_internet_access =
        # true (default-OPEN), so flox forces it to false below and lists only the
        # manifest's [[options.sandbox.network]] :80/:443 hosts. E2B filters by
        # host/SNI on ports 80 and 443 only and does NOT filter QUIC/UDP — a
        # declared lossiness. The allowlist below is also the argument an operator
        # applies live to a running sandbox via `updateNetwork` (replace-not-merge).
        #   policy: {policy_summary}
        template_name = "{template_name}"
        dockerfile = "{dockerfile}"
        start_cmd = {start_cmd_lit}

        [network]
        # Explicit deny posture: overrides E2B's default-open. Only the hosts in
        # `allowed_hosts` are reachable (on ports 80/443).
        allow_internet_access = {allow_internet_access}
        allowed_hosts = {allowlist_toml}
        "#,
        template_name = params.template_name,
        dockerfile = E2B_DOCKERFILE_NAME,
        start_cmd_lit = start_cmd_lit,
        policy_summary = policy_summary,
        allow_internet_access = params.network.allow_internet_access,
        allowlist_toml = allowlist_toml,
    }
}

// ── Launch path ────────────────────────────────────────────────────────────────

/// Bake the image, compile the policy, generate the template artifacts, then
/// fail at the launch boundary — never fake the remote launch.
fn wrap_e2b(
    dot_flox_path: &Path,
    env_name: &str,
    lockfile: &Lockfile,
    autobake: bool,
    container_builder_params: &ContainerBuilderParams,
) -> Result<Infallible> {
    let dot_flox =
        std::fs::canonicalize(dot_flox_path).unwrap_or_else(|_| dot_flox_path.to_path_buf());
    let project = dot_flox.parent().unwrap_or(&dot_flox).to_path_buf();

    // Bake and tag the image under E2B's own namespace (`<env>-e2b`), with the
    // shared compat layer. The pushed artifact is recognizable as E2B's and
    // never collides with the other backends' tags on a shared registry.
    let repo = e2b_repo(env_name);
    let hash12 = lockfile_hash12(lockfile);

    // Ensure the local hash-tagged image exists (baking with the shared compat
    // layer if absent). E2B builds the template by pulling the image from a
    // registry, but baking it locally first is the same content-addressed step
    // every OCI-ingesting provider shares.
    ensure_local_image(
        &repo,
        env_name,
        dot_flox_path,
        lockfile,
        autobake,
        container_builder_params,
        "E2B image",
    )?;

    // Compile the manifest network policy into E2B's egress vocabulary. This
    // always yields an explicit deny posture — E2B is default-open.
    let rules = manifest_network_rules(lockfile)?;
    let network = compile_e2b_network_policy(&rules)?;

    // Build the template artifacts.
    let registry_prefix = std::env::var(FLOX_SANDBOX_E2B_REGISTRY_VAR)
        .ok()
        .filter(|v| !v.is_empty());
    let image_ref = registry_image_ref(&repo, &hash12, registry_prefix.as_deref());
    let template_name = flox_sanitized_name(env_name);
    let params = TemplateParams {
        template_name: &template_name,
        image_ref: &image_ref,
        network: &network,
        // The baked image's entrypoint starts the activated shell; the template
        // start command defers to it via `/bin/sh -lc` so the sandbox boots into
        // the activation.
        start_cmd: "/bin/sh -lc 'exec \"$@\"' _",
    };
    let dockerfile = render_e2b_dockerfile(&params);
    let toml = render_e2b_toml(&params);
    let (dockerfile_path, toml_path) = write_e2b_template(&project, &dockerfile, &toml)?;

    // Fail at the launch boundary with the two concrete prerequisites.
    let registry_hint = match &registry_prefix {
        Some(prefix) => {
            let prefix = prefix.trim_end_matches('/');
            format!("tag and push it as '{prefix}/{repo}:{hash12}'")
        },
        None => format!(
            "set {FLOX_SANDBOX_E2B_REGISTRY_VAR}=<registry-prefix> and re-run, then push '<prefix>/{repo}:{hash12}'"
        ),
    };
    bail!(
        "The 'e2b' sandbox backend launches a remote E2B sandbox, which requires two \
         prerequisites this host cannot satisfy automatically:\n  \
         1. Push the baked image '{repo}:{hash12}' to a registry E2B's builder can pull \
         ({registry_hint}), then build the template with 'e2b template build'.\n  \
         2. An E2B account and API key (preflight confirmed the CLI; the template build and \
         sandbox launch call the E2B API).\n\
         flox generated the E2B template hand-off at:\n  {dockerfile}\n  {toml}\n\
         With the image pushed and E2B authenticated, run 'e2b template build' in that \
         directory, then launch a sandbox from the template.",
        dockerfile = dockerfile_path.display(),
        toml = toml_path.display()
    )
}

/// Write the generated template artifacts (`e2b.Dockerfile` + `e2b.toml`) to
/// the project root and return their paths.
///
/// The artifacts live at the project root (not under `.flox/cache/`) because
/// `e2b template build` reads them from the build-context directory — they are
/// meant to be committed alongside the project, like the ona devcontainer.
fn write_e2b_template(project: &Path, dockerfile: &str, toml: &str) -> Result<(PathBuf, PathBuf)> {
    let dockerfile_path = project.join(E2B_DOCKERFILE_NAME);
    std::fs::write(&dockerfile_path, dockerfile).with_context(|| {
        format!(
            "failed to write E2B Dockerfile to '{}'",
            dockerfile_path.display()
        )
    })?;
    let toml_path = project.join(E2B_TOML_NAME);
    std::fs::write(&toml_path, toml)
        .with_context(|| format!("failed to write e2b.toml to '{}'", toml_path.display()))?;
    debug!(
        dockerfile = %dockerfile_path.display(),
        toml = %toml_path.display(),
        "wrote e2b template artifacts"
    );
    Ok((dockerfile_path, toml_path))
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use flox_core::activate::sandbox_policy::SandboxNetworkAccess;

    use super::*;

    // ── install / upgrade hints ───────────────────────────────────────────────

    #[test]
    fn install_hint_points_at_npm_not_the_catalog() {
        // The '@e2b/cli' CLI is npm-only: it is not in the Flox Catalog (only
        // the Python SDK is), so the hint must not claim a catalog attribute
        // resolves for it.
        assert!(
            E2B_CLI_INSTALL_HINT.contains("npm install -g @e2b/cli"),
            "install hint must name the npm path: {E2B_CLI_INSTALL_HINT}"
        );
        assert!(
            E2B_CLI_INSTALL_HINT.contains("flox install nodejs"),
            "install hint must note nodejs provides npm: {E2B_CLI_INSTALL_HINT}"
        );
        assert!(
            !E2B_CLI_INSTALL_HINT.contains("nodePackages_latest.e2b"),
            "install hint must not claim the unresolvable catalog attr: {E2B_CLI_INSTALL_HINT}"
        );
    }

    #[test]
    fn upgrade_hint_points_at_npm_not_the_catalog() {
        assert!(
            E2B_CLI_UPGRADE_HINT.contains("npm install -g @e2b/cli@latest"),
            "upgrade hint must name the npm path: {E2B_CLI_UPGRADE_HINT}"
        );
        assert!(
            !E2B_CLI_UPGRADE_HINT.contains("nodePackages_latest.e2b"),
            "upgrade hint must not claim the unresolvable catalog attr: {E2B_CLI_UPGRADE_HINT}"
        );
    }

    // ── e2b_repo ──────────────────────────────────────────────────────────────

    #[test]
    fn repo_has_e2b_suffix() {
        assert_eq!(e2b_repo("myenv"), "myenv-e2b");
    }

    #[test]
    fn repo_never_collides_with_other_backends() {
        let env = "myenv";
        let hash = "abc123def456";
        let oci = format!("{env}:{hash}");
        let openshell = format!("{env}-openshell:{hash}");
        let modal = format!("{env}-modal:{hash}");
        let ona = format!("{env}-ona:{hash}");
        let docker_sbx = format!("{env}-docker-sbx:{hash}");
        let e2b = format!("{}:{hash}", e2b_repo(env));
        assert_ne!(e2b, oci);
        assert_ne!(e2b, openshell);
        assert_ne!(e2b, modal);
        assert_ne!(e2b, ona);
        assert_ne!(e2b, docker_sbx);
    }

    // ── compile_e2b_network_policy ────────────────────────────────────────────

    fn rule(endpoint: &str) -> SandboxNetworkRule {
        SandboxNetworkRule {
            endpoint: endpoint.to_string(),
            access: None,
            protocol: None,
            binary: None,
        }
    }

    #[test]
    fn no_rules_still_forces_explicit_deny() {
        // E2B is default-open, so even with no grants the compiled policy must
        // carry the explicit deny posture (allow_internet_access = false).
        let policy = compile_e2b_network_policy(&[]).unwrap();
        assert_eq!(policy, E2bNetworkPolicy {
            allow_internet_access: false,
            allowed_hosts: Vec::new(),
        });
    }

    #[test]
    fn tls_443_rules_compile_to_allowed_hosts() {
        let rules = [rule("api.github.com:443"), rule("api.anthropic.com:443")];
        let policy = compile_e2b_network_policy(&rules).unwrap();
        assert_eq!(policy, E2bNetworkPolicy {
            allow_internet_access: false,
            allowed_hosts: vec![
                "api.github.com".to_string(),
                "api.anthropic.com".to_string(),
            ],
        });
    }

    #[test]
    fn port_80_rules_are_allowed() {
        // E2B filters host/SNI on both 80 and 443, so a :80 grant is expressible.
        let policy = compile_e2b_network_policy(&[rule("cache.example.com:80")]).unwrap();
        assert_eq!(policy.allowed_hosts, vec!["cache.example.com".to_string()]);
    }

    #[test]
    fn duplicate_hosts_are_deduplicated() {
        let rules = [rule("api.github.com:443"), rule("api.github.com:443")];
        let policy = compile_e2b_network_policy(&rules).unwrap();
        assert_eq!(policy.allowed_hosts, vec!["api.github.com".to_string()]);
    }

    #[test]
    fn wildcard_host_is_preserved() {
        let policy = compile_e2b_network_policy(&[rule("*.github.com:443")]).unwrap();
        assert_eq!(policy.allowed_hosts, vec!["*.github.com".to_string()]);
    }

    #[test]
    fn access_and_binary_do_not_affect_compilation() {
        // E2B's host allowlist carries no method distinction; a scoped grant
        // compiles identically to an unscoped one (declared lossiness).
        let scoped = SandboxNetworkRule {
            endpoint: "api.github.com:443".to_string(),
            access: Some(SandboxNetworkAccess::ReadOnly),
            protocol: None,
            binary: Some("curl".to_string()),
        };
        let policy = compile_e2b_network_policy(&[scoped]).unwrap();
        assert_eq!(policy, E2bNetworkPolicy {
            allow_internet_access: false,
            allowed_hosts: vec!["api.github.com".to_string()],
        });
    }

    #[test]
    fn non_80_443_port_is_rejected() {
        let err = compile_e2b_network_policy(&[rule("db.example.com:5432")]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("80 and 443"), "got: {msg}");
        assert!(msg.contains("db.example.com:443"), "got: {msg}");
        // The error must name E2B's QUIC lossiness so a reviewer sees the honest
        // gap alongside the rejection.
        assert!(msg.contains("QUIC"), "got: {msg}");
    }

    #[test]
    fn endpoint_without_port_is_rejected() {
        let err = compile_e2b_network_policy(&[rule("example.com")]).unwrap_err();
        assert!(err.to_string().contains("<HOST>:<PORT>"), "got: {err}");
    }

    #[test]
    fn endpoint_with_invalid_host_is_rejected() {
        let err = compile_e2b_network_policy(&[rule("bad host\nhost:443")]).unwrap_err();
        assert!(
            err.to_string().contains("Invalid sandbox network endpoint"),
            "got: {err}"
        );
    }

    // ── render_e2b_dockerfile ─────────────────────────────────────────────────

    fn deny_all_policy() -> E2bNetworkPolicy {
        E2bNetworkPolicy {
            allow_internet_access: false,
            allowed_hosts: Vec::new(),
        }
    }

    #[test]
    fn dockerfile_from_is_the_image_ref() {
        let params = TemplateParams {
            template_name: "flox-myenv",
            image_ref: "myenv-e2b:abc123",
            network: &deny_all_policy(),
            start_cmd: "/bin/sh",
        };
        let doc = render_e2b_dockerfile(&params);
        assert!(
            doc.starts_with("# syntax=docker/dockerfile:1\n"),
            "got:\n{doc}"
        );
        assert!(doc.contains("FROM myenv-e2b:abc123"), "got:\n{doc}");
        assert!(
            doc.contains("flox activate --sandbox --sandbox-backend e2b"),
            "provenance header missing:\n{doc}"
        );
    }

    // ── render_e2b_toml ───────────────────────────────────────────────────────

    #[test]
    fn toml_deny_all_forces_closed_posture_and_empty_allowlist() {
        let params = TemplateParams {
            template_name: "flox-myenv",
            image_ref: "myenv-e2b:abc123",
            network: &deny_all_policy(),
            start_cmd: "/bin/sh -lc 'exec \"$@\"' _",
        };
        let doc = render_e2b_toml(&params);
        assert!(
            doc.contains("template_name = \"flox-myenv\""),
            "got:\n{doc}"
        );
        assert!(
            doc.contains("dockerfile = \"e2b.Dockerfile\""),
            "got:\n{doc}"
        );
        // The load-bearing line: E2B is default-open, flox forces it closed.
        assert!(
            doc.contains("allow_internet_access = false"),
            "explicit deny posture missing:\n{doc}"
        );
        assert!(doc.contains("allowed_hosts = []"), "got:\n{doc}");
        assert!(
            doc.contains("deny-all (no grants declared; E2B default-open overridden to closed)"),
            "policy summary comment missing:\n{doc}"
        );
        // The header must name the default-open override and the QUIC lossiness.
        assert!(doc.contains("default-OPEN"), "got:\n{doc}");
        assert!(doc.contains("QUIC"), "got:\n{doc}");
        // The toml body (below the comment header) must parse as TOML once the
        // leading comment block is included — comments are valid TOML.
        let parsed: toml::Value = toml::from_str(&doc)
            .unwrap_or_else(|e| panic!("e2b.toml must be valid TOML: {e}\n{doc}"));
        assert_eq!(
            parsed["network"]["allow_internet_access"].as_bool(),
            Some(false)
        );
    }

    #[test]
    fn toml_allowlist_rendered_and_valid_toml() {
        let net = E2bNetworkPolicy {
            allow_internet_access: false,
            allowed_hosts: vec!["api.github.com".to_string(), "*.anthropic.com".to_string()],
        };
        let params = TemplateParams {
            template_name: "flox-env",
            image_ref: "env-e2b:tag",
            network: &net,
            start_cmd: "/bin/sh",
        };
        let doc = render_e2b_toml(&params);
        assert!(
            doc.contains("allowed_hosts = [\"api.github.com\", \"*.anthropic.com\"]"),
            "got:\n{doc}"
        );
        assert!(
            doc.contains("deny-by-default; 80/443 SNI allowed: api.github.com, *.anthropic.com"),
            "policy summary comment missing:\n{doc}"
        );
        let parsed: toml::Value = toml::from_str(&doc)
            .unwrap_or_else(|e| panic!("e2b.toml must be valid TOML: {e}\n{doc}"));
        let allow = parsed["network"]["allowed_hosts"].as_array().unwrap();
        assert_eq!(allow[0].as_str(), Some("api.github.com"));
        assert_eq!(allow[1].as_str(), Some("*.anthropic.com"));
        // A wildcard host embedded in a TOML string must be double-quoted so it
        // is a literal, not a bare token.
        assert!(
            doc.contains("\"*.anthropic.com\""),
            "wildcard host must be quoted:\n{doc}"
        );
    }
}
