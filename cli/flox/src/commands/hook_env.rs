use std::borrow::Cow;
use std::io::{BufWriter, IsTerminal, Write, stdout};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use flox_activations::attach_diff::diff_serializer::FLOX_HOOK_DIFF_VAR;
use flox_activations::deactivate::embedded_hook_diff;
use flox_config::{AutoActivate, AutoActivationPreference, Config};
use flox_core::activate::context::{InvocationKind, SandboxMode};
use flox_core::activate::sandbox_backend::SandboxBackend;
use flox_core::activate::vars::{
    FLOX_AUTO_ACTIVATED_ENVIRONMENTS_VAR,
    FLOX_SUPPRESSED_ENVIRONMENTS_VAR,
};
use flox_core::hook_actions::{HookAction, take_hook_actions};
use flox_manifest::interfaces::AsLatestSchema;
use flox_manifest::{MANIFEST_FILENAME, Manifest};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{ENV_DIR_NAME, find_all_dot_flox};
use indoc::formatdoc;
use shell_gen::{GenerateShell, SetVar, Shell, ShellWithPath, UnsetVar};
use tracing::debug;

use super::activate::write_auto_activation_preference;
use super::activated_environments;
use super::deactivate::{
    emit_deactivate_script,
    flox_activate_tracelevel,
    open_deactivation_target,
};
use crate::subcommand_metric;
use crate::utils::dialog::{Confirm, Dialog};
use crate::utils::message;

#[derive(Debug, Clone, Bpaf)]
pub struct HookEnv {
    /// Shell to emit hook-env code for (bash, zsh, fish, tcsh)
    #[bpaf(long("shell"), argument("SHELL"))]
    shell: Shell,

    /// PID of the calling interactive shell ($$ / $fish_pid).
    ///
    /// The shell expands this before invoking `hook-env`, so it identifies the
    /// interactive shell even though `hook-env` itself runs in a command
    /// substitution subshell. It keys the prompt-hook action file this shell
    /// reads.
    #[bpaf(long("shell-pid"), argument("PID"))]
    shell_pid: i32,

    /// Invocation type of the activation the hook is running in
    /// (`$_FLOX_INVOCATION_TYPE`), used when emitting a deactivation script.
    ///
    /// Optional as a defensive measure. Every shell hook passes it (tcsh guards
    /// a possibly-unset value with `$?`); when a deactivate action is pending but
    /// none was provided, the hook falls back to `inplace`.
    #[bpaf(long("invocation-type"), argument("INVOCATION_TYPE"), optional)]
    invocation_kind: Option<InvocationKind>,
}

impl HookEnv {
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        let mut writer = BufWriter::new(stdout());

        // Consume any actions another flox command (e.g. `flox deactivate`) left
        // for this shell and emit the corresponding script. The common case is
        // no pending actions.
        let actions = take_hook_actions(&flox.runtime_dir, self.shell_pid)
            .context("failed to read prompt-hook actions")?;
        for action in &actions {
            match action {
                HookAction::Deactivate {
                    activation_state_dir,
                    flox_env,
                } => {
                    // Default to in-place when the shell didn't pass an
                    // invocation type. Every shell hook passes it today; this is
                    // a defensive fallback, and the prompt hook only ever
                    // deactivates in place.
                    let invocation_kind = self.invocation_kind.unwrap_or(InvocationKind::InPlace);
                    emit_deactivate_script(
                        ShellWithPath::from(self.shell),
                        invocation_kind,
                        activation_state_dir,
                        flox_env,
                        flox_activate_tracelevel(),
                        None,
                        &mut writer,
                    )?;
                },
            }
        }

        // The deactivate-action handling above runs unconditionally; the
        // auto-activation logic below stays gated behind the auto_activate
        // feature flag.
        if !flox.features.auto_activate {
            // Only record a metric when this run actually does something;
            // `hook-env` runs on every shell prompt, and recording the common
            // nothing-to-do case would be noise.
            if !actions.is_empty() {
                subcommand_metric!("hook-env");
            }
            writer.flush()?;
            return Ok(());
        }

        let ctx = gather_auto_activate_context(&config, &flox, !actions.is_empty())?;
        let plan = plan_auto_activation(&ctx);

        // Whether the deactivate sweep popped every planned layer. A re-insertion
        // or buried-leaver teardown plans survivors in `plan.reactivate` that are
        // only safe to replay once all the layers above them have actually been
        // popped; if the chain runs dry mid-sweep (an activation predates the
        // hook), the replay is suppressed below to avoid double-activating layers
        // that are still on the stack.
        let mut popped_all = true;
        if !plan.deactivate.is_empty() {
            // Auto-activations are always in-place, so each layer recorded
            // the previous value of `_FLOX_HOOK_DIFF` in its own diff. The
            // shell's variable holds the front layer's diff; walking the
            // embedded chain yields one diff per deeper layer, which is what
            // lets a single run pop several layers.
            let mut encoded_diff = std::env::var(FLOX_HOOK_DIFF_VAR).ok();
            let stack = activated_environments();
            let mut layers = stack.iter_full();
            for (layer_idx, project_dir) in plan.deactivate.iter().enumerate() {
                let layer = layers.next().with_context(|| {
                    format!(
                        "no active environment to auto-deactivate for '{}'",
                        project_dir.display()
                    )
                })?;
                let layer_dir = layer.environment.path().and_then(Path::parent);
                if layer_dir != Some(project_dir.as_path()) {
                    bail!(
                        "activation stack does not match the auto-deactivation plan (expected '{}')",
                        project_dir.display()
                    );
                }
                let Some(diff) = encoded_diff.take() else {
                    // This layer has no chained diff: its activation predates
                    // the prompt hook (or `_FLOX_HOOK_DIFF` was lost), so the
                    // walk can go no deeper. This layer and every tracked
                    // leaver still queued behind it are dropped from tracking
                    // below, so warn for each one here; otherwise they would
                    // be silently stranded, still active but no longer
                    // auto-managed. They can each still be unwound with
                    // 'flox deactivate'.
                    message::warning(formatdoc! {"
                        Did not auto-deactivate the environment in '{}' because its activation predates the prompt hook.
                        Run 'flox deactivate' to deactivate it.",
                        project_dir.display()
                    });
                    for buried in &plan.deactivate[layer_idx + 1..] {
                        message::warning(format!(
                            "Did not auto-deactivate the environment in '{}' because an environment above it could not be auto-deactivated.",
                            buried.display()
                        ));
                    }
                    popped_all = false;
                    break;
                };
                let target = open_deactivation_target(&flox, layer.clone())?;
                // Auto-activations are always in-place, so the matching
                // deactivation is too.
                emit_deactivate_script(
                    ShellWithPath::from(self.shell),
                    InvocationKind::InPlace,
                    &target.activation_state_dir,
                    &target.flox_env,
                    flox_activate_tracelevel(),
                    Some(&diff),
                    &mut writer,
                )?;
                encoded_diff = embedded_hook_diff(&diff)?;
            }
        }

        for path in &plan.abandoned {
            message::warning(formatdoc! {"
                Did not auto-deactivate the environment in '{}' because other environments are layered on top.
                Run 'flox deactivate' to deactivate them in order.",
                path.display()
            });
        }

        // Ask for consent once for all unregistered environments discovered
        // this run, rather than once per environment: walking into a deep tree
        // can surface several at once, and a prompt per layer is tedious. The
        // single answer applies to the whole batch (`plan.prompt`).
        let consent = if plan.prompt.is_empty() {
            AutoActivateConsent::NoTerminal
        } else {
            prompt_for_auto_activation(&plan.prompt).await?
        };

        // For each environment declaring a wrapping sandbox backend, prompt
        // for session-entry consent individually. Accepting starts a blocking
        // foreground session, so each entry needs its own answer; unlike
        // plain auto-activation, multiple pending sessions cannot share a
        // single prompt, and at most one session is entered per hook run.
        let mut sandbox_entries_entered: Vec<PathBuf> = Vec::new();
        for (path, backend) in &plan.prompt_sandbox {
            match prompt_for_sandbox_activation(path, *backend, self.shell)? {
                SandboxConsent::Accept => {
                    sandbox_entries_entered.push(path.clone());
                    // The session runs as a foreground child of the shell.
                    // The state exports emitted further below are evaluated
                    // only after the session ends, so the suppression
                    // recorded for this path lands once the user is back in
                    // their original shell.
                    write_sandbox_session_command(self.shell, path, &mut writer)?;
                    // Enter at most one sandboxed session per hook run.
                    // Remaining entries are suppressed below and re-prompt
                    // after the directory is left and re-entered.
                    break;
                },
                SandboxConsent::Decline => {
                    // Suppress for this shell session; the next shell or
                    // re-entry will ask again. Follow the same debounce
                    // semantics as the plain consent flow.
                    if !plan.suppressed.contains(path) {
                        // We'll add it to suppressed after the loop.
                    }
                },
                SandboxConsent::NoTerminal | SandboxConsent::UnsupportedShell => {
                    // Cannot start a sandboxed session without an interactive
                    // tty, or from this shell's prompt hook. Emit a notice
                    // pointing at `flox activate`.
                    message::info(formatdoc! {"
                        ℹ️  Run 'flox activate --dir {dir}' to enter this environment sandboxed via {backend}.",
                        dir = path.display(),
                    });
                },
            }
        }

        // Print a notice for each environment that declares a libsandbox
        // sandbox so the user knows in-place auto-activation is not mediated.
        for path in &plan.libsandbox_notice {
            message::info(formatdoc! {"
                ℹ️  This environment declares a libsandbox sandbox; \
                in-place auto-activation is not mediated — \
                run 'flox activate --sandbox <MODE> --dir {dir}' for a sandboxed session.",
                dir = path.display(),
            });
        }

        // Only record a metric when this run actually does something;
        // `hook-env` runs on every shell prompt, and recording the common
        // nothing-to-do case would be noise. Gate the prompt case on the
        // consent answer, not `plan.prompt`: a non-interactive shell has no
        // controlling terminal, so it yields `NoTerminal` and does nothing —
        // counting it would fire the metric on every prompt.
        if !actions.is_empty()
            || !plan.deactivate.is_empty()
            || !plan.activate.is_empty()
            || !plan.reactivate.is_empty()
            || plan.reinsert.is_some()
            || matches!(
                consent,
                AutoActivateConsent::Allow
                    | AutoActivateConsent::Deny
                    | AutoActivateConsent::Suppress
            )
            || !plan.abandoned.is_empty()
            || !sandbox_entries_entered.is_empty()
            || !plan.prompt_sandbox.is_empty()
        {
            subcommand_metric!("hook-env");
        }

