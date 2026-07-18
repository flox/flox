//! The `daytona` sandbox backend: run the environment in a remote Daytona
//! sandbox.
//!
//! Daytona is a cloud-API sandbox provider. Like the other cloud backends,
//! nothing runs on the host: flox bakes the environment's OCI image via the
//! shared lockfile-hash-tagged Docker bake (`super::bake`), under Daytona's own
//! `<env>-daytona:<hash12>` tag namespace, then generates the **Daytona launcher
//! program** — a Python script that registers the baked image as a Daytona
//! *snapshot* (`Image.base(<ref>)` via the declarative builder) and creates a
//! sandbox from that snapshot with the compiled network policy. The threat model
//! inverts relative to the host-local backends: the host filesystem is
//! unreachable from the remote sandbox, but the code and any injected secrets
//! leave the laptop. Credentials belong in Daytona's own secret mechanism, not a
//! local `.env`.
//!
//! # Why this backend does not complete the launch on any host today
//!
//! Two external prerequisites gate the remote launch, and neither can be
//! satisfied from a bare checkout:
//!
//! - **A Daytona account and API key.** The Daytona SDK/CLI authenticates with
//!   `DAYTONA_API_KEY` (the REST API takes `Authorization: Bearer <key>`);
//!   without it every API call fails. `preflight` distinguishes *CLI-missing*
//!   from *CLI-present-but-unauthenticated* cheaply and non-interactively (it
//!   never triggers the browser `daytona login` flow — env-key presence is a
//!   valid fallback probe).
//! - **The image registered as a snapshot.** Daytona ingests images as the base
//!   of a *snapshot* (`Image.base(<registry-ref>)`, built through Daytona's
//!   declarative builder; local images can also be pushed with the CLI). The
//!   locally baked `<env>-daytona:<hash12>` image must be referenced from the
//!   generated launcher and registered as a snapshot before a sandbox can be
//!   created from it.
//!
//! Rather than fake success, this backend implements the deepest honest slice:
//! it runs the real preflight, bakes the real image, compiles the manifest
//! network policy into Daytona's egress vocabulary, and *generates the launcher
//! program*. It then fails at the launch boundary with a clear "requires ..."
//! error that points at the generated artifact and names the two missing
//! prerequisites.
//!
//! # Network-policy compilation (the load-bearing lossiness)
//!
//! Daytona's per-sandbox egress vocabulary is three **mutually exclusive**
//! parameters (at most one may be non-empty, per the network-limits docs):
//! - `domain_allow_list` — domains and wildcard domains (e.g.
//!   `example.com,*.daytona.io`).
//! - `network_allow_list` — IPv4 CIDR ranges (e.g. `10.0.0.0/24`).
//! - `network_block_all` — deny all outbound (the default posture here when no
//!   grants are declared).
//!
//! flox's `[[options.sandbox.network]]` grants are host-scoped, so they compile
//! onto `domain_allow_list`: a `<host>:<port>` endpoint becomes a domain-allow
//! entry (native, faithful *at the domain level*). Two declared lossiness
//! points fall out of Daytona's shape:
//!
//! - **Port is dropped.** Daytona filters per-domain, not per-port, so the
//!   `:443` in a grant does not scope the rule — every port to that domain is
//!   reachable. The endpoint's port is still parsed and validated, so a
//!   malformed endpoint is still rejected.
//! - **CIDR grants are exclusive.** Because flox compiles host grants onto the
//!   domain list, a CIDR-shaped grant (an endpoint whose host part is a CIDR
//!   like `10.0.0.0/24`) cannot be combined with the domain list on the same
//!   sandbox — the parameters are mutually exclusive. Such a grant is declined
//!   with a clear error rather than silently dropped or widened.
//!
//! Daytona's allowlist carries no read/write method distinction and no
//! per-binary attribution, so the `access`, `protocol`, and `binary` fields of a
//! grant are recorded in the generated artifact as comments but do not constrain
//! traffic — declared lossiness, per the backend contract. On Tier 1/2
//! organizations the org-level network policy overrides sandbox-level settings
//! entirely; that ceiling is documented in the artifact and the demo.
//!
//! # Env knobs (prototype)
//!
//! The bake reuses the openshell/oci knobs:
//! - `FLOX_SANDBOX_OCI_IMAGE` — explicit image ref override (skips bake).
//! - `FLOX_SANDBOX_OCI_ALLOW_STALE` — run the newest existing image when the
//!   expected hash-tag is absent.
//! - `FLOX_SANDBOX_OCI_AUTOBAKE` — bake without prompting.
//! - `FLOX_SANDBOX_DAYTONA_REGISTRY` — registry prefix the launcher's snapshot
//!   base ref is built from (e.g. `docker.io/myuser`); recorded in the artifact
//!   so a credentialed operator does not have to hand-edit it.

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

