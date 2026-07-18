//! The `openshell` sandbox backend: run the environment via NVIDIA OpenShell.
//!
//! OpenShell wraps the activation inside a Docker-resident OCI container
//! managed by a local OpenShell gateway. flox's role is to bake the
//! environment's OCI image (reusing the existing containerize machinery),
//! load it into Docker's image store, and exec `openshell sandbox create`
//! with the right arguments.
//!
//! # Key behavioural differences from the `oci` backend
//!
//! - **Runtime is always Docker.** The builder stays as-is (Apple Container
//!   proxy on macOS / `MkContainerNix` on Linux), but the resulting image is
//!   loaded into Docker via `docker load`. Image resolution and tagging also
//!   run against Docker.
//! - **Tag namespace separation.** Images are tagged `<env>-openshell:<hash12>`
//!   to avoid colliding with the `oci` backend's `<env>:<hash12>` tags. The
//!   image contents differ (the compat layer adds the `sandbox` user, `iproute2`,
//!   and `/bin/sh`), so they must never share a tag.
//! - **Bake compat layer.** The image bake sets
//!   `_FLOX_CONTAINERIZE_OPENSHELL_COMPAT=1`, which causes `mkContainer.nix`
//!   to add the `sandbox` user/group (uid/gid 1000660000), `iproute2`, and
//!   `/bin/sh` — all required by the OpenShell supervisor.
//! - **Entrypoint recovery.** OpenShell replaces the image ENTRYPOINT with its
//!   own supervisor. The activation command is passed explicitly after `--` on
//!   `sandbox create`, recovered at runtime via `docker image inspect`.
//! - **Env re-injection.** `Config.Env` from the image (HOME, XDG_*, etc.) is
//!   not inherited by OpenShell's SSH-based execution; it is read via
//!   `docker image inspect` and re-injected as repeated `--env KEY=VALUE` flags.
//! - **Policy file.** OpenShell requires a `--policy <file>` YAML that declares
//!   filesystem visibility. flox generates one per activation under
//!   `<dot_flox>/cache/` and passes it to `sandbox create`. Network grants
//!   declared in the manifest (`[[options.sandbox.network]]`) are compiled
//!   into the policy's `network_policies` map, with `binary` install ids
//!   resolved to guest store paths via the lockfile — so declared endpoints
//!   are enforced from the sandbox's first instruction, and everything else
//!   stays deny-by-default.
//! - **Ephemerality.** `--no-keep` deletes the sandbox when the initial command
//!   exits, mirroring the OCI backend's `--rm`.
//! - **Working directory.** `sandbox create` has no `--workdir` flag; the cwd
//!   is set by wrapping the activation command in
//!   `/bin/sh -c 'cd "$1" && shift && exec "$@"' sh <workdir> <entrypoint...>`.
//!
//! # Env knobs (prototype)
//!
//! The same knobs used by the `oci` backend are reused for the prototype:
//! - `FLOX_SANDBOX_OCI_IMAGE` — explicit image ref override (skips bake).
//! - `FLOX_SANDBOX_OCI_ALLOW_STALE` — run the newest existing image even when
//!   the expected hash-tag is absent.
//! - `FLOX_SANDBOX_OCI_AUTOBAKE` — bake without prompting.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flox_core::activate::context::InvocationType;
use flox_core::activate::sandbox_backend::SandboxBackend;
use flox_core::activate::sandbox_policy::{
    SandboxNetworkAccess,
    SandboxNetworkProtocol,
    SandboxNetworkRule,
};
use flox_manifest::interfaces::AsLatestSchema;
use flox_manifest::lockfile::{LockedPackage, Lockfile};
use flox_rust_sdk::providers::container_builder::ContainerBuilderParams;
use semver::Version;
use serde_json::json;
use tracing::debug;

use super::bake::{
    bake_image,
    docker_image_entrypoint,
    docker_image_env,
    resolve_docker_image_state,
    stale_ref_for_state,
};
use super::preflight::{
    CliVersionCheck,
    DEFAULT_VERSION_ARGS,
    check_cli_version,
    first_on_path,
    split_endpoint,
};
use super::{ActivationSandbox, SandboxLaunchCtx};
use crate::commands::sandbox_backends::oci::{
    FLOX_SANDBOX_OCI_ALLOW_STALE_VAR,
    FLOX_SANDBOX_OCI_AUTOBAKE_VAR,
    OciBakeDecision,
    OciImageState,
    lockfile_hash12,
    should_bake_oci,
};

/// Marker env var that requests the OpenShell compat layer in `mkContainer.nix`
/// (sandbox user/group, iproute2, /bin/sh). Set by the shared bake pipeline
/// (`super::bake::bake_image`) and forwarded into the macOS builder VM by
/// `ContainerizeProxy`.
pub(crate) const OPENSHELL_COMPAT_ENV: &str = "_FLOX_CONTAINERIZE_OPENSHELL_COMPAT";

/// Tag repository suffix used for the OpenShell backend. Appended to `env_name`
/// so `<env>-openshell:<hash12>` never collides with the `oci` backend's
/// `<env>:<hash12>`.
const OPENSHELL_REPO_SUFFIX: &str = "-openshell";

/// Minimum supported OpenShell CLI version.
///
/// `sandbox create` gained `--env` in 0.0.59 and Docker bind mounts via
/// `--driver-config-json` in 0.0.62; the backend passes both, and older CLIs
/// reject them with a bare usage error. The prototype is tested against
/// 0.0.82.
const OPENSHELL_MIN_VERSION: Version = Version::new(0, 0, 62);

pub struct OpenshellBackend<'a> {
    dot_flox_path: PathBuf,
    env_name: String,
    invocation_type: &'a InvocationType,
    lockfile: &'a Lockfile,
    /// Whether to auto-bake without prompting. Consumed by `wrap_openshell`.
    sandbox_oci_autobake: bool,
    /// Narrow context for the container builder pipeline. Consumed by the
    /// shared bake pipeline (`super::bake::bake_image`).
    container_builder_params: ContainerBuilderParams,
}

