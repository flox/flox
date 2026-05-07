use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Command;

use anyhow::Result;
use flox_core::activate::context::{ActivateCtx, AttachCtx, AttachProjectCtx};
use flox_core::activate::vars::FLOX_ACTIVE_ENVIRONMENTS_VAR;
use flox_core::util::default_nix_env_vars;
use is_executable::IsExecutable;
use tracing::debug;

use crate::activation_diff::{self, DiffSerializer};
use crate::cli::fix_paths::{fix_manpath_var, fix_path_var};
use crate::cli::set_env_dirs::fix_env_dirs_var;
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
    command.envs(old_cli_envs(
        &context.attach_ctx,
        context.project_ctx.as_ref(),
    ));
    add_old_activate_script_exports(
        &mut command,
        &context.attach_ctx,
        context.project_ctx.as_ref(),
        subsystem_verbosity,
        vars_from_env,
    );
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
    /// Variables to set on the command/shell.
    pub sets: HashMap<String, String>,
    /// Variables to unset from the command/shell.
    pub removals: HashSet<String>,
    /// Pre-encoded diff string for _FLOX_HOOK_DIFF. None if snapshot unavailable.
    pub encoded_diff: Option<String>,
}

impl AttachDiff {
    /// Assemble all environment variable sets and removals needed for
    /// activation, and compute the activation diff if a pre-activation
    /// snapshot is available.
    ///
    /// Sources are applied in precedence order (later overrides earlier):
    /// 1. `old_cli_envs()` — FLOX_* context vars + default nix vars
    /// 2. `collect_activate_exports()` — activation context vars
    /// 3. `start_diff.additions` / `start_diff.deletions` — from activation scripts
    pub fn new(
        context: &AttachCtx,
        project: Option<&AttachProjectCtx>,
        subsystem_verbosity: u32,
        mut vars_from_env: VarsFromEnvironment,
        start_diff: &StartDiff,
    ) -> Result<Self> {
        // Extract the pre-activation snapshot before consuming vars_from_env.
        let full_env = vars_from_env.full_env.take();

        // Assemble sets and removals.
        let mut sets: HashMap<String, String> = HashMap::new();

        for (k, v) in old_cli_envs(context, project) {
            sets.insert(k.to_string(), v);
        }

        let (export_map, removal_list) =
            collect_activate_exports(context, project, subsystem_verbosity, vars_from_env);
        for (k, v) in export_map {
            sets.insert(k.to_string(), v);
        }

        for (k, v) in &start_diff.additions {
            sets.insert(k.clone(), v.clone());
        }

        let mut removals: HashSet<String> = HashSet::new();
        for k in &removal_list {
            removals.insert(k.to_string());
        }
        for k in &start_diff.deletions {
            removals.insert(k.clone());
        }

        // Compute the activation diff if we have a pre-activation snapshot.
        let encoded_diff = if let Some(ref current_env) = full_env {
            let diff = diff_env(current_env, &sets, &removals);
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
            sets,
            removals,
            encoded_diff,
        })
    }

    /// Apply the activation environment to a Command.
    ///
    /// Sets all accumulated variables, removes all accumulated removals,
    /// and sets the _FLOX_HOOK_DIFF env var if a diff was computed.
    pub fn apply_to_command(&self, command: &mut Command) {
        command.envs(&self.sets);
        for var in &self.removals {
            command.env_remove(var);
        }
        if let Some(ref encoded) = self.encoded_diff {
            command.env(activation_diff::FLOX_HOOK_DIFF_VAR, encoded);
        }
    }
}

