use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

pub mod diff_serializer;

use anyhow::Result;
use flox_core::activate::context::{ActivateCtx, AttachCtx, AttachProjectCtx};
use flox_core::activate::vars::FLOX_ACTIVE_ENVIRONMENTS_VAR;
use flox_core::util::default_nix_env_vars;
use is_executable::IsExecutable;
use itertools::Itertools;
use shell_gen::{GenerateShell, SetVar, Statement, UnsetVar};
use tracing::debug;

use crate::attach_diff::diff_serializer::{DiffSerializer, FLOX_HOOK_DIFF_VAR};
use crate::cli::fix_paths::{fix_manpath_var, fix_path_var};
use crate::cli::set_env_dirs::fix_env_dirs_var;
use crate::env_diff::EnvDiff;
use crate::start_diff::StartDiff;
use crate::vars_from_env::VarsFromEnvironment;
pub const FLOX_PROMPT_ENVIRONMENTS_VAR: &str = "FLOX_PROMPT_ENVIRONMENTS";

pub const FLOX_ACTIVATE_START_SERVICES_VAR: &str = "FLOX_ACTIVATE_START_SERVICES";
pub const FLOX_ENV_DIRS_VAR: &str = "FLOX_ENV_DIRS";

pub(super) fn assemble_activate_command(
    context: &ActivateCtx,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    start_state_dir: &Path,
) -> Command {
    let mut command = Command::new(context.attach_ctx.interpreter_path.join("activate"));
    command.envs(single_set_envs(&context.attach_ctx));
    let double_sets = double_set_envs(&context.attach_ctx, context.project_ctx.as_ref());
    command.envs(&double_sets.additions);
    for var in &double_sets.deletions {
        command.env_remove(var);
    }
    command.envs(non_in_place_exports(
        &context.attach_ctx,
        subsystem_verbosity,
        vars_from_env,
    ));
    add_activate_script_options(&mut command, context, start_state_dir);
    command
}

/// The complete set of environment variable changes needed for attaching.
///
/// Constructed once from the attach context, this struct is the single
/// source of truth for what variables to set and unset. All consumers
/// (command application, in-place export rendering, activation diff
/// computation) draw from the same data.
#[derive(Debug, Clone)]
pub struct AttachDiff {
    /// Variables that we only export once while activating
    /// - For in-place activations, these are printed as exports
    /// - For other activations, these are applied as environment variables before we exec
    ///
    /// It probably wouldn't hurt to double set them, but for variables we
    /// control, we currently skip that.
    single_sets: HashMap<String, String>,
    /// Variables that haven't yet been folded into either single or double sets.
    non_in_place_sets: HashMap<String, String>,
    /// Variables that we set (or unset) twice for non-in-place activations.
    /// We set (or unset) these:
    /// 1. As environment variables before we exec
    /// 2. Via our generated startup scripts, after user RC files have run.
    ///    This ensures we re-apply these variables after they could have been
    ///    changed, particularly if user RC files contain flox activations
    double_sets: EnvDiff,
    /// Pre-encoded diff string for _FLOX_HOOK_DIFF. None if snapshot unavailable.
    encoded_diff: Option<String>,
}

