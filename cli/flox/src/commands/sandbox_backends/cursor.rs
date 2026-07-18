//! The `cursor` sandbox backend: align Flox's policy with Cursor's agent
//! sandbox via a project-scoped permission config.
//!
//! Cursor's coding-agent CLI (`agent`) runs its own sandbox that re-skins the
//! host-native OS boundary (Seatbelt on macOS, Landlock on Linux) with a
//! product policy layer. Unlike every OCI-ingesting cloud backend, nothing is
//! baked and nothing leaves the laptop: this is a LOCAL, host-kernel backend.
//! The workload (the Cursor agent) runs on this machine, confined by the OS
//! sandbox Cursor drives, so the threat model is the host-local one — not the
//! inverted remote model of the cloud peers.
//!
//! The integration is a *policy-layer alignment*, not a launch wrapper. Cursor
//! configures its sandbox through settings files, not a programmatic launch
//! API. So the honest seam is: compile the manifest grants into Cursor's
//! project-scoped permission config (`<project>/.cursor/cli.json`), so Flox's
//! environment-and-policy source of truth and Cursor's enforcement *stack*
//! instead of conflicting, then bail at the launch boundary naming the missing
//! programmatic hook.
//!
//! # Why this backend does not complete the launch on any host today
//!
//! There is no public API that ingests a config path and execs `agent` under a
//! specific sandbox. The `agent` CLI reads `<project>/.cursor/cli.json`
//! *implicitly* from the working directory, and the `sandbox.mode` /
//! `sandbox.networkAccess` knobs that would gate all egress are global-only
//! (`~/.cursor/cli-config.json`), not project-scoped — so flox cannot set them
//! for one environment without mutating the user's global Cursor config.
//! Rather than clobber a global file or fake a launch, this backend writes the
//! project config and fails at the launch boundary with a clear message: a
//! credentialed operator with the `agent` CLI runs it *in the project* to pick
//! up the compiled policy.
//!
//! `preflight` detects the `agent` CLI on PATH (install hint on absence), gates
//! its version, and probes auth non-interactively (`CURSOR_API_KEY` presence —
//! it never triggers an interactive login).
//!
//! # Policy compilation (the load-bearing lossiness)
//!
//! Cursor's project config expresses policy as `permissions.allow` /
//! `permissions.deny` string tokens — `Read(glob)`, `Write(glob)`,
//! `Shell(base)`, `WebFetch(domain)`, `Mcp(server:tool)` — where **deny takes
//! precedence over allow** (verified against cursor.com/docs, 2026-07-18). flox
//! compiles the manifest's `[[options.sandbox.network]]` grants onto
//! `WebFetch(<host>)` allow entries: domain-scoped, wildcards preserved. Three
//! lossiness axes, declared here and in the demo:
//!
//! - **Web-fetch tool only.** `WebFetch` governs the agent's web-fetch tool, not
//!   arbitrary sockets. A grant becomes a fetch-domain allowance, not a general
//!   egress rule.
//! - **Port-blind.** `WebFetch(domain)` carries no port; the grant's `:443` is
//!   dropped. A non-443 endpoint cannot be faithfully expressed and is declined
//!   at compile time rather than silently widened.
//! - **Op-blind on the network axis.** The grant's `access` / `protocol` /
//!   `binary` scoping is not expressible through `WebFetch` and is dropped.
//!
//! flox also mirrors the host-native secret posture into the deny list: `.env`
//! files and private keys are denied for both `Read` and `Write` so the agent
//! cannot read or overwrite them even inside the project it is working on.

use std::convert::Infallible;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flox_core::activate::sandbox_backend::SandboxBackend;
use flox_core::activate::sandbox_policy::SandboxNetworkRule;
use flox_manifest::lockfile::Lockfile;
use semver::Version;
use tracing::debug;

use super::handoff::manifest_network_rules;
use super::preflight::{
    CliVersionCheck,
    DEFAULT_VERSION_ARGS,
    check_cli_version,
    first_on_path,
    split_endpoint,
};
use super::{ActivationSandbox, SandboxLaunchCtx};

/// Environment variable holding the Cursor API key. Preflight treats its
/// presence as a valid non-interactive proof of auth (the same fallback the
/// cloud backends use), never triggering a browser login.
pub(crate) const CURSOR_API_KEY_VAR: &str = "CURSOR_API_KEY";

/// The `agent` CLI binary name. Cursor's coding-agent CLI installs as `agent`
/// (via `curl https://cursor.com/install | bash`), not `cursor` (which is the
/// editor). Detected on PATH by preflight.
const CURSOR_CLI: &str = "agent";