use super::handoff::{
    ensure_local_image,
    flox_sanitized_name,
    manifest_network_rules,
    py_str_list,
    py_str_lit,
    registry_image_ref,
    sandbox_activation_command,
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
use crate::commands::sandbox_backends::openshell::openshell_guest_system;

/// Registry prefix the launcher's snapshot base ref is built from. When set
/// (e.g. `docker.io/myuser`), the generated artifact references
/// `<prefix>/<repo>:<hash12>` so a credentialed operator does not have to
/// hand-edit the registry ref before pushing and registering the snapshot.
pub(crate) const FLOX_SANDBOX_DAYTONA_REGISTRY_VAR: &str = "FLOX_SANDBOX_DAYTONA_REGISTRY";

/// SDK/CLI API-key environment variable Daytona authenticates with. The REST
/// API takes `Authorization: Bearer <key>`; the SDK and CLI read this variable.
const DAYTONA_API_KEY_VAR: &str = "DAYTONA_API_KEY";

/// Repository suffix for the Daytona backend's image tags. The image is baked
/// under `<env>-daytona:<hash12>` (with the shared compat layer) and the
/// launcher's snapshot base reuses that name, so the pushed artifact is
/// recognizable as Daytona's and never collides with the other backends' tags on
/// a shared registry.
const DAYTONA_REPO_SUFFIX: &str = "-daytona";

/// Minimum supported Daytona CLI version.
///
/// The snapshot-from-image and per-sandbox network-list flow the launcher
/// targets is present across the current CLI line. Pinned conservatively to a
/// 0.x floor (the catalog ships `daytona-bin@0.12.0`); the shared version gate
/// tolerates an unparseable/failed `--version` so an unusual build never blocks.
const DAYTONA_MIN_VERSION: Version = Version::new(0, 9, 0);

/// Install guidance for the Daytona CLI when it is absent from `PATH`.
///
/// The catalog ships the CLI as `daytona-bin` (binary name `daytona`); the auth
/// wall is `daytona login` / `DAYTONA_API_KEY`.
const DAYTONA_CLI_INSTALL_HINT: &str = "Install the Daytona CLI with 'flox install daytona-bin' (binary name 'daytona'), or \
     follow https://www.daytona.io/docs. Then run 'daytona login' or export \
     DAYTONA_API_KEY=<key> from the dashboard to authenticate.";

/// Upgrade guidance for a too-old Daytona CLI.
const DAYTONA_CLI_UPGRADE_HINT: &str =
    "Upgrade with 'flox install daytona-bin' (or your install method's latest), then re-run.";

pub struct DaytonaBackend<'a> {
    dot_flox_path: PathBuf,
    env_name: String,
    invocation_type: &'a InvocationType,
    lockfile: &'a Lockfile,
    sandbox_oci_autobake: bool,
    container_builder_params: ContainerBuilderParams,
}

