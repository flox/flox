//! The `modal` sandbox backend: run the environment in a remote Modal Sandbox.
//!
//! Modal is a cloud-API provider: unlike every other backend, nothing runs on
//! the host. flox bakes the environment's OCI image (reusing the openshell
//! backend's lockfile-hash-tagged Docker bake), then hands that image to Modal,
//! which pulls it, wraps it in a `modal.Image`, and launches a remote
//! `modal.Sandbox` running the activation. The threat model inverts relative to
//! the host-local backends: the host filesystem is unreachable from the remote
//! sandbox, but the code and any injected secrets leave the laptop.
//!
//! # Why this backend does not complete the launch on any host today
//!
//! Two external prerequisites gate the remote launch, and neither can be
//! satisfied from a bare checkout:
//!
//! - **A Modal account and token.** The Modal SDK/CLI authenticates against
//!   `~/.modal.toml`; without it every API call fails. `preflight` distinguishes
//!   *CLI-missing* from *CLI-present-but-unauthenticated* cheaply and
//!   non-interactively (it never triggers the browser `modal token new` flow).
//! - **A registry Modal can pull from.** The Modal SDK ingests images by
//!   registry reference only — `modal.Image.from_registry(tag)` /
//!   `from_aws_ecr` / `from_gcp_artifact_registry` / `from_dockerfile`. There is
//!   no local-Docker-daemon or tarball ingestion path. So the locally baked
//!   `<env>:<hash12>` image must be pushed to a registry Modal can reach before
//!   the sandbox can be created.
//!
//! Rather than fake success, this backend implements the deepest honest slice:
//! it runs the real preflight, bakes the real image, compiles the manifest
//! network policy into Modal's egress vocabulary, and *generates the Modal
//! launcher program* (a Python script that constructs the App / Image / Sandbox
//! with the compiled policy). It then fails at the launch boundary with a clear
//! "requires ..." error that points at the generated artifact and names the two
//! missing prerequisites. A credentialed operator with a registry can push the
//! image, edit the artifact's registry ref, and run it.
//!
//! # Network-policy compilation (the load-bearing lossiness)
//!
//! Modal's egress vocabulary is:
//! - `block_network=True` — deny all outbound (the default posture here when no
//!   grants are declared).
//! - `outbound_cidr_allowlist=[CIDR, …]` — any protocol, IP-range scoped.
//! - `outbound_domain_allowlist=[domain, …]` — **TLS/443 only** (Beta); accepts
//!   wildcards like `*.github.com`.
//!
//! A `[[options.sandbox.network]]` grant compiles as follows: a `<host>:443`
//! endpoint becomes a domain-allowlist entry (native, faithful); any other port
//! cannot be expressed as a domain rule and is declined with a clear error
//! rather than silently widened to all-ports or dropped. Modal's allowlist
//! carries no read/write method distinction and no per-binary attribution, so
//! the `access`, `protocol`, and `binary` fields of a grant are recorded in the
//! generated artifact as comments but do not constrain traffic — declared
//! lossiness, per the backend contract.
//!
//! # Env knobs (prototype)
//!
//! The bake reuses the openshell/oci knobs:
//! - `FLOX_SANDBOX_OCI_IMAGE` — explicit image ref override (skips bake).
//! - `FLOX_SANDBOX_OCI_ALLOW_STALE` — run the newest existing image when the
//!   expected hash-tag is absent.
//! - `FLOX_SANDBOX_OCI_AUTOBAKE` — bake without prompting.
//! - `FLOX_SANDBOX_MODAL_REGISTRY` — registry prefix the launcher's
//!   `from_registry` ref is built from (e.g. `docker.io/myuser`); recorded in
//!   the artifact so a credentialed operator does not have to hand-edit it.

use std::convert::Infallible;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flox_core::activate::context::InvocationType;
use flox_core::activate::sandbox_backend::SandboxBackend;
use flox_core::activate::sandbox_policy::SandboxNetworkRule;
use flox_manifest::interfaces::AsLatestSchema;
use flox_manifest::lockfile::Lockfile;
use flox_rust_sdk::providers::container_builder::ContainerBuilderParams;
use semver::Version;
use tracing::debug;

use super::{ActivationSandbox, SandboxLaunchCtx};
use crate::commands::sandbox_backends::oci::lockfile_hash12;
use crate::commands::sandbox_backends::openshell::{
    bake_openshell_image,
    openshell_guest_system,
    openshell_repo,
    resolve_docker_image_state,
};

/// Registry prefix the launcher's `from_registry` ref is built from. When set
/// (e.g. `docker.io/myuser`), the generated artifact references
/// `<prefix>/<repo>:<hash12>` so a credentialed operator does not have to
/// hand-edit the registry ref before pushing and running.
pub(crate) const FLOX_SANDBOX_MODAL_REGISTRY_VAR: &str = "FLOX_SANDBOX_MODAL_REGISTRY";