/// Project-relative path of the generated Cursor permission config. The `agent`
/// CLI reads `<project>/.cursor/cli.json` implicitly from the working
/// directory; only permissions are configurable at the project level (all other
/// settings are global), which is exactly the surface flox compiles onto.
const CURSOR_CONFIG_REL_PATH: &str = ".cursor/cli.json";

/// Minimum supported `agent` CLI version.
///
/// The project-level `permissions` config and the `WebFetch` token this backend
/// compiles to are the current (schema `version: 1`) surface; pinned
/// conservatively to a 1.0 floor. The shared version gate tolerates a failed or
/// unparseable `--version` (logged at debug), so a nonstandard build does not
/// hard-block.
const CURSOR_MIN_VERSION: Version = Version::new(1, 0, 0);

pub struct CursorBackend<'a> {
    dot_flox_path: PathBuf,
    lockfile: &'a Lockfile,
}

impl<'a> CursorBackend<'a> {
    pub fn new(ctx: SandboxLaunchCtx<'a>) -> Self {
        Self {
            dot_flox_path: ctx.dot_flox_path,
            lockfile: ctx.lockfile,
        }
    }
}

impl ActivationSandbox for CursorBackend<'_> {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::Cursor
    }

    fn preflight(&self) -> Result<()> {
        let Some(agent_path) = first_on_path(CURSOR_CLI) else {
            bail!(
                "The 'cursor' sandbox backend requires Cursor's agent CLI ('{CURSOR_CLI}'), which \
                 was not found on PATH.\n\
                 Install it with 'curl https://cursor.com/install -fsS | bash', then set \
                 CURSOR_API_KEY (or run '{CURSOR_CLI}' once to sign in)."
            );
        };
        check_cursor_version(&agent_path)?;
        // Non-interactive auth probe: the API key in the environment is a valid
        // proof of auth without opening a browser login. A signed-in `agent`
        // also stores credentials under ~/.cursor, but flox never triggers the
        // interactive flow, so an unset key is a soft note, not a hard failure —
        // the config is generated regardless and the launch boundary names the
        // auth wall.
        if std::env::var_os(CURSOR_API_KEY_VAR).is_none() {
            debug!(
                "{CURSOR_API_KEY_VAR} is unset; the Cursor config is generated regardless and the \
                 launch boundary names the auth wall"
            );
        }
        Ok(())
    }

    fn wrap_activation(self: Box<Self>) -> Result<Infallible> {
        wrap_cursor(&self.dot_flox_path, self.lockfile)
    }
}

/// Verify the resolved `agent` CLI meets [`CURSOR_MIN_VERSION`].
///
/// The shared gate turns a too-old client into an actionable message and
/// tolerates a failed or unparseable `--version` (logged at debug). The hint
/// carries the Cursor-specific upgrade instruction.
fn check_cursor_version(agent_path: &Path) -> Result<()> {
    check_cli_version(agent_path, &CliVersionCheck {
        tool_name: "Cursor agent",
        backend_id: "cursor",
        min_version: CURSOR_MIN_VERSION,
        upgrade_hint: "Upgrade by re-running the installer: 'curl https://cursor.com/install -fsS | bash'.",
        version_args: DEFAULT_VERSION_ARGS,
    })
}

// ── Policy compilation ─────────────────────────────────────────────────────────

/// The manifest network policy compiled into Cursor's permission vocabulary.
///
/// Cursor's project config has no `<host>:<port>` egress vocabulary; the closest
/// native construct is `WebFetch(<domain>)`, which allowlists the agent's
/// web-fetch tool by domain. `<host>:443` grants become fetch-domain allowances;
/// everything else is deny-by-default (an empty allowlist).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CursorNetworkPolicy {
    /// Deny all fetches by default (no grants declared → empty allowlist).
    pub deny_all: bool,
    /// Hosts granted web-fetch egress, in declaration order (deduplicated). Each
    /// renders as a `WebFetch(<host>)` allow token.
    pub fetch_domains: Vec<String>,
}