impl AttachDiff {
    /// Assemble all environment variable sets and unsets needed for
    /// activation, and compute the activation diff if a pre-activation
    /// snapshot is available.
    pub fn new(
        context: &AttachCtx,
        project: Option<&AttachProjectCtx>,
        subsystem_verbosity: u32,
        mut vars_from_env: VarsFromEnvironment,
        start_diff: &StartDiff,
        is_in_place: bool,
    ) -> Result<Self> {
        // Extract the pre-activation snapshot before consuming vars_from_env.
        let full_env = vars_from_env.full_env.take();

        let single_sets: HashMap<String, String> = single_set_envs(context)
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        let mut double_sets = double_set_envs(context, project);

        let mut non_in_place_sets: HashMap<String, String> = HashMap::new();

        if !is_in_place {
            for (k, v) in non_in_place_exports(context, subsystem_verbosity, vars_from_env) {
                non_in_place_sets.insert(k.to_string(), v);
            }
        }

        // For now don't prevent users overriding our variables
        double_sets.additions.extend(
            start_diff
                .additions()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        double_sets
            .deletions
            .extend(start_diff.deletions().iter().cloned());

        // Compute the activation diff if we have a pre-activation snapshot.
        let encoded_diff = if let Some(ref current_env) = full_env {
            let mut intended_sets = if is_in_place {
                // These variables are computed by set-env-dirs and fix-paths,
                // for which values must be computed dynamically at runtime
                HashSet::from([
                    FLOX_ENV_DIRS_VAR.to_string(),
                    "PATH".to_string(),
                    "MANPATH".to_string(),
                ])
            } else {
                non_in_place_sets.keys().cloned().collect()
            };
            intended_sets.extend(single_sets.keys().cloned());
            intended_sets.extend(double_sets.additions.keys().cloned());
            let intended_removals: HashSet<String> =
                double_sets.deletions.iter().cloned().collect();
            let diff = diff_env(current_env, &intended_sets, &intended_removals);
            let encoded = diff.encode()?;
            debug!(
                "captured activation diff: {} added, {} modified, {} removed ({} bytes encoded)",
                diff.added.len(),
                diff.modified.len(),
                diff.removed.len(),
                encoded.len(),
            );
            Some(encoded)
        } else {
            None
        };

        Ok(Self {
            single_sets,
            non_in_place_sets,
            double_sets,
            encoded_diff,
        })
    }

    /// The encoded `_FLOX_HOOK_DIFF` string, if a pre-activation snapshot
    /// was available when this `AttachDiff` was constructed.
    #[cfg(test)]
    pub fn encoded_diff(&self) -> Option<&str> {
        self.encoded_diff.as_deref()
    }

    /// Apply the activation environment to a Command.
    ///
    /// Sets all accumulated variables, removes all accumulated unsets,
    /// and sets the _FLOX_HOOK_DIFF env var if a diff was computed.
    pub fn apply_to_command(&self, command: &mut Command) {
        command.envs(&self.non_in_place_sets);
        command.envs(&self.single_sets);
        command.envs(&self.double_sets.additions);
        for var in &self.double_sets.deletions {
            command.env_remove(var);
        }
        if let Some(ref encoded) = self.encoded_diff {
            command.env(FLOX_HOOK_DIFF_VAR, encoded);
        }
    }

    /// Generate statements that apply the environment in a gen_rc script
    ///
    /// This is the same as `apply_to_command` except:
    /// - single_sets are skipped when not in in-place mode (since those have
    ///   already been applied by apply_to_command)
    /// - `sets`, which haven't yet been folded together
    pub(crate) fn generate_statements(&self, is_in_place: bool) -> Vec<Statement> {
        let mut stmts = Vec::new();
        if is_in_place {
            for (k, v) in self.single_sets.iter().sorted_by_key(|(k, _)| *k) {
                stmts.push(set_exported_unexpanded(k, v));
            }
            if let Some(ref encoded) = self.encoded_diff {
                stmts.push(set_exported_unexpanded(FLOX_HOOK_DIFF_VAR, encoded));
            }
        }
        for (k, v) in self.double_sets.additions.iter().sorted_by_key(|(k, _)| *k) {
            stmts.push(set_exported_unexpanded(k, v));
        }
        for name in self.double_sets.deletions.iter().sorted() {
            stmts.push(unset(name));
        }
        stmts
    }
}

// We define this here without `pub` so we don't accidentally export variables
// in gen_rc that we don't capture for `flox deactivate`
fn set_exported_unexpanded(name: impl AsRef<str>, value: impl AsRef<str>) -> Statement {
    SetVar::exported_no_expansion(name, value).to_stmt()
}

// We define this here without `pub` so we don't accidentally export variables
// in gen_rc that we don't capture for `flox deactivate`
#[allow(dead_code)]
fn set_exported_expanded(name: impl AsRef<str>, value: impl AsRef<str>) -> Statement {
    SetVar::exported_with_expansion(name, value).to_stmt()
}

// We define this here without `pub` so we don't accidentally change variables
// in gen_rc that we don't capture for `flox deactivate`
fn unset(name: impl AsRef<str>) -> Statement {
    UnsetVar::new(name).to_stmt()
}

// ─── Inline set/unset NOT yet folded into AttachDiff ────────────────────────
// Each call to a `todo_drop_*` helper below is a marker for an emission this
// refactor deferred. Move each caller into `AttachDiff::generate_statements`
// (or its inputs) and delete the corresponding shim when the last caller is
// gone.
//
// Helper-routed inline leaks:
//   • `_activate_d`           — gen_rc/{bash,fish,tcsh}.rs (export)
//   • `_flox_activations`     — gen_rc/{bash,fish,tcsh}.rs (export)
//   • `_flox_activate_tracer` — gen_rc/{bash,fish,tcsh}.rs (export, via args.flox_activate_tracer)
//   • `_flox_sourcing_rc`     — gen_rc/bash.rs            (export + unset, bashrc-sourcing dance)
//   • `fish_trace` (opener)   — gen_rc/fish.rs            (export, when tracelevel ≥ 2)
//
// Zsh emits no exports inline — its `_flox_activate_tracelevel` and
// `_activate_d` go through the non-export `set_unexported_unexpanded` helper,
// which is NOT moving out of `shell_gen`.
// ────────────────────────────────────────────────────────────────────────────

pub(crate) fn todo_drop_set_exported_unexpanded(
    name: impl AsRef<str>,
    value: impl AsRef<str>,
) -> Statement {
    SetVar::exported_no_expansion(name, value).to_stmt()
}

pub(crate) fn todo_drop_unset(name: impl AsRef<str>) -> Statement {
    UnsetVar::new(name).to_stmt()
}

/// Compute the diff between the current environment and the intended
/// post-activation state.
///
/// `intended_sets` and `intended_removals` are pre-assembled by the caller.
fn diff_env(
    current_env: &HashMap<String, String>,
    intended_sets: &HashSet<String>,
    intended_removals: &HashSet<String>,
) -> DiffSerializer {
    let mut added = HashSet::new();
    let mut modified = HashMap::new();
    let mut removed = HashMap::new();

    for k in intended_sets {
        match current_env.get(k) {
            None => {
                added.insert(k.clone());
            },
            Some(old_val) => {
                modified.insert(k.clone(), old_val.clone());
            },
        }
    }

    for k in intended_removals {
        if let Some(old_val) = current_env.get(k) {
            removed.insert(k.clone(), old_val.clone());
        }
    }

    DiffSerializer {
        added,
        modified,
        removed,
    }
}

pub fn single_set_envs(context: &AttachCtx) -> HashMap<&'static str, String> {
    let mut exports = HashMap::from([
        (
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            context.flox_active_environments.clone(),
        ),
        ("FLOX_PROMPT_COLOR_1", context.prompt_color_1.clone()),
        ("FLOX_PROMPT_COLOR_2", context.prompt_color_2.clone()),
        // Set `FLOX_PROMPT_ENVIRONMENTS` to the constructed prompt string,
        // which may be ""
        // This is used by set-prompt script, and tcsh in particular does not
        // tolerate references to undefined variables.
        (
            FLOX_PROMPT_ENVIRONMENTS_VAR,
            context.flox_prompt_environments.clone(),
        ),
    ]);

