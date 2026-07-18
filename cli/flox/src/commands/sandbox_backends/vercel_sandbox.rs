//! The `vercel-sandbox` backend: run the environment in a remote Vercel Sandbox.
//!
//! Vercel Sandbox is a cloud-API code-execution provider backed by Firecracker
//! microVMs (Amazon Linux 2023). It is the first BOOTSTRAP-shaped backend, and
//! it is deliberately unlike every OCI-ingesting cloud backend here:
//! `Sandbox.create` boots one of a FIXED set of base runtimes (`node22`,
//! `node24`, `python3.13`) and seeds it from a git source — there is no path to
//! hand it the baked OCI closure. So flox does not bake. Instead it generates a
//! two-artifact hand-off:
//!
//! 1. **A flox bootstrap script** (`vercel-sandbox-bootstrap.sh`): runs inside
//!    the fixed runtime, installs Flox, and activates the environment from
//!    FloxHub. This is the "environment closure" for a runtime flox cannot pre-
//!    seed with an image.
//! 2. **A `@vercel/sandbox` launcher** (`vercel-sandbox-launch.mjs`): a Node/ESM
//!    program that `Sandbox.create`s a fixed-runtime sandbox, uploads the
//!    bootstrap, and runs it, streaming the output.
//!
//! Cloud-remote: nothing runs on the host, so host assertions are preflight-only
//! and the threat model inverts (the host filesystem is unreachable from the
//! sandbox, but the code and any injected secrets leave the laptop).
//! Credentials belong in Vercel's own secret mechanism, not a local `.env`.
//!
//! # The determinism tradeoff: FloxHub-remote activation, not a pushed closure
//!
//! A bootstrap-shaped provider forces a choice the OCI backends never face. Two
//! options seed a locked environment into a fixed runtime:
//!
//! - **FloxHub-remote activation** (chosen here): the bootstrap runs
//!   `flox activate -r <owner>/<env>`, pulling the environment from FloxHub
//!   inside the sandbox. Self-contained — no artifact store to stand up — but it
//!   depends on FloxHub availability and on the sandbox's egress reaching
//!   FloxHub at bootstrap time, and it resolves the *pushed* revision rather than
//!   a byte-for-byte closure captured at `flox activate --sandbox` time.
//! - **A pushed closure** (not chosen): copy the content-addressed store closure
//!   to an artifact store the sandbox can pull from, then realize it in-VM. Fully
//!   deterministic, but it needs an artifact-store stage this seam does not have
//!   yet (see the seam-friction note: a shared "bootstrap bundle" stage).
//!
//! FloxHub-remote wins for the prototype because it completes honestly with only
//! a `flox push` as the extra prerequisite — no bespoke artifact plumbing — and
//! the reproducibility gap is bounded (the FloxHub revision is itself a locked
//! environment). The pushed-closure path is recorded as the durable improvement.
//!
//! # Network policy: DECLINED (the load-bearing lossiness)
//!
//! The `@vercel/sandbox` SDK exposes NO per-sandbox egress allowlist or firewall
//! vocabulary. `ports` on `Sandbox.create` governs INBOUND exposure only
//! (`sandbox.domain(port)` returns a public URL for a port the sandbox listens
//! on); it does not filter outbound traffic. There is no `allowedDomains`,
//! `blockNetwork`, or CIDR analog. So flox cannot compile a manifest egress grant
//! onto this provider at all. Per the backend contract's "decline what the
//! provider cannot express rather than silently widen" rule, a
//! `[[options.sandbox.network]]` grant is DECLINED with a clear error naming the
//! missing egress vocabulary — never dropped (which would falsely imply the grant
//! was honored) and never widened. The absence of grants is fine (the sandbox
//! runs with Vercel's default network posture, which flox does not control). This
//! is why the capabilities row declares `domain_egress = false`, unlike every
//! other cloud backend here.
//!
//! # Why this backend does not complete the launch on any host today
//!
//! Two external prerequisites gate the remote launch, and neither can be
//! satisfied from a bare checkout:
//!
//! - **A Vercel account and token.** The SDK authenticates with a Vercel OIDC
//!   token (`vercel env pull` writes it to `.env.local`, 12-hour lifetime) or an
//!   access token (`VERCEL_TOKEN` + `VERCEL_TEAM_ID` + `VERCEL_PROJECT_ID`).
//!   `preflight` distinguishes *CLI-missing* from *CLI-present-but-
//!   unauthenticated* cheaply and non-interactively (it never opens a browser).
//! - **The environment reachable from FloxHub.** Because the bootstrap activates
//!   `-r <owner>/<env>`, the environment must be pushed to FloxHub first. The
//!   operator names the FloxHub ref via `FLOX_SANDBOX_VERCEL_FLOXHUB_REF`;
//!   without it the artifact uses a `<owner>/<env>` placeholder the launch-
//!   boundary message calls out.
//!
//! Rather than fake success, this backend implements the deepest honest slice: it
//! runs the real preflight, generates the real bootstrap + launcher artifacts,
//! and then fails at the launch boundary with a "requires ..." error naming the
//! two prerequisites and pointing at the artifacts.
//!
//! # Env knobs (prototype)
//!
//! - `FLOX_SANDBOX_VERCEL_RUNTIME` — the fixed base runtime the launcher boots
//!   (`node22` | `node24` | `python3.13`; default `node24`). Validated against
//!   the known set.
//! - `FLOX_SANDBOX_VERCEL_FLOXHUB_REF` — the `<owner>/<env>` FloxHub reference the
//!   bootstrap activates. Recorded in the artifacts so a credentialed operator
//!   does not have to hand-edit them after `flox push`.