impl<'a> DaytonaBackend<'a> {
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

impl ActivationSandbox for DaytonaBackend<'_> {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::Daytona
    }

    fn preflight(&self) -> Result<()> {
        // Daytona is cloud-remote: the image bake runs through Docker, so Docker
        // is one genuinely required host tool. The Daytona CLI is the other — it
        // registers the snapshot from the baked image and drives the launch.
        if first_on_path("docker").is_none() {
            bail!(
                "The 'daytona' sandbox backend bakes the environment image with Docker, which was \
                 not found on PATH.\n\
                 Install Docker Desktop or the Docker CLI, then re-run."
            );
        }
        let Some(daytona_path) = first_on_path("daytona") else {
            bail!(
                "The 'daytona' sandbox backend requires the Daytona CLI, which was not found on \
                 PATH.\n{}",
                DAYTONA_CLI_INSTALL_HINT
            );
        };
        check_daytona_version(&daytona_path)?;
        // Distinguish CLI-present-but-unauthenticated from CLI-present-and-ready
        // without triggering the interactive `daytona login` web flow. An API key
        // in the environment is the SDK/REST auth path and is accepted directly;
        // otherwise a `daytona whoami` (or `organization list`) probe reports the
        // signed-in state and fails non-zero, without a prompt, when no session
        // exists.
        if !daytona_authenticated(&daytona_path) {
            bail!(
                "The Daytona CLI is installed but not authenticated (no signed-in session and no \
                 {DAYTONA_API_KEY_VAR} in the environment).\n\
                 Run 'daytona login' to sign in (opens a browser; requires a Daytona account — \
                 the free tier suffices), or export {DAYTONA_API_KEY_VAR}=<key> from the Daytona \
                 dashboard."
            );
        }
        Ok(())
    }

    fn wrap_activation(self: Box<Self>) -> Result<Infallible> {
        wrap_daytona(
            &self.dot_flox_path,
            &self.env_name,
            self.invocation_type,
            self.lockfile,
            self.sandbox_oci_autobake,
            &self.container_builder_params,
        )
    }
}

/// Verify the resolved Daytona CLI meets [`DAYTONA_MIN_VERSION`].
///
/// The shared gate runs `daytona --version`, parses the output, and turns a
/// too-old client into an actionable message while tolerating a failed or
/// unparseable `--version` (logged at debug). The hint carries the
/// Daytona-specific upgrade instructions.
fn check_daytona_version(daytona_path: &Path) -> Result<()> {
    check_cli_version(daytona_path, &CliVersionCheck {
        tool_name: "Daytona",
        backend_id: "daytona",
        min_version: DAYTONA_MIN_VERSION,
        upgrade_hint: DAYTONA_CLI_UPGRADE_HINT,
        version_args: DEFAULT_VERSION_ARGS,
    })
}

/// Return `true` when the Daytona CLI is authenticated, or an API key is present
/// in the environment.
///
/// Env-key presence is checked first: the SDK/REST launch path authenticates
/// with `DAYTONA_API_KEY` directly and does not require a CLI session. Otherwise
/// `daytona whoami` is a cheap, non-interactive probe that reports the signed-in
/// identity and exits non-zero (without opening a browser) when no session
/// exists.
fn daytona_authenticated(daytona_path: &Path) -> bool {
    let env_key_present = std::env::var_os(DAYTONA_API_KEY_VAR)
        .filter(|v| !v.is_empty())
        .is_some();
    if env_key_present {
        return true;
    }
    std::process::Command::new(daytona_path)
        .arg("whoami")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Return the `<env>-daytona` repository name used for Docker image tagging.
fn daytona_repo(env_name: &str) -> String {
    format!("{env_name}{DAYTONA_REPO_SUFFIX}")
}

/// The remote sandbox runs on Linux; lockfile lookups for guest paths use the
/// Linux system for the host's architecture, mirroring the openshell backend.
fn daytona_guest_system() -> &'static str {
    openshell_guest_system()
}

// ── Network policy compilation ─────────────────────────────────────────────────

/// The manifest network policy compiled into Daytona's egress vocabulary.
///
/// Daytona expresses egress as three mutually exclusive parameters:
/// `network_block_all` (deny-all), `domain_allow_list` (domains + wildcards),
/// and `network_allow_list` (CIDR ranges). flox's host-scoped grants compile
/// onto the domain list; the deny-all posture applies when no grants are
/// declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DaytonaNetworkPolicy {
    /// Deny all outbound traffic (no grants declared).
    pub block_all: bool,
    /// Hosts granted egress, in declaration order (deduplicated). Filtered
    /// per-domain by Daytona — the grant's port does not scope the rule.
    pub domain_allow_list: Vec<String>,
}