/// Compute the diff between the current environment and the intended
/// post-activation state.
///
/// `intended_sets` and `intended_removals` are pre-assembled by the caller.
fn diff_env(
    current_env: &HashMap<String, String>,
    intended_sets: &HashMap<String, String>,
    intended_removals: &HashSet<String>,
) -> DiffSerializer {
    let mut added = HashMap::new();
    let mut modified = HashMap::new();
    let mut removed = HashMap::new();

    for (k, new_val) in intended_sets {
        match current_env.get(k) {
            None => {
                added.insert(k.clone(), new_val.clone());
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

/// Build environment variables from activation context.
pub fn old_cli_envs(
    context: &AttachCtx,
    project: Option<&AttachProjectCtx>,
) -> HashMap<&'static str, String> {
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
        ("_FLOX_SET_PROMPT", context.set_prompt.to_string()),
        // This is user-facing and documented
        (
            FLOX_ACTIVATE_START_SERVICES_VAR,
            project
                .is_some_and(|p| !p.services_to_start.is_empty())
                .to_string(),
        ),
    ]);

    exports.extend(default_nix_env_vars());

    exports
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

/// Prior to the refactor, these variables were exported in the activate script
// TODO: we still use std::env::var in this function,
// so we should either drop those uses and get those vars in VarsFromEnvironment,
// or we should completely drop VarsFromEnvironment .
fn add_old_activate_script_exports(
    command: &mut Command,
    context: &AttachCtx,
    project: Option<&AttachProjectCtx>,
    subsystem_verbosity: u32,
    vars_from_environment: VarsFromEnvironment,
) {
    let (exports, removals) =
        collect_activate_exports(context, project, subsystem_verbosity, vars_from_environment);
    command.envs(&exports);
    for var in &removals {
        command.env_remove(var);
    }
}

/// Collect the environment variables that should be set and unset for activation.
///
/// Returns a tuple of (exports, removals) where exports maps variable names to
/// values and removals is a list of variable names to unset.
///
/// This is split out from `add_old_activate_script_exports` so that the data
/// can be inspected independently — for example when computing the activation diff.
pub fn collect_activate_exports(
    context: &AttachCtx,
    project: Option<&AttachProjectCtx>,
    subsystem_verbosity: u32,
    vars_from_environment: VarsFromEnvironment,
) -> (HashMap<&'static str, String>, Vec<&'static str>) {
    let mut removals = Vec::new();
    let mut exports = HashMap::from([
        ("_flox_activate_tracelevel", subsystem_verbosity.to_string()),
        // Propagate required variables that are documented as exposed.
        ("FLOX_ENV", context.env.clone()),
        (
            "FLOX_ENV_CACHE",
            context.env_cache.to_string_lossy().to_string(),
        ),
        ("FLOX_ENV_DESCRIPTION", context.env_description.clone()),
        // These are used by various scripts...custom ZDOTDIR files, set-prompt,
        // .tcshrc
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
    // Propagate optional variables that are documented as exposed.
    // NB: `generate_*_start_commands()` performs the same logic except for zsh.
    if let Some(project) = project {
        exports.insert(
            "FLOX_ENV_PROJECT",
            project.env_project.to_string_lossy().to_string(),
        );
    } else {
        removals.push("FLOX_ENV_PROJECT");
    }

    exports.extend(fixed_vars_to_export(&context.env, vars_from_environment));

    (exports, removals)
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

    fn make_removals(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn compute_additions() {
        let current = make_env(&[("EXISTING", "value")]);
        let sets = make_env(&[("NEW_VAR", "new_value")]);
        let diff = diff_env(&current, &sets, &make_removals(&[]));

        assert_eq!(diff.added, make_env(&[("NEW_VAR", "new_value")]));
        assert!(diff.modified.is_empty());
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn compute_modifications() {
        let current = make_env(&[("MY_VAR", "old_value")]);
        let sets = make_env(&[("MY_VAR", "new_value")]);
        let diff = diff_env(&current, &sets, &make_removals(&[]));

        // modified stores original value
        assert!(diff.added.is_empty());
        assert_eq!(diff.modified, make_env(&[("MY_VAR", "old_value")]));
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn compute_removals() {
        let current = make_env(&[("GONE_VAR", "gone_value")]);
        let diff = diff_env(&current, &HashMap::new(), &make_removals(&["GONE_VAR"]));

        // removed stores original value
        assert!(diff.added.is_empty());
        assert!(diff.modified.is_empty());
        assert_eq!(diff.removed, make_env(&[("GONE_VAR", "gone_value")]));
    }

    #[test]
    fn compute_mixed() {
        let current = make_env(&[("MODIFIED_VAR", "orig"), ("REMOVED_VAR", "to_remove")]);
        let sets = make_env(&[("NEW_VAR", "new"), ("MODIFIED_VAR", "changed")]);
        let diff = diff_env(&current, &sets, &make_removals(&["REMOVED_VAR"]));

        assert_eq!(diff.added, make_env(&[("NEW_VAR", "new")]));
        assert_eq!(diff.modified, make_env(&[("MODIFIED_VAR", "orig")]));
        assert_eq!(diff.removed, make_env(&[("REMOVED_VAR", "to_remove")]));
    }

    #[test]
    fn same_value_tracked_in_modified() {
        let current = make_env(&[("UNCHANGED", "value")]);
        // Even when old == new, we track in modified so deactivation can
        // restore the original value if the user changes the var manually.
        let sets = make_env(&[("UNCHANGED", "value")]);
        let diff = diff_env(&current, &sets, &make_removals(&[]));

        assert!(diff.added.is_empty());
        assert_eq!(diff.modified, make_env(&[("UNCHANGED", "value")]));
        assert!(diff.removed.is_empty());
    }
}