use std::convert::Infallible;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flox_core::activate::context::InvocationType;
use flox_core::activate::sandbox_backend::SandboxBackend;
use flox_manifest::lockfile::Lockfile;
use semver::Version;
use tracing::debug;

use super::handoff::{json_str_lit, manifest_network_rules};
use super::preflight::{CliVersionCheck, DEFAULT_VERSION_ARGS, check_cli_version, first_on_path};
use super::{ActivationSandbox, SandboxLaunchCtx};

/// The fixed base runtime the launcher boots the sandbox from. Vercel Sandbox
/// does not ingest an arbitrary image, so this selects one of the provider's
/// stock runtimes; the bootstrap installs Flox on top of it.
pub(crate) const FLOX_SANDBOX_VERCEL_RUNTIME_VAR: &str = "FLOX_SANDBOX_VERCEL_RUNTIME";

/// The `<owner>/<env>` FloxHub reference the bootstrap activates with
/// `flox activate -r`. Because the runtime is fixed and no image is pushed, the
/// environment must be reachable from FloxHub; this names it so the artifacts do
/// not carry a placeholder.
pub(crate) const FLOX_SANDBOX_VERCEL_FLOXHUB_REF_VAR: &str = "FLOX_SANDBOX_VERCEL_FLOXHUB_REF";

/// Access-token environment variable the Vercel SDK authenticates with (used
/// together with `VERCEL_TEAM_ID` and `VERCEL_PROJECT_ID`).
const VERCEL_TOKEN_VAR: &str = "VERCEL_TOKEN";

/// OIDC-token environment variable the Vercel SDK prefers. `vercel env pull`
/// writes it to `.env.local`; the SDK also reads it from the environment.
const VERCEL_OIDC_TOKEN_VAR: &str = "VERCEL_OIDC_TOKEN";

/// Minimum supported Vercel CLI version.
///
/// The OIDC-token auth flow the SDK relies on (`vercel env pull` writing
/// `VERCEL_OIDC_TOKEN`) is well-established in the 30+ CLI line; the prototype is
/// developed against 56.x. Pinned conservatively to a 30.0 floor, and the shared
/// gate tolerates a failed or unparseable `--version` so an unusual build never
/// blocks.
const VERCEL_MIN_VERSION: Version = Version::new(30, 0, 0);

/// The fixed base runtimes `Sandbox.create` accepts. Verified against the
/// `@vercel/sandbox` README (2026-07): the SDK boots one of these Amazon Linux
/// 2023 images; there is no arbitrary-image ingestion on the stock-runtime path.
const VERCEL_RUNTIMES: &[&str] = &["node22", "node24", "python3.13"];

/// The default runtime when `FLOX_SANDBOX_VERCEL_RUNTIME` is unset. `node24` is
/// the newest Node LTS runtime and ships the tooling the bootstrap's `curl`-based
/// Flox install needs.
const VERCEL_DEFAULT_RUNTIME: &str = "node24";

/// Install guidance for the Vercel CLI when it is absent from `PATH`.
///
/// The `vercel` npm package is not in the Flox Catalog, so the supported path is
/// npm — with `nodejs` from the catalog supplying `npm` (the same npm-via-nodejs
/// pattern the E2B backend uses).
const VERCEL_CLI_INSTALL_HINT: &str = "The 'vercel' package is not in the Flox Catalog, so install it with npm: \
     'flox install nodejs' to provide npm, then 'npm install -g vercel'. Run \
     'vercel login' and 'vercel link' + 'vercel env pull' to authenticate.";

/// Upgrade guidance for a too-old Vercel CLI. npm-only for the same reason as
/// [`VERCEL_CLI_INSTALL_HINT`].
const VERCEL_CLI_UPGRADE_HINT: &str = "Upgrade with npm: 'npm install -g vercel@latest' (the 'vercel' package is not \
     in the Flox Catalog), then re-run.";

/// Project-relative filename (under `.flox/cache/`) of the generated flox
/// bootstrap script.
const VERCEL_BOOTSTRAP_NAME: &str = "vercel-sandbox-bootstrap.sh";

/// Project-relative filename (under `.flox/cache/`) of the generated
/// `@vercel/sandbox` launcher program.
const VERCEL_LAUNCHER_NAME: &str = "vercel-sandbox-launch.mjs";

pub struct VercelSandboxBackend<'a> {
    dot_flox_path: PathBuf,
    env_name: String,
    invocation_type: &'a InvocationType,
    lockfile: &'a Lockfile,
}

impl<'a> VercelSandboxBackend<'a> {
    pub fn new(ctx: SandboxLaunchCtx<'a>) -> Self {
        Self {
            dot_flox_path: ctx.dot_flox_path,
            env_name: ctx.env_name,
            invocation_type: ctx.invocation_type,
            lockfile: ctx.lockfile,
        }
    }
}