/// Repository suffix used to name the pushed registry image for the Modal
/// backend. The image is *baked* under the openshell tag namespace
/// (`<env>-openshell:<hash12>`, sharing the compat layer), but the launcher's
/// registry reference is named `<env>-modal:<hash12>` so the pushed artifact is
/// recognizable as Modal's and never collides with the other backends' tags on
/// a shared registry.
const MODAL_REPO_SUFFIX: &str = "-modal";

/// Minimum supported Modal client version.
///
/// `outbound_domain_allowlist` (the native domain-egress path this backend
/// compiles to) landed in the 1.x client line; the launcher artifact uses it
/// unconditionally, so an older client would fail at `Sandbox.create` with an
/// unexpected-keyword error. Pinned conservatively to the 1.0 floor; the
/// prototype is developed against 1.4.
const MODAL_MIN_VERSION: Version = Version::new(1, 0, 0);

pub struct ModalBackend<'a> {
    dot_flox_path: PathBuf,
    env_name: String,
    invocation_type: &'a InvocationType,
    lockfile: &'a Lockfile,
    sandbox_oci_autobake: bool,
    container_builder_params: ContainerBuilderParams,
}

impl<'a> ModalBackend<'a> {
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

impl ActivationSandbox for ModalBackend<'_> {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::Modal
    }

    fn preflight(&self) -> Result<()> {
        let Some(modal_path) = first_on_path("modal") else {
            bail!(
                "The 'modal' sandbox backend requires the Modal CLI, which was not found on \
                 PATH.\n\
                 Install it with 'flox install python313Packages.modal' or \
                 'pip install modal', then run 'modal token new' to authenticate."
            );
        };
        check_modal_version(&modal_path)?;
        // Distinguish CLI-present-but-unauthenticated from CLI-present-and-ready
        // without triggering the interactive `modal token new` web flow.
        // `modal token info` reports the token currently in use and fails
        // (non-zero, no prompt) when no credentials are configured.
        let authed = std::process::Command::new(&modal_path)
            .args(["token", "info"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !authed {
            bail!(
                "The Modal CLI is installed but not authenticated (no credentials in \
                 '~/.modal.toml').\n\
                 Run 'modal token new' to sign in (opens a browser; requires a Modal \
                 account — the free tier suffices)."
            );
        }
        Ok(())
    }

    fn wrap_activation(self: Box<Self>) -> Result<Infallible> {
        wrap_modal(
            &self.dot_flox_path,
            &self.env_name,
            self.invocation_type,
            self.lockfile,
            self.sandbox_oci_autobake,
            &self.container_builder_params,
        )
    }
}

/// Resolve the first executable named `name` on `PATH`.
fn first_on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

/// Verify the resolved Modal CLI meets [`MODAL_MIN_VERSION`].
///
/// A too-old client would surface as an unexpected-keyword error deep inside
/// `Sandbox.create`; the gate turns that into an actionable message. A failed
/// or unparseable `--version` invocation skips the gate (logged at debug)
/// rather than blocking on an unknown output format.
fn check_modal_version(modal_path: &Path) -> Result<()> {
    let output = std::process::Command::new(modal_path)
        .arg("--version")
        .output();
    let raw = match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => {
            debug!(
                path = %modal_path.display(),
                "could not run 'modal --version'; skipping version gate"
            );
            return Ok(());
        },
    };
    let Some(version) = parse_modal_version(&raw) else {
        debug!(
            path = %modal_path.display(),
            output = raw.trim(),
            "unparseable 'modal --version' output; skipping version gate"
        );
        return Ok(());
    };
    if version < MODAL_MIN_VERSION {
        bail!(
            "Modal CLI version {version} is too old for the 'modal' sandbox backend (needs {MODAL_MIN_VERSION} or newer).\n\
             Resolved binary: {path}\n\
             Upgrade with 'flox install python313Packages.modal' or 'pip install --upgrade modal', then re-run.",
            path = modal_path.display()
        );
    }
    debug!(%version, "modal CLI version meets the minimum");
    Ok(())
}

/// Parse the version from `modal --version` output (e.g. `modal client version: 1.4.2`
/// or a bare `1.4.2`).
///
/// Returns `None` when no whitespace/colon-separated token parses as a semver
/// version (an optional leading `v` is tolerated).
pub(crate) fn parse_modal_version(output: &str) -> Option<Version> {
    output
        .split(|c: char| c.is_whitespace() || c == ':')
        .filter(|t| !t.is_empty())
        .find_map(|token| Version::parse(token.strip_prefix('v').unwrap_or(token)).ok())
}