impl<'a> OpenshellBackend<'a> {
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

impl ActivationSandbox for OpenshellBackend<'_> {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::Openshell
    }

    fn preflight(&self) -> Result<()> {
        let Some(openshell_path) = first_on_path("openshell") else {
            bail!(
                "The 'openshell' sandbox backend requires the OpenShell CLI, which was not \
                 found on PATH.\n\
                 Install it from https://github.com/NVIDIA/OpenShell#install, then re-run."
            );
        };
        check_openshell_version(&openshell_path)?;
        if first_on_path("docker").is_none() {
            bail!(
                "The 'openshell' sandbox backend requires Docker for image management, which \
                 was not found on PATH.\n\
                 Install Docker Desktop or the Docker CLI, then re-run."
            );
        }
        // Lightweight gateway reachability check.
        let status = std::process::Command::new("openshell")
            .arg("status")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        match status {
            Ok(s) if s.success() => {},
            _ => {
                bail!(
                    "The OpenShell gateway is not reachable ('openshell status' failed).\n\
                     Start it with 'openshell gateway select <name>' or check the gateway \
                     service is running."
                );
            },
        }
        Ok(())
    }

    fn wrap_activation(self: Box<Self>) -> Result<Infallible> {
        wrap_openshell(
            &self.dot_flox_path,
            &self.env_name,
            self.invocation_type,
            self.lockfile,
            self.sandbox_oci_autobake,
            &self.container_builder_params,
        )
    }
}

/// Verify the resolved OpenShell CLI meets [`OPENSHELL_MIN_VERSION`].
///
/// A too-old CLI would otherwise surface as a raw `unexpected argument
/// '--env'` usage error from `sandbox create`. The shared gate tolerates a
/// failed or unparseable `--version` (logged at debug); the message here
/// carries the OpenShell-specific shadowing and install guidance.
fn check_openshell_version(openshell_path: &Path) -> Result<()> {
    check_cli_version(openshell_path, &CliVersionCheck {
        tool_name: "OpenShell",
        backend_id: "openshell",
        min_version: OPENSHELL_MIN_VERSION,
        upgrade_hint: "A Flox environment providing 'openshell' may be shadowing a newer install.\n\
             If one is installed elsewhere, put its directory earlier on PATH.\n\
             Otherwise install the latest release from https://github.com/NVIDIA/OpenShell#install, then re-run.",
        version_args: DEFAULT_VERSION_ARGS,
    })
}

/// Return the `<env>-openshell` repository name used for Docker image tagging.
pub(crate) fn openshell_repo(env_name: &str) -> String {
    format!("{env_name}{OPENSHELL_REPO_SUFFIX}")
}

/// Run the activation inside an OpenShell sandbox, then never return.
fn wrap_openshell(
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

    let repo = openshell_repo(env_name);
    let state = resolve_docker_image_state(&repo, lockfile);

    let image_ref = match state {
        OciImageState::Explicit(ref image_ref) => {
            debug!(image_ref, "using explicit FLOX_SANDBOX_OCI_IMAGE override");
            image_ref.clone()
        },
        OciImageState::Present { ref image_ref } => {
            debug!(image_ref, "cache hit: content-hash tag present in Docker");
            image_ref.clone()
        },
        OciImageState::Stale {
            ref expected_ref, ..
        }
        | OciImageState::Missing { ref expected_ref } => {
            let is_missing = matches!(state, OciImageState::Missing { .. });
            let stale_ref_opt = if is_missing {
                None
            } else {
                stale_ref_for_state(&state)
            };

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
                stale_ref_opt,
            );

            match decision {
                OciBakeDecision::RunStale(ref run_ref) => {
                    eprintln!(
                        "⚠️  Running stale image '{run_ref}' (expected '{expected_ref}').\n   \
                         The environment has changed since this image was built.\n   \
                         Unset {FLOX_SANDBOX_OCI_ALLOW_STALE_VAR} and re-run to bake a fresh \
                         image."
                    );
                    run_ref.clone()
                },
                OciBakeDecision::Bake => {
                    bake_image(
                        &repo,
                        env_name,
                        dot_flox_path,
                        container_builder_params,
                        lockfile,
                    )?;
                    format!("{repo}:{}", lockfile_hash12(lockfile))
                },
                OciBakeDecision::Prompt => {
                    let reason = if is_missing {
                        "missing"
                    } else {
                        "stale (environment has changed since last bake)"
                    };
                    let stale_note = if let Some(s) = stale_ref_opt {
                        format!("\nExisting image: {s}")
                    } else {
                        String::new()
                    };
                    let msg = format!(
                        "OpenShell image '{expected_ref}' is {reason}.{stale_note}\n\
                         Bake now? (~2–5 min on first bake; later bakes reuse layers)"
                    );
                    let confirmed = inquire::Confirm::new(&msg)
                        .with_default(true)
                        .prompt()
                        .unwrap_or(false);
                    if confirmed {
                        bake_image(
                            &repo,
                            env_name,
                            dot_flox_path,
                            container_builder_params,
                            lockfile,
                        )?;
                        format!("{repo}:{}", lockfile_hash12(lockfile))
                    } else {
                        bail!(
                            "Bake declined. To build the image manually:\n  \
                             FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate --sandbox enforce \
                             --sandbox-backend openshell\n  \
                             or set sandbox_oci_autobake = true in 'flox config'."
                        );
                    }
                },
                OciBakeDecision::FailFast {
                    ref expected_ref,
                    ref stale_hint,
                } => {
                    bail!(
                        "OpenShell image '{expected_ref}' not found in the local Docker image \
                         store.\n\
                         To bake and load it automatically, set \
                         {FLOX_SANDBOX_OCI_AUTOBAKE_VAR}=true \
                         or run on an interactive terminal.{stale_hint}\n\
                         To build and load the image manually:\n  \
                         flox containerize -f img.tar --runtime docker\n  \
                         docker image load --input img.tar\n  \
                         (then: flox activate --sandbox enforce --sandbox-backend openshell)"
                    );
                },
            }
        },
    };

    let cwd = std::env::current_dir().unwrap_or_else(|_| project.clone());

    // Read the baked entrypoint and image env from Docker.
    let entrypoint = docker_image_entrypoint(&image_ref)?;
    let image_env = docker_image_env(&image_ref)?;

    // Generate the policy YAML for this activation, including any network
    // grants declared in the manifest.
    let policy_path = write_openshell_policy(dot_flox_path, &project, lockfile)?;

    // Build the `openshell sandbox create` argv.
    let sandbox_name = openshell_sandbox_name(env_name);
    let tty = resolve_tty(invocation);
    let (_, argv) = openshell_create_argv(&CreateArgvParams {
        image_ref: &image_ref,
        entrypoint: &entrypoint,
        image_env: &image_env,
        project: &project,
        cwd: &cwd,
        invocation,
        policy_path: &policy_path,
        sandbox_name: &sandbox_name,
        tty,
    });

    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new("openshell").args(&argv).exec();
    Err(anyhow::anyhow!(
        "Failed to launch the openshell sandbox: {err}."
    ))
}