    exports.extend(default_nix_env_vars());

    exports
}

pub fn double_set_envs(context: &AttachCtx, project: Option<&AttachProjectCtx>) -> EnvDiff {
    let mut deletions = Vec::new();
    let mut exports = HashMap::from([
        (
            FLOX_ACTIVATE_START_SERVICES_VAR,
            project
                .is_some_and(|p| !p.services_to_start.is_empty())
                .to_string(),
        ),
        // Propagate required variables that are documented as exposed.
        ("FLOX_ENV", context.env.clone()),
        (
            "FLOX_ENV_CACHE",
            context.env_cache.to_string_lossy().to_string(),
        ),
        ("FLOX_ENV_DESCRIPTION", context.env_description.clone()),
    ]);
    // Propagate optional variables that are documented as exposed.
    if let Some(project) = project {
        exports.insert(
            "FLOX_ENV_PROJECT",
            project.env_project.to_string_lossy().to_string(),
        );
    } else {
        deletions.push("FLOX_ENV_PROJECT".to_string());
    }

    let additions = exports
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    EnvDiff::from_parts(additions, deletions)
}

/// Options parsed by getopt in the activate script
fn add_activate_script_options(
    command: &mut Command,
    context: &ActivateCtx,
    start_state_dir: &Path,
) {
    command.arg("--env").arg(&context.attach_ctx.env);

    // Pass down the activation mode
    command.arg("--mode").arg(context.mode.to_string());

    command.args(["--start-state-dir", &start_state_dir.to_string_lossy()]);

    if context.attach_ctx.flox_env_cuda_detection == "1" {
        command.arg("--cuda-detection");
    }
}