/// Compile the manifest's `[[options.sandbox.network]]` rules into Daytona's
/// egress vocabulary.
///
/// - No rules → `network_block_all=True` (deny-all, secure-by-default).
/// - A `<host>:<port>` rule → a domain-allowlist entry. Daytona filters
///   per-domain, so the port is dropped (declared lossiness); the endpoint is
///   still parsed to reject a malformed grant.
/// - A CIDR-shaped host (e.g. `10.0.0.0/24:443`) → a hard error: Daytona's CIDR
///   list is mutually exclusive with the domain list flox already compiles host
///   grants onto, so combining them on one sandbox is impossible. Declining is
///   the honest choice over silently dropping the CIDR or widening the policy.
pub(crate) fn compile_daytona_network_policy(
    rules: &[SandboxNetworkRule],
) -> Result<DaytonaNetworkPolicy> {
    if rules.is_empty() {
        return Ok(DaytonaNetworkPolicy {
            block_all: true,
            domain_allow_list: Vec::new(),
        });
    }
    let mut domains: Vec<String> = Vec::with_capacity(rules.len());
    for rule in rules {
        // A CIDR-shaped host carries a `/` (e.g. `10.0.0.0/24`), which
        // `split_endpoint`'s hostname charset already rejects. Detect it up
        // front so the error names Daytona's mutual-exclusivity ceiling rather
        // than the generic "invalid endpoint" message.
        if is_cidr_endpoint(&rule.endpoint) {
            bail!(
                "The 'daytona' sandbox backend compiles host grants onto its domain allowlist, \
                 which is mutually exclusive with its CIDR allowlist, but rule '{endpoint}' is a \
                 CIDR range.\n\
                 Daytona accepts at most one of a domain list or a CIDR list per sandbox — mixing \
                 them is not expressible. Rewrite the grant as a host (e.g. 'api.github.com:443'), \
                 or select a backend that expresses CIDR and domain grants together (e.g. \
                 'openshell').",
                endpoint = rule.endpoint
            );
        }
        // Daytona filters per-domain, so the port does not scope the rule; parse
        // it anyway to reject a malformed endpoint and to keep the shape uniform
        // with the other cloud backends.
        let (host, _port) = split_endpoint(&rule.endpoint)?;
        if !domains.contains(&host) {
            domains.push(host);
        }
    }
    Ok(DaytonaNetworkPolicy {
        block_all: false,
        domain_allow_list: domains,
    })
}

/// Return `true` when the endpoint's host part looks like an IPv4 CIDR range
/// (contains a `/`), which Daytona expresses only via its CIDR allowlist —
/// mutually exclusive with the domain allowlist flox compiles host grants onto.
fn is_cidr_endpoint(endpoint: &str) -> bool {
    // Strip a trailing `:<port>` (if any) before checking for the CIDR slash, so
    // `10.0.0.0/24:443` is recognized as CIDR while a bare `host:443` is not.
    let host = endpoint.rsplit_once(':').map_or(endpoint, |(h, _)| h);
    host.contains('/')
}

// ── Launcher artifact generation ───────────────────────────────────────────────

/// Inputs to [`render_daytona_launcher`].
///
/// Grouping the fields keeps the pure renderer's signature self-documenting and
/// under clippy's argument-count limit.
pub(crate) struct LauncherParams<'a> {
    /// Daytona snapshot name (`flox-<sanitized-env>`).
    pub snapshot_name: &'a str,
    /// Registry image reference Daytona registers as the snapshot base via
    /// `Image.base`.
    pub image_ref: &'a str,
    /// Compiled egress policy.
    pub network: &'a DaytonaNetworkPolicy,
    /// The activation command to run in the sandbox (already split into argv
    /// members).
    pub command: &'a [String],
    /// Working directory inside the sandbox.
    pub workdir: &'a str,
}