impl ActivationSandbox for VercelSandboxBackend<'_> {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::VercelSandbox
    }

    fn preflight(&self) -> Result<()> {
        // Vercel Sandbox is bootstrap-shaped: nothing is baked, so Docker is NOT
        // required. The one genuinely required host tool is the Vercel CLI, which
        // authenticates the SDK (OIDC token via `vercel env pull`).
        let Some(vercel_path) = first_on_path("vercel") else {
            bail!(
                "The 'vercel-sandbox' backend requires the Vercel CLI, which was not found on \
                 PATH.\n{VERCEL_CLI_INSTALL_HINT}"
            );
        };
        check_vercel_version(&vercel_path)?;
        // Distinguish CLI-present-but-unauthenticated from CLI-present-and-ready
        // without triggering the interactive `vercel login` web flow. `vercel
        // whoami` reports the signed-in identity and fails (non-zero, no prompt)
        // when no session exists. As a fallback, an OIDC or access token in the
        // environment is treated as authenticated (the SDK reads these directly).
        if !vercel_authenticated(&vercel_path) {
            bail!(
                "The Vercel CLI is installed but not authenticated (no signed-in session and no \
                 {VERCEL_OIDC_TOKEN_VAR}/{VERCEL_TOKEN_VAR} in the environment).\n\
                 Run 'vercel login', then 'vercel link' + 'vercel env pull' to download an OIDC \
                 token (requires a Vercel account — the free tier suffices), or export \
                 {VERCEL_TOKEN_VAR}=<token> with VERCEL_TEAM_ID/VERCEL_PROJECT_ID from your \
                 Vercel dashboard."
            );
        }
        Ok(())
    }

    fn wrap_activation(self: Box<Self>) -> Result<Infallible> {
        wrap_vercel_sandbox(
            &self.dot_flox_path,
            &self.env_name,
            self.invocation_type,
            self.lockfile,
        )
    }
}

/// Verify the resolved Vercel CLI meets [`VERCEL_MIN_VERSION`].
///
/// The shared gate runs `vercel --version`, parses the output, and turns a too-
/// old client into an actionable message while tolerating a failed or unparseable
/// `--version` (logged at debug). The hint carries the Vercel-specific upgrade
/// instructions.
fn check_vercel_version(vercel_path: &Path) -> Result<()> {
    check_cli_version(vercel_path, &CliVersionCheck {
        tool_name: "Vercel",
        backend_id: "vercel-sandbox",
        min_version: VERCEL_MIN_VERSION,
        upgrade_hint: VERCEL_CLI_UPGRADE_HINT,
        version_args: DEFAULT_VERSION_ARGS,
    })
}