/// Return the `<env>-modal` repository name used for Docker image tagging.
fn modal_repo(env_name: &str) -> String {
    format!("{env_name}{MODAL_REPO_SUFFIX}")
}

/// Generate a Modal app name from the environment name.
///
/// Modal app/sandbox names must be non-empty; this lowercases the env name and
/// replaces any character outside `[a-z0-9-]` with a dash, then prefixes
/// `flox-` so the app is recognizable in the Modal dashboard. Unlike the
/// openshell sandbox name, no PID suffix is added: the app is looked up with
/// `create_if_missing=True`, so a stable name lets repeated activations reuse
/// one app.
pub(crate) fn modal_app_name(env_name: &str) -> String {
    let sanitized: String = env_name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("flox-{sanitized}")
}

/// The remote sandbox runs on Linux; lockfile lookups for guest paths use the
/// Linux system for the host's architecture, mirroring the openshell backend.
fn modal_guest_system() -> &'static str {
    openshell_guest_system()
}

// ── Network policy compilation ─────────────────────────────────────────────────

/// The manifest network policy compiled into Modal's egress vocabulary.
///
/// Modal expresses egress as `block_network` (deny-all), a domain allowlist
/// (TLS/443 only), and a CIDR allowlist (any protocol). `grants.toml`-style
/// endpoints compile onto the domain allowlist when they target port 443;
/// everything else is declined rather than silently widened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModalNetworkPolicy {
    /// Deny all outbound traffic (no grants declared).
    pub block_network: bool,
    /// Hosts granted TLS/443 egress, in declaration order (deduplicated).
    pub domain_allowlist: Vec<String>,
}

/// Compile the manifest's `[[options.sandbox.network]]` rules into Modal's
/// egress vocabulary.
///
/// - No rules → `block_network=True` (deny-all, secure-by-default).
/// - A `<host>:443` rule → a domain-allowlist entry (native, faithful).
/// - Any non-443 port → a hard error: Modal's domain allowlist is TLS/443-only,
///   and silently promoting the grant to a CIDR/all-ports rule (or dropping it)
///   would violate the "never silently widen or narrow grants" contract.
pub(crate) fn compile_modal_network_policy(
    rules: &[SandboxNetworkRule],
) -> Result<ModalNetworkPolicy> {
    if rules.is_empty() {
        return Ok(ModalNetworkPolicy {
            block_network: true,
            domain_allowlist: Vec::new(),
        });
    }
    let mut domains: Vec<String> = Vec::with_capacity(rules.len());
    for rule in rules {
        let (host, port) = split_endpoint(&rule.endpoint)?;
        if port != 443 {
            bail!(
                "The 'modal' sandbox backend can only grant TLS/443 endpoints via its domain allowlist, but rule '{endpoint}' targets port {port}.\n\
                 Modal's outbound_domain_allowlist governs port 443 only; rewrite the endpoint as '{host}:443', or select a backend with per-port egress (e.g. 'openshell').",
                endpoint = rule.endpoint
            );
        }
        if !domains.contains(&host) {
            domains.push(host);
        }
    }
    Ok(ModalNetworkPolicy {
        block_network: false,
        domain_allowlist: domains,
    })
}

/// Split a `<HOST>:<PORT>` endpoint and validate both halves.
///
/// The host charset is restricted to hostname characters plus a leading-label
/// wildcard (`*`), which also guarantees the host is safe to embed in the
/// single-quoted Python string literals the launcher artifact emits.
fn split_endpoint(endpoint: &str) -> Result<(String, u16)> {
    let invalid = || {
        anyhow::anyhow!(
            "Invalid sandbox network endpoint '{endpoint}'.\nWrite the endpoint as <HOST>:<PORT>, e.g. 'api.github.com:443'."
        )
    };
    let (host, port) = endpoint.rsplit_once(':').ok_or_else(invalid)?;
    let host_ok = !host.is_empty()
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '*'));
    if !host_ok {
        return Err(invalid());
    }
    let port: u16 = port.parse().map_err(|_| invalid())?;
    Ok((host.to_string(), port))
}

// ── Launcher artifact generation ───────────────────────────────────────────────

/// Inputs to [`render_modal_launcher`].
///
/// Grouping the fields keeps the pure renderer's signature self-documenting and
/// under clippy's argument-count limit.
pub(crate) struct LauncherParams<'a> {
    /// Modal app name (`flox-<sanitized-env>`).
    pub app_name: &'a str,
    /// Registry image reference Modal pulls via `Image.from_registry`.
    pub image_ref: &'a str,
    /// Compiled egress policy.
    pub network: &'a ModalNetworkPolicy,
    /// The activation command to run as the sandbox CMD (already split into
    /// argv members).
    pub command: &'a [String],
    /// Working directory inside the sandbox.
    pub workdir: &'a str,
    /// Sandbox wall-clock timeout, in seconds.
    pub timeout_secs: u32,
}