        // Walk discovered directories outermost-first so activations stack in
        // the right order. Allowed environments activate directly; unregistered
        // ones (only present in `prompt` mode) activate or are suppressed
        // according to the consent answer. Prompt outcomes adjust the
        // tracked-state lists the planner produced.
        let mut auto_activated = plan.auto_activated.clone();
        let mut suppressed = plan.suppressed.clone();

        // Record every sandbox-consent candidate as suppressed so the hook
        // stops asking while the shell stays inside the directory. Entered
        // paths are suppressed too: the sandboxed session runs as a
        // foreground child, so the shell survives the session and would
        // otherwise re-prompt at the very next prompt draw. Leaving the
        // directory clears the suppression, so re-entering asks again.
        suppress_sandbox_candidates(&plan.prompt_sandbox, &mut suppressed);

        for path in &ctx.discovered {
            if plan.activate.contains(path) {
                write_activate_command(self.shell, path, &mut writer)?;
            } else if plan.prompt.contains(path) {
                match consent {
                    AutoActivateConsent::Allow => {
                        write_auto_activation_preference(
                            &config.flox.config_dir,
                            path,
                            AutoActivationPreference::Allow,
                        )?;
                        write_activate_command(self.shell, path, &mut writer)?;
                        if !auto_activated.contains(path) {
                            auto_activated.push(path.clone());
                        }
                    },
                    // Deny: remember the refusal by persisting `Deny` to config,
                    // exactly as `flox activate deny` does.
                    // The user-facing note is emitted once after the
                    // loop, not per environment.
                    AutoActivateConsent::Deny => {
                        write_auto_activation_preference(
                            &config.flox.config_dir,
                            path,
                            AutoActivationPreference::Deny,
                        )?;
                    },
                    // Skip: suppress for this shell so the hook stops asking
                    // while the shell stays within the directory. Leaving clears
                    // the suppression (re-entering asks again); answering `N`
                    // makes the refusal permanent. Reached when the user bails
                    // out of the prompt (Esc or Ctrl-C). The user-facing note is
                    // emitted once after the loop, not per environment.
                    AutoActivateConsent::Suppress => {
                        if !suppressed.contains(path) {
                            suppressed.push(path.clone());
                        }
                    },
                    // No terminal to prompt on (non-interactive shell): leave
                    // the environment unregistered so a later interactive prompt
                    // can still ask. Take no action and record nothing.
                    AutoActivateConsent::NoTerminal => {},
                }
            }
        }

        // Commit the re-insertion target, if any, between the deactivations and
        // the reactivate replay so the diff chain rebuilds in ancestor order:
        // deactivate(descendants) -> activate(target) -> reactivate(descendants).
        //
        // Only when the deactivate sweep popped every descendant. If the chain
        // ran dry mid-sweep the descendants are still on the stack, so
        // activating the target now would stack it on top (out of order) and,
        // once recorded in `auto_activated`, `is_active(target)` would block
        // re-insertion forever. Defer instead: emit and record nothing this run;
        // the next prompt retries cleanly (descendants still present, target
        // still allowed/inside/inactive).
        if let Some(target) = &plan.reinsert
            && popped_all
        {
            write_activate_command(self.shell, target, &mut writer)?;
            if !auto_activated.contains(target) {
                auto_activated.push(target.clone());
            }
        }

        // Replay the survivors torn down to rebuild the stack in ancestor order
        // (re-insertion or buried-leaver teardown). They are emitted bottom-up,
        // after the deactivations and the inserted layer, so each re-activation
        // re-reads the now-unwound environment and chains onto it correctly.
        //
        // Only replay when the deactivate sweep actually popped every planned
        // layer. If the chain ran dry, the survivors are still on the stack, so
        // replaying them would activate a second copy. No warning is needed
        // here: the deactivate sweep already warned per layer for every
        // un-replayed survivor (they are among the buried layers it listed), and
        // a deferred re-insertion (`plan.reinsert` set but not committed) retries
        // silently on the next prompt.
        if !plan.reactivate.is_empty() && popped_all {
            for path in &plan.reactivate {
                write_activate_command(self.shell, path, &mut writer)?;
            }
        }

        // One note for the whole batch: the prompt already listed the
        // environments, so the note repeats neither the list nor a per
        // environment explanation. Reached only on the run the user answers;
        // afterwards the planner drops the suppressed (Skip) or denied
        // (Deny) environments from the prompt, so `plan.prompt` is empty.
        if !plan.prompt.is_empty() {
            let environments = if plan.prompt.len() == 1 {
                "the environment"
            } else {
                "these environments"
            };
            match consent {
                AutoActivateConsent::Suppress => message::info(formatdoc! {"
                    Did not auto-activate {environments}.
                    You will be asked again in a new shell or when you re-enter the directory.
                    Run 'flox activate deny --dir <PATH>' to stop being asked."
                }),
                AutoActivateConsent::Deny => message::info(formatdoc! {"
                    Disabled auto-activation for {environments}.
                    Run 'flox activate allow --dir <PATH>' to re-enable."
                }),
                AutoActivateConsent::Allow | AutoActivateConsent::NoTerminal => {},
            }
        }

        write_path_list_update(
            self.shell,
            FLOX_AUTO_ACTIVATED_ENVIRONMENTS_VAR,
            &ctx.auto_activated,
            &auto_activated,
            &mut writer,
        )?;
        write_path_list_update(
            self.shell,
            FLOX_SUPPRESSED_ENVIRONMENTS_VAR,
            &ctx.suppressed,
            &suppressed,
            &mut writer,
        )?;

        writer.flush()?;
        Ok(())
    }
}

/// The sandbox class of a discovered environment's manifest declaration.
///
/// Used to route the auto-activation decision: wrapping backends require
/// explicit session-replacement consent; libsandbox is in-place advisory and
/// handled with a notice; absent/off means today's plain in-place flow.
#[derive(Clone, Debug, PartialEq)]
enum SandboxClass {
    /// No sandbox declared (`options.sandbox` absent or `off`).
    None,
    /// Advisory libsandbox declared. In-place activation proceeds, but the
    /// user sees a one-line info note explaining that the sandbox is not
    /// mediated under auto-activation.
    Libsandbox,
    /// A wrapping backend (host-native, srt, oci, libkrun, nix). In-place
    /// activation is never permitted; the hook must obtain explicit consent
    /// before starting the sandboxed session.
    Wrapping(SandboxBackend),
}

/// Inputs to [`plan_auto_activation`].
///
/// Gathered from the runtime environment by [`gather_auto_activate_context`]
/// so the planning logic itself is pure and unit-testable.
#[derive(Clone, Debug, PartialEq)]
struct AutoActivateContext {
    /// Canonicalized working directory of the interactive shell.
    cwd: PathBuf,
    /// Project directories with a discoverable `.flox`, outermost-first.
    discovered: Vec<PathBuf>,
    /// Sandbox class for each discovered directory, parallel to `discovered`.
    ///
    /// When the manifest cannot be read (I/O error, parse error), the entry
    /// defaults to `SandboxClass::None` so the failure is non-fatal and
    /// degrades to today's behaviour.
    discovered_sandbox: Vec<SandboxClass>,
    /// Project directories of active environments, most recently activated
    /// first. `None` for environments without a local directory (remote).
    active: Vec<Option<PathBuf>>,
    /// Project directories auto-activated by the hook in this shell.
    auto_activated: Vec<PathBuf>,
    /// Project directories suppressed from auto-activation in this shell.
    suppressed: Vec<PathBuf>,
    /// Project directories the user has allowed auto-activation for via the
    /// consent prompt or `flox activate allow` (config
    /// `auto_activate_environments`).
    allowed: Vec<PathBuf>,
    /// Project directories the user has denied auto-activation for via
    /// `flox activate deny` (config `auto_activate_environments`).
    denied: Vec<PathBuf>,
    /// Whether to prompt before auto-activating an environment that is neither
    /// allowed nor denied (config `auto_activate = "prompt"`). When false
    /// (`auto_activate = "allowed"`), unregistered environments are skipped
    /// silently.
    prompt_unregistered: bool,
    /// Whether this run consumed pending prompt-hook deactivation actions.
    pending_deactivations: bool,
    /// Whether the `sandbox_activate` feature flag is on.
    sandbox_activate_enabled: bool,
}

/// What the prompt hook should do this run, plus the new values of the
/// auto-activation state variables.
#[derive(Clone, Debug, PartialEq)]
struct AutoActivatePlan {
    /// Project directories to activate, outermost-first. These are environments
    /// the user has already allowed.
    activate: Vec<PathBuf>,
    /// Unregistered project directories to prompt the user about before
    /// activating, outermost-first. Empty unless `auto_activate = "prompt"`.
    prompt: Vec<PathBuf>,
    /// Project directories with a wrapping sandbox backend that require
    /// explicit session-entry consent before entering. Each entry pairs
    /// the project directory with the backend that would enforce it, so the
    /// consent prompt can name the backend. These are never in-place activated.
    prompt_sandbox: Vec<(PathBuf, SandboxBackend)>,
    /// Project directories that declare a libsandbox sandbox. Auto-activation
    /// proceeds in-place, but the hook prints a one-line info note per
    /// directory explaining that advisory mediation is not applied.
    libsandbox_notice: Vec<PathBuf>,
    /// Project directories to deactivate, front of stack first. Includes both
    /// true leavers (gone for good) and survivors that are torn down only to
    /// insert or remove a layer beneath them; the survivors are listed in
    /// `reactivate`.
    deactivate: Vec<PathBuf>,
    /// Active layers to re-push in ancestor order after the deactivations,
    /// outermost-first (bottom-up). These were torn down only to rebuild the
    /// stack around a layer inserted or removed beneath them, so they remain
    /// tracked in `auto_activated`.
    reactivate: Vec<PathBuf>,
    /// The single environment to re-insert beneath its tracked descendants this
    /// run, if any. Held separately from `activate` because its activation must
    /// be deferred when the deactivate sweep can't pop every descendant: in that
    /// case the descendants are still on the stack, so activating the target now
    /// would stack it on top (out of order) and, once tracked, block any retry.
    /// `handle()` emits it — and records it in `auto_activated` — only after the
    /// sweep confirms every descendant was popped. Emitted between the
    /// deactivations and the `reactivate` replay so the `_FLOX_HOOK_DIFF` chain
    /// rebuilds as deactivate(descendants) -> activate(target) -> reactivate.
    reinsert: Option<PathBuf>,
    /// Auto-activated environments that should be deactivated but are buried
    /// under a layer that cannot be popped (manually activated, remote, or an
    /// activation predating the prompt hook). Tearing down across such a layer
    /// is not supported, so these are dropped from tracking with a warning.
    abandoned: Vec<PathBuf>,
    /// New value for [`FLOX_AUTO_ACTIVATED_ENVIRONMENTS_VAR`].
    auto_activated: Vec<PathBuf>,
    /// New value for [`FLOX_SUPPRESSED_ENVIRONMENTS_VAR`].
    suppressed: Vec<PathBuf>,
}