// ── Policy YAML ───────────────────────────────────────────────────────────────

/// Policy YAML template for an OpenShell sandbox activation.
///
/// The `{project}` placeholder is replaced with the bind-mounted project
/// directory before writing. The paths listed here give the sandbox workable
/// read-only access to the Nix store and standard Linux directories, and
/// read-write access to the working dirs and the project bind-mount.
/// `network` holds the manifest's resolved `[[options.sandbox.network]]`
/// grants; an empty slice keeps the deny-all `network_policies: {}`.
pub(crate) fn openshell_policy_yaml(project: &Path, network: &[ResolvedNetworkRule]) -> String {
    format!(
        "version: 1\n\
         filesystem_policy:\n\
         \x20 include_workdir: true\n\
         \x20 read_only:\n\
         \x20   - /nix\n\
         \x20   - /etc\n\
         \x20   - /usr\n\
         \x20   - /lib\n\
         \x20   - /bin\n\
         \x20   - /proc\n\
         \x20   - /dev/urandom\n\
         \x20 read_write:\n\
         \x20   - /sandbox\n\
         \x20   - /tmp\n\
         \x20   - /dev/null\n\
         \x20   - /run\n\
         \x20   - /home/flox\n\
         \x20   - {}\n\
         landlock:\n\
         \x20 compatibility: best_effort\n\
         process:\n\
         \x20 run_as_user: sandbox\n\
         \x20 run_as_group: sandbox\n\
         {}",
        project.display(),
        render_network_policies(network)
    )
}

/// The Linux system the sandbox guest runs as.
///
/// The image is baked for the host's architecture on a Linux kernel; macOS
/// hosts run the guest inside the gateway's Docker VM. Lockfile lookups for
/// guest paths must therefore use this system, not the host's.
pub(crate) fn openshell_guest_system() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64-linux"
    } else {
        "x86_64-linux"
    }
}

/// A `[[options.sandbox.network]]` rule resolved against the lockfile:
/// endpoint split into host/port, defaults applied, and the `binary` spec
/// resolved to an absolute path inside the guest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedNetworkRule {
    /// Rule-map key, `allow_<host>_<port>` matching OpenShell's own
    /// generated-rule naming so audit-log lines look native. Deduplicated
    /// with a numeric suffix when several rules grant the same endpoint.
    key: String,
    host: String,
    port: u16,
    access: SandboxNetworkAccess,
    protocol: SandboxNetworkProtocol,
    binary: Option<String>,
}

/// Render the `network_policies` section of the policy YAML.
fn render_network_policies(rules: &[ResolvedNetworkRule]) -> String {
    if rules.is_empty() {
        return "network_policies: {}\n".to_string();
    }
    let mut out = String::from("network_policies:\n");
    for rule in rules {
        out.push_str(&format!("  {}:\n", rule.key));
        out.push_str(&format!("    name: {}\n", rule.key));
        out.push_str("    endpoints:\n");
        // Single-quoted: a wildcard host like `*.github.com` is an alias
        // token to a YAML parser when unquoted. The endpoint charset check
        // guarantees no quote or newline can appear in the host.
        out.push_str(&format!("      - host: '{}'\n", rule.host));
        out.push_str(&format!("        port: {}\n", rule.port));
        out.push_str(&format!("        protocol: {}\n", rule.protocol));
        // OpenShell's `enforcement` defaults to `audit` (violations are
        // logged but the request is allowed); enforcement must be explicit.
        out.push_str("        enforcement: enforce\n");
        out.push_str(&format!("        access: {}\n", rule.access));
        if let Some(binary) = &rule.binary {
            out.push_str("    binaries:\n");
            out.push_str(&format!("      - path: '{}'\n", binary.replace('\'', "''")));
        }
    }
    out
}

/// Resolve the manifest's network rules against the lockfile for the guest
/// system.
pub(crate) fn resolve_network_rules(
    rules: &[SandboxNetworkRule],
    lockfile: &Lockfile,
    system: &str,
) -> Result<Vec<ResolvedNetworkRule>> {
    let mut used_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut resolved = Vec::with_capacity(rules.len());
    for rule in rules {
        let (host, port) = split_endpoint(&rule.endpoint)?;
        let base_key = format!(
            "allow_{}_{}",
            host.chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect::<String>(),
            port
        );
        let mut key = base_key.clone();
        let mut suffix = 2;
        while !used_keys.insert(key.clone()) {
            key = format!("{base_key}_{suffix}");
            suffix += 1;
        }
        let binary = rule
            .binary
            .as_deref()
            .map(|spec| resolve_policy_binary(lockfile, system, spec))
            .transpose()?;
        resolved.push(ResolvedNetworkRule {
            key,
            host,
            port,
            access: rule.access.unwrap_or_default(),
            protocol: rule.protocol.unwrap_or_default(),
            binary,
        });
    }
    Ok(resolved)
}

/// Resolve a rule's `binary` spec to an absolute path inside the guest.
///
/// Accepted forms: an absolute path (used verbatim), an install id
/// (`"curl"` → `<store path>/bin/curl`), or `"<install-id>/<exe>"` for
/// packages whose executable name differs from the install id
/// (`"claude-code/.claude-wrapped"`). The store path comes from the
/// lockfile's locked outputs for `system`, so the grant tracks upgrades.
pub(crate) fn resolve_policy_binary(
    lockfile: &Lockfile,
    system: &str,
    spec: &str,
) -> Result<String> {
    // Every branch interpolates the spec into a single-quoted YAML scalar;
    // control characters or quotes would break out of it.
    if spec.chars().any(|c| c.is_ascii_control() || c == '\'') {
        bail!("Invalid sandbox network binary '{}'.", spec.escape_debug());
    }
    if spec.starts_with('/') {
        return Ok(spec.to_string());
    }
    let (install_id, exe) = match spec.split_once('/') {
        Some((install_id, exe)) => (install_id, exe),
        None => (spec, spec),
    };
    let package = lockfile
        .packages
        .iter()
        .find(|p| p.install_id() == install_id && p.system().as_str() == system)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Sandbox network rule names binary '{spec}', but package '{install_id}' is not locked for system '{system}'.\nAdd '{install_id}' to '[install]' in the manifest, or use an absolute path in the rule."
            )
        })?;
    let store_path = match package {
        LockedPackage::Catalog(pkg) => pick_binary_output(&pkg.outputs),
        LockedPackage::Flake(pkg) => pick_binary_output(&pkg.locked_installable.outputs),
        LockedPackage::StorePath(pkg) => Some(pkg.store_path.clone()),
    }
    .ok_or_else(|| {
        anyhow::anyhow!(
            "Package '{install_id}' has no locked store path outputs to resolve binary '{spec}' against."
        )
    })?;
    Ok(format!("{store_path}/bin/{exe}"))
}

