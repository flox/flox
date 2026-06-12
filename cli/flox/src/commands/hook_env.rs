use std::borrow::Cow;
use std::io::{BufWriter, Write, stdout};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_core::activate::context::InvocationKind;
use flox_core::activate::vars::{
    FLOX_AUTO_ACTIVATED_ENVIRONMENTS_VAR,
    FLOX_SUPPRESSED_ENVIRONMENTS_VAR,
};
use flox_core::hook_actions::{HookAction, take_hook_actions};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::find_all_dot_flox;
use shell_gen::{GenerateShell, SetVar, Shell, ShellWithPath, UnsetVar};
use tracing::debug;

use super::activated_environments;
use super::deactivate::{
    emit_deactivate_script,
    flox_activate_tracelevel,
    open_deactivation_target,
};
use crate::subcommand_metric;
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
    pub fn handle(self, flox: Flox) -> Result<()> {
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

        let ctx = gather_auto_activate_context(!actions.is_empty())?;
        let plan = plan_auto_activation(&ctx);

        // Only record a metric when this run actually does something;
        // `hook-env` runs on every shell prompt, and recording the common
        // nothing-to-do case would be noise.
        if !actions.is_empty()
            || plan.deactivate_front
            || !plan.activate.is_empty()
            || !plan.abandoned.is_empty()
        {
            subcommand_metric!("hook-env");
        }

        if plan.deactivate_front {
            let active = activated_environments()
                .last_active_full()
                .context("no active environment to auto-deactivate")?;
            let target = open_deactivation_target(&flox, active)?;
            // Auto-activations are always in-place, so the matching
            // deactivation is too.
            emit_deactivate_script(
                ShellWithPath::from(self.shell),
                InvocationKind::InPlace,
                &target.activation_state_dir,
                &target.flox_env,
                flox_activate_tracelevel(),
                &mut writer,
            )?;
        }

        for path in &plan.abandoned {
            message::warning(format!(
                "Did not auto-deactivate the environment in '{}' because other environments are layered on top.\nRun 'flox deactivate' to deactivate them in order.",
                path.display()
            ));
        }

        for path in &plan.activate {
            write_activate_command(self.shell, path, &mut writer)?;
        }

        write_path_list_update(
            self.shell,
            FLOX_AUTO_ACTIVATED_ENVIRONMENTS_VAR,
            &ctx.auto_activated,
            &plan.auto_activated,
            &mut writer,
        )?;
        write_path_list_update(
            self.shell,
            FLOX_SUPPRESSED_ENVIRONMENTS_VAR,
            &ctx.suppressed,
            &plan.suppressed,
            &mut writer,
        )?;

        writer.flush()?;
        Ok(())
    }
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
    /// Project directories of active environments, most recently activated
    /// first. `None` for environments without a local directory (remote).
    active: Vec<Option<PathBuf>>,
    /// Project directories auto-activated by the hook in this shell.
    auto_activated: Vec<PathBuf>,
    /// Project directories suppressed from auto-activation in this shell.
    suppressed: Vec<PathBuf>,
    /// Whether this run consumed pending prompt-hook deactivation actions.
    pending_deactivations: bool,
}

/// What the prompt hook should do this run, plus the new values of the
/// auto-activation state variables.
#[derive(Clone, Debug, PartialEq)]
struct AutoActivatePlan {
    /// Project directories to activate, outermost-first.
    activate: Vec<PathBuf>,
    /// Deactivate the front-of-stack environment.
    deactivate_front: bool,
    /// Auto-activated environments that should be deactivated but are buried
    /// under other activations. Tearing down the middle of the stack is
    /// deferred (DEV-85), so these are dropped from tracking with a warning.
    abandoned: Vec<PathBuf>,
    /// New value for [`FLOX_AUTO_ACTIVATED_ENVIRONMENTS_VAR`].
    auto_activated: Vec<PathBuf>,
    /// New value for [`FLOX_SUPPRESSED_ENVIRONMENTS_VAR`].
    suppressed: Vec<PathBuf>,
}

