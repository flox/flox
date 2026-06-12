use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

pub mod diff_serializer;

use anyhow::Result;
use flox_core::activate::context::{ActivateCtx, AttachCtx, AttachProjectCtx, SandboxMode};
use flox_core::activate::vars::{FLOX_ACTIVATIONS_BIN, FLOX_ACTIVE_ENVIRONMENTS_VAR};
use flox_core::util::default_nix_env_vars;
use is_executable::IsExecutable;
use itertools::Itertools;
use shell_gen::{GenerateShell, SetVar, Statement, UnsetVar};
use tracing::debug;

use crate::attach_diff::diff_serializer::{
    DiffSerializer,
    FLOX_HOOK_DIFF_VAR,
    FLOX_INVOCATION_TYPE_VAR,
};
use crate::cli::fix_paths::{fix_manpath_var, fix_path_var};
use crate::cli::set_env_dirs::fix_env_dirs_var;
use crate::env_diff::EnvDiff;
use crate::sandbox::seed::SeedContext;
use crate::sandbox::{
    FLOX_SANDBOX_ALLOW_NET_VAR,
    PRELOAD_VAR,
    sandbox_env,
    verdict_socket_path,
};
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
                // for which values must be computed dynamically at runtime.
                // Also track _FLOX_HOOK_DIFF and _FLOX_INVOCATION_TYPE so that
                // nested (stacked) activations can restore the outer values on
                // deactivation rather than unconditionally unsetting them.
                HashSet::from([
                    FLOX_ENV_DIRS_VAR.to_string(),
                    "PATH".to_string(),
                    "MANPATH".to_string(),
                    FLOX_HOOK_DIFF_VAR.to_string(),
                    FLOX_INVOCATION_TYPE_VAR.to_string(),
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
        // double_sets includes user variables which we want to override
        // single_sets, so set them after single_sets.
        // TODO: we should keep track of that in a way that's less brittle
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
        // double_sets includes user variables which we want to override
        // single_sets, so set them after single_sets.
        // TODO: we should keep track of that in a way that's less brittle
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
//   • `_activate_d`             — gen_rc/{bash,fish,tcsh}.rs (export)
//   • `_flox_activations`       — gen_rc/{bash,fish,tcsh}.rs (export)
//   • `_flox_activate_tracer`   — gen_rc/{bash,fish,tcsh}.rs (export, via args.flox_activate_tracer)
//   • `_flox_sourcing_rc`       — gen_rc/bash.rs            (export + unset, bashrc-sourcing dance)
//   • `fish_trace` (opener)     — gen_rc/fish.rs            (export, when tracelevel ≥ 2)
//   • `_FLOX_INVOCATION_TYPE`   — gen_rc/{bash,zsh,fish,tcsh}.rs (export; cleanup handled by diff)
//
// Note: `_FLOX_HOOK_DIFF` is not listed above — it is set by the Rust binary
// via `generate_statements` / `apply_to_command`, not by a gen_rc helper, so
// it is not an inline leak in this sense.
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
        // Path to the flox-activations binary. Folded in here (rather than
        // emitted inline by the gen_rc scripts) so it is captured in the
        // activation diff and unset on deactivate. Runtime profile scripts
        // re-derive it from `@flox_activations@`, so losing the gen_rc
        // re-export does not affect them.
        (
            "_flox_activations",
            FLOX_ACTIVATIONS_BIN.to_string_lossy().to_string(),
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

    let mut additions: HashMap<String, String> = exports
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();

    // Sandbox preload and policy. These go through double_sets specifically:
    // the rc-script re-export is the only way DYLD_INSERT_LIBRARIES survives
    // macOS's SIP strip at the /bin/zsh boundary and reaches user-spawned
    // children. A failure to locate the library is fatal — silently
    // activating unsandboxed would betray a user who explicitly asked for a
    // sandbox.
    additions.extend(sandbox_double_sets(context, project));

    EnvDiff::from_parts(additions, deletions)
}

/// Compute the sandbox env vars folded into `double_set_envs`.
///
/// Empty when the activation is not sandboxed, or when there is no project
/// context (the grants dir is anchored under `.flox/`, which only exists for
/// project activations; container activations reject `ask` upstream).
///
/// Panics on library-resolution failure rather than returning a `Result`,
/// because `double_set_envs` is infallible by construction and the failure
/// is a build/environment defect, not a runtime condition the caller can
/// recover from. The message is actionable (it names the missing path).
fn sandbox_double_sets(
    context: &AttachCtx,
    project: Option<&AttachProjectCtx>,
) -> HashMap<String, String> {
    if context.sandbox_mode == SandboxMode::Off {
        return HashMap::new();
    }
    let Some(project) = project else {
        return HashMap::new();
    };

    // The grants dir lives under the gitignored .flox/cache/ tree. Create it
    // up front so the engine's write guard has a real directory to compare
    // against, and so a later batch can drop grants.toml there.
    let grants_dir = project.dot_flox_path.join("cache").join("sandbox");
    if let Err(err) = std::fs::create_dir_all(&grants_dir) {
        debug!(?grants_dir, %err, "could not create sandbox grants dir");
    }

    let seed_ctx = SeedContext {
        shell_binary: std::env::var_os("SHELL").map(PathBuf::from),
        interpreter_path: context.interpreter_path.clone(),
        home_dir: dirs::home_dir(),
        runtime_dir: std::env::var_os("FLOX_RUNTIME_DIR").map(PathBuf::from),
    };

    let existing_preload = std::env::var(PRELOAD_VAR).ok();
    // Honor an operator-supplied FLOX_SANDBOX_ALLOW_NET (e.g. a CI step or a
    // one-off `FLOX_SANDBOX_ALLOW_NET=host flox activate`) by merging it with
    // the seeds rather than discarding it.
    let existing_allow_net = std::env::var(FLOX_SANDBOX_ALLOW_NET_VAR).ok();

    // The verdict socket path is a pure function of the services socket, so it
    // matches the path the broker binds inside the executive — no shared
    // mutable state, no second channel. Only `ask` actually exports it.
    let verdict_socket = verdict_socket_path(&project.flox_services_socket);

    sandbox_env(
        context.sandbox_mode,
        &seed_ctx,
        &project.env_project,
        &grants_dir,
        &verdict_socket,
        existing_preload.as_deref(),
        existing_allow_net.as_deref(),
    )
    .unwrap_or_else(|err| panic!("failed to assemble sandbox environment: {err:#}"))
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
    use std::ffi::OsStr;

    use tempfile::TempDir;

    use super::*;
    use crate::sandbox::{
        FLOX_SANDBOX_ALLOW_DIRS_VAR,
        FLOX_SANDBOX_ALLOW_FOREIGN_EXE_VAR,
        FLOX_SANDBOX_ALLOW_VAR,
        FLOX_SANDBOX_GRANTS_DIR_VAR,
        FLOX_SANDBOX_SOCKET_VAR,
        FLOX_SRC_DIR_VAR,
        FLOX_VIRTUAL_SANDBOX_VAR,
        LIBSANDBOX_FILENAME_FOR_TESTS,
    };

    fn make_env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn make_keys(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    /// All env var names this batch injects for an active sandbox, except the
    /// per-OS preload var (asserted separately).
    const SANDBOX_VAR_NAMES: &[&str] = &[
        FLOX_VIRTUAL_SANDBOX_VAR,
        FLOX_SANDBOX_ALLOW_VAR,
        FLOX_SANDBOX_ALLOW_DIRS_VAR,
        FLOX_SANDBOX_ALLOW_NET_VAR,
        FLOX_SRC_DIR_VAR,
        FLOX_SANDBOX_GRANTS_DIR_VAR,
        FLOX_SANDBOX_ALLOW_FOREIGN_EXE_VAR,
    ];

    /// Build an `(AttachCtx, AttachProjectCtx)` rooted at a real tempdir so
    /// the grants-dir creation and seed canonicalization have somewhere to
    /// go. The dot_flox_path lives under the tempdir.
    fn test_contexts(tmp: &TempDir, sandbox_mode: SandboxMode) -> (AttachCtx, AttachProjectCtx) {
        let project_dir = tmp.path().join("project");
        let dot_flox = project_dir.join(".flox");
        std::fs::create_dir_all(&dot_flox).unwrap();
        let interpreter = tmp.path().join("interpreter");
        std::fs::create_dir_all(&interpreter).unwrap();
        let attach = AttachCtx {
            env: "/flox_env".to_string(),
            env_cache: tmp.path().join("cache"),
            env_description: "test".to_string(),
            flox_active_environments: "[]".to_string(),
            prompt_color_1: "1".to_string(),
            prompt_color_2: "2".to_string(),
            flox_prompt_environments: "".to_string(),
            set_prompt: false,
            flox_env_cuda_detection: "0".to_string(),
            interpreter_path: interpreter,
            sandbox_mode,
        };
        let project = AttachProjectCtx {
            env_project: project_dir,
            dot_flox_path: dot_flox,
            flox_env_log_dir: tmp.path().join("log"),
            // Real services sockets are `runtime_dir/flox.<id>.sock`; use that
            // shape so the derived verdict socket asserts the production form.
            flox_services_socket: tmp.path().join("flox.testid.sock"),
            process_compose_bin: PathBuf::from("/nix/store/fake-process-compose"),
            services_to_start: Vec::new(),
        };
        (attach, project)
    }

    /// Create a fake package-builder libexec with `flox-build.mk` and the
    /// platform libsandbox file, returning the makefile path to point
    /// `FLOX_BUILD_MK` at.
    fn fake_build_mk(tmp: &TempDir) -> PathBuf {
        let libexec = tmp.path().join("libexec");
        std::fs::create_dir_all(&libexec).unwrap();
        std::fs::write(libexec.join("flox-build.mk"), b"# fake\n").unwrap();
        std::fs::write(libexec.join(LIBSANDBOX_FILENAME_FOR_TESTS), b"\x7fELF").unwrap();
        libexec.join("flox-build.mk")
    }

    #[test]
    fn double_set_envs_omits_sandbox_vars_when_off() {
        let tmp = TempDir::new().unwrap();
        let (attach, project) = test_contexts(&tmp, SandboxMode::Off);
        // Even with a valid library configured, Off injects nothing sandbox.
        let build_mk = fake_build_mk(&tmp);
        let diff = temp_env::with_var("FLOX_BUILD_MK", Some(build_mk.as_os_str()), || {
            double_set_envs(&attach, Some(&project))
        });
        for name in SANDBOX_VAR_NAMES {
            assert!(
                !diff.additions.contains_key(*name),
                "Off mode should not inject {name}"
            );
        }
        assert!(!diff.additions.contains_key(PRELOAD_VAR));
        // The non-sandbox exports are still present.
        assert!(diff.additions.contains_key("FLOX_ENV"));
    }

    #[test]
    fn double_set_envs_injects_all_sandbox_vars_when_active() {
        let tmp = TempDir::new().unwrap();
        let (attach, project) = test_contexts(&tmp, SandboxMode::Enforce);
        let build_mk = fake_build_mk(&tmp);

        let diff = temp_env::with_vars(
            [
                ("FLOX_BUILD_MK", Some(build_mk.as_os_str())),
                // Clear any inherited preload so the assertion is exact.
                (PRELOAD_VAR, None::<&OsStr>),
            ],
            || double_set_envs(&attach, Some(&project)),
        );

        for name in SANDBOX_VAR_NAMES {
            assert!(
                diff.additions.contains_key(*name),
                "active mode should inject {name}"
            );
        }
        assert_eq!(
            diff.additions.get(FLOX_VIRTUAL_SANDBOX_VAR).unwrap(),
            "enforce"
        );
        assert!(diff.additions.contains_key(PRELOAD_VAR));
        // Enforce never contacts a broker, so it exports no verdict socket.
        assert!(!diff.additions.contains_key(FLOX_SANDBOX_SOCKET_VAR));
        // The grants dir was created so the engine write guard has a target.
        assert!(project.dot_flox_path.join("cache").join("sandbox").is_dir());
    }

    #[test]
    fn double_set_envs_exports_verdict_socket_for_ask() {
        let tmp = TempDir::new().unwrap();
        let (attach, project) = test_contexts(&tmp, SandboxMode::Ask);
        let build_mk = fake_build_mk(&tmp);

        let diff = temp_env::with_vars(
            [
                ("FLOX_BUILD_MK", Some(build_mk.as_os_str())),
                (PRELOAD_VAR, None::<&OsStr>),
            ],
            || double_set_envs(&attach, Some(&project)),
        );

        // Ask exports the verdict socket, and its value is the sibling of the
        // services socket the broker binds — the contract both sides share.
        let expected = project
            .flox_services_socket
            .parent()
            .unwrap()
            .join("sbx.testid.sock");
        assert_eq!(
            diff.additions.get(FLOX_SANDBOX_SOCKET_VAR).unwrap(),
            &expected.to_string_lossy().into_owned()
        );
    }

    #[test]
    fn double_set_envs_excludes_sandbox_for_container_without_project() {
        let tmp = TempDir::new().unwrap();
        let (attach, _project) = test_contexts(&tmp, SandboxMode::Enforce);
        let build_mk = fake_build_mk(&tmp);
        // No project context (container path) means no grants dir anchor, so
        // sandbox vars are omitted even for an active mode.
        let diff = temp_env::with_var("FLOX_BUILD_MK", Some(build_mk.as_os_str()), || {
            double_set_envs(&attach, None)
        });
        for name in SANDBOX_VAR_NAMES {
            assert!(
                !diff.additions.contains_key(*name),
                "container activation should not inject {name}"
            );
        }
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