/// Compile the manifest's `[[options.sandbox.network]]` rules into Cursor's
/// web-fetch allowlist.
///
/// - No rules → deny-all (secure-by-default; the config's allow list holds no
///   `WebFetch` entry, so every fetch prompts / is denied).
/// - A `<host>:443` rule → a `WebFetch(<host>)` allow entry (domain-scoped, the
///   port dropped — a declared lossiness).
/// - Any non-443 port → a hard error: `WebFetch` is domain-only and HTTPS-shaped,
///   and silently promoting the grant (or dropping it) would violate the "never
///   silently widen or narrow grants" contract.
pub(crate) fn compile_cursor_network_policy(
    rules: &[SandboxNetworkRule],
) -> Result<CursorNetworkPolicy> {
    if rules.is_empty() {
        return Ok(CursorNetworkPolicy {
            deny_all: true,
            fetch_domains: Vec::new(),
        });
    }
    let mut domains: Vec<String> = Vec::with_capacity(rules.len());
    for rule in rules {
        let (host, port) = split_endpoint(&rule.endpoint)?;
        if port != 443 {
            bail!(
                "The 'cursor' sandbox backend expresses egress as web-fetch domains (WebFetch), \
                 which are HTTPS/443-shaped, but rule '{endpoint}' targets port {port}.\n\
                 Rewrite the endpoint as '{host}:443', or select a backend with per-port egress \
                 (e.g. 'openshell').",
                endpoint = rule.endpoint
            );
        }
        if !domains.contains(&host) {
            domains.push(host);
        }
    }
    Ok(CursorNetworkPolicy {
        deny_all: false,
        fetch_domains: domains,
    })
}

/// Deny tokens that keep secrets unreadable and unwritable to the agent, even
/// inside the project it is working on. Mirrors the host-native backend's `.env`
/// secrecy rule, extended to private keys. Deny takes precedence over allow in
/// Cursor's model, so these hold regardless of any allow entry.
const SECRET_DENY_TOKENS: [&str; 4] = [
    "Read(**/.env*)",
    "Write(**/.env*)",
    "Read(**/*.key)",
    "Write(**/*.key)",
];

/// Build the ordered `permissions.allow` token list for the config.
///
/// The project root is read/write (the code the agent works on), and each
/// granted host becomes a `WebFetch(<host>)` token. Ordering is deterministic
/// (filesystem tokens first, then the network allowlist in declaration order) so
/// the rendered config is stable across runs.
pub(crate) fn cursor_allow_tokens(network: &CursorNetworkPolicy) -> Vec<String> {
    let mut tokens = vec![
        "Read(**)".to_string(),
        "Write(**)".to_string(),
    ];
    tokens.extend(
        network
            .fetch_domains
            .iter()
            .map(|host| format!("WebFetch({host})")),
    );
    tokens
}

/// Build the ordered `permissions.deny` token list for the config.
///
/// The secret-protection tokens are constant; deny-precedence in Cursor's model
/// means they override the broad `Read(**)` / `Write(**)` allow above.
pub(crate) fn cursor_deny_tokens() -> Vec<String> {
    SECRET_DENY_TOKENS.iter().map(|t| t.to_string()).collect()
}

// ── Config artifact generation ──────────────────────────────────────────────────

/// Render the Cursor project permission config (`<project>/.cursor/cli.json`).
///
/// The emitted JSON is the schema-`version: 1` project config: a `permissions`
/// object with `allow` / `deny` token arrays. The tokens come from validated
/// sources (fixed filesystem globs; hosts passed through `split_endpoint`'s
/// charset check, which forbids quotes and newlines), and `serde_json`
/// serialization escapes them, so the output is injection-safe. Returned as a
/// pretty-printed string; the caller writes it to disk.
pub(crate) fn render_cursor_config(network: &CursorNetworkPolicy) -> String {
    let allow = cursor_allow_tokens(network);
    let deny = cursor_deny_tokens();
    let config = serde_json::json!({
        "version": 1,
        "permissions": {
            "allow": allow,
            "deny": deny,
        },
    });
    serde_json::to_string_pretty(&config)
        .expect("serializing a literal JSON value cannot fail")
}

// ── Launch path ────────────────────────────────────────────────────────────────