/// Render the Daytona launcher program (pure function, no I/O).
///
/// The emitted Python constructs the `Daytona` client, registers the baked image
/// as a snapshot (`Image.base(<ref>)`), creates a sandbox from that snapshot with
/// the compiled network policy, runs the activation command, streams the output,
/// and exits with its return code. A credentialed operator with the image pushed
/// to `image_ref`'s registry runs it with `python`.
pub(crate) fn render_daytona_launcher(params: &LauncherParams<'_>) -> String {
    let command_lit = py_str_list(params.command);
    // Deny-all uses `network_block_all=True`; a grant set uses the native domain
    // allowlist (comma-joined by the Daytona API). The two are mutually
    // exclusive on Daytona — set at most one non-empty value.
    let net_kwarg = if params.network.block_all {
        "    network_block_all=True,".to_string()
    } else {
        format!(
            "    domain_allow_list={},",
            py_str_lit(&params.network.domain_allow_list.join(","))
        )
    };
    // image_ref, snapshot_name, and workdir come from validated sources (repo +
    // hash12 tag, sanitized snapshot name, canonical path), so single-quoted
    // literals are injection-safe; py_str_lit additionally escapes each command
    // member and the joined allowlist.
    indoc::formatdoc! {r#"
        #!/usr/bin/env python3
        # Generated by `flox activate --sandbox --sandbox-backend daytona`.
        # This is the launch artifact for the Daytona backend. flox baked the
        # environment image locally; Daytona ingests images as a snapshot base
        # (Image.base(<ref>) via the declarative builder), so push that image to a
        # registry Daytona can pull (as '{image_ref}'), then run this program with
        # `python {{this file}}` (needs DAYTONA_API_KEY in the environment).
        #
        # Egress is deny-by-default. The grant below is the manifest's
        # [[options.sandbox.network]] rules compiled to Daytona's vocabulary.
        # Daytona's domain_allow_list / network_allow_list / network_block_all are
        # MUTUALLY EXCLUSIVE (at most one non-empty). flox compiles host grants
        # onto domain_allow_list; the grant's port and access/protocol/binary
        # scoping are NOT enforceable on Daytona and are dropped here. Note: on
        # Tier 1/2 organizations the org-level network policy overrides these
        # sandbox-level settings entirely.
        import sys
        from daytona import (
            CreateSandboxFromSnapshotParams,
            CreateSnapshotParams,
            Daytona,
            Image,
            Resources,
        )

        daytona = Daytona()

        # Register the baked image as a snapshot (idempotent by name).
        image = Image.base('{image_ref}')
        daytona.snapshot.create(
            CreateSnapshotParams(
                name='{snapshot_name}',
                image=image,
                resources=Resources(cpu=2, memory=4, disk=8),
            ),
        )

        sandbox = daytona.create(
            CreateSandboxFromSnapshotParams(
                snapshot='{snapshot_name}',
        {net_kwarg}
            )
        )

        # Run the activation command, then exit with its return code.
        response = sandbox.process.exec(
            ' '.join({command_lit}),
            cwd='{workdir}',
        )
        print(response.result, end='')
        daytona.delete(sandbox)
        sys.exit(response.exit_code)
        "#,
        snapshot_name = params.snapshot_name,
        image_ref = params.image_ref,
        workdir = params.workdir,
        net_kwarg = net_kwarg,
        command_lit = command_lit,
    }
}

// ── Launch path ────────────────────────────────────────────────────────────────