/// Return `true` when the Vercel CLI is authenticated, or an OIDC / access token
/// is present in the environment.
///
/// `vercel whoami` is a cheap, non-interactive probe: it prints the signed-in
/// scope and exits non-zero (without opening a browser) when no session exists.
/// Env-token presence is accepted as a fallback because the SDK launch path
/// authenticates with `VERCEL_OIDC_TOKEN` / `VERCEL_TOKEN` directly and does not
/// require a CLI session.
fn vercel_authenticated(vercel_path: &Path) -> bool {
    let env_token_present = std::env::var_os(VERCEL_OIDC_TOKEN_VAR)
        .filter(|v| !v.is_empty())
        .is_some()
        || std::env::var_os(VERCEL_TOKEN_VAR)
            .filter(|v| !v.is_empty())
            .is_some();
    if env_token_present {
        return true;
    }
    std::process::Command::new(vercel_path)
        .arg("whoami")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── Runtime selection ────────────────────────────────────────────────────────────

/// Resolve the fixed base runtime from `FLOX_SANDBOX_VERCEL_RUNTIME`, validating
/// it against the known set.
///
/// An unset or empty value falls back to [`VERCEL_DEFAULT_RUNTIME`]. An unknown
/// value is a hard error naming the accepted runtimes — silently defaulting would
/// hide a typo and boot the wrong runtime.
pub(crate) fn resolve_runtime(raw: Option<&str>) -> Result<String> {
    let candidate = raw.map(str::trim).filter(|v| !v.is_empty());
    let Some(runtime) = candidate else {
        return Ok(VERCEL_DEFAULT_RUNTIME.to_string());
    };
    if VERCEL_RUNTIMES.contains(&runtime) {
        Ok(runtime.to_string())
    } else {
        bail!(
            "Unknown Vercel Sandbox runtime '{runtime}' (set via {FLOX_SANDBOX_VERCEL_RUNTIME_VAR}).\n\
             Vercel Sandbox boots a FIXED base runtime; choose one of: {accepted}.",
            accepted = VERCEL_RUNTIMES.join(", ")
        )
    }
}

// ── FloxHub reference ─────────────────────────────────────────────────────────────

/// Build the `<owner>/<env>` FloxHub reference the bootstrap activates.
///
/// When `FLOX_SANDBOX_VERCEL_FLOXHUB_REF` is set, it is used verbatim (the
/// operator names the pushed environment). Otherwise a `<owner>/<env_name>`
/// placeholder is returned so the artifacts are readable, and the launch-boundary
/// message calls out that the operator must push and set the ref.
pub(crate) fn floxhub_ref(env_name: &str, override_ref: Option<&str>) -> String {
    match override_ref.map(str::trim).filter(|v| !v.is_empty()) {
        Some(r) => r.to_string(),
        None => format!("<owner>/{env_name}"),
    }
}

/// Sanitize the environment name into a Vercel Sandbox `name`.
///
/// Vercel Sandbox names are used as a DNS-ish label; lowercase the env name,
/// replace any character outside `[a-z0-9-]` with a dash, and prefix `flox-` so
/// the sandbox is recognizable in the Vercel dashboard. No PID suffix: a stable
/// name lets repeated hand-offs describe the same sandbox.
pub(crate) fn vercel_sandbox_name(env_name: &str) -> String {
    let sanitized: String = env_name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    format!("flox-{sanitized}")
}

// ── Bootstrap script generation ────────────────────────────────────────────────

/// Build the in-sandbox activation command the bootstrap runs, as a single POSIX
/// `sh` command line.
///
/// The bootstrap installs Flox, then activates the FloxHub environment. The three
/// invocation shapes map onto `flox activate -r <ref>` variants; the flox binary
/// is on PATH inside the sandbox after the installer runs. `InPlace` cannot reach
/// this backend (blocked upstream by `ensure_sandbox_not_in_place`).
pub(crate) fn bootstrap_activation_command(invocation: &InvocationType, floxhub_ref: &str) -> String {
    match invocation {
        InvocationType::Interactive => format!("flox activate -r {floxhub_ref}"),
        InvocationType::ExecCommand(cmd) => {
            // Each argv member is single-quoted for the `--` passthrough so an
            // adversarial member cannot break out of the command line.
            let joined = cmd
                .iter()
                .map(|a| sh_single_quote(a))
                .collect::<Vec<_>>()
                .join(" ");
            format!("flox activate -r {floxhub_ref} -- {joined}")
        },
        InvocationType::ShellCommand(shell_cmd) => {
            format!("flox activate -r {floxhub_ref} -- sh -c {}", sh_single_quote(shell_cmd))
        },
        InvocationType::InPlace => {
            unreachable!(
                "in-place invocation cannot reach the vercel-sandbox backend (blocked by \
                 ensure_sandbox_not_in_place)"
            );
        },
    }
}

/// Render the flox bootstrap script (pure function, no I/O).
///
/// The script runs inside the fixed Vercel Sandbox runtime: it installs Flox via
/// the official installer, then activates the FloxHub environment. This is the
/// "closure" for a runtime flox cannot pre-seed with an OCI image — the
/// determinism tradeoff documented in the module header.
pub(crate) fn render_bootstrap(floxhub_ref: &str, activation_cmd: &str) -> String {
    // floxhub_ref and activation_cmd derive from validated sources (env name
    // sanitized, argv members single-quoted in the command builder), so embedding
    // them in the heredoc-free script body is injection-safe for the shapes flox
    // produces.
    indoc::formatdoc! {r#"
        #!/usr/bin/env bash
        # Generated by `flox activate --sandbox --sandbox-backend vercel-sandbox`.
        #
        # This runs INSIDE a Vercel Sandbox (a fixed Amazon Linux 2023 runtime).
        # Vercel Sandbox boots a stock runtime rather than a baked image, so flox
        # cannot pre-seed the environment closure. Instead this bootstrap installs
        # Flox and activates the environment from FloxHub:
        #     {floxhub_ref}
        #
        # Determinism note: this activates the FloxHub-pushed revision, not a
        # byte-for-byte closure captured at `flox activate --sandbox` time. Push
        # the environment first ('flox push') and set
        # {ref_var}=<owner>/<env> so this ref resolves.
        set -euo pipefail

        if ! command -v flox >/dev/null 2>&1; then
          echo "Installing Flox into the sandbox runtime..."
          curl -fsSL https://install.flox.dev/install.sh | bash
          # The installer adds flox to PATH via the profile; source it for this
          # non-login shell.
          export PATH="$HOME/.local/bin:/usr/local/bin:$PATH"
          # shellcheck disable=SC1091
          [ -f /etc/profile.d/flox.sh ] && . /etc/profile.d/flox.sh || true
        fi

        echo "Activating the FloxHub environment '{floxhub_ref}'..."
        exec {activation_cmd}
        "#,
        floxhub_ref = floxhub_ref,
        ref_var = FLOX_SANDBOX_VERCEL_FLOXHUB_REF_VAR,
        activation_cmd = activation_cmd,
    }
}

// ── Launcher artifact generation ───────────────────────────────────────────────

/// Inputs to [`render_vercel_launcher`].
pub(crate) struct LauncherParams<'a> {
    /// Sandbox name (`flox-<sanitized-env>`).
    pub sandbox_name: &'a str,
    /// Fixed base runtime (`node22` | `node24` | `python3.13`).
    pub runtime: &'a str,
    /// The FloxHub reference the bootstrap activates (recorded for readability).
    pub floxhub_ref: &'a str,
    /// The bootstrap script body the launcher uploads and runs.
    pub bootstrap: &'a str,
    /// Sandbox wall-clock timeout, in milliseconds (the SDK's `timeout` unit).
    pub timeout_ms: u64,
}