/// _flox_activate_tracelevel, _flox_activate_tracer, and _activate_d still need some cleanup
/// fixed_vars_to_export are used for interactive activations but are handled
/// differently for in-place activations
pub fn non_in_place_exports(
    context: &AttachCtx,
    subsystem_verbosity: u32,
    vars_from_environment: VarsFromEnvironment,
) -> HashMap<&'static str, String> {
    let mut exports = HashMap::from([
        ("_flox_activate_tracelevel", subsystem_verbosity.to_string()),
        (
            "_flox_activate_tracer",
            activate_tracer(&context.interpreter_path),
        ),
        (
            "_activate_d",
            context
                .interpreter_path
                .join("activate.d")
                .to_string_lossy()
                .to_string(),
        ),
    ]);
    exports.extend(fixed_vars_to_export(&context.env, vars_from_environment));
    exports
}

/// Calculate values for FLOX_ENV_DIRS, PATH, and MANPATH
fn fixed_vars_to_export(
    flox_env: impl AsRef<str>,
    vars_from_environment: VarsFromEnvironment,
) -> HashMap<&'static str, String> {
    let new_flox_env_dirs = fix_env_dirs_var(
        flox_env.as_ref(),
        vars_from_environment
            .flox_env_dirs
            .unwrap_or("".to_string()),
    );
    let new_path = fix_path_var(
        &new_flox_env_dirs,
        &vars_from_environment.path.unwrap_or("".to_string()),
    );
    let new_manpath = fix_manpath_var(
        &new_flox_env_dirs,
        &vars_from_environment.manpath.unwrap_or("".to_string()),
    );
    HashMap::from([
        (FLOX_ENV_DIRS_VAR, new_flox_env_dirs),
        ("PATH", new_path),
        ("MANPATH", new_manpath),
    ])
}

/// The activate_tracer is set from the FLOX_ACTIVATE_TRACE env var.
/// If that env var is empty then activate_tracer is set to the full path of the `true` command in the PATH.
/// If that env var is not empty and refers to an executable then then activate_tracer is set to that value.
/// Else activate_tracer is set to refer to {interpreter_path}/activate.d/trace.
// TODO: we should probably pass this around rather than recomputing it
pub fn activate_tracer(interpreter_path: impl AsRef<Path>) -> String {
    if let Ok(trace_path) = std::env::var("FLOX_ACTIVATE_TRACE") {
        if Path::new(&trace_path).is_executable() {
            trace_path
        } else {
            interpreter_path
                .as_ref()
                .join("activate.d")
                .join("trace")
                .to_string_lossy()
                .to_string()
        }
    } else {
        "true".to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn make_env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn make_keys(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn compute_additions() {
        let current = make_env(&[("EXISTING", "value")]);
        let sets = make_keys(&["NEW_VAR"]);
        let diff = diff_env(&current, &sets, &make_keys(&[]));

        assert_eq!(diff.added, make_keys(&["NEW_VAR"]));
        assert!(diff.modified.is_empty());
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn compute_modifications() {
        let current = make_env(&[("MY_VAR", "old_value")]);
        let sets = make_keys(&["MY_VAR"]);
        let diff = diff_env(&current, &sets, &make_keys(&[]));

        // modified stores original value
        assert!(diff.added.is_empty());
        assert_eq!(diff.modified, make_env(&[("MY_VAR", "old_value")]));
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn compute_removals() {
        let current = make_env(&[("GONE_VAR", "gone_value")]);
        let diff = diff_env(&current, &HashSet::new(), &make_keys(&["GONE_VAR"]));

        // removed stores original value
        assert!(diff.added.is_empty());
        assert!(diff.modified.is_empty());
        assert_eq!(diff.removed, make_env(&[("GONE_VAR", "gone_value")]));
    }

    #[test]
    fn compute_mixed() {
        let current = make_env(&[("MODIFIED_VAR", "orig"), ("REMOVED_VAR", "to_remove")]);
        let sets = make_keys(&["NEW_VAR", "MODIFIED_VAR"]);
        let diff = diff_env(&current, &sets, &make_keys(&["REMOVED_VAR"]));

        assert_eq!(diff.added, make_keys(&["NEW_VAR"]));
        assert_eq!(diff.modified, make_env(&[("MODIFIED_VAR", "orig")]));
        assert_eq!(diff.removed, make_env(&[("REMOVED_VAR", "to_remove")]));
    }

    #[test]
    fn same_value_tracked_in_modified() {
        let current = make_env(&[("UNCHANGED", "value")]);
        // Even when old == new, we track in modified so deactivation can
        // restore the original value if the user changes the var manually.
        let sets = make_keys(&["UNCHANGED"]);
        let diff = diff_env(&current, &sets, &make_keys(&[]));

        assert!(diff.added.is_empty());
        assert_eq!(diff.modified, make_env(&[("UNCHANGED", "value")]));
        assert!(diff.removed.is_empty());
    }
}