/// Bake the image, compile the policy, generate the launcher artifact, then
/// fail at the launch boundary — never fake the remote launch.
fn wrap_daytona(
    dot_flox_path: &Path,
    env_name: &str,
    invocation: &InvocationType,
    lockfile: &Lockfile,
    autobake: bool,
    container_builder_params: &ContainerBuilderParams,
) -> Result<Infallible> {
    let dot_flox =
        std::fs::canonicalize(dot_flox_path).unwrap_or_else(|_| dot_flox_path.to_path_buf());
    let project = dot_flox.parent().unwrap_or(&dot_flox).to_path_buf();

    // Bake and tag the image under Daytona's own namespace (`<env>-daytona`),
    // with the shared compat layer. The pushed artifact is recognizable as
    // Daytona's and never collides with the other backends' tags on a shared
    // registry.
    let repo = daytona_repo(env_name);
    let hash12 = lockfile_hash12(lockfile);

    // Ensure the local hash-tagged image exists (baking with the shared compat
    // layer if absent). Daytona registers the snapshot by pulling the image from
    // a registry, but baking it locally first is the same content-addressed step
    // every OCI-ingesting provider shares.
    ensure_local_image(
        &repo,
        env_name,
        dot_flox_path,
        lockfile,
        autobake,
        container_builder_params,
        "Daytona image",
    )?;

    // Compile the manifest network policy into Daytona's egress vocabulary.
    let rules = manifest_network_rules(lockfile)?;
    // Touch the guest-system helper so a future guest-path resolution shares the
    // openshell backend's Linux-guest assumption rather than the host's.
    let _ = daytona_guest_system();
    let network = compile_daytona_network_policy(&rules)?;

    // Build the launcher artifact.
    let registry_prefix = std::env::var(FLOX_SANDBOX_DAYTONA_REGISTRY_VAR)
        .ok()
        .filter(|v| !v.is_empty());
    let image_ref = registry_image_ref(&repo, &hash12, registry_prefix.as_deref());
    let snapshot_name = flox_sanitized_name(env_name);
    let cwd = std::env::current_dir().unwrap_or_else(|_| project.clone());
    let workdir = if cwd.starts_with(&project) {
        cwd.display().to_string()
    } else {
        project.display().to_string()
    };
    // The baked entrypoint is recovered at launch time from the registered
    // snapshot; on this host it is unknown, so the launcher runs the image's own
    // entrypoint by passing an empty explicit command.
    let entrypoint: Vec<String> = Vec::new();
    let command = sandbox_activation_command(invocation, &entrypoint, "daytona");
    let launcher = render_daytona_launcher(&LauncherParams {
        snapshot_name: &snapshot_name,
        image_ref: &image_ref,
        network: &network,
        command: &command,
        workdir: &workdir,
    });
    let artifact_path = write_daytona_launcher(dot_flox_path, &launcher)?;

    // Fail at the launch boundary with the two concrete prerequisites.
    let registry_hint = match &registry_prefix {
        Some(prefix) => {
            let prefix = prefix.trim_end_matches('/');
            format!("tag and push it as '{prefix}/{repo}:{hash12}'")
        },
        None => format!(
            "set {FLOX_SANDBOX_DAYTONA_REGISTRY_VAR}=<registry-prefix> and re-run, then push '<prefix>/{repo}:{hash12}'"
        ),
    };
    bail!(
        "The 'daytona' sandbox backend launches a remote Daytona sandbox, which requires two \
         prerequisites this host cannot satisfy automatically:\n  \
         1. Push the baked image '{repo}:{hash12}' to a registry Daytona can pull \
         ({registry_hint}), which the launcher registers as the snapshot base.\n  \
         2. A Daytona account and API key (preflight confirmed the CLI; the snapshot \
         registration and sandbox launch call the Daytona API).\n\
         flox generated the launch program at:\n  {artifact}\n\
         With the image pushed and {DAYTONA_API_KEY_VAR} set, run it with 'python {artifact}'.",
        artifact = artifact_path.display()
    )
}