/// Render the `@vercel/sandbox` launcher program (pure function, no I/O).
///
/// The emitted ESM (`.mjs`) constructs a `Sandbox` with the fixed runtime, writes
/// the flox bootstrap into the sandbox, and runs it — streaming stdout/stderr to
/// the local terminal, then exiting with the bootstrap's return code. A
/// credentialed operator with the environment pushed to FloxHub runs it with
/// `node --experimental-strip-types` (or plain `node` for `.mjs`) after
/// `vercel env pull`.
///
/// Note the honest network stance: no egress-policy argument is emitted, because
/// the SDK has no per-sandbox egress vocabulary (see the module header). `ports`
/// is intentionally omitted — it governs inbound exposure, not the activation.
pub(crate) fn render_vercel_launcher(params: &LauncherParams<'_>) -> String {
    // The bootstrap is embedded as a JSON string literal so an arbitrary body
    // (multi-line, quote-bearing) is safe inside the JS source. A JSON string
    // escapes control characters — critically the bootstrap's newlines become
    // `\n` — so the literal stays on one JS statement; a raw multi-line body
    // would be invalid JavaScript. JSON string syntax is a subset JS accepts
    // verbatim. `serde_json::to_string` on a `&str` cannot fail.
    let bootstrap_lit = serde_json::to_string(params.bootstrap)
        .expect("serializing a &str to a JSON string cannot fail");
    // sandbox_name, runtime, and floxhub_ref come from validated sources
    // (sanitized name, validated runtime, operator ref); embed them as JSON
    // string literals for the same injection-safety reason.
    let name_lit = json_str_lit(params.sandbox_name);
    let runtime_lit = json_str_lit(params.runtime);
    indoc::formatdoc! {r#"
        #!/usr/bin/env node
        // Generated by `flox activate --sandbox --sandbox-backend vercel-sandbox`.
        //
        // This is the launch artifact for the Vercel Sandbox backend. Vercel
        // Sandbox boots a FIXED base runtime ({runtime}) — it does not ingest a
        // baked image — so flox installs itself inside the sandbox via the
        // bootstrap below and activates the FloxHub environment '{floxhub_ref}'.
        //
        // Prerequisites: a Vercel account + OIDC/access token (run `vercel env
        // pull` first), and the environment pushed to FloxHub. Then run:
        //     node {launcher}
        //
        // Network: the @vercel/sandbox SDK has no per-sandbox egress allowlist, so
        // no egress policy is applied here — manifest egress grants are declined
        // by flox at generation time, not silently dropped.
        import {{ Sandbox }} from "@vercel/sandbox";

        const BOOTSTRAP = {bootstrap_lit};

        async function main() {{
          const sandbox = await Sandbox.create({{
            name: {name_lit},
            runtime: {runtime_lit},
            timeout: {timeout_ms},
          }});
          console.log(`Sandbox ${{sandbox.name}} created (runtime {runtime})`);

          await sandbox.writeFiles([
            {{ path: "flox-bootstrap.sh", content: Buffer.from(BOOTSTRAP) }},
          ]);

          const run = await sandbox.runCommand({{
            cmd: "bash",
            args: ["flox-bootstrap.sh"],
            stdout: process.stdout,
            stderr: process.stderr,
          }});

          await sandbox.stop();
          process.exit(run.exitCode);
        }}

        main().catch((err) => {{
          console.error(err);
          process.exit(1);
        }});
        "#,
        runtime = params.runtime,
        floxhub_ref = params.floxhub_ref,
        launcher = VERCEL_LAUNCHER_NAME,
        bootstrap_lit = bootstrap_lit,
        name_lit = name_lit,
        runtime_lit = runtime_lit,
        timeout_ms = params.timeout_ms,
    }
}

// ── Network decline ──────────────────────────────────────────────────────────────

/// Reject any manifest egress grant, because Vercel Sandbox has no egress
/// vocabulary to compile onto.
///
/// The backend contract requires declining an inexpressible grant rather than
/// silently widening or dropping it. Vercel Sandbox's SDK exposes only inbound
/// `ports`, so a `[[options.sandbox.network]]` rule cannot be honored — this
/// bails with a message naming the missing capability and pointing at a backend
/// that does have per-domain egress. No rules is the only accepted case.
pub(crate) fn ensure_no_network_grants(rule_count: usize) -> Result<()> {
    if rule_count == 0 {
        return Ok(());
    }
    bail!(
        "The 'vercel-sandbox' backend cannot enforce network egress grants: the \
         @vercel/sandbox SDK has no per-sandbox egress allowlist or firewall — its `ports` option \
         governs INBOUND exposure only.\n\
         The manifest declares {rule_count} [[options.sandbox.network]] grant(s) that this backend \
         cannot express, so they are declined rather than silently ignored.\n\
         Remove the grants to run on Vercel Sandbox with its default network posture, or select a \
         backend with domain egress (e.g. 'openshell', 'e2b', or 'daytona')."
    )
}

// ── Launch path ────────────────────────────────────────────────────────────────