/// Pick the output most likely to hold executables: `bin`, then `out`, then
/// the alphabetically first.
fn pick_binary_output(outputs: &BTreeMap<String, String>) -> Option<String> {
    outputs
        .get("bin")
        .or_else(|| outputs.get("out"))
        .or_else(|| outputs.values().next())
        .cloned()
}

/// Write the per-activation OpenShell policy YAML under `.flox/cache/` and
/// return the path.
fn write_openshell_policy(
    dot_flox_path: &Path,
    project: &Path,
    lockfile: &Lockfile,
) -> Result<PathBuf> {
    let manifest = lockfile
        .migrated_manifest()
        .context("failed to migrate the manifest for sandbox policy generation")?;
    let rules = manifest
        .as_latest_schema()
        .options
        .sandbox
        .as_ref()
        .and_then(|sandbox| sandbox.network.clone())
        .unwrap_or_default();
    let network = resolve_network_rules(&rules, lockfile, openshell_guest_system())?;
    let cache_dir = dot_flox_path.join("cache");
    std::fs::create_dir_all(&cache_dir)
        .with_context(|| format!("failed to create cache dir '{}'", cache_dir.display()))?;
    let policy_path = cache_dir.join("openshell-policy.yaml");
    let yaml = openshell_policy_yaml(project, &network);
    std::fs::write(&policy_path, &yaml)
        .with_context(|| format!("failed to write policy to '{}'", policy_path.display()))?;
    debug!(path = %policy_path.display(), "wrote openshell policy YAML");
    Ok(policy_path)
}

// ── Sandbox name ──────────────────────────────────────────────────────────────

/// Generate an OpenShell sandbox name from the environment name.
///
/// The name is `flox-<sanitized-env>-<pid>`, where `<sanitized-env>` is the
/// env name lowercased with any character that is not `[a-z0-9]` replaced by
/// a dash. The PID suffix ensures uniqueness when the same environment is
/// activated concurrently.
pub(crate) fn openshell_sandbox_name(env_name: &str) -> String {
    let sanitized: String = env_name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("flox-{}-{}", sanitized, std::process::id())
}

// ── TTY detection ─────────────────────────────────────────────────────────────

/// Resolve the `--tty` / `--no-tty` flag for `openshell sandbox create`.
///
/// Mirrors the OCI backend's logic: force `--tty` for Interactive invocations
/// when stdin is a terminal, `--no-tty` otherwise. Exec and shell-command
/// invocations always use `--no-tty` (no interactive terminal needed).
pub(crate) fn resolve_tty(invocation: &InvocationType) -> bool {
    match invocation {
        InvocationType::Interactive => std::io::stdin().is_terminal(),
        InvocationType::ExecCommand(_) | InvocationType::ShellCommand(_) => false,
        InvocationType::InPlace => false,
    }
}

// ── Argv construction ─────────────────────────────────────────────────────────

/// Parameters for [`openshell_create_argv`].
///
/// Grouping them here avoids exceeding clippy's default function-argument limit
/// while keeping the constructor call self-documenting.
pub(crate) struct CreateArgvParams<'a> {
    /// Docker image reference (`<repo>:<hash12>`).
    pub image_ref: &'a str,
    /// Baked image entrypoint recovered via `docker image inspect`.
    pub entrypoint: &'a [String],
    /// `Config.Env` entries from the image, pre-filtered for valid OpenShell
    /// names, to be re-injected as `--env KEY=VALUE` flags.
    pub image_env: &'a [String],
    /// Absolute path to the project directory.
    pub project: &'a Path,
    /// Current working directory on the host.
    pub cwd: &'a Path,
    /// How `flox activate` was invoked.
    pub invocation: &'a InvocationType,
    /// Path to the generated policy YAML.
    pub policy_path: &'a Path,
    /// Sandbox name passed as `--name`.
    pub sandbox_name: &'a str,
    /// Whether to pass `--tty` (`true`) or `--no-tty` (`false`).
    pub tty: bool,
}