/// Decide what the prompt hook should do given the shell's current location
/// and activation state.
///
/// The stack is rebuilt by tearing down a contiguous prefix from the front and
/// replaying the survivors in ancestor order. Each in-place activation's diff
/// embeds the previous value of `_FLOX_HOOK_DIFF`, so one run can emit a
/// deactivation script per layer and they restore state correctly when
/// evaluated in order; the replays are emitted afterwards and chain back on
/// top. Two operations use this:
///
/// - **Leaver teardown.** Layers whose directory the shell has left are torn
///   down. The prefix popped runs from the front to the deepest leaver; tracked
///   survivors caught above a leaver are replayed in ancestor order. A
///   non-poppable layer (manually activated, remote, or predating the hook) in
///   that prefix blocks the teardown, and the buried leavers are abandoned with
///   a warning.
/// - **Re-insertion.** An allowed environment the shell is inside but that is
///   stacked below tracked descendants (e.g. it was denied, the shell entered a
///   child, then it was re-allowed) is re-inserted in ancestor order: its
///   descendants are popped, it activates on top of its ancestor, and the
///   descendants replay on top. If a descendant above it cannot be popped, it
///   falls back to activating on top (out of ancestor order but still correct).
///
/// Only one such rebuild happens per run; the next prompt settles any further
/// reordering. While a rebuild's survivors are queued to replay, or while a
/// tracked layer the shell has left is still unwinding, newly discovered
/// environments are not activated yet — they would bury the unsettled layers.
///
/// Auto-activation is opt-in. A discovered environment activates only if the
/// user has allowed it (via the consent prompt or `flox activate allow`).
/// An environment that is neither allowed nor denied is "unregistered": in
/// `prompt` mode it is added to the plan's `prompt` list so the hook can ask
/// for consent; in `allowed` mode it is skipped silently. Denied environments
/// are never activated and never prompted.
///
/// Allow/deny govern future auto-activation only: an environment that is
/// already active (whether activated manually or before being denied) is left
/// running, and is auto-deactivated as usual once the shell leaves its
/// directory.
fn plan_auto_activation(ctx: &AutoActivateContext) -> AutoActivatePlan {
    let inside = |path: &Path| ctx.cwd.starts_with(path);
    let is_active = |path: &PathBuf| ctx.active.iter().flatten().any(|active| active == path);
    let is_allowed = |path: &PathBuf| ctx.allowed.contains(path);
    let is_denied = |path: &PathBuf| ctx.denied.contains(path);

    // Reconcile tracked state with the actual activation stack. A suppressed
    // environment stays suppressed only while the shell remains inside its
    // directory. A tracked auto-activation that is no longer active was
    // deactivated out-of-band (or failed to activate): suppress it while
    // still inside so it isn't immediately re-activated, otherwise forget it.
    // Filter to entries the shell is still inside, fully de-duplicating while
    // preserving order: this is treated as a set, and a corrupted or
    // hand-edited state variable could contain non-adjacent duplicates that
    // `Vec::dedup` (consecutive-only) would miss and keep re-emitting.
    let mut suppressed: Vec<PathBuf> = Vec::new();
    for path in &ctx.suppressed {
        if inside(path) && !suppressed.contains(path) {
            suppressed.push(path.clone());
        }
    }
    let mut auto_activated = Vec::new();
    for path in &ctx.auto_activated {
        if is_active(path) {
            if !auto_activated.contains(path) {
                auto_activated.push(path.clone());
            }
        } else if inside(path) && !suppressed.contains(path) {
            suppressed.push(path.clone());
        }
    }

    // A pending deactivation consumed this run targets the front of the
    // stack. Suppress it while the shell is still inside its directory so the
    // next prompt doesn't undo the deactivation the user just asked for, and
    // take no further action this run; the next prompt sees settled state.
    if ctx.pending_deactivations {
        if let Some(Some(front)) = ctx.active.first() {
            if inside(front) && !suppressed.contains(front) {
                suppressed.push(front.clone());
            }
            auto_activated.retain(|path| path != front);
        }
        return AutoActivatePlan {
            activate: Vec::new(),
            prompt: Vec::new(),
            prompt_sandbox: Vec::new(),
            libsandbox_notice: Vec::new(),
            deactivate: Vec::new(),
            reactivate: Vec::new(),
            reinsert: None,
            abandoned: Vec::new(),
            auto_activated,
            suppressed,
        };
    }

    // A layer can be popped only if it is a tracked auto-activation: manually
    // activated layers (active but not in the tracked set) and remote layers
    // (`None`, no local directory) cannot be torn down by the hook. Takes the
    // tracked set explicitly so it captures nothing and never conflicts with
    // the mutations to `auto_activated` below.
    let poppable = |layer: &Option<PathBuf>, tracked: &[PathBuf]| matches!(layer, Some(path) if tracked.contains(path));

    // Auto-deactivate environments whose directory the shell has left. Pop the
    // contiguous front prefix down to the deepest leaver: leavers are torn down
    // for good, while tracked layers above them are torn down only to rebuild
    // the stack without the leavers and are replayed afterwards (see the
    // function docs). A non-poppable layer (manual or remote) in that prefix
    // blocks the teardown: the buried leavers are abandoned with a warning.
    let mut deactivate = Vec::new();
    let mut reactivate = Vec::new();
    let mut abandoned = Vec::new();
    let leavers: Vec<PathBuf> = auto_activated
        .iter()
        .filter(|path| !inside(path))
        .cloned()
        .collect();
    if !leavers.is_empty() {
        // The prefix to tear down ends at the deepest leaver in the stack.
        let deepest_leaver = ctx
            .active
            .iter()
            .rposition(|layer| matches!(layer, Some(path) if leavers.contains(path)));
        if let Some(end) = deepest_leaver {
            let prefix = &ctx.active[..=end];
            if prefix.iter().all(|layer| poppable(layer, &auto_activated)) {
                // Every layer down to the deepest leaver can be popped. Tear
                // them down front-first; replay the survivors (non-leavers)
                // bottom-up so the stack settles in ancestor order.
                for layer in prefix.iter().flatten() {
                    deactivate.push(layer.clone());
                    if !leavers.contains(layer) {
                        reactivate.push(layer.clone());
                    }
                }
                reactivate.reverse();
                auto_activated.retain(|path| !leavers.contains(path));
            } else {
                // A non-poppable layer is buried among the leavers: drop the
                // leavers from tracking with a warning. They stay active and
                // can be unwound with 'flox deactivate'.
                abandoned = leavers;
                auto_activated.retain(|path| inside(path));
            }
        }
    }

    // Walk discovered environments outermost-first so the innermost ends up on
    // top. Auto-activation is opt-in: only allowed environments activate.
    // Unregistered ones (neither allowed nor denied) are queued for a consent
    // prompt in `prompt` mode, or skipped in `allowed` mode; denied ones are
    // always skipped. A registered preference is per-directory, so a denied or
    // unregistered directory in the middle of a stack does not block its
    // allowed ancestors or descendants.
    //
    // Defer all of this while survivors are queued to replay on top of a
    // teardown (re-insertion or buried-leaver teardown), or while a tracked
    // environment the shell has left is still unwinding: activating now would
    // bury layers that still need to settle. A plain pop-and-replace (leavers
    // torn down, no survivors to replay) does not defer — the replacement
    // activates on top in the same run, after the deactivations.
    let unwinding = !reactivate.is_empty() || auto_activated.iter().any(|path| !inside(path));
    let mut activate = Vec::new();
    let mut prompt = Vec::new();
    let mut prompt_sandbox: Vec<(PathBuf, SandboxBackend)> = Vec::new();
    let mut libsandbox_notice = Vec::new();
    let mut reinsert: Option<PathBuf> = None;

    // Look up the sandbox class for a discovered path by index in
    // `ctx.discovered`.  Returns `SandboxClass::None` when the index is
    // out-of-range (defensive: lengths always match in practice).
    let sandbox_class_for = |path: &Path| -> &SandboxClass {
        ctx.discovered
            .iter()
            .position(|p| p == path)
            .and_then(|i| ctx.discovered_sandbox.get(i))
            .unwrap_or(&SandboxClass::None)
    };

    // Whether `path` can be re-inserted in ancestor order this run, and the
    // descendants that would have to be popped to do so.
    enum Reinsertion {
        /// Every layer above the insertion floor is a poppable tracked
        /// descendant of `path` (front-first, always non-empty): re-insertion
        /// can run cleanly.
        Clean(Vec<PathBuf>),
        /// `path` has active descendants but re-insertion can't run cleanly this
        /// run — a descendant is manual/remote, or an unrelated layer is
        /// interleaved above the floor. `has_descendant` distinguishes the
        /// "defer because a planned teardown will clear the obstruction" case
        /// from the genuine on-top fallback.
        Blocked { has_descendant: bool },
        /// Nothing is stacked above the insertion point: a plain top activation.
        NoDescendant,
    }

    // Classify `path` for re-insertion: the active descendants stacked above
    // where it belongs, i.e. the layers that would have to be popped to insert
    // it in ancestor order.
    let descendants_above = |path: &Path, tracked: &[PathBuf]| -> Reinsertion {
        // Compute "has any descendant" with a full independent scan rather than
        // reusing the prefix walk below: a well-formed stack keeps descendants
        // in front of their ancestor, but the walk can break early on an
        // interleaved layer, and the defer decision must stay correct either way.
        let has_descendant = ctx
            .active
            .iter()
            .flatten()
            .any(|active| active.starts_with(path) && active != path);
        let mut pop = Vec::new();
        for layer in &ctx.active {
            match layer {
                Some(active) if active.starts_with(path) && active != path => {
                    if poppable(layer, tracked) {
                        pop.push(active.clone());
                    } else {
                        return Reinsertion::Blocked { has_descendant };
                    }
                },
                // The insertion floor (an ancestor of `path`): a clean
                // contiguous descendant prefix ends here.
                Some(ancestor) if path.starts_with(ancestor) => break,
                // Any other layer above the floor (an unrelated env or a
                // remote layer) interrupts the contiguous prefix: re-insertion
                // can't run cleanly this run.
                _ => return Reinsertion::Blocked { has_descendant },
            }
        }
        if pop.is_empty() {
            Reinsertion::NoDescendant
        } else {
            Reinsertion::Clean(pop)
        }
    };

    if !unwinding {
        for path in &ctx.discovered {
            if is_active(path)
                || suppressed.contains(path)
                || is_denied(path)
                || activate.contains(path)
                || prompt.contains(path)
                || prompt_sandbox.iter().any(|(p, _)| p == path)
            {
                continue;
            }

            // Classify the manifest sandbox declaration for this environment
            // and decide whether it requires session-replacement consent,
            // an advisory notice, or the plain in-place activation path.
            //
            // When the sandbox_activate flag is off, wrapping-backend
            // declarations are treated as absent — the manifest warns once on
            // a regular `flox activate`, and the auto-activation path follows
            // the same "warn once, treat as off" rule (ADR-002 item 5).
            let sandbox = sandbox_class_for(path);
            let sandbox = if !ctx.sandbox_activate_enabled {
                // Degrade: ignore manifest sandbox when the feature is off,
                // mirroring resolve_sandbox_mode's warn-and-downgrade path.
                &SandboxClass::None
            } else {
                sandbox
            };

            match sandbox {
                // Wrapping backends require an explicit session-entry
                // consent prompt, even when the directory is on the allow
                // list. A previous allow may predate the sandbox declaration,
                // and handing the terminal to a sandboxed session is a bigger
                // step than in-place env mutation.
                SandboxClass::Wrapping(backend) => {
                    prompt_sandbox.push((path.clone(), *backend));
                },
                // Libsandbox is advisory and in-place activation still
                // proceeds, but the user sees a notice explaining that
                // auto-activation does not mediate the advisory sandbox.
                SandboxClass::Libsandbox => {
                    libsandbox_notice.push(path.clone());
                    // Fall through to the normal allowed/prompt logic so the
                    // environment still activates in-place.
                    if is_allowed(path) {
                        match descendants_above(path, &auto_activated) {
                            Reinsertion::Clean(pop) if deactivate.is_empty() => {
                                reactivate = pop.iter().rev().cloned().collect();
                                deactivate = pop;
                                reinsert = Some(path.clone());
                            },
                            Reinsertion::Clean(_)
                            | Reinsertion::Blocked {
                                has_descendant: true,
                            } if !deactivate.is_empty() => {},
                            _ => {
                                activate.push(path.clone());
                                auto_activated.push(path.clone());
                            },
                        }
                    } else if ctx.prompt_unregistered {
                        prompt.push(path.clone());
                    }
                },
                // No sandbox declared: today's plain in-place flow.
                SandboxClass::None => {
                    // An allowed environment with tracked descendants stacked
                    // above it (e.g. it was denied, the shell entered a child,
                    // then it was re-allowed) must be re-inserted in ancestor
                    // order, not activated on top. Defer it unless it can be
                    // re-inserted this run: only one re-insertion happens per
                    // run (it reuses the deactivate/reactivate slots), and a
                    // teardown already planned this run takes precedence.
                    // A descendant that can't be popped (manual or remote)
                    // makes re-insertion impossible, so fall through and
                    // activate on top.
                    if is_allowed(path) {
                        match descendants_above(path, &auto_activated) {
                            // Clean re-insertion and no teardown already
                            // planned this run: pop the descendants, defer
                            // the target's activation to `reinsert`
                            // (committed by `handle()` only once the pops
                            // succeed), and replay the descendants bottom-up.
                            Reinsertion::Clean(pop) if deactivate.is_empty() => {
                                reactivate = pop.iter().rev().cloned().collect();
                                deactivate = pop;
                                reinsert = Some(path.clone());
                            },
                            // Has active descendants but re-insertion can't
                            // run cleanly this run, and a teardown is already
                            // planned: the teardown will clear the
                            // obstruction, so defer to the next prompt rather
                            // than stack `path` out of order. Do nothing.
                            Reinsertion::Clean(_)
                            | Reinsertion::Blocked {
                                has_descendant: true,
                            } if !deactivate.is_empty() => {},
                            // Re-insertion is impossible (a descendant is
                            // manual or remote) with no teardown to clear it,
                            // or there are no descendants at all: activating
                            // on top is out of order but still correct — fall
                            // back to that.
                            _ => {
                                activate.push(path.clone());
                                auto_activated.push(path.clone());
                            },
                        }
                    } else if ctx.prompt_unregistered {
                        // Unregistered: ask before activating. Not tracked as
                        // auto-activated yet — the hook adds it only if the
                        // user consents.
                        prompt.push(path.clone());
                    }
                },
            }
        }
    }

    AutoActivatePlan {
        activate,
        prompt,
        prompt_sandbox,
        libsandbox_notice,
        deactivate,
        reactivate,
        reinsert,
        abandoned,
        auto_activated,
        suppressed,
    }
}