/// Generate the bootstrap + launcher artifacts, then fail at the launch boundary
/// — never fake the remote launch, and never bake (Vercel ingests no image).
fn wrap_vercel_sandbox(
    dot_flox_path: &Path,
    env_name: &str,
    invocation: &InvocationType,
    lockfile: &Lockfile,
) -> Result<Infallible> {
    // Decline any egress grant up front: Vercel Sandbox has no egress vocabulary,
    // and silently dropping a grant would falsely imply it was honored.
    let rules = manifest_network_rules(lockfile)?;
    ensure_no_network_grants(rules.len())?;

    let runtime = resolve_runtime(
        std::env::var(FLOX_SANDBOX_VERCEL_RUNTIME_VAR)
            .ok()
            .as_deref(),
    )?;
    let floxhub_override = std::env::var(FLOX_SANDBOX_VERCEL_FLOXHUB_REF_VAR).ok();
    let floxhub = floxhub_ref(env_name, floxhub_override.as_deref());
    let sandbox_name = vercel_sandbox_name(env_name);

    // Generate the two artifacts: the flox bootstrap (installs + activates inside
    // the fixed runtime) and the @vercel/sandbox launcher (creates the sandbox,
    // uploads the bootstrap, runs it).
    let activation_cmd = bootstrap_activation_command(invocation, &floxhub);
    let bootstrap = render_bootstrap(&floxhub, &activation_cmd);
    let launcher = render_vercel_launcher(&LauncherParams {
        sandbox_name: &sandbox_name,
        runtime: &runtime,
        floxhub_ref: &floxhub,
        bootstrap: &bootstrap,
        // Default 5 minutes (Vercel's own default); expressed in ms for the SDK.
        timeout_ms: 5 * 60 * 1000,
    });
    let (bootstrap_path, launcher_path) = write_artifacts(dot_flox_path, &bootstrap, &launcher)?;

    // Fail at the launch boundary with the two concrete prerequisites.
    let ref_hint = if floxhub_override.is_some() {
        format!("the environment pushed to FloxHub as '{floxhub}'")
    } else {
        format!(
            "the environment pushed to FloxHub ('flox push'), then set \
             {FLOX_SANDBOX_VERCEL_FLOXHUB_REF_VAR}=<owner>/{env_name} and re-run"
        )
    };
    bail!(
        "The 'vercel-sandbox' backend launches a remote Vercel Sandbox, which requires two \
         prerequisites this host cannot satisfy automatically:\n  \
         1. A Vercel account and token (preflight confirmed the CLI; the launch calls the Vercel \
         API — run 'vercel env pull' for an OIDC token or export {VERCEL_TOKEN_VAR}).\n  \
         2. {ref_hint} — Vercel Sandbox boots a fixed runtime and cannot ingest the baked image, so \
         the bootstrap installs Flox in-sandbox and activates from FloxHub.\n\
         flox generated the bootstrap-shaped hand-off at:\n  {bootstrap}\n  {launcher}\n\
         With Vercel authenticated and the environment on FloxHub, run 'node {launcher}'.",
        bootstrap = bootstrap_path.display(),
        launcher = launcher_path.display(),
    )
}

/// Write the generated bootstrap + launcher under `.flox/cache/` and return their
/// paths.
///
/// Both artifacts live under `.flox/cache/` (not the project root): they are
/// prototype hand-off scaffolding, not files meant to be committed alongside the
/// project, so they mirror the modal launcher's cache-local placement rather than
/// the e2b/ona project-root artifacts.
fn write_artifacts(
    dot_flox_path: &Path,
    bootstrap: &str,
    launcher: &str,
) -> Result<(PathBuf, PathBuf)> {
    let cache_dir = dot_flox_path.join("cache");
    std::fs::create_dir_all(&cache_dir)
        .with_context(|| format!("failed to create cache dir '{}'", cache_dir.display()))?;
    let bootstrap_path = cache_dir.join(VERCEL_BOOTSTRAP_NAME);
    std::fs::write(&bootstrap_path, bootstrap).with_context(|| {
        format!(
            "failed to write bootstrap to '{}'",
            bootstrap_path.display()
        )
    })?;
    let launcher_path = cache_dir.join(VERCEL_LAUNCHER_NAME);
    std::fs::write(&launcher_path, launcher)
        .with_context(|| format!("failed to write launcher to '{}'", launcher_path.display()))?;
    debug!(
        bootstrap = %bootstrap_path.display(),
        launcher = %launcher_path.display(),
        "wrote vercel-sandbox artifacts"
    );
    Ok((bootstrap_path, launcher_path))
}

