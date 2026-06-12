//! `flox sandbox` — review and manage sandbox grants for an `ask`-mode
//! activation.
//!
//! The broker (hosted in the activation executive) owns the pending queue and
//! the grant set; this command is the human-facing front end. It discovers the
//! broker's control socket from the environment's services socket (the sibling
//! rule in `flox_activations::sandbox`), never from the session env — keeping
//! the control path off the session env is part of the self-approval guard.
//!
//! Subcommands:
//!
//! - bare `flox sandbox` — a status summary plus, on a TTY with pending
//!   requests, an interactive grouped review (the trust-flow `Select` idiom).
//! - `list` — saved grants, session grants, the seeded and sensitive sets, and
//!   cap consumption.
//! - `allow <glob>` / `revoke <glob>` — non-interactive grant edits, refused
//!   from inside the sandboxed session (client-side env marker here; the broker
//!   enforces it again server-side via peer credentials).
//!
//! All of this is gated behind the same `sandbox_activate` feature flag as
//! `flox activate --sandbox`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_activations::sandbox::grants::{self, Grant, GrantsFile};
use flox_activations::sandbox::sensitive::SensitiveSet;
use flox_activations::sandbox::{FLOX_VIRTUAL_SANDBOX_VAR, control_socket_path};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use serde::{Deserialize, Serialize};

use super::{EnvironmentSelect, environment_select};
use crate::config::Config;
use crate::utils::dialog::{Dialog, Select};
use crate::utils::message;

/// `flox sandbox` and its subcommands.
#[derive(Debug, Clone, Bpaf)]
pub enum SandboxCommands {
    /// Prints help information
    #[bpaf(command, hide)]
    Help,