/// Gather the runtime inputs for [`plan_auto_activation`]: the shell's
/// working directory, the environments discoverable from it, the activation
/// stack, the hook's tracked state variables, and the user's per-directory
/// auto-activation preferences.
fn gather_auto_activate_context(
    config: &Config,
    flox: &Flox,
    pending_deactivations: bool,
) -> Result<AutoActivateContext> {
    let cwd = std::env::current_dir().context("failed to read current directory")?;
    let discovered_dot_flox =
        find_all_dot_flox(&cwd).context("failed to discover environments for auto-activation")?;
    let discovered: Vec<PathBuf> = discovered_dot_flox
        .iter()
        .filter_map(|dot_flox| dot_flox.path.parent().map(Path::to_path_buf))
        .collect();

    // Read the manifest sandbox declaration for each discovered environment.
    // Errors (missing manifest, parse failure) fall back to `SandboxClass::None`
    // so a corrupt or unconventional environment does not break every prompt.
    let discovered_sandbox: Vec<SandboxClass> = discovered
        .iter()
        .map(|project_dir| read_sandbox_class(project_dir))
        .collect();

    // `find_all_dot_flox` canonicalized the same starting path, so reuse its
    // canonicalization rules for the containment checks.
    let cwd = cwd
        .canonicalize()
        .context("failed to canonicalize current directory")?;
    // Active environments are recorded into `_FLOX_ACTIVE_ENVIRONMENTS` from
    // the opened `ConcreteEnvironment`, whose `.flox` path is a `CanonicalPath`
    // (activation can't open an environment without canonicalizing it). So
    // these paths are already canonical and comparable to `discovered` and the
    // state variables without re-canonicalizing here.
    let active = activated_environments()
        .iter()
        .map(|env| env.path().and_then(Path::parent).map(Path::to_path_buf))
        .collect();
    // `flox activate allow`/`deny` key the config by the environment's parent
    // path, which they derive by popping `.flox` off a `CanonicalPath`. Those
    // keys are therefore already canonical and comparable to `discovered`
    // without re-canonicalizing here.
    let preference = |want: AutoActivationPreference| {
        config
            .flox
            .auto_activate_environments
            .iter()
            .filter(move |(_, pref)| **pref == want)
            .map(|(path, _)| path.clone())
            .collect::<Vec<_>>()
    };
    let allowed = preference(AutoActivationPreference::Allow);
    let denied = preference(AutoActivationPreference::Deny);
    let prompt_unregistered = matches!(
        config.flox.auto_activate.clone().unwrap_or_default(),
        AutoActivate::Prompt
    );
    Ok(AutoActivateContext {
        cwd,
        discovered,
        discovered_sandbox,
        active,
        auto_activated: read_path_list_var(FLOX_AUTO_ACTIVATED_ENVIRONMENTS_VAR),
        suppressed: read_path_list_var(FLOX_SUPPRESSED_ENVIRONMENTS_VAR),
        allowed,
        denied,
        prompt_unregistered,
        pending_deactivations,
        sandbox_activate_enabled: flox.features.sandbox_activate,
    })
}

/// Read a manifest from `<project_dir>/.flox/env/manifest.toml` and classify
/// the declared sandbox option as a [`SandboxClass`].
///
/// Only the manifest file is consulted — no lockfile migration — because
/// the hook has no lockfile at hand and the sandbox declaration lives in the
/// user-authored manifest rather than being generated. Falls back to
/// [`SandboxClass::None`] on any I/O or parse error so a broken manifest
/// does not make every prompt fail.
fn read_sandbox_class(project_dir: &Path) -> SandboxClass {
    let manifest_path = project_dir
        .join(".flox")
        .join(ENV_DIR_NAME)
        .join(MANIFEST_FILENAME);

    let contents = match std::fs::read_to_string(&manifest_path) {
        Ok(s) => s,
        Err(_) => return SandboxClass::None,
    };

    let manifest = match Manifest::parse_and_migrate(&contents, None) {
        Ok(m) => m,
        Err(_) => return SandboxClass::None,
    };

    let opts = &manifest.as_latest_schema().options;
    let mode = opts.sandbox;
    let backend = opts.sandbox_backend.unwrap_or_default();

    // Absent mode or explicit `off` → no sandbox.
    match mode {
        None | Some(SandboxMode::Off) => SandboxClass::None,
        Some(_) if backend.capabilities().enforces => SandboxClass::Wrapping(backend),
        Some(_) => SandboxClass::Libsandbox,
    }
}

/// Read a JSON array of paths from an environment variable, treating an
/// unset, empty, or unparseable value as empty so a corrupted state variable
/// can't fail every shell prompt.
fn read_path_list_var(var: &str) -> Vec<PathBuf> {
    let Ok(value) = std::env::var(var) else {
        return Vec::new();
    };
    if value.is_empty() {
        return Vec::new();
    }
    match serde_json::from_str(&value) {
        Ok(paths) => paths,
        Err(err) => {
            debug!(%err, var, "ignoring unparseable auto-activation state variable");
            Vec::new()
        },
    }
}