/// Compile the policy, write the project config, then fail at the launch
/// boundary — never fake driving the agent.
fn wrap_cursor(dot_flox_path: &Path, lockfile: &Lockfile) -> Result<Infallible> {
    let dot_flox =
        std::fs::canonicalize(dot_flox_path).unwrap_or_else(|_| dot_flox_path.to_path_buf());
    let project = dot_flox.parent().unwrap_or(&dot_flox).to_path_buf();

    // Compile the manifest network policy into Cursor's web-fetch allowlist.
    let rules = manifest_network_rules(lockfile)?;
    let network = compile_cursor_network_policy(&rules)?;

    // Write the project config the agent reads implicitly.
    let config = render_cursor_config(&network);
    let artifact_path = write_cursor_config(&project, &config)?;

    // Fail at the launch boundary. Name whether the CLI and auth were detected so
    // the message is precise about the wall.
    let cli_note = match first_on_path(CURSOR_CLI) {
        Some(_) => format!("the '{CURSOR_CLI}' CLI is installed"),
        None => format!("the '{CURSOR_CLI}' CLI was not found on PATH"),
    };
    let auth_note = if std::env::var_os(CURSOR_API_KEY_VAR).is_some() {
        format!("{CURSOR_API_KEY_VAR} is set")
    } else {
        format!("{CURSOR_API_KEY_VAR} is unset (run '{CURSOR_CLI}' once to sign in, or export it)")
    };
    let policy_summary = if network.deny_all {
        "deny-all (no grants declared)".to_string()
    } else {
        format!("web-fetch allowed: {}", network.fetch_domains.join(", "))
    };
    bail!(
        "The 'cursor' sandbox backend aligns Flox's policy with Cursor's agent sandbox, but \
         Cursor exposes no launch API that runs the agent under a config path — so flox cannot \
         re-exec the activation under it.\n\
         flox compiled the manifest grants into Cursor's project permission config \
         ({policy_summary}) and wrote it to:\n  {artifact}\n\
         The '{CURSOR_CLI}' CLI reads '<project>/.cursor/cli.json' implicitly; {cli_note}, and \
         {auth_note}.\n\
         Run the agent yourself from the project to pick up the compiled policy, e.g. \
         'cd {project} && {CURSOR_CLI}'.",
        artifact = artifact_path.display(),
        project = project.display(),
    )
}