/// Render the Modal launcher program (pure function, no I/O).
///
/// The emitted Python constructs the App, ingests the baked image by registry
/// reference, and creates a `modal.Sandbox` with the compiled egress policy,
/// then streams the sandbox's output and exits with its return code. A
/// credentialed operator with the image pushed to `image_ref`'s registry runs
/// it with `modal run` (or plain `python`).
pub(crate) fn render_modal_launcher(params: &LauncherParams<'_>) -> String {
    let command_lit = py_str_list(params.command);
    // `Sandbox.create` takes the CMD as *args; render each member on its own
    // indented line ahead of the keyword arguments.
    let command_args: String = params
        .command
        .iter()
        .map(|arg| format!("    {},\n", py_str_lit(arg)))
        .collect();
    // Deny-all uses `block_network=True`; a grant set uses the native domain
    // allowlist (TLS/443 only). The two are mutually exclusive on Modal.
    let net_kwarg = if params.network.block_network {
        "    block_network=True,".to_string()
    } else {
        format!(
            "    outbound_domain_allowlist={},",
            py_str_list(&params.network.domain_allowlist)
        )
    };
    // image_ref, app_name, and workdir come from validated sources (repo +
    // hash12 tag, sanitized app name, canonical path), so single-quoted literals
    // are injection-safe; py_str_lit additionally escapes each command member.
    indoc::formatdoc! {r#"
        #!/usr/bin/env python3
        # Generated by `flox activate --sandbox --sandbox-backend modal`.
        # This is the launch artifact for the Modal Sandboxes backend. flox baked
        # the environment image locally; Modal ingests images by registry reference
        # only, so push that image to a registry Modal can pull (as '{image_ref}'),
        # then run this program with `modal run <this file>`.
        #
        # Egress is deny-by-default. Grants below are the manifest's
        # [[options.sandbox.network]] rules compiled to Modal's vocabulary
        # (domain allowlist = TLS/443 only). access/protocol/binary scoping from
        # the manifest is NOT enforceable on Modal and is dropped here.
        import sys
        import modal

        app = modal.App.lookup('{app_name}', create_if_missing=True)
        image = modal.Image.from_registry('{image_ref}')

        sb = modal.Sandbox.create(
        {command_args}    app=app,
            image=image,
            workdir='{workdir}',
            timeout={timeout_secs},
        {net_kwarg}
        )

        # Stream the activation's output, then exit with its return code.
        p = sb.exec({command_lit}, workdir='{workdir}')
        for line in p.stdout:
            print(line, end='')
        returncode = p.wait()
        sb.terminate()
        sys.exit(returncode)
        "#,
        app_name = params.app_name,
        image_ref = params.image_ref,
        workdir = params.workdir,
        timeout_secs = params.timeout_secs,
        command_args = command_args,
        net_kwarg = net_kwarg,
        command_lit = command_lit,
    }
}