/// Build the `openshell sandbox create` argv (pure function, no I/O).
///
/// Returns `("openshell", argv)` where `argv` is the full argument list
/// (excluding the binary itself).
pub(crate) fn openshell_create_argv(params: &CreateArgvParams<'_>) -> (String, Vec<String>) {
    let image_ref = params.image_ref;
    let entrypoint = params.entrypoint;
    let image_env = params.image_env;
    let project = params.project;
    let cwd = params.cwd;
    let invocation = params.invocation;
    let policy_path = params.policy_path;
    let sandbox_name = params.sandbox_name;
    let tty = params.tty;
    let mut argv: Vec<String> = vec!["sandbox".to_string(), "create".to_string()];

    argv.push("--from".to_string());
    argv.push(image_ref.to_string());

    argv.push("--name".to_string());
    argv.push(sandbox_name.to_string());

    argv.push("--no-keep".to_string());

    argv.push("--policy".to_string());
    argv.push(policy_path.display().to_string());

    // Re-inject Config.Env (not inherited by OpenShell's SSH execution path).
    for entry in image_env {
        argv.push("--env".to_string());
        argv.push(entry.clone());
    }

    // Bind-mount the project at its identical absolute path so the guest
    // sees the same paths as the host. enable_bind_mounts must be true in
    // the local gateway config.
    //
    // Build via serde_json so that paths containing `"` or other special
    // characters are properly JSON-escaped (a format!-interpolated string
    // would produce malformed JSON in that case).
    let driver_config = json!({
        "docker": {
            "mounts": [{
                "type": "bind",
                "source": project,
                "target": project,
                "read_only": false
            }]
        }
    })
    .to_string();
    argv.push("--driver-config-json".to_string());
    argv.push(driver_config);

    // TTY: mirror OCI backend logic.
    if tty {
        argv.push("--tty".to_string());
    } else {
        argv.push("--no-tty".to_string());
    }

    // Determine the effective working directory: use cwd when under the
    // project, otherwise fall back to the project root.
    let effective_cwd = if cwd.starts_with(project) {
        cwd
    } else {
        project
    };

    // `sandbox create` has no --workdir; wrap the command to set cwd.
    // /bin/sh is guaranteed by the compat layer.
    argv.push("--".to_string());

    match invocation {
        InvocationType::Interactive => {
            // Interactive: cd to effective_cwd, then exec the entrypoint.
            // The entrypoint starts the activated shell.
            append_workdir_wrapper(&mut argv, effective_cwd, entrypoint, &[]);
        },
        InvocationType::ExecCommand(cmd) => {
            // Exec: cd to effective_cwd, then exec the entrypoint followed by
            // the user's command so the activation context wraps it.
            append_workdir_wrapper(&mut argv, effective_cwd, entrypoint, cmd);
        },
        InvocationType::ShellCommand(shell_cmd) => {
            // Shell command: wrap in `sh -c` so pipelines and builtins work.
            let sh_cmd = vec!["sh".to_string(), "-c".to_string(), shell_cmd.clone()];
            append_workdir_wrapper(&mut argv, effective_cwd, entrypoint, &sh_cmd);
        },
        InvocationType::InPlace => {
            unreachable!(
                "in-place invocation cannot reach the openshell backend (blocked by \
                 ensure_sandbox_not_in_place)"
            );
        },
    }

    ("openshell".to_string(), argv)
}