/// Write the generated permission config to `<project>/.cursor/cli.json` and
/// return its path.
///
/// The config lives at the project root (not under `.flox/cache/`) because the
/// `agent` CLI reads it from the working directory — like the Ona devcontainer,
/// it is meant to sit in the project the agent runs against.
fn write_cursor_config(project: &Path, config: &str) -> Result<PathBuf> {
    let artifact_path = project.join(CURSOR_CONFIG_REL_PATH);
    let dir = artifact_path
        .parent()
        .expect("cursor config path always has a parent");
    std::fs::create_dir_all(dir)
        .with_context(|| format!("failed to create Cursor config dir '{}'", dir.display()))?;
    std::fs::write(&artifact_path, config).with_context(|| {
        format!(
            "failed to write Cursor config to '{}'",
            artifact_path.display()
        )
    })?;
    debug!(path = %artifact_path.display(), "wrote cursor permission config");
    Ok(artifact_path)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use flox_core::activate::sandbox_policy::SandboxNetworkAccess;

    use super::*;

    // ── compile_cursor_network_policy ─────────────────────────────────────────

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
        let policy = compile_cursor_network_policy(&[]).unwrap();
        assert_eq!(policy, CursorNetworkPolicy {
            deny_all: true,
            fetch_domains: Vec::new(),
        });
    }

    #[test]
    fn tls_443_rules_compile_to_fetch_domains() {
        let rules = [rule("api.github.com:443"), rule("api.anthropic.com:443")];
        let policy = compile_cursor_network_policy(&rules).unwrap();
        assert_eq!(policy, CursorNetworkPolicy {
            deny_all: false,
            fetch_domains: vec![
                "api.github.com".to_string(),
                "api.anthropic.com".to_string(),
            ],
        });
    }

    #[test]
    fn duplicate_hosts_are_deduplicated() {
        let rules = [rule("api.github.com:443"), rule("api.github.com:443")];
        let policy = compile_cursor_network_policy(&rules).unwrap();
        assert_eq!(policy.fetch_domains, vec!["api.github.com".to_string()]);
    }

    #[test]
    fn wildcard_host_is_preserved() {
        let policy = compile_cursor_network_policy(&[rule("*.github.com:443")]).unwrap();
        assert_eq!(policy.fetch_domains, vec!["*.github.com".to_string()]);
    }

    #[test]
    fn access_and_binary_do_not_affect_compilation() {
        // WebFetch carries no method distinction or per-binary scope; a scoped
        // grant compiles identically to an unscoped one (declared lossiness).
        let scoped = SandboxNetworkRule {
            endpoint: "api.github.com:443".to_string(),
            access: Some(SandboxNetworkAccess::ReadOnly),
            protocol: None,
            binary: Some("curl".to_string()),
        };
        let policy = compile_cursor_network_policy(&[scoped]).unwrap();
        assert_eq!(policy, CursorNetworkPolicy {
            deny_all: false,
            fetch_domains: vec!["api.github.com".to_string()],
        });
    }

    #[test]
    fn non_443_port_is_rejected() {
        let err = compile_cursor_network_policy(&[rule("db.example.com:5432")]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("WebFetch"), "got: {msg}");
        assert!(msg.contains("db.example.com:443"), "got: {msg}");
    }

    #[test]
    fn endpoint_without_port_is_rejected() {
        let err = compile_cursor_network_policy(&[rule("example.com")]).unwrap_err();
        assert!(err.to_string().contains("<HOST>:<PORT>"), "got: {err}");
    }

    #[test]
    fn endpoint_with_invalid_host_is_rejected() {
        let err = compile_cursor_network_policy(&[rule("bad host\nhost:443")]).unwrap_err();
        assert!(
            err.to_string().contains("Invalid sandbox network endpoint"),
            "got: {err}"
        );
    }

    // ── token construction ────────────────────────────────────────────────────

    #[test]
    fn allow_tokens_lead_with_project_read_write() {
        let policy = CursorNetworkPolicy {
            deny_all: true,
            fetch_domains: Vec::new(),
        };
        assert_eq!(cursor_allow_tokens(&policy), vec![
            "Read(**)".to_string(),
            "Write(**)".to_string(),
        ]);
    }

    #[test]
    fn allow_tokens_append_web_fetch_per_domain() {
        let policy = CursorNetworkPolicy {
            deny_all: false,
            fetch_domains: vec!["api.github.com".to_string(), "*.anthropic.com".to_string()],
        };
        assert_eq!(cursor_allow_tokens(&policy), vec![
            "Read(**)".to_string(),
            "Write(**)".to_string(),
            "WebFetch(api.github.com)".to_string(),
            "WebFetch(*.anthropic.com)".to_string(),
        ]);
    }

    #[test]
    fn deny_tokens_protect_env_and_key_files() {
        // Secrets stay unreadable and unwritable even inside the project; deny
        // precedence in Cursor's model means these override Read(**)/Write(**).
        assert_eq!(cursor_deny_tokens(), vec![
            "Read(**/.env*)".to_string(),
            "Write(**/.env*)".to_string(),
            "Read(**/*.key)".to_string(),
            "Write(**/*.key)".to_string(),
        ]);
    }

    // ── render_cursor_config ──────────────────────────────────────────────────

    fn deny_all_policy() -> CursorNetworkPolicy {
        CursorNetworkPolicy {
            deny_all: true,
            fetch_domains: Vec::new(),
        }
    }

    #[test]
    fn config_deny_all_has_no_web_fetch_and_valid_json() {
        let doc = render_cursor_config(&deny_all_policy());
        let parsed: serde_json::Value = doc
            .parse()
            .unwrap_or_else(|e| panic!("cursor config must be valid JSON: {e}\n{doc}"));
        assert_eq!(parsed["version"], 1);
        let allow = &parsed["permissions"]["allow"];
        assert_eq!(allow[0], "Read(**)");
        assert_eq!(allow[1], "Write(**)");
        // Deny-all: no WebFetch token in the allow list.
        assert!(
            !doc.contains("WebFetch("),
            "deny-all must not emit a WebFetch allow entry:\n{doc}"
        );
        // Secret protection is always present.
        let deny = &parsed["permissions"]["deny"];
        assert_eq!(deny[0], "Read(**/.env*)");
    }

    #[test]
    fn config_allowlist_rendered_and_valid_json() {
        let net = CursorNetworkPolicy {
            deny_all: false,
            fetch_domains: vec!["api.github.com".to_string(), "*.anthropic.com".to_string()],
        };
        let doc = render_cursor_config(&net);
        let parsed: serde_json::Value = doc
            .parse()
            .unwrap_or_else(|e| panic!("cursor config must be valid JSON: {e}\n{doc}"));
        let allow = &parsed["permissions"]["allow"];
        assert_eq!(allow[2], "WebFetch(api.github.com)");
        assert_eq!(allow[3], "WebFetch(*.anthropic.com)");
    }

    #[test]
    fn config_escapes_embedded_quotes_via_serde() {
        // A host that somehow carried a double quote must not break the JSON —
        // serde_json escapes it. (split_endpoint forbids quotes upstream, so this
        // is belt-and-suspenders; the config renderer must not depend on that.)
        let net = CursorNetworkPolicy {
            deny_all: false,
            fetch_domains: vec!["evil\".com".to_string()],
        };
        let doc = render_cursor_config(&net);
        let parsed: serde_json::Value = doc
            .parse()
            .unwrap_or_else(|e| panic!("cursor config must stay valid JSON: {e}\n{doc}"));
        assert_eq!(parsed["permissions"]["allow"][2], "WebFetch(evil\".com)");
    }
}