/// The user's answer to the auto-activation consent prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutoActivateConsent {
    /// Activate the environments and remember the choice (persist `Allow`).
    Allow,
    /// Don't activate and remember the refusal (persist `Deny`)
    Deny,
    /// Don't activate and suppress re-prompting for this shell session only.
    /// Leaving and re-entering the directory (or a new shell) asks again.
    Suppress,
    /// No response was possible: there was no tty to prompt on. Make no
    /// decision this run so a later interactive prompt can still ask.
    NoTerminal,
}

/// Ask the user, on the controlling terminal, whether to auto-activate the
/// unregistered environments discovered this run.
///
/// One prompt covers the whole batch: a single `cd` into a deep tree can
/// surface several environments at once, and asking per environment is tedious.
/// The answer applies to all of `project_dirs`.
///
/// When there is no terminal to prompt on, returns
/// [`AutoActivateConsent::NoResponse`] so the caller doesn't suppress
/// prompting.
async fn prompt_for_auto_activation(project_dirs: &[PathBuf]) -> Result<AutoActivateConsent> {
    // Instead of using `Dialog::can_prompt`, only check if stderr is a terminal.
    // We know stdout is not a terminal, which would cause `Dialog::can_prompt` to return false.
    if !std::io::stderr().is_terminal() {
        return Ok(AutoActivateConsent::NoTerminal);
    }

    let message = match project_dirs {
        [dir] => format!("Auto-activate the environment in '{}'?", dir.display()),
        dirs => {
            let mut message = format!("Auto-activate these {} environments?", dirs.len());
            for dir in dirs {
                message.push_str(&format!("\n  {}", dir.display()));
            }
            message
        },
    };

    let consent = Dialog {
        message: &message,
        help_message: None,
        typed: Confirm {
            default: Some(false),
        },
    }
    .prompt()
    .await;

    match consent {
        Ok(true) => Ok(AutoActivateConsent::Allow),
        Ok(false) => Ok(AutoActivateConsent::Deny),
        // Bailing out of the prompt (Esc or Ctrl-C) makes no lasting decision:
        // suppress for this shell session so the hook stops asking, but ask
        // again in a new shell or on re-entering the directory.
        Err(
            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted,
        ) => Ok(AutoActivateConsent::Suppress),
        Err(inquire::InquireError::NotTTY) => Ok(AutoActivateConsent::NoTerminal),
        Err(err) => Err(err).context("failed to prompt for auto-activation"),
    }
}

/// Emit a command that activates the environment in `project_dir` in place.
///
/// The emitted command is itself evaluated by the shell's prompt hook, and
/// runs `flox activate` with stdout captured — which selects in-place mode —
/// so this reuses the full activation path (hooks, services, attach
/// semantics) of `eval "$(flox activate)"`.
fn write_activate_command(shell: Shell, project_dir: &Path, writer: &mut impl Write) -> Result<()> {
    let flox_bin = std::env::current_exe().context("failed to determine flox executable path")?;
    let flox_bin = flox_bin.to_string_lossy().to_string();
    let escaped_bin = shell_escape::escape(Cow::Borrowed(&*flox_bin));
    let dir = project_dir.to_string_lossy().to_string();
    let escaped_dir = shell_escape::escape(Cow::Borrowed(&*dir));
    match shell {
        Shell::Bash | Shell::Zsh => {
            writeln!(
                writer,
                r#"eval "$({escaped_bin} activate --dir {escaped_dir})";"#
            )?;
        },
        Shell::Fish => {
            writeln!(
                writer,
                "{escaped_bin} activate --dir {escaped_dir} | source;"
            )?;
        },
        Shell::Tcsh => {
            writeln!(
                writer,
                r#"eval "`{escaped_bin} activate --dir {escaped_dir}`";"#
            )?;
        },
    }
    Ok(())
}

/// The user's answer to the sandbox session-entry consent prompt.
#[derive(Clone, Copy)]
enum SandboxConsent {
    /// Enter the sandboxed session (run it as a foreground child).
    Accept,
    /// Decline; suppress re-prompting for this shell session.
    Decline,
    /// No controlling terminal to prompt on.
    NoTerminal,
    /// Shell does not support starting the session from the prompt hook
    /// (fish, tcsh).
    UnsupportedShell,
}

/// Ask the user, on the controlling terminal, whether to enter an environment
/// as a sandboxed session run as a foreground child of the current shell.
///
/// This is a stronger action than plain auto-activation because it hands the
/// terminal to a new process tree confined by the sandbox backend for the
/// duration of the session. The consent prompt is therefore shown even when
/// the directory is already on the auto-activate allow list: a prior allow may
/// predate the sandbox declaration, and entering a session needs explicit
/// intent on every entry.
///
/// Returns [`SandboxConsent::NoTerminal`] when `/dev/tty` is unavailable
/// (non-interactive shell) and [`SandboxConsent::UnsupportedShell`] for fish
/// and tcsh, where starting an interactive session from the prompt-hook eval
/// context is not supported.
fn prompt_for_sandbox_activation(
    project_dir: &Path,
    backend: SandboxBackend,
    shell: Shell,
) -> Result<SandboxConsent> {
    // Starting an interactive sandboxed session from the fish and tcsh
    // prompt-hook eval contexts is not supported. Report the gap and fall
    // back to the non-tty notice path.
    if matches!(shell, Shell::Fish | Shell::Tcsh) {
        return Ok(SandboxConsent::UnsupportedShell);
    }

    let Ok(tty) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
    else {
        return Ok(SandboxConsent::NoTerminal);
    };

    let question = format!(
        "Enter '{}' (sandboxed via {backend})? [Y/n] ",
        project_dir.display(),
    );

    let mut tty_writer = &tty;
    tty_writer
        .write_all(question.as_bytes())
        .context("failed to write the sandbox consent prompt")?;
    tty_writer
        .flush()
        .context("failed to flush the sandbox consent prompt")?;

    let mut answer = String::new();
    BufReader::new(&tty)
        .read_line(&mut answer)
        .context("failed to read the sandbox consent response")?;
    let answer = answer.trim();

    // Default is yes (bare Enter confirms, only explicit "n"/"no" declines).
    if answer.is_empty() || answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes") {
        Ok(SandboxConsent::Accept)
    } else {
        Ok(SandboxConsent::Decline)
    }
}

/// Emit a command that runs a sandboxed activation of the environment in
/// `project_dir` as a foreground child of the current shell.
///
/// Unlike [`write_activate_command`] (which emits `eval "$(flox activate)"`
/// for in-place activation), this emits a plain `flox activate --dir <path>`
/// invocation: the sandboxed session takes over the terminal for its
/// duration, and when it exits the user returns to their original shell. The
/// environment is never activated unsandboxed in the host shell — once the
/// session ends the host shell has nothing activated, the same posture as
/// declining. The manifest supplies the sandbox mode and backend through
/// normal resolution.
///
/// Only bash and zsh are supported. Fish and tcsh callers should check
/// [`prompt_for_sandbox_activation`]'s `UnsupportedShell` result and emit a
/// notice instead.
fn write_sandbox_session_command(
    shell: Shell,
    project_dir: &Path,
    writer: &mut impl Write,
) -> Result<()> {
    let flox_bin = std::env::current_exe().context("failed to determine flox executable path")?;
    let flox_bin = flox_bin.to_string_lossy().to_string();
    let escaped_bin = shell_escape::escape(Cow::Borrowed(&*flox_bin));
    let dir = project_dir.to_string_lossy().to_string();
    let escaped_dir = shell_escape::escape(Cow::Borrowed(&*dir));
    match shell {
        Shell::Bash | Shell::Zsh => {
            writeln!(writer, "{escaped_bin} activate --dir {escaped_dir};")?;
        },
        // Fish and tcsh: callers should not reach this path; if they do,
        // fall back to the regular in-place command rather than silently
        // doing the wrong thing.
        Shell::Fish | Shell::Tcsh => {
            write_activate_command(shell, project_dir, writer)?;
        },
    }
    Ok(())
}

/// Mark every sandbox-consent candidate as suppressed for this shell session.
///
/// Applies to entered paths as well as declined ones: the sandboxed session
/// runs as a foreground child of the interactive shell, so the shell survives
/// the session and its prompt hook would immediately re-prompt for the same
/// directory without the suppression. The suppression is per-shell state;
/// leaving the directory clears it (see [`plan_auto_activation`]), so
/// re-entering the directory prompts again.
fn suppress_sandbox_candidates(
    candidates: &[(PathBuf, SandboxBackend)],
    suppressed: &mut Vec<PathBuf>,
) {
    for (path, _backend) in candidates {
        if !suppressed.contains(path) {
            suppressed.push(path.clone());
        }
    }
}