/// Append the workdir-wrapper construction to `argv`.
///
/// Produces: `/bin/sh -c 'cd "$1" && shift && exec "$@"' sh <workdir>
///           [entrypoint...] [extra_cmd...]`
///
/// This is the only portable way to set the working directory when the
/// runtime provides no `--workdir` flag. The inner `sh` is used as `$0`
/// (argv[0] for the shell invocation, not executed). The construction
/// preserves argument boundaries without quoting: each element is a separate
/// argv member passed directly to exec, so no shell injection is possible.
fn append_workdir_wrapper(
    argv: &mut Vec<String>,
    workdir: &Path,
    entrypoint: &[String],
    extra_cmd: &[String],
) {
    argv.push("/bin/sh".to_string());
    argv.push("-c".to_string());
    // The cd-shift-exec script: positional 1 is the workdir, the rest is the
    // command to exec. "shift" removes $1 so "$@" is the command only.
    argv.push("cd \"$1\" && shift && exec \"$@\"".to_string());
    // $0 for the inner sh (cosmetic; shown in process listings).
    argv.push("sh".to_string());
    // $1: the working directory.
    argv.push(workdir.display().to_string());
    // $2…: entrypoint followed by any extra command.
    argv.extend(entrypoint.iter().cloned());
    argv.extend(extra_cmd.iter().cloned());
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::bake::env_entry_valid;
    use super::*;

    // ── openshell_sandbox_name ────────────────────────────────────────────────

    #[test]
    fn sandbox_name_prefix_and_pid() {
        let name = openshell_sandbox_name("MyEnv");
        assert!(name.starts_with("flox-myenv-"), "got: {name}");
        let pid_str = name.strip_prefix("flox-myenv-").unwrap();
        assert!(
            pid_str.parse::<u32>().is_ok(),
            "suffix must be the PID: {pid_str}"
        );
    }

    #[test]
    fn sandbox_name_sanitizes_special_chars() {
        let name = openshell_sandbox_name("my.env-v2 beta");
        assert!(name.starts_with("flox-my-env-v2-beta-"), "got: {name}");
    }

    // ── policy YAML ───────────────────────────────────────────────────────────

    #[test]
    fn policy_yaml_contains_project_path() {
        let project = Path::new("/home/user/myproject");
        let yaml = openshell_policy_yaml(project, &[]);
        assert!(yaml.contains("/home/user/myproject"), "got:\n{yaml}");
        assert!(yaml.contains("version: 1"), "got:\n{yaml}");
        assert!(yaml.contains("run_as_user: sandbox"), "got:\n{yaml}");
        assert!(yaml.contains("run_as_group: sandbox"), "got:\n{yaml}");
        assert!(yaml.contains("read_only:"), "got:\n{yaml}");
        assert!(yaml.contains("read_write:"), "got:\n{yaml}");
        assert!(yaml.contains("/nix"), "got:\n{yaml}");
        assert!(yaml.contains("network_policies: {}"), "got:\n{yaml}");
        assert!(yaml.contains("landlock:"), "got:\n{yaml}");
        assert!(yaml.contains("best_effort"), "got:\n{yaml}");
    }

    // ── manifest network rules ────────────────────────────────────────────────

    fn fixture_lockfile(env: &str) -> Lockfile {
        let path = flox_test_utils::GENERATED_DATA.join(format!("envs/{env}/manifest.lock"));
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        content
            .parse()
            .unwrap_or_else(|e| panic!("parse {}: {e:?}", path.display()))
    }

    fn network_rule(endpoint: &str, binary: Option<&str>) -> SandboxNetworkRule {
        SandboxNetworkRule {
            endpoint: endpoint.to_string(),
            access: None,
            protocol: None,
            binary: binary.map(String::from),
        }
    }

    #[test]
    fn network_rules_render_into_policy_yaml() {
        let lockfile = fixture_lockfile("hello");
        let rules = [SandboxNetworkRule {
            endpoint: "api.github.com:443".to_string(),
            access: Some(SandboxNetworkAccess::ReadOnly),
            protocol: None,
            binary: Some("hello".to_string()),
        }];
        let resolved = resolve_network_rules(&rules, &lockfile, "aarch64-linux").unwrap();
        let yaml = openshell_policy_yaml(Path::new("/home/user/p"), &resolved);
        let expected = indoc::indoc! {"
            network_policies:
              allow_api_github_com_443:
                name: allow_api_github_com_443
                endpoints:
                  - host: 'api.github.com'
                    port: 443
                    protocol: rest
                    enforcement: enforce
                    access: read-only
                binaries:
                  - path: '/nix/store/g383j16f2lxz4zzs967i9hjgvivy6q7h-hello-2.12.3/bin/hello'
        "};
        assert!(yaml.ends_with(expected), "got:\n{yaml}");
        assert!(!yaml.contains("network_policies: {}"), "got:\n{yaml}");
    }

    #[test]
    fn network_rule_defaults_are_full_rest_unscoped() {
        let lockfile = fixture_lockfile("hello");
        let resolved = resolve_network_rules(
            &[network_rule("example.com:8080", None)],
            &lockfile,
            "aarch64-linux",
        )
        .unwrap();
        assert_eq!(resolved, vec![ResolvedNetworkRule {
            key: "allow_example_com_8080".to_string(),
            host: "example.com".to_string(),
            port: 8080,
            access: SandboxNetworkAccess::Full,
            protocol: SandboxNetworkProtocol::Rest,
            binary: None,
        }]);
        let yaml = render_network_policies(&resolved);
        assert!(!yaml.contains("binaries:"), "got:\n{yaml}");
    }

    #[test]
    fn duplicate_endpoints_get_deduplicated_keys() {
        let lockfile = fixture_lockfile("hello");
        let rules = [
            network_rule("example.com:443", None),
            network_rule("example.com:443", Some("hello")),
        ];
        let resolved = resolve_network_rules(&rules, &lockfile, "aarch64-linux").unwrap();
        assert_eq!(resolved[0].key, "allow_example_com_443");
        assert_eq!(resolved[1].key, "allow_example_com_443_2");
    }

    #[test]
    fn endpoint_without_port_is_rejected() {
        let lockfile = fixture_lockfile("hello");
        let err = resolve_network_rules(
            &[network_rule("example.com", None)],
            &lockfile,
            "aarch64-linux",
        )
        .unwrap_err();
        assert!(err.to_string().contains("<HOST>:<PORT>"), "got: {err}");
    }

    #[test]
    fn endpoint_with_invalid_host_is_rejected() {
        let lockfile = fixture_lockfile("hello");
        let err = resolve_network_rules(
            &[network_rule("bad host\nhost:443", None)],
            &lockfile,
            "aarch64-linux",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("Invalid sandbox network endpoint"),
            "got: {err}"
        );
    }

    #[test]
    fn binary_install_id_resolves_to_guest_store_path() {
        let lockfile = fixture_lockfile("hello");
        let path = resolve_policy_binary(&lockfile, "aarch64-linux", "hello").unwrap();
        assert_eq!(
            path,
            "/nix/store/g383j16f2lxz4zzs967i9hjgvivy6q7h-hello-2.12.3/bin/hello"
        );
    }

    #[test]
    fn binary_id_slash_exe_form_overrides_executable_name() {
        let lockfile = fixture_lockfile("hello");
        let path =
            resolve_policy_binary(&lockfile, "aarch64-linux", "hello/.hello-wrapped").unwrap();
        assert_eq!(
            path,
            "/nix/store/g383j16f2lxz4zzs967i9hjgvivy6q7h-hello-2.12.3/bin/.hello-wrapped"
        );
    }

    #[test]
    fn binary_absolute_path_passes_through() {
        let lockfile = fixture_lockfile("hello");
        let path = resolve_policy_binary(&lockfile, "aarch64-linux", "/usr/bin/curl").unwrap();
        assert_eq!(path, "/usr/bin/curl");
    }

    #[test]
    fn wildcard_host_renders_single_quoted() {
        let lockfile = fixture_lockfile("hello");
        let resolved = resolve_network_rules(
            &[network_rule("*.github.com:443", None)],
            &lockfile,
            "aarch64-linux",
        )
        .unwrap();
        let yaml = render_network_policies(&resolved);
        // Unquoted, `*.github.com` is a YAML alias token and the whole
        // policy fails to parse.
        assert!(yaml.contains("- host: '*.github.com'"), "got:\n{yaml}");
    }

    #[test]
    fn binary_spec_with_control_chars_rejected() {
        let lockfile = fixture_lockfile("hello");
        for spec in ["hello/foo\nbar", "/usr/bin/cu\nrl", "hello'"] {
            let err = resolve_policy_binary(&lockfile, "aarch64-linux", spec).unwrap_err();
            assert!(
                err.to_string().contains("Invalid sandbox network binary"),
                "spec {spec:?} got: {err}"
            );
        }
    }

    #[test]
    fn binary_unknown_install_id_errors_with_next_step() {
        let lockfile = fixture_lockfile("hello");
        let err = resolve_policy_binary(&lockfile, "aarch64-linux", "curl").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("not locked for system 'aarch64-linux'"),
            "got: {msg}"
        );
        assert!(msg.contains("[install]"), "got: {msg}");
    }

    #[test]
    fn guest_system_is_linux() {
        assert!(openshell_guest_system().ends_with("-linux"));
    }

    // ── resolve_tty ───────────────────────────────────────────────────────────

    #[test]
    fn exec_command_never_tty() {
        let inv = InvocationType::ExecCommand(vec!["ls".to_string()]);
        assert!(!resolve_tty(&inv));
    }

    #[test]
    fn shell_command_never_tty() {
        let inv = InvocationType::ShellCommand("echo hi".to_string());
        assert!(!resolve_tty(&inv));
    }

    // ── openshell_create_argv ─────────────────────────────────────────────────

    fn fake_entrypoint() -> Vec<String> {
        vec![
            "/nix/store/abc/libexec/flox-activations".to_string(),
            "activate".to_string(),
            "--activate-data".to_string(),
            "/nix/store/def/activate-ctx".to_string(),
        ]
    }

    fn fake_image_env() -> Vec<String> {
        vec![
            "HOME=/home/flox".to_string(),
            "PATH=/usr/bin:/bin".to_string(),
        ]
    }

    #[test]
    fn argv_starts_with_sandbox_create() {
        let project = Path::new("/home/user/proj");
        let cwd = Path::new("/home/user/proj");
        let policy = Path::new("/tmp/policy.yaml");
        let inv = InvocationType::ExecCommand(vec!["ls".to_string()]);
        let (bin, argv) = openshell_create_argv(&CreateArgvParams {
            image_ref: "myenv-openshell:abc123def456",
            entrypoint: &fake_entrypoint(),
            image_env: &fake_image_env(),
            project,
            cwd,
            invocation: &inv,
            policy_path: policy,
            sandbox_name: "flox-myenv-1234",
            tty: false,
        });
        assert_eq!(bin, "openshell");
        assert_eq!(argv[0], "sandbox");
        assert_eq!(argv[1], "create");
    }

    #[test]
    fn argv_has_no_keep_and_policy() {
        let project = Path::new("/home/user/proj");
        let cwd = project;
        let policy = Path::new("/home/user/proj/.flox/cache/openshell-policy.yaml");
        let inv = InvocationType::ExecCommand(vec!["true".to_string()]);
        let (_, argv) = openshell_create_argv(&CreateArgvParams {
            image_ref: "myenv-openshell:abc123",
            entrypoint: &fake_entrypoint(),
            image_env: &fake_image_env(),
            project,
            cwd,
            invocation: &inv,
            policy_path: policy,
            sandbox_name: "flox-myenv-99",
            tty: false,
        });
        assert!(argv.contains(&"--no-keep".to_string()), "argv: {argv:?}");
        let pol_pos = argv.iter().position(|a| a == "--policy").unwrap();
        assert_eq!(
            argv[pol_pos + 1],
            "/home/user/proj/.flox/cache/openshell-policy.yaml"
        );
    }

    #[test]
    fn argv_no_tty_flag() {
        let project = Path::new("/proj");
        let cwd = project;
        let policy = Path::new("/tmp/p.yaml");
        let inv = InvocationType::ExecCommand(vec!["ls".to_string()]);
        let (_, argv) = openshell_create_argv(&CreateArgvParams {
            image_ref: "img:tag",
            entrypoint: &[],
            image_env: &[],
            project,
            cwd,
            invocation: &inv,
            policy_path: policy,
            sandbox_name: "flox-env-1",
            tty: false,
        });
        assert!(argv.contains(&"--no-tty".to_string()), "argv: {argv:?}");
        assert!(!argv.contains(&"--tty".to_string()), "argv: {argv:?}");
    }

    #[test]
    fn argv_tty_flag_when_requested() {
        let project = Path::new("/proj");
        let cwd = project;
        let policy = Path::new("/tmp/p.yaml");
        let inv = InvocationType::Interactive;
        let (_, argv) = openshell_create_argv(&CreateArgvParams {
            image_ref: "img:tag",
            entrypoint: &fake_entrypoint(),
            image_env: &[],
            project,
            cwd,
            invocation: &inv,
            policy_path: policy,
            sandbox_name: "flox-env-1",
            tty: true,
        });
        assert!(argv.contains(&"--tty".to_string()), "argv: {argv:?}");
        assert!(!argv.contains(&"--no-tty".to_string()), "argv: {argv:?}");
    }

    #[test]
    fn argv_env_reinjection() {
        let project = Path::new("/proj");
        let cwd = project;
        let policy = Path::new("/tmp/p.yaml");
        let image_env = vec![
            "HOME=/home/flox".to_string(),
            "XDG_RUNTIME_DIR=/run".to_string(),
        ];
        let inv = InvocationType::ExecCommand(vec!["ls".to_string()]);
        let (_, argv) = openshell_create_argv(&CreateArgvParams {
            image_ref: "img:tag",
            entrypoint: &[],
            image_env: &image_env,
            project,
            cwd,
            invocation: &inv,
            policy_path: policy,
            sandbox_name: "flox-env-1",
            tty: false,
        });
        // Each env entry should be preceded by --env
        for entry in &image_env {
            let pos = argv.iter().position(|a| a == entry).expect(entry);
            assert_eq!(argv[pos - 1], "--env", "missing --env before {entry}");
        }
    }

    #[test]
    fn argv_env_invalid_name_filtered() {
        let project = Path::new("/proj");
        let cwd = project;
        let policy = Path::new("/tmp/p.yaml");
        // mix of valid and invalid entries
        let image_env: Vec<String> = vec![
            "HOME=/home/flox".to_string(),
            "OPENSHELL_TOKEN=secret".to_string(),
            "1INVALID=nope".to_string(),
            "PATH=/usr/bin".to_string(),
        ]
        .into_iter()
        .filter(|e| env_entry_valid(e))
        .collect();
        let inv = InvocationType::ExecCommand(vec!["ls".to_string()]);
        let (_, argv) = openshell_create_argv(&CreateArgvParams {
            image_ref: "img:tag",
            entrypoint: &[],
            image_env: &image_env,
            project,
            cwd,
            invocation: &inv,
            policy_path: policy,
            sandbox_name: "flox-env-1",
            tty: false,
        });
        assert!(
            !argv.iter().any(|a| a.contains("OPENSHELL_TOKEN")),
            "OPENSHELL_TOKEN must be filtered: {argv:?}"
        );
        assert!(
            !argv.iter().any(|a| a.contains("1INVALID")),
            "1INVALID must be filtered: {argv:?}"
        );
        assert!(
            argv.iter().any(|a| a == "HOME=/home/flox"),
            "HOME must survive: {argv:?}"
        );
    }

    #[test]
    fn argv_bind_mount_json_shape() {
        let project = Path::new("/home/user/project");
        let cwd = project;
        let policy = Path::new("/tmp/p.yaml");
        let inv = InvocationType::ExecCommand(vec!["ls".to_string()]);
        let (_, argv) = openshell_create_argv(&CreateArgvParams {
            image_ref: "img:tag",
            entrypoint: &[],
            image_env: &[],
            project,
            cwd,
            invocation: &inv,
            policy_path: policy,
            sandbox_name: "flox-env-1",
            tty: false,
        });
        let dconf_pos = argv
            .iter()
            .position(|a| a == "--driver-config-json")
            .unwrap();
        let json_val = &argv[dconf_pos + 1];
        let parsed: serde_json::Value =
            serde_json::from_str(json_val).expect("driver-config-json must be valid JSON");
        let mount = &parsed["docker"]["mounts"][0];
        assert_eq!(mount["type"], "bind");
        assert_eq!(mount["source"], "/home/user/project");
        assert_eq!(mount["target"], "/home/user/project");
        assert_eq!(mount["read_only"], false);
    }

    #[test]
    fn argv_bind_mount_path_with_space_is_valid_json() {
        // Paths containing spaces (or other special chars) must be JSON-escaped;
        // a format!-interpolated path would produce malformed JSON.
        let project = Path::new("/home/user/my project");
        let cwd = project;
        let policy = Path::new("/tmp/p.yaml");
        let inv = InvocationType::ExecCommand(vec!["ls".to_string()]);
        let (_, argv) = openshell_create_argv(&CreateArgvParams {
            image_ref: "img:tag",
            entrypoint: &fake_entrypoint(),
            image_env: &[],
            project,
            cwd,
            invocation: &inv,
            policy_path: policy,
            sandbox_name: "flox-env-1",
            tty: false,
        });
        let dconf_pos = argv
            .iter()
            .position(|a| a == "--driver-config-json")
            .unwrap();
        let json_val = &argv[dconf_pos + 1];
        // Must parse without error — a format!-built string would have produced
        // e.g. `"source":"/home/user/my project"` with an unescaped space,
        // which is valid JSON, but a path with `"` would break the format!
        // approach. serde_json::json! handles all cases correctly.
        let parsed: serde_json::Value =
            serde_json::from_str(json_val).expect("driver-config-json must be valid JSON");
        let mount = &parsed["docker"]["mounts"][0];
        assert_eq!(mount["source"], "/home/user/my project");
        assert_eq!(mount["target"], "/home/user/my project");
        // Also verify the workdir wrapper uses the path verbatim (no corruption).
        assert!(
            argv.contains(&"/home/user/my project".to_string()),
            "space in path must appear verbatim in workdir wrapper: {argv:?}"
        );
    }

    #[test]
    fn argv_workdir_under_project_uses_cwd() {
        let project = Path::new("/home/user/proj");
        let cwd = Path::new("/home/user/proj/src");
        let policy = Path::new("/tmp/p.yaml");
        let inv = InvocationType::ExecCommand(vec!["ls".to_string()]);
        let (_, argv) = openshell_create_argv(&CreateArgvParams {
            image_ref: "img:tag",
            entrypoint: &fake_entrypoint(),
            image_env: &[],
            project,
            cwd,
            invocation: &inv,
            policy_path: policy,
            sandbox_name: "flox-env-1",
            tty: false,
        });
        // The cd argument must be cwd (under project)
        let sh_script_pos = argv.iter().position(|a| a == "sh").unwrap();
        // After "-- /bin/sh -c <script> sh", next is the workdir
        assert!(
            argv.contains(&"/home/user/proj/src".to_string()),
            "cwd under project must be used as workdir: {argv:?}"
        );
        let _ = sh_script_pos; // used to locate context
    }

    #[test]
    fn argv_workdir_outside_project_falls_back_to_project() {
        let project = Path::new("/home/user/proj");
        let cwd = Path::new("/tmp/other");
        let policy = Path::new("/tmp/p.yaml");
        let inv = InvocationType::ExecCommand(vec!["ls".to_string()]);
        let (_, argv) = openshell_create_argv(&CreateArgvParams {
            image_ref: "img:tag",
            entrypoint: &fake_entrypoint(),
            image_env: &[],
            project,
            cwd,
            invocation: &inv,
            policy_path: policy,
            sandbox_name: "flox-env-1",
            tty: false,
        });
        assert!(
            argv.contains(&"/home/user/proj".to_string()),
            "project must be workdir when cwd is outside: {argv:?}"
        );
        assert!(
            !argv.contains(&"/tmp/other".to_string()),
            "external cwd must not appear: {argv:?}"
        );
    }

    #[test]
    fn argv_shell_command_wraps_in_sh_c() {
        let project = Path::new("/proj");
        let cwd = project;
        let policy = Path::new("/tmp/p.yaml");
        let inv = InvocationType::ShellCommand("echo hello | cat".to_string());
        let (_, argv) = openshell_create_argv(&CreateArgvParams {
            image_ref: "img:tag",
            entrypoint: &fake_entrypoint(),
            image_env: &[],
            project,
            cwd,
            invocation: &inv,
            policy_path: policy,
            sandbox_name: "flox-env-1",
            tty: false,
        });
        // The sh -c wrapper should wrap around the entrypoint + [sh, -c, ...]
        let after_sep = argv.iter().position(|a| a == "--").unwrap() + 1;
        assert_eq!(argv[after_sep], "/bin/sh");
        assert_eq!(argv[after_sep + 1], "-c");
        // The script is the cd-shift-exec idiom
        assert!(
            argv[after_sep + 2].contains("exec"),
            "must contain exec in workdir wrapper: {:?}",
            argv[after_sep + 2]
        );
        // The shell command itself must appear after the entrypoint
        assert!(
            argv.contains(&"echo hello | cat".to_string()),
            "shell_cmd must appear in argv: {argv:?}"
        );
    }

    #[test]
    fn argv_workdir_wrapper_quoting_exec_command() {
        // Verify that the argv is boundary-safe: each element is a separate
        // string (no shell parsing of arguments) so no quoting injection
        // is possible even with spaces or special chars in the command.
        let project = Path::new("/proj");
        let cwd = project;
        let policy = Path::new("/tmp/p.yaml");
        let cmd = vec![
            "python3".to_string(),
            "-c".to_string(),
            "print('hello world')".to_string(),
        ];
        let inv = InvocationType::ExecCommand(cmd.clone());
        let (_, argv) = openshell_create_argv(&CreateArgvParams {
            image_ref: "img:tag",
            entrypoint: &fake_entrypoint(),
            image_env: &[],
            project,
            cwd,
            invocation: &inv,
            policy_path: policy,
            sandbox_name: "flox-env-1",
            tty: false,
        });
        // Each element of cmd must appear as a distinct argv element.
        for part in &cmd {
            assert!(argv.contains(part), "cmd part '{part}' missing: {argv:?}");
        }
    }

    // ── tag namespace ─────────────────────────────────────────────────────────

    #[test]
    fn openshell_repo_has_suffix() {
        let repo = openshell_repo("myenv");
        assert_eq!(repo, "myenv-openshell");
    }

    #[test]
    fn openshell_repo_never_collides_with_oci_tag() {
        let env = "myenv";
        let hash = "abc123def456";
        let oci_tag = format!("{env}:{hash}");
        let os_tag = format!("{}:{hash}", openshell_repo(env));
        assert_ne!(oci_tag, os_tag);
    }
}