/// Render a Python single-quoted string literal, escaping backslashes and
/// single quotes so arbitrary argv members are safe to embed.
fn py_str_lit(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

/// Render a Python list of single-quoted string literals.
fn py_str_list(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|s| py_str_lit(s))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

/// Build the activation command argv for the sandbox CMD.
///
/// The baked image's entrypoint starts the activated shell; the command wraps
/// the effective working directory and any user command the same way the
/// openshell backend does, but flattened here because the launcher passes it as
/// the sandbox CMD.
pub(crate) fn modal_activation_command(
    invocation: &InvocationType,
    entrypoint: &[String],
) -> Vec<String> {
    match invocation {
        InvocationType::Interactive => entrypoint.to_vec(),
        InvocationType::ExecCommand(cmd) => {
            let mut v = entrypoint.to_vec();
            v.extend(cmd.iter().cloned());
            v
        },
        InvocationType::ShellCommand(shell_cmd) => {
            let mut v = entrypoint.to_vec();
            v.extend(["sh".to_string(), "-c".to_string(), shell_cmd.clone()]);
            v
        },
        InvocationType::InPlace => {
            unreachable!(
                "in-place invocation cannot reach the modal backend (blocked by \
                 ensure_sandbox_not_in_place)"
            );
        },
    }
}

/// Build the registry image reference the launcher's `from_registry` uses.
///
/// When `FLOX_SANDBOX_MODAL_REGISTRY` is set, the ref is
/// `<prefix>/<repo>:<hash12>`; otherwise the bare local `<repo>:<hash12>` tag is
/// used as a placeholder (the operator must retag/push before running).
pub(crate) fn modal_image_ref(repo: &str, hash12: &str, registry_prefix: Option<&str>) -> String {
    match registry_prefix {
        Some(prefix) => {
            let prefix = prefix.trim_end_matches('/');
            format!("{prefix}/{repo}:{hash12}")
        },
        None => format!("{repo}:{hash12}"),
    }
}

// ── Launch path ────────────────────────────────────────────────────────────────

/// Bake the image, compile the policy, generate the launcher artifact, then
/// fail at the launch boundary — never fake the remote launch.
fn wrap_modal(
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

    // The image is baked under the openshell tag namespace (shared compat
    // layer); the registry image is *named* under the modal namespace so the
    // pushed artifact is recognizable and collision-free.
    let bake_repo = openshell_repo(env_name);
    let registry_repo = modal_repo(env_name);
    let hash12 = lockfile_hash12(lockfile);

    // Ensure the local hash-tagged image exists (reusing the openshell bake,
    // which produces an image with the compat layer). The Modal launch needs
    // this image pushed to a registry, but baking it locally first is the same
    // content-addressed step every OCI-ingesting provider shares.
    ensure_local_image(
        &bake_repo,
        env_name,
        dot_flox_path,
        lockfile,
        autobake,
        container_builder_params,
    )?;

    // Compile the manifest network policy into Modal's egress vocabulary.
    let rules = manifest_network_rules(lockfile)?;
    // Touch the guest-system helper so a future guest-path resolution shares the
    // openshell backend's Linux-guest assumption rather than the host's.
    let _ = modal_guest_system();
    let network = compile_modal_network_policy(&rules)?;

    // Build the launcher artifact.
    let registry_prefix = std::env::var(FLOX_SANDBOX_MODAL_REGISTRY_VAR)
        .ok()
        .filter(|v| !v.is_empty());
    let image_ref = modal_image_ref(&registry_repo, &hash12, registry_prefix.as_deref());
    let app_name = modal_app_name(env_name);
    let cwd = std::env::current_dir().unwrap_or_else(|_| project.clone());
    let workdir = if cwd.starts_with(&project) {
        cwd.display().to_string()
    } else {
        project.display().to_string()
    };
    // The baked entrypoint is recovered at launch time from the pushed image; on
    // this host it is unknown, so the launcher runs the image's own CMD by
    // passing an empty explicit command (Modal falls back to the image CMD).
    let entrypoint: Vec<String> = Vec::new();
    let command = modal_activation_command(invocation, &entrypoint);
    let launcher = render_modal_launcher(&LauncherParams {
        app_name: &app_name,
        image_ref: &image_ref,
        network: &network,
        command: &command,
        workdir: &workdir,
        timeout_secs: 3600,
    });
    let artifact_path = write_modal_launcher(dot_flox_path, &launcher)?;

    // Fail at the launch boundary with the two concrete prerequisites.
    let registry_hint = match &registry_prefix {
        Some(prefix) => {
            let prefix = prefix.trim_end_matches('/');
            format!("tag and push it as '{prefix}/{registry_repo}:{hash12}'")
        },
        None => format!(
            "set {FLOX_SANDBOX_MODAL_REGISTRY_VAR}=<registry-prefix> and re-run, then push '<prefix>/{registry_repo}:{hash12}'"
        ),
    };
    bail!(
        "The 'modal' sandbox backend launches a remote Modal Sandbox, which requires two \
         prerequisites this host cannot satisfy automatically:\n  \
         1. Push the baked image '{bake_repo}:{hash12}' to a registry Modal can pull \
         ({registry_hint}).\n  \
         2. A Modal account and token (preflight confirmed the CLI; the launch itself \
         calls the Modal API).\n\
         flox generated the launch program at:\n  {artifact}\n\
         With the image pushed and Modal authenticated, run it with 'modal run {artifact}'.",
        artifact = artifact_path.display()
    )
}

/// Read the manifest's `[[options.sandbox.network]]` rules from the lockfile.
fn manifest_network_rules(lockfile: &Lockfile) -> Result<Vec<SandboxNetworkRule>> {
    let manifest = lockfile
        .migrated_manifest()
        .context("failed to migrate the manifest for sandbox policy generation")?;
    Ok(manifest
        .as_latest_schema()
        .options
        .sandbox
        .as_ref()
        .and_then(|sandbox| sandbox.network.clone())
        .unwrap_or_default())
}

/// Write the generated launcher program under `.flox/cache/` and return its
/// path.
fn write_modal_launcher(dot_flox_path: &Path, launcher: &str) -> Result<PathBuf> {
    let cache_dir = dot_flox_path.join("cache");
    std::fs::create_dir_all(&cache_dir)
        .with_context(|| format!("failed to create cache dir '{}'", cache_dir.display()))?;
    let artifact_path = cache_dir.join("modal-launch.py");
    std::fs::write(&artifact_path, launcher)
        .with_context(|| format!("failed to write launcher to '{}'", artifact_path.display()))?;
    debug!(path = %artifact_path.display(), "wrote modal launcher artifact");
    Ok(artifact_path)
}

/// Ensure the `<repo>:<hash12>` image exists in the local Docker store, baking
/// it (with the openshell compat layer) if absent.
///
/// The Modal launch pushes this image to a registry; baking it locally first is
/// the content-addressed bake every OCI-ingesting provider shares. When the
/// image is already present (cache hit), this is a no-op.
fn ensure_local_image(
    bake_repo: &str,
    env_name: &str,
    dot_flox_path: &Path,
    lockfile: &Lockfile,
    autobake: bool,
    container_builder_params: &ContainerBuilderParams,
) -> Result<()> {
    use std::io::IsTerminal;

    use crate::commands::sandbox_backends::oci::{
        FLOX_SANDBOX_OCI_ALLOW_STALE_VAR,
        OciBakeDecision,
        OciImageState,
        should_bake_oci,
    };

    let state = resolve_docker_image_state(bake_repo, lockfile);
    match state {
        OciImageState::Explicit(_) | OciImageState::Present { .. } => Ok(()),
        OciImageState::Stale {
            ref expected_ref, ..
        }
        | OciImageState::Missing { ref expected_ref } => {
            let is_missing = matches!(state, OciImageState::Missing { .. });
            let allow_stale = std::env::var(FLOX_SANDBOX_OCI_ALLOW_STALE_VAR)
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let is_tty = std::io::stdin().is_terminal();
            let decision = should_bake_oci(
                is_missing,
                allow_stale,
                autobake,
                is_tty,
                expected_ref,
                None,
            );
            match decision {
                // Running a stale image is acceptable — the operator pushes
                // whatever is present; a fresh bake is not forced here.
                OciBakeDecision::RunStale(_) => Ok(()),
                OciBakeDecision::Bake => bake_openshell_image(
                    env_name,
                    dot_flox_path,
                    container_builder_params,
                    lockfile,
                ),
                OciBakeDecision::Prompt => {
                    let msg = format!(
                        "Modal image '{expected_ref}' is not baked locally.\n\
                         Bake now? (~2–5 min on first bake; later bakes reuse layers)"
                    );
                    let confirmed = inquire::Confirm::new(&msg)
                        .with_default(true)
                        .prompt()
                        .unwrap_or(false);
                    if confirmed {
                        bake_openshell_image(
                            env_name,
                            dot_flox_path,
                            container_builder_params,
                            lockfile,
                        )
                    } else {
                        bail!(
                            "Bake declined. To build the image manually, set \
                             FLOX_SANDBOX_OCI_AUTOBAKE=true and re-run."
                        );
                    }
                },
                OciBakeDecision::FailFast { expected_ref, .. } => {
                    bail!(
                        "Modal image '{expected_ref}' not found in the local Docker image store.\n\
                         To bake it automatically, set FLOX_SANDBOX_OCI_AUTOBAKE=true or run on an interactive terminal."
                    );
                },
            }
        },
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use flox_core::activate::sandbox_policy::SandboxNetworkAccess;

    use super::*;

    // ── parse_modal_version ───────────────────────────────────────────────────

    #[test]
    fn version_parses_plain_output() {
        assert_eq!(parse_modal_version("1.4.2"), Some(Version::new(1, 4, 2)));
    }

    #[test]
    fn version_parses_labeled_output() {
        assert_eq!(
            parse_modal_version("modal client version: 1.4.2"),
            Some(Version::new(1, 4, 2))
        );
    }

    #[test]
    fn version_parses_v_prefixed() {
        assert_eq!(
            parse_modal_version("modal v1.0.0"),
            Some(Version::new(1, 0, 0))
        );
    }

    #[test]
    fn version_unparseable_returns_none() {
        assert_eq!(parse_modal_version("not a version"), None);
        assert_eq!(parse_modal_version(""), None);
    }

    // ── modal_app_name ────────────────────────────────────────────────────────

    #[test]
    fn app_name_prefix_and_sanitization() {
        assert_eq!(modal_app_name("MyEnv"), "flox-myenv");
        assert_eq!(modal_app_name("my.env-v2 beta"), "flox-my-env-v2-beta");
    }

    // ── modal_repo ────────────────────────────────────────────────────────────

    #[test]
    fn repo_has_modal_suffix() {
        assert_eq!(modal_repo("myenv"), "myenv-modal");
    }

    #[test]
    fn repo_never_collides_with_other_backends() {
        let env = "myenv";
        let hash = "abc123def456";
        let oci = format!("{env}:{hash}");
        let openshell = format!("{env}-openshell:{hash}");
        let modal = format!("{}:{hash}", modal_repo(env));
        assert_ne!(modal, oci);
        assert_ne!(modal, openshell);
    }

    // ── modal_image_ref ───────────────────────────────────────────────────────

    #[test]
    fn image_ref_without_registry_is_bare_tag() {
        assert_eq!(
            modal_image_ref("myenv-modal", "abc123", None),
            "myenv-modal:abc123"
        );
    }

    #[test]
    fn image_ref_with_registry_prefixes_and_trims_slash() {
        assert_eq!(
            modal_image_ref("myenv-modal", "abc123", Some("docker.io/user")),
            "docker.io/user/myenv-modal:abc123"
        );
        assert_eq!(
            modal_image_ref("myenv-modal", "abc123", Some("docker.io/user/")),
            "docker.io/user/myenv-modal:abc123"
        );
    }

    // ── compile_modal_network_policy ──────────────────────────────────────────

    fn rule(endpoint: &str) -> SandboxNetworkRule {
        SandboxNetworkRule {
            endpoint: endpoint.to_string(),
            access: None,
            protocol: None,
            binary: None,
        }
    }

    #[test]
    fn no_rules_compiles_to_block_network() {
        let policy = compile_modal_network_policy(&[]).unwrap();
        assert_eq!(policy, ModalNetworkPolicy {
            block_network: true,
            domain_allowlist: Vec::new(),
        });
    }

    #[test]
    fn tls_443_rules_compile_to_domain_allowlist() {
        let rules = [rule("api.github.com:443"), rule("api.anthropic.com:443")];
        let policy = compile_modal_network_policy(&rules).unwrap();
        assert_eq!(policy, ModalNetworkPolicy {
            block_network: false,
            domain_allowlist: vec![
                "api.github.com".to_string(),
                "api.anthropic.com".to_string(),
            ],
        });
    }

    #[test]
    fn duplicate_hosts_are_deduplicated() {
        let rules = [rule("api.github.com:443"), rule("api.github.com:443")];
        let policy = compile_modal_network_policy(&rules).unwrap();
        assert_eq!(policy.domain_allowlist, vec!["api.github.com".to_string()]);
    }

    #[test]
    fn wildcard_host_is_preserved() {
        let policy = compile_modal_network_policy(&[rule("*.github.com:443")]).unwrap();
        assert_eq!(policy.domain_allowlist, vec!["*.github.com".to_string()]);
    }

    #[test]
    fn access_and_protocol_do_not_affect_compilation() {
        // Modal's allowlist carries no method distinction; a scoped grant
        // compiles identically to an unscoped one (declared lossiness).
        let scoped = SandboxNetworkRule {
            endpoint: "api.github.com:443".to_string(),
            access: Some(SandboxNetworkAccess::ReadOnly),
            protocol: None,
            binary: Some("curl".to_string()),
        };
        let policy = compile_modal_network_policy(&[scoped]).unwrap();
        assert_eq!(policy, ModalNetworkPolicy {
            block_network: false,
            domain_allowlist: vec!["api.github.com".to_string()],
        });
    }

    #[test]
    fn non_443_port_is_rejected() {
        let err = compile_modal_network_policy(&[rule("db.example.com:5432")]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("TLS/443"), "got: {msg}");
        assert!(msg.contains("db.example.com:443"), "got: {msg}");
    }

    #[test]
    fn endpoint_without_port_is_rejected() {
        let err = compile_modal_network_policy(&[rule("example.com")]).unwrap_err();
        assert!(err.to_string().contains("<HOST>:<PORT>"), "got: {err}");
    }

    #[test]
    fn endpoint_with_invalid_host_is_rejected() {
        let err = compile_modal_network_policy(&[rule("bad host\nhost:443")]).unwrap_err();
        assert!(
            err.to_string().contains("Invalid sandbox network endpoint"),
            "got: {err}"
        );
    }

    // ── modal_activation_command ──────────────────────────────────────────────

    #[test]
    fn interactive_command_is_entrypoint_only() {
        let entry = vec!["/entry".to_string(), "activate".to_string()];
        let cmd = modal_activation_command(&InvocationType::Interactive, &entry);
        assert_eq!(cmd, entry);
    }

    #[test]
    fn exec_command_appends_user_command() {
        let entry = vec!["/entry".to_string()];
        let inv = InvocationType::ExecCommand(vec!["ls".to_string(), "-la".to_string()]);
        let cmd = modal_activation_command(&inv, &entry);
        assert_eq!(cmd, vec![
            "/entry".to_string(),
            "ls".to_string(),
            "-la".to_string()
        ]);
    }

    #[test]
    fn shell_command_wraps_in_sh_c() {
        let entry = vec!["/entry".to_string()];
        let inv = InvocationType::ShellCommand("echo hi | cat".to_string());
        let cmd = modal_activation_command(&inv, &entry);
        assert_eq!(cmd, vec![
            "/entry".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            "echo hi | cat".to_string(),
        ]);
    }

    // ── render_modal_launcher ─────────────────────────────────────────────────

    fn block_all_policy() -> ModalNetworkPolicy {
        ModalNetworkPolicy {
            block_network: true,
            domain_allowlist: Vec::new(),
        }
    }

    #[test]
    fn launcher_deny_all_uses_block_network() {
        let cmd = vec!["/entry".to_string(), "activate".to_string()];
        let script = render_modal_launcher(&LauncherParams {
            app_name: "flox-myenv",
            image_ref: "myenv-modal:abc123",
            network: &block_all_policy(),
            command: &cmd,
            workdir: "/home/user/proj",
            timeout_secs: 3600,
        });
        assert!(script.contains("import modal"), "got:\n{script}");
        assert!(
            script.contains("modal.App.lookup('flox-myenv', create_if_missing=True)"),
            "got:\n{script}"
        );
        assert!(
            script.contains("modal.Image.from_registry('myenv-modal:abc123')"),
            "got:\n{script}"
        );
        assert!(script.contains("block_network=True"), "got:\n{script}");
        assert!(
            !script.contains("outbound_domain_allowlist"),
            "deny-all must not emit a domain allowlist:\n{script}"
        );
        assert!(
            script.contains("workdir='/home/user/proj'"),
            "got:\n{script}"
        );
        assert!(script.contains("timeout=3600"), "got:\n{script}");
        // The placeholder fixup must have run: no `app_arg` leaks.
        assert!(!script.contains("app_arg"), "got:\n{script}");
        assert!(script.contains("app=app,"), "got:\n{script}");
    }

    #[test]
    fn launcher_domain_allowlist_rendered() {
        let net = ModalNetworkPolicy {
            block_network: false,
            domain_allowlist: vec!["api.github.com".to_string(), "*.anthropic.com".to_string()],
        };
        let cmd = vec!["/entry".to_string()];
        let script = render_modal_launcher(&LauncherParams {
            app_name: "flox-env",
            image_ref: "env-modal:tag",
            network: &net,
            command: &cmd,
            workdir: "/proj",
            timeout_secs: 600,
        });
        assert!(
            script.contains("outbound_domain_allowlist=['api.github.com', '*.anthropic.com']"),
            "got:\n{script}"
        );
        assert!(!script.contains("block_network=True"), "got:\n{script}");
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
        let script = render_modal_launcher(&LauncherParams {
            app_name: "flox-env",
            image_ref: "env-modal:tag",
            network: &block_all_policy(),
            command: &cmd,
            workdir: "/proj",
            timeout_secs: 60,
        });
        assert!(
            script.contains("'print(\\'hi\\')'"),
            "single quotes in command members must be escaped:\n{script}"
        );
    }

    #[test]
    fn launcher_is_valid_python_prologue() {
        let script = render_modal_launcher(&LauncherParams {
            app_name: "flox-env",
            image_ref: "env-modal:tag",
            network: &block_all_policy(),
            command: &["/entry".to_string()],
            workdir: "/proj",
            timeout_secs: 60,
        });
        assert!(
            script.starts_with("#!/usr/bin/env python3\n"),
            "got:\n{script}"
        );
        assert!(script.contains("sys.exit(returncode)"), "got:\n{script}");
    }

    #[test]
    fn launcher_create_block_is_well_indented() {
        // The *args (CMD members) and the keyword arguments must each sit on
        // their own 4-space-indented line inside `Sandbox.create(...)`, so the
        // emitted Python parses. This guards the formatdoc interpolation of the
        // pre-rendered `command_args` block against indentation drift.
        let script = render_modal_launcher(&LauncherParams {
            app_name: "flox-env",
            image_ref: "env-modal:tag",
            network: &block_all_policy(),
            command: &["/entry".to_string(), "activate".to_string()],
            workdir: "/proj",
            timeout_secs: 60,
        });
        assert!(
            script.contains(
                "sb = modal.Sandbox.create(\n    '/entry',\n    'activate',\n    app=app,\n"
            ),
            "create-block indentation drifted:\n{script}"
        );
        assert!(
            script.contains("    image=image,\n    workdir='/proj',\n    timeout=60,\n    block_network=True,\n)\n"),
            "kwarg indentation drifted:\n{script}"
        );
    }
}