/// Decide what the prompt hook should do given the shell's current location
/// and activation state.
///
/// At most one environment is deactivated per run: the generated deactivation
/// script restores the next layer's `_FLOX_HOOK_DIFF` only once the shell has
/// applied it, so a deeper stack unwinds across consecutive prompts rather
/// than in a single run. While such an unwind is in progress, newly
/// discovered environments are not activated yet — they would bury the
/// still-unwinding layers and force them to be abandoned.
fn plan_auto_activation(ctx: &AutoActivateContext) -> AutoActivatePlan {
    let inside = |path: &Path| ctx.cwd.starts_with(path);
    let is_active = |path: &PathBuf| ctx.active.iter().flatten().any(|active| active == path);

    // Reconcile tracked state with the actual activation stack. A suppressed
    // environment stays suppressed only while the shell remains inside its
    // directory. A tracked auto-activation that is no longer active was
    // deactivated out-of-band (or failed to activate): suppress it while
    // still inside so it isn't immediately re-activated, otherwise forget it.
    let mut suppressed: Vec<PathBuf> = ctx
        .suppressed
        .iter()
        .filter(|path| inside(path))
        .cloned()
        .collect();
    suppressed.dedup();
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
            deactivate_front: false,
            abandoned: Vec::new(),
            auto_activated,
            suppressed,
        };
    }

    // Auto-deactivate environments whose directory the shell has left,
    // popping at most one layer per run (see the function docs).
    let mut deactivate_front = false;
    let mut abandoned = Vec::new();
    let leavers: Vec<PathBuf> = auto_activated
        .iter()
        .filter(|path| !inside(path))
        .cloned()
        .collect();
    if !leavers.is_empty() {
        match ctx.active.first() {
            Some(Some(front)) if leavers.contains(front) => {
                deactivate_front = true;
                auto_activated.retain(|path| path != front);
                // Any remaining leavers unwind on subsequent prompts.
            },
            _ => {
                abandoned = leavers;
                auto_activated.retain(|path| inside(path));
            },
        }
    }

    // Activate discovered environments that aren't already active or
    // suppressed, outermost-first so the innermost ends up on top. Defer
    // while tracked environments the shell has left are still unwinding:
    // activating now would stack the new environment on top of them, and a
    // buried environment can't be popped (it would be abandoned instead).
    let unwinding = auto_activated.iter().any(|path| !inside(path));
    let mut activate = Vec::new();
    if !unwinding {
        for path in &ctx.discovered {
            if is_active(path) || suppressed.contains(path) || activate.contains(path) {
                continue;
            }
            activate.push(path.clone());
            auto_activated.push(path.clone());
        }
    }

    AutoActivatePlan {
        activate,
        deactivate_front,
        abandoned,
        auto_activated,
        suppressed,
    }
}