/// Wrap a string in a POSIX `sh` single-quoted literal, escaping embedded single
/// quotes with the `'\''` idiom so an arbitrary argv member cannot break out of
/// the quoted command line the bootstrap builds.
fn sh_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use flox_core::activate::sandbox_policy::SandboxNetworkRule;

    use super::*;

    // ── install / upgrade hints ───────────────────────────────────────────────

    #[test]
    fn install_hint_points_at_npm_not_the_catalog() {
        // The 'vercel' CLI is npm-only: it is not in the Flox Catalog, so the hint
        // must name the npm path and the nodejs-provides-npm step (mirroring e2b).
        assert!(
            VERCEL_CLI_INSTALL_HINT.contains("npm install -g vercel"),
            "install hint must name the npm path: {VERCEL_CLI_INSTALL_HINT}"
        );
        assert!(
            VERCEL_CLI_INSTALL_HINT.contains("flox install nodejs"),
            "install hint must note nodejs provides npm: {VERCEL_CLI_INSTALL_HINT}"
        );
    }

    #[test]
    fn upgrade_hint_points_at_npm() {
        assert!(
            VERCEL_CLI_UPGRADE_HINT.contains("npm install -g vercel@latest"),
            "upgrade hint must name the npm path: {VERCEL_CLI_UPGRADE_HINT}"
        );
    }

    // ── resolve_runtime ───────────────────────────────────────────────────────

    #[test]
    fn runtime_unset_defaults_to_node24() {
        assert_eq!(resolve_runtime(None).unwrap(), "node24");
        assert_eq!(resolve_runtime(Some("")).unwrap(), "node24");
        assert_eq!(resolve_runtime(Some("  ")).unwrap(), "node24");
    }

    #[test]
    fn runtime_known_values_are_accepted() {
        for rt in VERCEL_RUNTIMES {
            assert_eq!(&resolve_runtime(Some(rt)).unwrap(), rt);
        }
        // Whitespace around a valid value is trimmed.
        assert_eq!(resolve_runtime(Some(" node22 ")).unwrap(), "node22");
    }

    #[test]
    fn runtime_unknown_value_is_rejected_naming_the_set() {
        let err = resolve_runtime(Some("node18")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("node18"), "got: {msg}");
        // The error must list the accepted runtimes so the operator can fix it.
        assert!(msg.contains("node22"), "got: {msg}");
        assert!(msg.contains("node24"), "got: {msg}");
        assert!(msg.contains("python3.13"), "got: {msg}");
    }

    // ── floxhub_ref ───────────────────────────────────────────────────────────

    #[test]
    fn floxhub_ref_uses_override_when_set() {
        assert_eq!(floxhub_ref("myenv", Some("djsauble/myenv")), "djsauble/myenv");
        // Whitespace-only override falls back to the placeholder.
        assert_eq!(floxhub_ref("myenv", Some("  ")), "<owner>/myenv");
    }

    #[test]
    fn floxhub_ref_placeholder_when_unset() {
        assert_eq!(floxhub_ref("myenv", None), "<owner>/myenv");
    }

    // ── vercel_sandbox_name ───────────────────────────────────────────────────

    #[test]
    fn sandbox_name_prefix_and_sanitization() {
        assert_eq!(vercel_sandbox_name("MyEnv"), "flox-myenv");
        assert_eq!(vercel_sandbox_name("my.env-v2 beta"), "flox-my-env-v2-beta");
    }

    // ── ensure_no_network_grants ──────────────────────────────────────────────

    #[test]
    fn no_grants_is_accepted() {
        assert!(ensure_no_network_grants(0).is_ok());
    }

    #[test]
    fn any_grant_is_declined_naming_the_missing_capability() {
        let err = ensure_no_network_grants(2).unwrap_err();
        let msg = err.to_string();
        // The decline must state the count and name the missing egress vocabulary
        // (not silently drop the grant).
        assert!(msg.contains("2 [[options.sandbox.network]]"), "got: {msg}");
        assert!(
            msg.contains("no per-sandbox egress allowlist or firewall"),
            "got: {msg}"
        );
        // It must point the operator at a backend that can express egress.
        assert!(msg.contains("openshell"), "got: {msg}");
    }

    // ── bootstrap_activation_command ──────────────────────────────────────────

    #[test]
    fn interactive_activation_is_remote_ref() {
        let cmd = bootstrap_activation_command(&InvocationType::Interactive, "djsauble/env");
        assert_eq!(cmd, "flox activate -r djsauble/env");
    }

    #[test]
    fn exec_command_is_appended_after_dashdash() {
        let inv = InvocationType::ExecCommand(vec!["ls".to_string(), "-la".to_string()]);
        let cmd = bootstrap_activation_command(&inv, "djsauble/env");
        assert_eq!(cmd, "flox activate -r djsauble/env -- 'ls' '-la'");
    }

    #[test]
    fn shell_command_is_wrapped_in_sh_c() {
        let inv = InvocationType::ShellCommand("echo hi | cat".to_string());
        let cmd = bootstrap_activation_command(&inv, "djsauble/env");
        assert_eq!(cmd, "flox activate -r djsauble/env -- sh -c 'echo hi | cat'");
    }

    #[test]
    fn exec_command_single_quotes_are_escaped() {
        // A member containing a single quote must be escaped with the '\'' idiom
        // so it cannot break out of the quoted command line.
        let inv = InvocationType::ExecCommand(vec!["echo".to_string(), "a'b".to_string()]);
        let cmd = bootstrap_activation_command(&inv, "djsauble/env");
        assert_eq!(cmd, "flox activate -r djsauble/env -- 'echo' 'a'\\''b'");
    }

    // ── render_bootstrap ──────────────────────────────────────────────────────

    #[test]
    fn bootstrap_installs_flox_and_activates_remote() {
        let cmd = bootstrap_activation_command(&InvocationType::Interactive, "djsauble/env");
        let script = render_bootstrap("djsauble/env", &cmd);
        assert!(script.starts_with("#!/usr/bin/env bash\n"), "got:\n{script}");
        assert!(
            script.contains("install.flox.dev/install.sh"),
            "bootstrap must install Flox:\n{script}"
        );
        assert!(
            script.contains("exec flox activate -r djsauble/env"),
            "bootstrap must exec the remote activation:\n{script}"
        );
        // The determinism tradeoff and push prerequisite must be documented in
        // the artifact so an operator reads it in place.
        assert!(script.contains("Determinism note"), "got:\n{script}");
        assert!(
            script.contains("FLOX_SANDBOX_VERCEL_FLOXHUB_REF"),
            "got:\n{script}"
        );
    }

    // ── render_vercel_launcher ────────────────────────────────────────────────

    fn sample_launcher(runtime: &str) -> String {
        let bootstrap = render_bootstrap(
            "djsauble/env",
            &bootstrap_activation_command(&InvocationType::Interactive, "djsauble/env"),
        );
        render_vercel_launcher(&LauncherParams {
            sandbox_name: "flox-env",
            runtime,
            floxhub_ref: "djsauble/env",
            bootstrap: &bootstrap,
            timeout_ms: 300_000,
        })
    }

    #[test]
    fn launcher_uses_the_sdk_and_fixed_runtime() {
        let script = sample_launcher("node24");
        assert!(script.starts_with("#!/usr/bin/env node\n"), "got:\n{script}");
        assert!(
            script.contains("import { Sandbox } from \"@vercel/sandbox\";"),
            "got:\n{script}"
        );
        assert!(
            script.contains("Sandbox.create({"),
            "launcher must call Sandbox.create:\n{script}"
        );
        assert!(
            script.contains("runtime: \"node24\""),
            "launcher must pass the fixed runtime:\n{script}"
        );
        assert!(
            script.contains("timeout: 300000"),
            "launcher must pass the timeout in ms:\n{script}"
        );
        assert!(
            script.contains("sandbox.runCommand({"),
            "launcher must run the bootstrap:\n{script}"
        );
        assert!(
            script.contains("process.exit(run.exitCode)"),
            "launcher must exit with the bootstrap's code:\n{script}"
        );
    }

    #[test]
    fn launcher_embeds_the_bootstrap_as_a_single_line_json_literal() {
        let script = sample_launcher("node24");
        // The bootstrap is embedded as a JSON string, so its newlines become \n
        // and it stays on one JS statement (`const BOOTSTRAP = "...";`).
        assert!(
            script.contains("const BOOTSTRAP = \""),
            "bootstrap must be a JSON string literal:\n{script}"
        );
        // A raw multi-line string literal is INVALID JavaScript. The bootstrap's
        // newlines must be escaped to `\n`, so the whole `const BOOTSTRAP = "...";`
        // must sit on a single physical source line. A control-char-blind escaper
        // (one that only handled quotes/backslashes) would break this; guard it
        // directly by isolating the BOOTSTRAP line and asserting it terminates.
        let bootstrap_line = script
            .lines()
            .find(|l| l.starts_with("const BOOTSTRAP = "))
            .expect("BOOTSTRAP declaration must be present");
        assert!(
            bootstrap_line.ends_with("\";"),
            "the BOOTSTRAP literal must be a single self-terminating line (no raw \
             newlines):\n{bootstrap_line}"
        );
        assert!(
            bootstrap_line.contains("\\n"),
            "embedded bootstrap newlines must be escaped to \\n:\n{bootstrap_line}"
        );
    }

    #[test]
    fn launcher_applies_no_egress_policy() {
        // The honest network stance: no egress-policy argument, no `ports` (which
        // is inbound-only). The comment must state why.
        let script = sample_launcher("python3.13");
        assert!(
            !script.contains("allowedDomains") && !script.contains("blockNetwork"),
            "launcher must not emit an egress policy the SDK does not support:\n{script}"
        );
        assert!(
            script.contains("no per-sandbox egress allowlist"),
            "launcher must document the missing egress vocabulary:\n{script}"
        );
        assert!(
            script.contains("runtime: \"python3.13\""),
            "runtime must be threaded through:\n{script}"
        );
    }

    #[test]
    fn launcher_embeds_a_bootstrap_with_a_single_quote_safely() {
        // A bootstrap containing a double quote (from a shell command member)
        // must stay a well-formed JS string literal.
        let inv = InvocationType::ShellCommand("echo \"hi\"".to_string());
        let bootstrap = render_bootstrap(
            "djsauble/env",
            &bootstrap_activation_command(&inv, "djsauble/env"),
        );
        let script = render_vercel_launcher(&LauncherParams {
            sandbox_name: "flox-env",
            runtime: "node24",
            floxhub_ref: "djsauble/env",
            bootstrap: &bootstrap,
            timeout_ms: 300_000,
        });
        // The embedded double quote must be backslash-escaped inside the JSON
        // literal (json_str_lit handles this).
        assert!(
            script.contains("\\\"hi\\\""),
            "double quotes in the bootstrap must be escaped in the JS literal:\n{script}"
        );
    }

    // ── network rules plumbing ────────────────────────────────────────────────

    #[test]
    fn a_single_rule_count_is_declined() {
        let _rule = SandboxNetworkRule {
            endpoint: "api.github.com:443".to_string(),
            access: None,
            protocol: None,
            binary: None,
        };
        // The decline is by count (the rules themselves cannot be honored), so one
        // rule is enough to bail.
        assert!(ensure_no_network_grants(1).is_err());
    }
}