/// Emit a state-variable update when `new` differs from `old`: an export of
/// the JSON-encoded list, or an unset when the list becomes empty.
fn write_path_list_update(
    shell: Shell,
    var: &str,
    old: &[PathBuf],
    new: &[PathBuf],
    writer: &mut impl Write,
) -> Result<()> {
    if old == new {
        return Ok(());
    }
    if new.is_empty() {
        UnsetVar::new(var).generate_with_newline(shell, writer)?;
        return Ok(());
    }
    let value = serde_json::to_string(new)
        .with_context(|| format!("failed to serialize state variable {var}"))?;
    SetVar::exported_no_expansion(var, value).generate_with_newline(shell, writer)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A context with nothing discovered, nothing active, and no state.
    ///
    /// Defaults to `allowed` mode (`prompt_unregistered: false`); tests that
    /// exercise the consent prompt set it to true explicitly.
    fn empty_ctx(cwd: &str) -> AutoActivateContext {
        AutoActivateContext {
            cwd: PathBuf::from(cwd),
            discovered: Vec::new(),
            discovered_sandbox: Vec::new(),
            active: Vec::new(),
            auto_activated: Vec::new(),
            suppressed: Vec::new(),
            allowed: Vec::new(),
            denied: Vec::new(),
            prompt_unregistered: false,
            pending_deactivations: false,
            sandbox_activate_enabled: false,
        }
    }

    fn paths(values: &[&str]) -> Vec<PathBuf> {
        values.iter().map(PathBuf::from).collect()
    }

    fn noop_plan() -> AutoActivatePlan {
        AutoActivatePlan {
            activate: Vec::new(),
            prompt: Vec::new(),
            prompt_sandbox: Vec::new(),
            libsandbox_notice: Vec::new(),
            deactivate: Vec::new(),
            reactivate: Vec::new(),
            reinsert: None,
            abandoned: Vec::new(),
            auto_activated: Vec::new(),
            suppressed: Vec::new(),
        }
    }

    #[test]
    fn nothing_to_do_in_plain_directory() {
        let ctx = empty_ctx("/home/user/plain");
        assert_eq!(plan_auto_activation(&ctx), noop_plan());
    }

    #[test]
    fn activates_discovered_env() {
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/proj"]),
            allowed: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/proj")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/home/user/proj"]),
            auto_activated: paths(&["/home/user/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn activates_stack_outermost_first() {
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/outer", "/home/user/outer/inner"]),
            allowed: paths(&["/home/user/outer", "/home/user/outer/inner"]),
            ..empty_ctx("/home/user/outer/inner")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/home/user/outer", "/home/user/outer/inner"]),
            auto_activated: paths(&["/home/user/outer", "/home/user/outer/inner"]),
            ..noop_plan()
        });
    }

    #[test]
    fn does_not_reactivate_active_env() {
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/proj"]),
            active: vec![Some(PathBuf::from("/home/user/proj"))],
            auto_activated: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/proj/subdir")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            auto_activated: paths(&["/home/user/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn activates_missing_inner_env_on_top_of_active_outer() {
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/outer", "/home/user/outer/inner"]),
            active: vec![Some(PathBuf::from("/home/user/outer"))],
            auto_activated: paths(&["/home/user/outer"]),
            allowed: paths(&["/home/user/outer", "/home/user/outer/inner"]),
            ..empty_ctx("/home/user/outer/inner")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/home/user/outer/inner"]),
            auto_activated: paths(&["/home/user/outer", "/home/user/outer/inner"]),
            ..noop_plan()
        });
    }

    #[test]
    fn pops_front_env_after_leaving_its_directory() {
        let ctx = AutoActivateContext {
            active: vec![Some(PathBuf::from("/home/user/proj"))],
            auto_activated: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/elsewhere")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            deactivate: paths(&["/home/user/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn pops_all_leaver_layers_in_one_run() {
        let ctx = AutoActivateContext {
            // Front of stack is the innermost env.
            active: vec![
                Some(PathBuf::from("/home/user/outer/inner")),
                Some(PathBuf::from("/home/user/outer")),
            ],
            auto_activated: paths(&["/home/user/outer", "/home/user/outer/inner"]),
            ..empty_ctx("/home/user/elsewhere")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            // Front-of-stack (innermost) first, the whole stack in one run.
            deactivate: paths(&["/home/user/outer/inner", "/home/user/outer"]),
            ..noop_plan()
        });
    }

    #[test]
    fn pops_and_activates_in_one_run_when_switching_projects() {
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/proj_b"]),
            active: vec![Some(PathBuf::from("/home/user/proj_a"))],
            auto_activated: paths(&["/home/user/proj_a"]),
            allowed: paths(&["/home/user/proj_b"]),
            ..empty_ctx("/home/user/proj_b")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/home/user/proj_b"]),
            deactivate: paths(&["/home/user/proj_a"]),
            auto_activated: paths(&["/home/user/proj_b"]),
            ..noop_plan()
        });
    }

    #[test]
    fn pops_whole_stack_and_activates_replacement_in_one_run() {
        // Leaving a nested stack for a different project unwinds every
        // tracked layer and activates the replacement in the same run, so
        // the shell never shows a stale stack or buries an unwinding layer.
        let ctx = AutoActivateContext {
            discovered: paths(&["/tmp/z"]),
            active: vec![
                Some(PathBuf::from("/tmp/a/b")),
                Some(PathBuf::from("/tmp/a")),
            ],
            auto_activated: paths(&["/tmp/a", "/tmp/a/b"]),
            allowed: paths(&["/tmp/z"]),
            ..empty_ctx("/tmp/z")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/tmp/z"]),
            deactivate: paths(&["/tmp/a/b", "/tmp/a"]),
            auto_activated: paths(&["/tmp/z"]),
            ..noop_plan()
        });
    }

    #[test]
    fn abandons_buried_leavers_split_by_manual_activation() {
        // Leaving /tmp/a/b for /tmp/z with a manual activation between the two
        // tracked layers: the prefix down to the deepest leaver (/tmp/a)
        // contains a non-poppable manual layer, so neither leaver can be torn
        // down in ancestor order. Both are abandoned with a warning, and the
        // new environment activates in the same run (nothing is left unwinding).
        let ctx = AutoActivateContext {
            discovered: paths(&["/tmp/z"]),
            active: vec![
                Some(PathBuf::from("/tmp/a/b")),
                Some(PathBuf::from("/tmp/manual")),
                Some(PathBuf::from("/tmp/a")),
            ],
            auto_activated: paths(&["/tmp/a", "/tmp/a/b"]),
            allowed: paths(&["/tmp/z"]),
            ..empty_ctx("/tmp/z")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/tmp/z"]),
            abandoned: paths(&["/tmp/a", "/tmp/a/b"]),
            auto_activated: paths(&["/tmp/z"]),
            ..noop_plan()
        });
    }

    #[test]
    fn abandons_leaver_buried_under_manual_activation() {
        // A single tracked leaver buried under a manual (non-poppable) layer is
        // abandoned with a warning, and the new environment activates in the
        // same run.
        let ctx = AutoActivateContext {
            discovered: paths(&["/tmp/z"]),
            active: vec![
                Some(PathBuf::from("/tmp/manual")),
                Some(PathBuf::from("/tmp/a")),
            ],
            auto_activated: paths(&["/tmp/a"]),
            allowed: paths(&["/tmp/z"]),
            ..empty_ctx("/tmp/z")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/tmp/z"]),
            abandoned: paths(&["/tmp/a"]),
            auto_activated: paths(&["/tmp/z"]),
            ..noop_plan()
        });
    }

    #[test]
    fn settled_state_is_a_noop() {
        // A hook run with nothing to do (e.g. zsh fires the hook from both
        // chpwd and precmd, so a second run often sees the first run's plan
        // already applied) must plan no further changes.
        let ctx = AutoActivateContext {
            discovered: paths(&["/tmp/z"]),
            active: vec![Some(PathBuf::from("/tmp/z"))],
            auto_activated: paths(&["/tmp/z"]),
            ..empty_ctx("/tmp/z")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            auto_activated: paths(&["/tmp/z"]),
            ..noop_plan()
        });
    }

    #[test]
    fn abandons_env_buried_under_manual_activation() {
        let ctx = AutoActivateContext {
            // A manually activated env (not tracked) sits on top of the
            // auto-activated one the shell has left.
            active: vec![
                Some(PathBuf::from("/home/user/manual")),
                Some(PathBuf::from("/home/user/proj")),
            ],
            auto_activated: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/elsewhere")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            abandoned: paths(&["/home/user/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn abandons_env_buried_under_remote_activation() {
        let ctx = AutoActivateContext {
            // `None` is a remote env (no local directory) sitting on top of
            // the auto-activated one the shell has left.
            active: vec![None, Some(PathBuf::from("/home/user/proj"))],
            auto_activated: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/elsewhere")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            abandoned: paths(&["/home/user/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn tears_down_buried_leaver_under_poppable_layer() {
        // Leaving /tmp/a/b for /tmp/a/c: the inner /tmp/a/b is a leaver buried
        // under nothing, but /tmp/a (its parent, still inside) sits below it.
        // Here the buryer is a poppable tracked layer, so the whole prefix down
        // to the deepest leaver is torn down and the survivor /tmp/a replayed in
        // ancestor order rather than abandoned.
        let ctx = AutoActivateContext {
            // Front is the inner leaver; /tmp/x is a tracked survivor above it.
            active: vec![
                Some(PathBuf::from("/tmp/x")),
                Some(PathBuf::from("/tmp/a/b")),
            ],
            auto_activated: paths(&["/tmp/a/b", "/tmp/x"]),
            ..empty_ctx("/tmp/x")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            // Pop both front-first; replay only the survivor /tmp/x bottom-up.
            deactivate: paths(&["/tmp/x", "/tmp/a/b"]),
            reactivate: paths(&["/tmp/x"]),
            auto_activated: paths(&["/tmp/x"]),
            ..noop_plan()
        });
    }

    #[test]
    fn reallows_mid_stack_env_reinserts_in_ancestor_order() {
        // The repro: /a/b/c/d/e with c denied, then re-allowed while the shell
        // is in e. c was never activated (front-first active is [e, d, b, a]).
        // Re-allowing c re-inserts it in ancestor order: pop e and d, then the
        // target c goes to `reinsert` (committed by handle() after the pops,
        // above b), then replay d and e. c is not yet in `activate` or
        // `auto_activated` — handle() records it only once the pops succeed.
        let ctx = AutoActivateContext {
            discovered: paths(&["/a", "/a/b", "/a/b/c", "/a/b/c/d", "/a/b/c/d/e"]),
            active: vec![
                Some(PathBuf::from("/a/b/c/d/e")),
                Some(PathBuf::from("/a/b/c/d")),
                Some(PathBuf::from("/a/b")),
                Some(PathBuf::from("/a")),
            ],
            auto_activated: paths(&["/a", "/a/b", "/a/b/c/d", "/a/b/c/d/e"]),
            allowed: paths(&["/a", "/a/b", "/a/b/c", "/a/b/c/d", "/a/b/c/d/e"]),
            ..empty_ctx("/a/b/c/d/e")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            reinsert: Some(PathBuf::from("/a/b/c")),
            deactivate: paths(&["/a/b/c/d/e", "/a/b/c/d"]),
            reactivate: paths(&["/a/b/c/d", "/a/b/c/d/e"]),
            auto_activated: paths(&["/a", "/a/b", "/a/b/c/d", "/a/b/c/d/e"]),
            ..noop_plan()
        });
    }

    #[test]
    fn reallows_mid_stack_env_with_single_descendant_above() {
        // Minimal re-insertion: target /a/b with one tracked descendant /a/b/c
        // above it. Pop /a/b/c, defer /a/b to `reinsert`, replay /a/b/c. The
        // target is not in `activate`/`auto_activated`; handle() records it once
        // the pop succeeds.
        let ctx = AutoActivateContext {
            discovered: paths(&["/a", "/a/b", "/a/b/c"]),
            active: vec![Some(PathBuf::from("/a/b/c")), Some(PathBuf::from("/a"))],
            auto_activated: paths(&["/a", "/a/b/c"]),
            allowed: paths(&["/a", "/a/b", "/a/b/c"]),
            ..empty_ctx("/a/b/c")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            reinsert: Some(PathBuf::from("/a/b")),
            deactivate: paths(&["/a/b/c"]),
            reactivate: paths(&["/a/b/c"]),
            auto_activated: paths(&["/a", "/a/b/c"]),
            ..noop_plan()
        });
    }

    #[test]
    fn reinsertion_aborts_when_manual_layer_above_target() {
        // A descendant of the target above it is manually activated (not
        // tracked), so the stack can't be rebuilt in ancestor order. The target
        // activates on top instead (no deactivate/reactivate).
        let ctx = AutoActivateContext {
            discovered: paths(&["/a", "/a/b", "/a/b/c"]),
            active: vec![
                // /a/b/c is active but not auto-activated => manual.
                Some(PathBuf::from("/a/b/c")),
                Some(PathBuf::from("/a")),
            ],
            auto_activated: paths(&["/a"]),
            allowed: paths(&["/a", "/a/b", "/a/b/c"]),
            ..empty_ctx("/a/b/c")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/a/b"]),
            auto_activated: paths(&["/a", "/a/b"]),
            ..noop_plan()
        });
    }

    #[test]
    fn reinsertion_aborts_when_remote_layer_above_target() {
        // A remote env (`None`, no local dir) sits above the target, so the
        // stack can't be rebuilt in ancestor order. The target activates on top.
        let ctx = AutoActivateContext {
            discovered: paths(&["/a", "/a/b"]),
            active: vec![None, Some(PathBuf::from("/a"))],
            auto_activated: paths(&["/a"]),
            allowed: paths(&["/a", "/a/b"]),
            ..empty_ctx("/a/b")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/a/b"]),
            auto_activated: paths(&["/a", "/a/b"]),
            ..noop_plan()
        });
    }

    #[test]
    fn reinsertion_skipped_when_target_suppressed() {
        // A suppressed target is not re-inserted even after being allowed: a
        // per-shell decline stands until the shell leaves and re-enters.
        let ctx = AutoActivateContext {
            discovered: paths(&["/a", "/a/b", "/a/b/c"]),
            active: vec![Some(PathBuf::from("/a/b/c")), Some(PathBuf::from("/a"))],
            auto_activated: paths(&["/a", "/a/b/c"]),
            allowed: paths(&["/a", "/a/b", "/a/b/c"]),
            suppressed: paths(&["/a/b"]),
            ..empty_ctx("/a/b/c")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            auto_activated: paths(&["/a", "/a/b/c"]),
            suppressed: paths(&["/a/b"]),
            ..noop_plan()
        });
    }

    #[test]
    fn reinsertion_does_not_fire_when_no_descendant_above() {
        // The target is allowed and inside but nothing is stacked above where it
        // belongs, so it activates on top as usual (no reorder needed).
        let ctx = AutoActivateContext {
            discovered: paths(&["/a", "/a/b"]),
            active: vec![Some(PathBuf::from("/a"))],
            auto_activated: paths(&["/a"]),
            allowed: paths(&["/a", "/a/b"]),
            ..empty_ctx("/a/b")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/a/b"]),
            auto_activated: paths(&["/a", "/a/b"]),
            ..noop_plan()
        });
    }

    #[test]
    fn settled_after_reinsertion_is_a_noop() {
        // After the repro's rebuild is applied by the shell, the stack is
        // [e, d, c, b, a] and c is active and allowed: the next prompt plans
        // nothing further.
        let ctx = AutoActivateContext {
            discovered: paths(&["/a", "/a/b", "/a/b/c", "/a/b/c/d", "/a/b/c/d/e"]),
            active: vec![
                Some(PathBuf::from("/a/b/c/d/e")),
                Some(PathBuf::from("/a/b/c/d")),
                Some(PathBuf::from("/a/b/c")),
                Some(PathBuf::from("/a/b")),
                Some(PathBuf::from("/a")),
            ],
            auto_activated: paths(&["/a", "/a/b", "/a/b/c", "/a/b/c/d", "/a/b/c/d/e"]),
            allowed: paths(&["/a", "/a/b", "/a/b/c", "/a/b/c/d", "/a/b/c/d/e"]),
            ..empty_ctx("/a/b/c/d/e")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            auto_activated: paths(&["/a", "/a/b", "/a/b/c", "/a/b/c/d", "/a/b/c/d/e"]),
            ..noop_plan()
        });
    }

    #[test]
    fn reinsertion_not_combined_with_leaver_teardown() {
        // A front leaver and a re-insertable target both present: only the
        // leaver teardown fires this run. The next prompt settles the
        // re-insertion. /tmp/gone is a leaver (shell left it); /a/b is allowed,
        // inside, with /a/b/c stacked above it.
        let ctx = AutoActivateContext {
            discovered: paths(&["/a", "/a/b", "/a/b/c"]),
            active: vec![
                Some(PathBuf::from("/tmp/gone")),
                Some(PathBuf::from("/a/b/c")),
                Some(PathBuf::from("/a")),
            ],
            auto_activated: paths(&["/a", "/a/b/c", "/tmp/gone"]),
            allowed: paths(&["/a", "/a/b", "/a/b/c"]),
            ..empty_ctx("/a/b/c")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            deactivate: paths(&["/tmp/gone"]),
            auto_activated: paths(&["/a", "/a/b/c"]),
            ..noop_plan()
        });
    }

    #[test]
    fn pending_deactivation_suppresses_front_and_skips_activation() {
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/proj"]),
            active: vec![Some(PathBuf::from("/home/user/proj"))],
            auto_activated: paths(&["/home/user/proj"]),
            pending_deactivations: true,
            ..empty_ctx("/home/user/proj")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            suppressed: paths(&["/home/user/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn pending_deactivation_outside_directory_does_not_suppress() {
        let ctx = AutoActivateContext {
            active: vec![Some(PathBuf::from("/home/user/proj"))],
            auto_activated: paths(&["/home/user/proj"]),
            pending_deactivations: true,
            ..empty_ctx("/home/user/elsewhere")
        };
        assert_eq!(plan_auto_activation(&ctx), noop_plan());
    }

    #[test]
    fn pending_deactivation_of_manual_activation_suppresses_reactivation() {
        // The user manually activated the discovered env, then ran
        // 'flox deactivate' while still inside the project directory. The
        // next prompt must not auto-activate it again.
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/proj"]),
            active: vec![Some(PathBuf::from("/home/user/proj"))],
            pending_deactivations: true,
            ..empty_ctx("/home/user/proj")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            suppressed: paths(&["/home/user/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn suppressed_env_is_not_reactivated_while_inside() {
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/proj"]),
            suppressed: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/proj")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            suppressed: paths(&["/home/user/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn suppression_is_cleared_after_leaving_the_directory() {
        let ctx = AutoActivateContext {
            suppressed: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/elsewhere")
        };
        assert_eq!(plan_auto_activation(&ctx), noop_plan());
    }

    #[test]
    fn tracked_env_that_is_no_longer_active_is_suppressed_while_inside() {
        // The activation failed (or was deactivated out-of-band): tracked as
        // auto-activated but absent from the stack. Suppress instead of
        // retrying on every prompt.
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/proj"]),
            auto_activated: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/proj")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            suppressed: paths(&["/home/user/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn tracked_env_that_is_no_longer_active_is_forgotten_after_leaving() {
        let ctx = AutoActivateContext {
            auto_activated: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/elsewhere")
        };
        assert_eq!(plan_auto_activation(&ctx), noop_plan());
    }

    #[test]
    fn reentry_after_suppression_cleared_reactivates() {
        // The suppression entry was pruned on a previous prompt (outside the
        // directory); coming back in looks like a fresh discovery.
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/proj"]),
            allowed: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/proj")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/home/user/proj"]),
            auto_activated: paths(&["/home/user/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn does_not_activate_denied_env() {
        // The user denied auto-activation for this directory, so discovering
        // it must not activate it.
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/proj"]),
            denied: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/proj")
        };
        assert_eq!(plan_auto_activation(&ctx), noop_plan());
    }

    #[test]
    fn denied_inner_env_does_not_block_allowed_outer() {
        // Denying the innermost environment leaves its allowed ancestor free
        // to auto-activate.
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/outer", "/home/user/outer/inner"]),
            allowed: paths(&["/home/user/outer"]),
            denied: paths(&["/home/user/outer/inner"]),
            ..empty_ctx("/home/user/outer/inner")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/home/user/outer"]),
            auto_activated: paths(&["/home/user/outer"]),
            ..noop_plan()
        });
    }

    #[test]
    fn denied_outer_env_does_not_block_allowed_inner() {
        // Denying an ancestor leaves an allowed descendant free to
        // auto-activate; the deny applies per-directory, not to the subtree.
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/outer", "/home/user/outer/inner"]),
            allowed: paths(&["/home/user/outer/inner"]),
            denied: paths(&["/home/user/outer"]),
            ..empty_ctx("/home/user/outer/inner")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/home/user/outer/inner"]),
            auto_activated: paths(&["/home/user/outer/inner"]),
            ..noop_plan()
        });
    }

    #[test]
    fn deny_while_inside_does_not_deactivate_active_env() {
        // Denying an environment that was already auto-activated does not tear
        // it down; deny only governs future auto-activation. It stays tracked
        // and active until the shell leaves its directory.
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/proj"]),
            active: vec![Some(PathBuf::from("/home/user/proj"))],
            auto_activated: paths(&["/home/user/proj"]),
            denied: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/proj")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            auto_activated: paths(&["/home/user/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn denied_env_still_pops_after_leaving() {
        // An environment denied while it was active is auto-deactivated as
        // usual once the shell leaves its directory.
        let ctx = AutoActivateContext {
            active: vec![Some(PathBuf::from("/home/user/proj"))],
            auto_activated: paths(&["/home/user/proj"]),
            denied: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/elsewhere")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            deactivate: paths(&["/home/user/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn manually_activated_env_is_not_popped_on_leave() {
        // Active but never tracked as auto-activated: leaving its directory
        // must not deactivate it.
        let ctx = AutoActivateContext {
            active: vec![Some(PathBuf::from("/home/user/proj"))],
            ..empty_ctx("/home/user/elsewhere")
        };
        assert_eq!(plan_auto_activation(&ctx), noop_plan());
    }

    #[test]
    fn unregistered_env_is_prompted_in_prompt_mode() {
        // Auto-activation is opt-in: an environment with no recorded preference
        // is queued for a consent prompt rather than activated, and is not
        // tracked until the user consents.
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/proj"]),
            prompt_unregistered: true,
            ..empty_ctx("/home/user/proj")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            prompt: paths(&["/home/user/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn unregistered_env_is_skipped_in_allowed_mode() {
        // In `allowed` mode an unregistered environment is skipped silently —
        // no activation, no prompt.
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/proj"]),
            prompt_unregistered: false,
            ..empty_ctx("/home/user/proj")
        };
        assert_eq!(plan_auto_activation(&ctx), noop_plan());
    }

    #[test]
    fn denied_env_is_never_prompted() {
        // A denied environment is skipped even in prompt mode.
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/proj"]),
            denied: paths(&["/home/user/proj"]),
            prompt_unregistered: true,
            ..empty_ctx("/home/user/proj")
        };
        assert_eq!(plan_auto_activation(&ctx), noop_plan());
    }

    #[test]
    fn allowed_activates_while_unregistered_prompts_in_one_stack() {
        // In a stack, an allowed ancestor activates directly while an
        // unregistered descendant is queued for a prompt — both in
        // outermost-first order.
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/outer", "/home/user/outer/inner"]),
            allowed: paths(&["/home/user/outer"]),
            prompt_unregistered: true,
            ..empty_ctx("/home/user/outer/inner")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/home/user/outer"]),
            prompt: paths(&["/home/user/outer/inner"]),
            auto_activated: paths(&["/home/user/outer"]),
            ..noop_plan()
        });
    }

    #[test]
    fn sibling_directory_is_not_inside() {
        // Path containment must compare whole components: /home/user/proj2
        // is not inside /home/user/proj.
        let ctx = AutoActivateContext {
            active: vec![Some(PathBuf::from("/home/user/proj"))],
            auto_activated: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/proj2")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            deactivate: paths(&["/home/user/proj"]),
            ..noop_plan()
        });
    }

    // ── Sandbox decision table ────────────────────────────────────────────────
    //
    // Rows: sandbox option × backend class × tty × allow-state × flags → plan
    //
    // The planner never reads tty state (tty drives HookEnv::handle), but the
    // tests below cover the plan outputs that tty-dependent code relies on.

    fn discovered_with_sandbox(cwd: &str, path: &str, class: SandboxClass) -> AutoActivateContext {
        AutoActivateContext {
            discovered: paths(&[path]),
            discovered_sandbox: vec![class],
            ..empty_ctx(cwd)
        }
    }

    #[test]
    fn no_sandbox_allowed_env_activates_in_place() {
        // When `options.sandbox` is absent the planner takes the normal
        // in-place activation path.
        let ctx = AutoActivateContext {
            allowed: paths(&["/tmp/proj"]),
            ..discovered_with_sandbox("/tmp/proj", "/tmp/proj", SandboxClass::None)
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/tmp/proj"]),
            auto_activated: paths(&["/tmp/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn wrapping_backend_allowed_env_goes_to_prompt_sandbox() {
        // An allowed environment with a wrapping backend is never in-place
        // activated; it is queued in `prompt_sandbox` for session-entry
        // consent even though the directory is already on the allow list.
        let ctx = AutoActivateContext {
            allowed: paths(&["/tmp/proj"]),
            sandbox_activate_enabled: true,
            ..discovered_with_sandbox(
                "/tmp/proj",
                "/tmp/proj",
                SandboxClass::Wrapping(SandboxBackend::Oci),
            )
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            prompt_sandbox: vec![(PathBuf::from("/tmp/proj"), SandboxBackend::Oci)],
            ..noop_plan()
        });
    }

    #[test]
    fn wrapping_backend_unregistered_env_goes_to_prompt_sandbox_in_prompt_mode() {
        // An unregistered environment with a wrapping backend is queued for
        // sandbox consent regardless of the plain `prompt_unregistered` mode.
        let ctx = AutoActivateContext {
            prompt_unregistered: true,
            sandbox_activate_enabled: true,
            ..discovered_with_sandbox(
                "/tmp/proj",
                "/tmp/proj",
                SandboxClass::Wrapping(SandboxBackend::HostNative),
            )
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            prompt_sandbox: vec![(PathBuf::from("/tmp/proj"), SandboxBackend::HostNative)],
            ..noop_plan()
        });
    }

    #[test]
    fn wrapping_backend_flag_off_degrades_to_none() {
        // When the sandbox_activate feature flag is off, a manifest-declared
        // wrapping backend is treated as absent — the env falls through to
        // the normal allow/prompt flow.
        let ctx = AutoActivateContext {
            allowed: paths(&["/tmp/proj"]),
            sandbox_activate_enabled: false, // flag off
            ..discovered_with_sandbox(
                "/tmp/proj",
                "/tmp/proj",
                SandboxClass::Wrapping(SandboxBackend::Oci),
            )
        };
        // With the flag off the env activates in-place as if no sandbox.
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/tmp/proj"]),
            auto_activated: paths(&["/tmp/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn libsandbox_env_activates_in_place_with_notice() {
        // A libsandbox-declared env activates in-place (allowed path) and
        // is added to `libsandbox_notice` so the hook emits an info line.
        let ctx = AutoActivateContext {
            allowed: paths(&["/tmp/proj"]),
            sandbox_activate_enabled: true,
            ..discovered_with_sandbox("/tmp/proj", "/tmp/proj", SandboxClass::Libsandbox)
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/tmp/proj"]),
            auto_activated: paths(&["/tmp/proj"]),
            libsandbox_notice: paths(&["/tmp/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn libsandbox_env_prompts_when_unregistered() {
        // An unregistered libsandbox env in prompt mode is queued for the
        // plain consent prompt (not for sandbox consent) and listed in
        // `libsandbox_notice`.
        let ctx = AutoActivateContext {
            prompt_unregistered: true,
            sandbox_activate_enabled: true,
            ..discovered_with_sandbox("/tmp/proj", "/tmp/proj", SandboxClass::Libsandbox)
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            prompt: paths(&["/tmp/proj"]),
            libsandbox_notice: paths(&["/tmp/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn wrapping_backend_denied_env_is_skipped() {
        // A denied environment with a wrapping backend is never queued for
        // sandbox consent — the deny takes precedence.
        let ctx = AutoActivateContext {
            denied: paths(&["/tmp/proj"]),
            sandbox_activate_enabled: true,
            ..discovered_with_sandbox(
                "/tmp/proj",
                "/tmp/proj",
                SandboxClass::Wrapping(SandboxBackend::Oci),
            )
        };
        assert_eq!(plan_auto_activation(&ctx), noop_plan());
    }

    #[test]
    fn wrapping_backend_already_active_is_skipped() {
        // An already-active environment with a wrapping backend is not
        // re-queued; `is_active` takes precedence.
        let ctx = AutoActivateContext {
            active: vec![Some(PathBuf::from("/tmp/proj"))],
            auto_activated: paths(&["/tmp/proj"]),
            allowed: paths(&["/tmp/proj"]),
            sandbox_activate_enabled: true,
            ..discovered_with_sandbox(
                "/tmp/proj",
                "/tmp/proj",
                SandboxClass::Wrapping(SandboxBackend::Oci),
            )
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            auto_activated: paths(&["/tmp/proj"]),
            ..noop_plan()
        });
    }

    #[test]
    fn sandbox_session_bash_runs_foreground_child() {
        let mut buf = Vec::new();
        write_sandbox_session_command(Shell::Bash, Path::new("/tmp/proj"), &mut buf).unwrap();
        let script = String::from_utf8(buf).unwrap();
        assert!(
            !script.contains("exec ") && script.contains("activate --dir /tmp/proj;"),
            "expected foreground child form, got: {script}"
        );
    }

    #[test]
    fn sandbox_session_zsh_runs_foreground_child() {
        let mut buf = Vec::new();
        write_sandbox_session_command(Shell::Zsh, Path::new("/tmp/proj"), &mut buf).unwrap();
        let script = String::from_utf8(buf).unwrap();
        assert!(
            !script.contains("exec ") && script.contains("activate --dir /tmp/proj;"),
            "expected foreground child form, got: {script}"
        );
    }

    #[test]
    fn sandbox_candidates_are_suppressed_including_entered() {
        // The sandboxed session runs as a foreground child, so the entered
        // path must land in the suppressed list alongside declined ones —
        // otherwise the surviving shell re-prompts at the next prompt draw.
        let candidates = vec![
            (PathBuf::from("/tmp/entered"), SandboxBackend::Oci),
            (PathBuf::from("/tmp/declined"), SandboxBackend::Oci),
        ];
        let mut suppressed = paths(&["/tmp/declined"]);
        suppress_sandbox_candidates(&candidates, &mut suppressed);
        assert_eq!(suppressed, paths(&["/tmp/declined", "/tmp/entered"]));
    }

    #[test]
    fn activate_command_is_shell_escaped() {
        // A smoke test that the directory is passed through `shell_escape` and
        // the `eval "$(...)"` wrapping stays intact; a space is the canonical
        // needs-quoting case. The full range of shell-significant characters
        // (`&`, `;`, `$`, quotes, globs, ...) is exercised by the
        // `shell-escape` crate's own tests, not re-tested here.
        let mut buf = Vec::new();
        write_activate_command(Shell::Bash, Path::new("/home/user/my proj"), &mut buf).unwrap();
        let script = String::from_utf8(buf).unwrap();
        assert!(
            script.contains("activate --dir '/home/user/my proj'"),
            "{script}"
        );
        assert!(script.starts_with(r#"eval "$("#), "{script}");
    }

    #[test]
    fn state_var_update_emitted_only_on_change() {
        let unchanged = paths(&["/home/user/proj"]);
        let mut buf = Vec::new();
        write_path_list_update(
            Shell::Bash,
            FLOX_AUTO_ACTIVATED_ENVIRONMENTS_VAR,
            &unchanged,
            &unchanged,
            &mut buf,
        )
        .unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), "");

        let mut buf = Vec::new();
        write_path_list_update(
            Shell::Bash,
            FLOX_AUTO_ACTIVATED_ENVIRONMENTS_VAR,
            &[],
            &unchanged,
            &mut buf,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "export _FLOX_AUTO_ACTIVATED_ENVIRONMENTS='[\"/home/user/proj\"]';\n"
        );

        let mut buf = Vec::new();
        write_path_list_update(
            Shell::Bash,
            FLOX_AUTO_ACTIVATED_ENVIRONMENTS_VAR,
            &unchanged,
            &[],
            &mut buf,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "unset _FLOX_AUTO_ACTIVATED_ENVIRONMENTS;\n"
        );
    }
}