/// Gather the runtime inputs for [`plan_auto_activation`]: the shell's
/// working directory, the environments discoverable from it, the activation
/// stack, and the hook's tracked state variables.
fn gather_auto_activate_context(pending_deactivations: bool) -> Result<AutoActivateContext> {
    let cwd = std::env::current_dir().context("failed to read current directory")?;
    let discovered = find_all_dot_flox(&cwd)
        .context("failed to discover environments for auto-activation")?
        .into_iter()
        .filter_map(|dot_flox| dot_flox.path.parent().map(Path::to_path_buf))
        .collect();
    // `find_all_dot_flox` canonicalized the same starting path, so reuse its
    // canonicalization rules for the containment checks.
    let cwd = cwd
        .canonicalize()
        .context("failed to canonicalize current directory")?;
    let active = activated_environments()
        .iter()
        .map(|env| env.path().and_then(Path::parent).map(Path::to_path_buf))
        .collect();
    Ok(AutoActivateContext {
        cwd,
        discovered,
        active,
        auto_activated: read_path_list_var(FLOX_AUTO_ACTIVATED_ENVIRONMENTS_VAR),
        suppressed: read_path_list_var(FLOX_SUPPRESSED_ENVIRONMENTS_VAR),
        pending_deactivations,
    })
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
    fn empty_ctx(cwd: &str) -> AutoActivateContext {
        AutoActivateContext {
            cwd: PathBuf::from(cwd),
            discovered: Vec::new(),
            active: Vec::new(),
            auto_activated: Vec::new(),
            suppressed: Vec::new(),
            pending_deactivations: false,
        }
    }

    fn paths(values: &[&str]) -> Vec<PathBuf> {
        values.iter().map(PathBuf::from).collect()
    }

    fn noop_plan() -> AutoActivatePlan {
        AutoActivatePlan {
            activate: Vec::new(),
            deactivate_front: false,
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
            deactivate_front: true,
            ..noop_plan()
        });
    }

    #[test]
    fn pops_one_layer_per_run() {
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
            deactivate_front: true,
            // The outer env unwinds on the next prompt.
            auto_activated: paths(&["/home/user/outer"]),
            ..noop_plan()
        });
    }

    #[test]
    fn pops_and_activates_in_one_run_when_switching_projects() {
        let ctx = AutoActivateContext {
            discovered: paths(&["/home/user/proj_b"]),
            active: vec![Some(PathBuf::from("/home/user/proj_a"))],
            auto_activated: paths(&["/home/user/proj_a"]),
            ..empty_ctx("/home/user/proj_b")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/home/user/proj_b"]),
            deactivate_front: true,
            auto_activated: paths(&["/home/user/proj_b"]),
            ..noop_plan()
        });
    }

    #[test]
    fn defers_activation_while_stack_unwinds() {
        // Leaving /tmp/a/b for /tmp/z: the front pops this run, but /tmp/a is
        // still unwinding, so /tmp/z is not activated yet — activating it now
        // would bury /tmp/a and force an abandon on a later run.
        let ctx = AutoActivateContext {
            discovered: paths(&["/tmp/z"]),
            active: vec![
                Some(PathBuf::from("/tmp/a/b")),
                Some(PathBuf::from("/tmp/a")),
            ],
            auto_activated: paths(&["/tmp/a", "/tmp/a/b"]),
            ..empty_ctx("/tmp/z")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            deactivate_front: true,
            auto_activated: paths(&["/tmp/a"]),
            ..noop_plan()
        });
    }

    #[test]
    fn unwinds_fully_then_activates_in_final_run() {
        // Continuation of defers_activation_while_stack_unwinds, with the
        // previous plan applied by the shell: the next hook run pops the last
        // unwinding layer and activates the new environment in the same run.
        let ctx = AutoActivateContext {
            discovered: paths(&["/tmp/z"]),
            active: vec![Some(PathBuf::from("/tmp/a"))],
            auto_activated: paths(&["/tmp/a"]),
            ..empty_ctx("/tmp/z")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/tmp/z"]),
            deactivate_front: true,
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
            ..empty_ctx("/home/user/proj")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            activate: paths(&["/home/user/proj"]),
            auto_activated: paths(&["/home/user/proj"]),
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
    fn sibling_directory_is_not_inside() {
        // Path containment must compare whole components: /home/user/proj2
        // is not inside /home/user/proj.
        let ctx = AutoActivateContext {
            active: vec![Some(PathBuf::from("/home/user/proj"))],
            auto_activated: paths(&["/home/user/proj"]),
            ..empty_ctx("/home/user/proj2")
        };
        assert_eq!(plan_auto_activation(&ctx), AutoActivatePlan {
            deactivate_front: true,
            ..noop_plan()
        });
    }

    #[test]
    fn activate_command_is_shell_escaped() {
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