    /// Review and act on pending sandbox requests (default)
    #[bpaf(command, hide)]
    Review(#[bpaf(external(review_args))] ReviewArgs),

    /// List saved grants, session grants, and the seeded and sensitive sets
    #[bpaf(command)]
    List(#[bpaf(external(list_args))] ListArgs),

    /// Allow a path glob without prompting
    #[bpaf(command)]
    Allow(#[bpaf(external(allow_args))] AllowArgs),

    /// Revoke a saved or session grant
    #[bpaf(command)]
    Revoke(#[bpaf(external(revoke_args))] RevokeArgs),
}

#[derive(Debug, Clone, Bpaf)]
pub struct ReviewArgs {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

#[derive(Debug, Clone, Bpaf)]
pub struct ListArgs {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

#[derive(Debug, Clone, Bpaf)]
pub struct AllowArgs {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// The path glob to allow (e.g. '~/.cargo/registry/**').
    #[bpaf(positional("GLOB"))]
    glob: String,

    /// Save the grant to grants.toml so future sessions do not ask.
    #[bpaf(long("persist"))]
    persist: bool,
}

#[derive(Debug, Clone, Bpaf)]
pub struct RevokeArgs {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// The path glob to revoke.
    #[bpaf(positional("GLOB"))]
    glob: String,
}

impl SandboxCommands {
    pub async fn handle(self, _config: Config, mut flox: Flox) -> Result<()> {
        // Gate behind the same feature flag as `flox activate --sandbox`.
        if !flox.features.sandbox_activate {
            bail!(
                "'flox sandbox' requires the sandbox_activate feature flag. Set FLOX_FEATURES_SANDBOX_ACTIVATE=true."
            );
        }

        match self {
            SandboxCommands::Help => {
                super::display_help(Some("sandbox".to_string()));
                Ok(())
            },
            SandboxCommands::Review(args) => review(&mut flox, args).await,
            SandboxCommands::List(args) => list(&mut flox, args).await,
            SandboxCommands::Allow(args) => allow(&mut flox, args).await,
            SandboxCommands::Revoke(args) => revoke(&mut flox, args).await,
        }
    }
}

// --- Control-socket protocol (client half) -------------------------------
//
// Mirrors the broker's `control.rs` types. Defined here rather than shared
// from flox-activations because the CLI only needs the wire shape, and a thin
// duplicate keeps the flox crate from depending on the broker's internal
// executive module.

#[derive(Debug, Clone, Serialize)]
struct ControlRequest {
    cmd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    persist: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence: Option<u64>,
}

impl ControlRequest {
    fn new(cmd: &str) -> Self {
        Self {
            cmd: cmd.to_string(),
            pattern: None,
            source: None,
            persist: false,
            created: None,
            evidence: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ControlResponse {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    pending: Vec<PendingView>,
    #[serde(default)]
    grants: Vec<GrantView>,
    #[serde(default)]
    satisfied: Option<usize>,
    #[serde(default)]
    status: Option<StatusView>,
}

#[derive(Debug, Clone, Deserialize)]
struct PendingView {
    req: u64,
    op: String,
    path: String,
    hits: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct GrantView {
    pattern: String,
    #[serde(default)]
    source: Option<String>,
    persisted: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct StatusView {
    mode: String,
    granted: usize,
    pending: usize,
    uptime_secs: u64,
}

/// A live connection to the broker control socket for one environment.
struct Broker {
    socket: PathBuf,
}

impl Broker {
    /// Send one request and read one response. A fresh connection per request,
    /// matching the broker's one-exchange-per-connection protocol.
    fn call(&self, request: &ControlRequest) -> Result<ControlResponse> {
        let stream = UnixStream::connect(&self.socket).map_err(|err| {
            anyhow::anyhow!(
                "could not reach the sandbox broker at {}: {err}.\n\
                 Is the environment activated with '--sandbox ask'?",
                self.socket.display()
            )
        })?;
        let mut line = serde_json::to_string(request)?;
        line.push('\n');
        let mut writer = stream.try_clone()?;
        writer.write_all(line.as_bytes())?;
        writer.flush()?;

        let mut reader = BufReader::new(stream);
        let mut response = String::new();
        reader.read_line(&mut response)?;
        let response: ControlResponse = serde_json::from_str(response.trim())?;
        Ok(response)
    }

    fn list_pending(&self) -> Result<Vec<PendingView>> {
        Ok(self.call(&ControlRequest::new("list-pending"))?.pending)
    }

    fn list_grants(&self) -> Result<Vec<GrantView>> {
        Ok(self.call(&ControlRequest::new("list-grants"))?.grants)
    }

    fn status(&self) -> Result<Option<StatusView>> {
        Ok(self.call(&ControlRequest::new("status"))?.status)
    }

    /// Allow a pattern. Returns how many pending entries it cleared.
    fn allow(
        &self,
        pattern: &str,
        source: &str,
        persist: bool,
        evidence: Option<u64>,
    ) -> Result<usize> {
        let mut request = ControlRequest::new("allow");
        request.pattern = Some(pattern.to_string());
        request.source = Some(source.to_string());
        request.persist = persist;
        request.created = Some(today());
        request.evidence = evidence;
        let response = self.call(&request)?;
        if !response.ok {
            bail!(
                "{}",
                response
                    .error
                    .unwrap_or_else(|| "the broker refused the grant".to_string())
            );
        }
        Ok(response.satisfied.unwrap_or(0))
    }

    fn revoke(&self, pattern: &str) -> Result<()> {
        let mut request = ControlRequest::new("revoke");
        request.pattern = Some(pattern.to_string());
        let response = self.call(&request)?;
        if !response.ok {
            bail!(
                "{}",
                response
                    .error
                    .unwrap_or_else(|| "the broker refused the revoke".to_string())
            );
        }
        Ok(())
    }
}

// --- Discovery ------------------------------------------------------------

/// The pieces of an environment the sandbox commands need: where grants live
/// and where the broker control socket would be.
struct SandboxEnv {
    /// `.flox/cache/sandbox` — grants.toml and the journal.
    grants_dir: PathBuf,
    /// The broker control socket path (may or may not be bound).
    control_socket: PathBuf,
    /// A short description for headers ("environment 'myproject'").
    description: String,
}

impl SandboxEnv {
    /// True when the broker control socket is currently bound (the environment
    /// is activated with `--sandbox ask`).
    fn broker(&self) -> Option<Broker> {
        UnixStream::connect(&self.control_socket)
            .ok()
            .map(|_| Broker {
                socket: self.control_socket.clone(),
            })
    }
}

/// Resolve the environment and derive its grants dir and control socket path.
async fn resolve(flox: &mut Flox, selection: &EnvironmentSelect) -> Result<SandboxEnv> {
    let concrete = selection
        .detect_concrete_environment(flox, "Environment to inspect")
        .await?;
    let grants_dir = concrete.dot_flox_path().join("cache").join("sandbox");
    let services_socket = concrete.services_socket_path(flox)?;
    let control_socket = control_socket_path(&services_socket);
    let description = concrete.dot_flox_path().to_string_lossy().into_owned();
    Ok(SandboxEnv {
        grants_dir,
        control_socket,
        description,
    })
}

// --- Subcommands ----------------------------------------------------------

/// bare `flox sandbox` — summary plus interactive review.
async fn review(flox: &mut Flox, args: ReviewArgs) -> Result<()> {
    let env = resolve(flox, &args.environment).await?;
    let Some(broker) = env.broker() else {
        message::info(format!(
            "No active 'ask' sandbox for {}. Activate with '--sandbox ask' first.",
            env.description
        ));
        return Ok(());
    };

    let status = broker.status()?;
    print_summary(&env, status.as_ref());

    let pending = broker.list_pending()?;
    if pending.is_empty() {
        message::info("No pending requests.");
        return Ok(());
    }

    // Non-TTY: deny-and-queue is fully headless, but interactive review needs a
    // terminal. Bail with the headless-grant hint, the trust-flow precedent.
    if !Dialog::can_prompt() {
        bail!(
            "interactive review requires a terminal.\n  Grant non-interactively with: flox sandbox allow '<glob>'"
        );
    }

    let sensitive = sensitive_set();
    review_pending(&broker, &pending, &sensitive).await
}

/// Print the status summary block (§7.4 mockup).
fn print_summary(env: &SandboxEnv, status: Option<&StatusView>) {
    let header = match status {
        Some(status) => format!("Sandbox '{}' — {} (active)", status.mode, env.description),
        None => format!("Sandbox 'ask' — {}", env.description),
    };
    message::plain(header);
    if let Some(status) = status {
        message::plain(format!(
            "\n  Granted (session)   {}\n  Pending             {}\n  Uptime              {}s",
            status.granted, status.pending, status.uptime_secs
        ));
    }
}

/// Walk the pending queue, prompting per group and applying the choice.
async fn review_pending(
    broker: &Broker,
    pending: &[PendingView],
    sensitive: &SensitiveSet,
) -> Result<()> {
    let total = pending.len();
    for (index, entry) in pending.iter().enumerate() {
        let remaining = total - index - 1;
        let is_sensitive = sensitive.is_sensitive(&entry.path);

        let sensitive_tag = if is_sensitive { "   [sensitive]" } else { "" };
        message::warning(format!(
            "request wants to {} {} (req {}, x{}){}",
            entry.op, entry.path, entry.req, entry.hits, sensitive_tag
        ));

        // The directory-scope option is offered only when the parent is a
        // sensible scope AND the path is not sensitive (a credentials dir must
        // never be foldable into one grant).
        let scope = (!is_sensitive)
            .then(|| directory_scope(&entry.path))
            .flatten();

        let outcome = prompt_choice(&entry.path, scope.as_deref(), remaining).await?;
        apply_choice(broker, &entry.path, scope.as_deref(), outcome)?;
        if matches!(outcome, Choice::DecideLater) {
            // Decide-later leaves it queued; keep going to the next request.
            continue;
        }
    }
    Ok(())
}

/// The five review options (§Q3), plus the directory-scope variant when
/// offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choice {
    AllowFileSession,
    AllowFileAlways,
    AllowDirAlways,
    DenySession,
    DecideLater,
}

/// Build the ordered (label, choice) list for one request. The directory
/// option appears only when `scope` is `Some`, and never for a sensitive path
/// (the caller passes `None` then). Factored out so the option order — which
/// the review mockups pin — is unit-testable without a terminal.
fn choice_options(scope: Option<&str>) -> Vec<(String, Choice)> {
    let mut options = vec![
        (
            "Allow this file, this session".to_string(),
            Choice::AllowFileSession,
        ),
        (
            "Allow this file, always (save to environment allowlist)".to_string(),
            Choice::AllowFileAlways,
        ),
    ];
    if let Some(scope) = scope {
        options.push((
            format!("Allow everything in {scope}/, always"),
            Choice::AllowDirAlways,
        ));
    }
    options.push((
        "Deny, don't ask again this session".to_string(),
        Choice::DenySession,
    ));
    options.push(("Decide later".to_string(), Choice::DecideLater));
    options
}

/// Prompt for one request, returning the chosen action. The directory option
/// appears only when `scope` is `Some`.
async fn prompt_choice(path: &str, scope: Option<&str>, remaining: usize) -> Result<Choice> {
    let options = choice_options(scope);

    let help = if remaining > 0 {
        format!("[↑↓ move, enter selects, esc keeps it queued · +{remaining} more pending]")
    } else {
        "[↑↓ move, enter selects, esc keeps it queued]".to_string()
    };

    let message = format!("Allow {path}?");
    let labels: Vec<String> = options.iter().map(|(label, _)| label.clone()).collect();
    let dialog = Dialog {
        message: &message,
        help_message: Some(&help),
        typed: Select { options: labels },
    };

    // Esc/Ctrl-C maps to decide-later, matching the help line and the trust
    // flow's non-committal default.
    match dialog.raw_prompt() {
        Ok((index, _)) => Ok(options[index].1),
        Err(inquire::InquireError::OperationCanceled)
        | Err(inquire::InquireError::OperationInterrupted) => Ok(Choice::DecideLater),
        Err(err) => Err(anyhow::Error::new(err)),
    }
}

/// Apply a review choice via the control socket.
fn apply_choice(broker: &Broker, path: &str, scope: Option<&str>, choice: Choice) -> Result<()> {
    match choice {
        Choice::AllowFileSession => {
            broker.allow(path, "review", false, None)?;
            message::updated(format!("Allowed {path} for this session."));
        },
        Choice::AllowFileAlways => {
            broker.allow(path, "review", true, None)?;
            message::updated(format!(
                "Saved grant '{path}' to grants.toml — future sessions won't ask."
            ));
        },
        Choice::AllowDirAlways => {
            let scope = scope.expect("directory choice requires a scope");
            let pattern = format!("{scope}/**");
            broker.allow(&pattern, "review", true, None)?;
            message::updated(format!(
                "Saved grant '{pattern}' to grants.toml — future sessions won't ask."
            ));
        },
        Choice::DenySession => {
            // The engine has no persisted-deny mechanism; "deny this session"
            // is recorded by leaving it unqueued. There is no broker verb for a
            // session deny in this batch, so the receipt-silencing is a no-op
            // beyond not granting — the path simply stays denied (its default).
            message::info(format!("Denied {path} for this session."));
        },
        Choice::DecideLater => {
            message::info(format!("Left {path} queued."));
        },
    }
    Ok(())
}

/// `flox sandbox list` — the grants/provenance/cap readout (the saved-grants
/// inspection surface).
async fn list(flox: &mut Flox, args: ListArgs) -> Result<()> {
    let env = resolve(flox, &args.environment).await?;
    let saved = grants::read_grants(&env.grants_dir);
    let sensitive = sensitive_set();

    message::plain(format!("Saved grants for {}", env.description));
    message::plain(format!(
        "({} — edit by hand or flox sandbox allow|revoke)\n",
        grants::grants_file_path(&env.grants_dir).display()
    ));

    if saved.grants.is_empty() {
        message::plain("  (no saved grants)\n");
    } else {
        message::plain(
            "  PATTERN                          OPS    SOURCE              ADDED       EVIDENCE",
        );
        for grant in &saved.grants {
            message::plain(format!("  {}", format_grant_row(grant)));
        }
        message::plain("");
    }

    // Session grants (only available when the broker is up).
    if let Some(broker) = env.broker()
        && let Ok(grants) = broker.list_grants()
    {
        let session_only: Vec<&GrantView> = grants.iter().filter(|g| !g.persisted).collect();
        if !session_only.is_empty() {
            message::plain("Session grants (expire with the activation):");
            for grant in session_only {
                message::plain(format!(
                    "  {:<32} {}",
                    grant.pattern,
                    grant.source.as_deref().unwrap_or("review")
                ));
            }
            message::plain("");
        }
    }

    // The sensitive set: never auto-granted, never folded into a directory
    // grant.
    message::plain("Sensitive (never auto-granted, never folded into a directory grant):");
    message::plain(format!("  {}", sensitive.patterns().join(" ")));
    message::plain("");

    // Cap consumption.
    let entries = saved.grants.len();
    let bytes: usize = saved.grants.iter().map(|g| g.pattern.len()).sum();
    message::plain(format!(
        "{entries} saved grant(s) use {entries} of {} allow entries ({:.1} of {} KB).",
        flox_activations::sandbox::ALLOW_ENTRIES_MAX,
        bytes as f64 / 1024.0,
        flox_activations::sandbox::ALLOW_BYTES_MAX / 1024,
    ));
    message::info("OPS is informational; saved grants allow all access kinds in this prototype.");

    // Tamper diff: grants present in the file but missing from the journal.
    let unjournaled = grants::unjournaled_patterns(&env.grants_dir);
    if !unjournaled.is_empty() {
        message::warning(format!(
            "{} grant(s) present in the file but missing from the journal (added outside flox — possibly self-approved):",
            unjournaled.len()
        ));
        for pattern in &unjournaled {
            message::plain(format!("    {pattern}"));
        }
        message::plain("  Keep them if intentional, or remove with: flox sandbox revoke '<glob>'");
    }

    Ok(())
}

/// Render one grants.toml row for the list table.
fn format_grant_row(grant: &Grant) -> String {
    let ops = if grant.ops.is_empty() {
        "any".to_string()
    } else {
        grant.ops.join(",")
    };
    let source = grant.source.as_deref().unwrap_or("-");
    let added = grant.created.as_deref().unwrap_or("-");
    let evidence = match grant.evidence {
        Some(n) => format!("{n} files"),
        None => "manual".to_string(),
    };
    format!(
        "{:<32} {:<6} {:<19} {:<11} {}",
        grant.pattern, ops, source, added, evidence
    )
}

/// `flox sandbox allow <glob>` — non-interactive grant.
async fn allow(flox: &mut Flox, args: AllowArgs) -> Result<()> {
    refuse_if_in_session("allow")?;
    let env = resolve(flox, &args.environment).await?;

    if let Some(broker) = env.broker() {
        // A live broker enforces the self-approval guard server-side and
        // applies the grant to the running session immediately. `allow` always
        // persists (it is the non-interactive "save" verb).
        let satisfied = broker.allow(&args.glob, "allow", true, None)?;
        message::updated(format!(
            "Saved grant '{}' (cleared {satisfied} pending) — future sessions won't ask.",
            args.glob
        ));
    } else {
        // No active broker: edit grants.toml directly so a grant can be
        // pre-seeded before the next activation. The grant is journaled so it
        // is not flagged as self-approved.
        edit_grants_file(&env.grants_dir, |file| {
            file.grants.retain(|g| g.pattern != args.glob);
            file.grants.push(Grant {
                pattern: args.glob.clone(),
                ops: Vec::new(),
                source: Some("allow".to_string()),
                created: Some(today()),
                evidence: None,
            });
        })?;
        grants::append_journal(&env.grants_dir, &grants::JournalRecord {
            event: "grant".to_string(),
            pattern: Some(args.glob.clone()),
            source: Some("allow".to_string()),
            created: Some(today()),
        });
        message::updated(format!(
            "Saved grant '{}' to grants.toml — it applies at the next activation.",
            args.glob
        ));
    }
    let _ = args.persist; // `allow` always persists; the flag is accepted for symmetry.
    Ok(())
}

/// `flox sandbox revoke <glob>` — non-interactive revoke.
async fn revoke(flox: &mut Flox, args: RevokeArgs) -> Result<()> {
    refuse_if_in_session("revoke")?;
    let env = resolve(flox, &args.environment).await?;

    if let Some(broker) = env.broker() {
        broker.revoke(&args.glob)?;
        message::updated(format!("Revoked '{}'.", args.glob));
    } else {
        edit_grants_file(&env.grants_dir, |file| {
            file.grants.retain(|g| g.pattern != args.glob);
        })?;
        message::updated(format!("Removed '{}' from grants.toml.", args.glob));
    }
    Ok(())
}

// --- Helpers --------------------------------------------------------------

/// Refuse an approval verb run from inside the sandboxed session.
///
/// The client-side half of the self-approval guard: `FLOX_VIRTUAL_SANDBOX` is
/// exported into the session, so its presence means this `flox sandbox` is
/// running inside the very activation it would modify. The broker enforces the
/// same refusal server-side via peer credentials (which an env-var unset cannot
/// evade), so this check is the friendly early bail, not the load-bearing one.
fn refuse_if_in_session(verb: &str) -> Result<()> {
    if std::env::var_os(FLOX_VIRTUAL_SANDBOX_VAR).is_some() {
        bail!(
            "refusing to {verb} from inside the sandboxed session.\n  \
             Run it from another terminal: flox sandbox {verb} '<glob>'"
        );
    }
    Ok(())
}

/// Read, mutate, and atomically write back grants.toml.
fn edit_grants_file(grants_dir: &Path, mutate: impl FnOnce(&mut GrantsFile)) -> Result<()> {
    let mut file = grants::read_grants(grants_dir);
    mutate(&mut file);
    grants::write_grants(grants_dir, &file)?;
    Ok(())
}

/// Build the sensitive set, honoring `FLOX_SANDBOX_SENSITIVE` and `$HOME`.
fn sensitive_set() -> SensitiveSet {
    SensitiveSet::from_env(dirs::home_dir().as_deref())
}

/// The directory-scope suggestion for a path: its parent, when that parent is a
/// sensible scope (not `$HOME` itself and not `/`). Returns the directory
/// without a trailing `/`; the caller appends `/**` when it grants.
fn directory_scope(path: &str) -> Option<String> {
    let parent = Path::new(path).parent()?;
    let parent_str = parent.to_str()?;
    if parent_str.is_empty() || parent_str == "/" {
        return None;
    }
    if let Some(home) = dirs::home_dir()
        && parent == home
    {
        // Never suggest "$HOME/**" — far too broad.
        return None;
    }
    Some(parent_str.to_string())
}

/// Today's date as `YYYY-MM-DD`, for the `created` stamp on a grant. The broker
/// stays clock-free; the CLI stamps the value and passes it in.
fn today() -> String {
    let now = time::OffsetDateTime::now_utc();
    format!(
        "{:04}-{:02}-{:02}",
        now.year(),
        now.month() as u8,
        now.day()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_scope_suggests_the_parent_dir() {
        assert_eq!(
            directory_scope("/home/dev/.cargo/registry/index/foo"),
            Some("/home/dev/.cargo/registry/index".to_string())
        );
    }

    #[test]
    fn directory_scope_never_suggests_root() {
        assert_eq!(directory_scope("/etc"), None);
    }

    #[test]
    fn the_in_session_marker_blocks_approval_verbs() {
        // With the marker set, an approval verb refuses; cleared, it passes.
        temp_env::with_var(FLOX_VIRTUAL_SANDBOX_VAR, Some("ask"), || {
            assert!(refuse_if_in_session("allow").is_err());
        });
        temp_env::with_var(FLOX_VIRTUAL_SANDBOX_VAR, None::<&str>, || {
            assert!(refuse_if_in_session("allow").is_ok());
        });
    }

    #[test]
    fn a_sensitive_path_gets_no_directory_suggestion_in_review() {
        // The review layer suppresses the directory option for a sensitive
        // path even though its parent would otherwise be a valid scope.
        let sensitive = SensitiveSet::from_entries(vec!["/home/dev/.aws/**".to_string()], None);
        let path = "/home/dev/.aws/credentials";
        let scope = (!sensitive.is_sensitive(path))
            .then(|| directory_scope(path))
            .flatten();
        assert_eq!(scope, None, "a sensitive path must not offer a dir scope");

        // A non-sensitive sibling still gets its directory scope.
        let safe = "/home/dev/.cargo/registry/foo";
        let scope = (!sensitive.is_sensitive(safe))
            .then(|| directory_scope(safe))
            .flatten();
        assert_eq!(scope, Some("/home/dev/.cargo/registry".to_string()));
    }

    #[test]
    fn choice_options_order_matches_the_review_mockup() {
        // Without a scope: file-session, file-always, deny, decide-later.
        let without = choice_options(None);
        let choices: Vec<Choice> = without.iter().map(|(_, c)| *c).collect();
        assert_eq!(choices, vec![
            Choice::AllowFileSession,
            Choice::AllowFileAlways,
            Choice::DenySession,
            Choice::DecideLater,
        ]);

        // With a scope: the directory option slots in third, before deny.
        let with = choice_options(Some("/home/dev/.config/gh"));
        let choices: Vec<Choice> = with.iter().map(|(_, c)| *c).collect();
        assert_eq!(choices, vec![
            Choice::AllowFileSession,
            Choice::AllowFileAlways,
            Choice::AllowDirAlways,
            Choice::DenySession,
            Choice::DecideLater,
        ]);
        // The directory label names the scope as the mockup shows.
        assert!(
            with[2].0.contains("/home/dev/.config/gh/"),
            "got {:?}",
            with[2].0
        );
    }

    #[test]
    fn today_is_iso_dated() {
        let today = today();
        assert_eq!(today.len(), 10);
        assert_eq!(today.as_bytes()[4], b'-');
        assert_eq!(today.as_bytes()[7], b'-');
    }
}