/// Write the generated launcher program under `.flox/cache/` and return its
/// path.
fn write_daytona_launcher(dot_flox_path: &Path, launcher: &str) -> Result<PathBuf> {
    let cache_dir = dot_flox_path.join("cache");
    std::fs::create_dir_all(&cache_dir)
        .with_context(|| format!("failed to create cache dir '{}'", cache_dir.display()))?;
    let artifact_path = cache_dir.join("daytona-launch.py");
    std::fs::write(&artifact_path, launcher)
        .with_context(|| format!("failed to write launcher to '{}'", artifact_path.display()))?;
    debug!(path = %artifact_path.display(), "wrote daytona launcher artifact");
    Ok(artifact_path)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use flox_core::activate::sandbox_policy::SandboxNetworkAccess;

    use super::*;

    // ── install / upgrade hints ───────────────────────────────────────────────

    #[test]
    fn install_hint_names_the_catalog_package_and_auth_wall() {
        // daytona-bin resolves in the catalog (verified 2026-07-18); the hint
        // must name it and the DAYTONA_API_KEY auth wall.
        assert!(
            DAYTONA_CLI_INSTALL_HINT.contains("flox install daytona-bin"),
            "install hint must name the catalog package: {DAYTONA_CLI_INSTALL_HINT}"
        );
        assert!(
            DAYTONA_CLI_INSTALL_HINT.contains("DAYTONA_API_KEY"),
            "install hint must name the auth wall: {DAYTONA_CLI_INSTALL_HINT}"
        );
    }

    // ── daytona_repo ──────────────────────────────────────────────────────────

    #[test]
    fn repo_has_daytona_suffix() {
        assert_eq!(daytona_repo("myenv"), "myenv-daytona");
    }

    #[test]
    fn repo_never_collides_with_other_backends() {
        let env = "myenv";
        let hash = "abc123def456";
        let oci = format!("{env}:{hash}");
        let openshell = format!("{env}-openshell:{hash}");
        let modal = format!("{env}-modal:{hash}");
        let e2b = format!("{env}-e2b:{hash}");
        let daytona = format!("{}:{hash}", daytona_repo(env));
        assert_ne!(daytona, oci);
        assert_ne!(daytona, openshell);
        assert_ne!(daytona, modal);
        assert_ne!(daytona, e2b);
    }

    // ── is_cidr_endpoint ──────────────────────────────────────────────────────

    #[test]
    fn cidr_endpoint_detected_with_and_without_port() {
        assert!(is_cidr_endpoint("10.0.0.0/24"));
        assert!(is_cidr_endpoint("10.0.0.0/24:443"));
        assert!(is_cidr_endpoint("192.168.1.0/24:80"));
        assert!(!is_cidr_endpoint("api.github.com:443"));
        assert!(!is_cidr_endpoint("api.github.com"));
    }

    // ── compile_daytona_network_policy ────────────────────────────────────────

    fn rule(endpoint: &str) -> SandboxNetworkRule {
        SandboxNetworkRule {
            endpoint: endpoint.to_string(),
            access: None,
            protocol: None,
            binary: None,
        }
    }

    #[test]
    fn no_rules_compiles_to_block_all() {
        let policy = compile_daytona_network_policy(&[]).unwrap();
        assert_eq!(policy, DaytonaNetworkPolicy {
            block_all: true,
            domain_allow_list: Vec::new(),
        });
    }

    #[test]
    fn host_rules_compile_to_domain_allow_list_dropping_port() {
        let rules = [rule("api.github.com:443"), rule("api.anthropic.com:8443")];
        let policy = compile_daytona_network_policy(&rules).unwrap();
        // Daytona filters per-domain, so both compile to the bare host — the
        // differing ports are dropped (declared lossiness).
        assert_eq!(policy, DaytonaNetworkPolicy {
            block_all: false,
            domain_allow_list: vec![
                "api.github.com".to_string(),
                "api.anthropic.com".to_string(),
            ],
        });
    }

    #[test]
    fn duplicate_hosts_are_deduplicated() {
        // Same host on two ports collapses to one domain entry (port dropped).
        let rules = [rule("api.github.com:443"), rule("api.github.com:80")];
        let policy = compile_daytona_network_policy(&rules).unwrap();
        assert_eq!(policy.domain_allow_list, vec!["api.github.com".to_string()]);
    }

    #[test]
    fn wildcard_host_is_preserved() {
        let policy = compile_daytona_network_policy(&[rule("*.github.com:443")]).unwrap();
        assert_eq!(policy.domain_allow_list, vec!["*.github.com".to_string()]);
    }

    #[test]
    fn access_and_binary_do_not_affect_compilation() {
        // Daytona's allowlist carries no method distinction; a scoped grant
        // compiles identically to an unscoped one (declared lossiness).
        let scoped = SandboxNetworkRule {
            endpoint: "api.github.com:443".to_string(),
            access: Some(SandboxNetworkAccess::ReadOnly),
            protocol: None,
            binary: Some("curl".to_string()),
        };
        let policy = compile_daytona_network_policy(&[scoped]).unwrap();
        assert_eq!(policy, DaytonaNetworkPolicy {
            block_all: false,
            domain_allow_list: vec!["api.github.com".to_string()],
        });
    }

    #[test]
    fn cidr_grant_is_declined_for_mutual_exclusivity() {
        let err = compile_daytona_network_policy(&[rule("10.0.0.0/24:443")]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("mutually exclusive"), "got: {msg}");
        assert!(msg.contains("CIDR"), "got: {msg}");
    }

    #[test]
    fn endpoint_without_port_is_rejected() {
        let err = compile_daytona_network_policy(&[rule("example.com")]).unwrap_err();
        assert!(err.to_string().contains("<HOST>:<PORT>"), "got: {err}");
    }

    #[test]
    fn endpoint_with_invalid_host_is_rejected() {
        let err = compile_daytona_network_policy(&[rule("bad host\nhost:443")]).unwrap_err();
        assert!(
            err.to_string().contains("Invalid sandbox network endpoint"),
            "got: {err}"
        );
    }

    // ── render_daytona_launcher ───────────────────────────────────────────────

    fn block_all_policy() -> DaytonaNetworkPolicy {
        DaytonaNetworkPolicy {
            block_all: true,
            domain_allow_list: Vec::new(),
        }
    }

    #[test]
    fn launcher_deny_all_uses_block_all() {
        let cmd = vec!["/entry".to_string(), "activate".to_string()];
        let script = render_daytona_launcher(&LauncherParams {
            snapshot_name: "flox-myenv",
            image_ref: "myenv-daytona:abc123",
            network: &block_all_policy(),
            command: &cmd,
            workdir: "/home/user/proj",
        });
        assert!(script.contains("from daytona import"), "got:\n{script}");
        assert!(
            script.contains("Image.base('myenv-daytona:abc123')"),
            "got:\n{script}"
        );
        assert!(script.contains("snapshot='flox-myenv'"), "got:\n{script}");
        assert!(script.contains("network_block_all=True"), "got:\n{script}");
        // The deny-all path must not emit the domain-list *keyword argument*
        // (the comment header mentions the parameter by name, so a bare
        // substring check would false-positive).
        assert!(
            !script.contains("domain_allow_list="),
            "deny-all must not emit a domain allowlist kwarg:\n{script}"
        );
        assert!(script.contains("cwd='/home/user/proj'"), "got:\n{script}");
    }

    #[test]
    fn launcher_domain_allow_list_is_comma_joined() {
        let net = DaytonaNetworkPolicy {
            block_all: false,
            domain_allow_list: vec!["api.github.com".to_string(), "*.anthropic.com".to_string()],
        };
        let cmd = vec!["/entry".to_string()];
        let script = render_daytona_launcher(&LauncherParams {
            snapshot_name: "flox-env",
            image_ref: "env-daytona:tag",
            network: &net,
            command: &cmd,
            workdir: "/proj",
        });
        // Daytona takes a single comma-joined string, not a Python list.
        assert!(
            script.contains("domain_allow_list='api.github.com,*.anthropic.com'"),
            "got:\n{script}"
        );
        assert!(!script.contains("network_block_all=True"), "got:\n{script}");
    }

    #[test]
    fn launcher_command_members_are_escaped() {
        // A command member containing a single quote must be escaped so the
        // emitted Python literal stays well-formed.
        let cmd = vec![
            "python3".to_string(),
            "-c".to_string(),
            "print('hi')".to_string(),
        ];
        let script = render_daytona_launcher(&LauncherParams {
            snapshot_name: "flox-env",
            image_ref: "env-daytona:tag",
            network: &block_all_policy(),
            command: &cmd,
            workdir: "/proj",
        });
        assert!(
            script.contains("'print(\\'hi\\')'"),
            "single quotes in command members must be escaped:\n{script}"
        );
    }

    #[test]
    fn launcher_is_valid_python_prologue() {
        let script = render_daytona_launcher(&LauncherParams {
            snapshot_name: "flox-env",
            image_ref: "env-daytona:tag",
            network: &block_all_policy(),
            command: &["/entry".to_string()],
            workdir: "/proj",
        });
        assert!(
            script.starts_with("#!/usr/bin/env python3\n"),
            "got:\n{script}"
        );
        assert!(
            script.contains("sys.exit(response.exit_code)"),
            "got:\n{script}"
        );
    }

    #[test]
    fn launcher_documents_mutual_exclusivity_and_tier_ceiling() {
        // The load-bearing lossiness must be stated in the generated artifact so
        // an operator reading it sees the honest gaps.
        let script = render_daytona_launcher(&LauncherParams {
            snapshot_name: "flox-env",
            image_ref: "env-daytona:tag",
            network: &block_all_policy(),
            command: &["/entry".to_string()],
            workdir: "/proj",
        });
        assert!(
            script.contains("MUTUALLY EXCLUSIVE"),
            "artifact must state the mutual-exclusivity ceiling:\n{script}"
        );
        assert!(
            script.contains("Tier 1/2"),
            "artifact must state the org-tier override ceiling:\n{script}"
        );
    }
}
