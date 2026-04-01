use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_core::activate::context::{AttachCtx, AttachProjectCtx, AutoStartCtx, AutoStartResult};
use flox_core::activate::mode::ActivateMode;
use flox_core::activate::vars::{FLOX_ACTIVATIONS_BIN, FLOX_ACTIVE_ENVIRONMENTS_VAR};
use flox_core::activations::activation_state_dir_path;
use flox_core::hook_state::{
    ActivationInfo,
    ActivationTracking,
    HOOK_VAR_ACTIVATIONS,
    HOOK_VAR_CWD,
    HOOK_VAR_DIFF,
    HOOK_VAR_DIRS,
    HOOK_VAR_EXCLUDE_DIRS,
    HOOK_VAR_EXCLUDE_NAMES,
    HOOK_VAR_NOTIFIED,
    HOOK_VAR_SUPPRESSED,
    HOOK_VAR_WATCHES,
    HookDiff,
    HookState,
    OnActivateEnvDiff,
    WatchEntry,
};
use flox_core::preference::{PreferenceManager, PreferenceStatus};
use flox_core::trust::{TrustManager, TrustStatus};
use flox_manifest::interfaces::CommonFields;
use flox_manifest::lockfile::Lockfile;
use flox_manifest::parsed::Inner;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{
    DotFlox,
    Environment,
    EnvironmentPointer,
    UninitializedEnvironment,
    find_all_dot_flox,
};
use flox_rust_sdk::providers::services::process_compose::PROCESS_COMPOSE_BIN;
use flox_rust_sdk::utils::FLOX_INTERPRETER;
use regex::Regex;
use shell_gen::{GenerateShell, SetVar, Shell, UnsetVar};
use tracing::{debug, error};

use crate::config::{AutoActivateConfig, Config};
use crate::utils::active_environments::activated_environments;
use crate::utils::colors::{INDIGO_300, INDIGO_400};

#[derive(Bpaf, Clone, Debug)]
pub struct HookEnv {
    /// Shell to emit hook-env code for (bash, zsh, fish, tcsh)
    #[bpaf(long("shell"), argument("SHELL"))]
    shell: String,
}

impl HookEnv {
    pub fn handle(self, config: Config, flox: Flox) -> Result<()> {
        let shell: Shell = self
            .shell
            .parse()
            .map_err(|_| anyhow::anyhow!("unsupported shell: {}", self.shell))?;

        let state = HookState::from_env()?;
        let cwd = std::env::current_dir().context("failed to get current directory")?;

        // Discover .flox dirs in ancestor chain.
        let discovered = find_all_dot_flox(&cwd).unwrap_or_else(|e| {
            debug!("find_all_dot_flox failed: {e}");
            Vec::new()
        });

        let trust_manager = TrustManager::new(&flox.data_dir);
        let preference_manager = PreferenceManager::new(&flox.state_dir);
        let auto_activate_config = &config.flox.auto_activate;

        if is_fast_path(
            &state,
            &cwd,
            &discovered,
            &trust_manager,
            &preference_manager,
            auto_activate_config,
        ) {
            return Ok(());
        }

        let (mut trusted_dot_flox, suppressed_dirs, notified_dirs) = filter_by_eligibility(
            &state,
            &cwd,
            &discovered,
            &trust_manager,
            &preference_manager,
            auto_activate_config,
        );

        // Filter out environments that are manually activated via `flox activate`
        // subshells — these are tracked by _FLOX_HOOK_EXCLUDE_DIRS.
        let exclude_dirs: Vec<PathBuf> = std::env::var(HOOK_VAR_EXCLUDE_DIRS)
            .unwrap_or_default()
            .split(':')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();
        trusted_dot_flox.retain(|d| !exclude_dirs.contains(&d.path));

        let new_active_dirs: Vec<PathBuf> =
            trusted_dot_flox.iter().map(|d| d.path.clone()).collect();

        // Check if the set of active dirs actually changed.
        let dirs_changed = new_active_dirs != state.active_dirs;
        let watches_changed = state.watches_changed();

        if !dirs_changed && !watches_changed {
            let notified_changed = notified_dirs != state.notified_dirs;
            let suppressed_changed = suppressed_dirs != state.suppressed_dirs;
            let cwd_changed = state.last_cwd.as_ref() != Some(&cwd);

            if !cwd_changed && !notified_changed && !suppressed_changed {
                // Nothing changed at all.
                return Ok(());
            }
            // Only CWD/notified/suppressed changed — update tracking without
            // re-resolving all environments (avoids redundant lock/build/symlink
            // reads).
            let mut stdout = std::io::stdout().lock();

            // On the first hook-env call in a subshell (last_cwd is None),
            // emit the prompt so that manually-activated environments show
            // the PS1 prefix. Without this, neither set-prompt.zsh/bash
            // (which defers to hook-env when exclude vars are set) nor the
            // full update path (which isn't reached) would set the prompt.
            if state.last_cwd.is_none() {
                let all_names = build_prompt_names(&trusted_dot_flox);
                emit_prompt(&all_names, shell, &mut stdout)?;
            }

            if cwd_changed {
                SetVar::exported_no_expansion(HOOK_VAR_CWD, cwd.display().to_string())
                    .generate_with_newline(shell, &mut stdout)?;
            }
            if notified_changed {
                let notified_str = HookState::format_path_list(&notified_dirs);
                SetVar::exported_no_expansion(HOOK_VAR_NOTIFIED, &notified_str)
                    .generate_with_newline(shell, &mut stdout)?;
            }
            if suppressed_changed {
                let suppressed_str = HookState::format_path_list(&suppressed_dirs);
                SetVar::exported_no_expansion(HOOK_VAR_SUPPRESSED, &suppressed_str)
                    .generate_with_newline(shell, &mut stdout)?;
            }
            return Ok(());
        }

        let mut stdout = std::io::stdout().lock();

        // Revert previous diff.
        emit_revert(&state.diff, shell, &mut stdout)?;

        // Build new env vars and manage activation lifecycle.
        let (new_tracking, combined_env, path_additions, env_dirs_additions, new_watches) =
            manage_activations(&state, &trusted_dot_flox, &flox)?;

        // Merge PATH/FLOX_ENV_DIRS/MANPATH additions and compute diff.
        let (new_diff, combined_env) = compute_new_diff(
            &state.diff,
            combined_env,
            path_additions,
            env_dirs_additions,
        );

        // Emit new exports.
        emit_apply(&new_diff, &combined_env, shell, &mut stdout)?;

        // Build unified prompt with both excluded (manually-activated) and
        // auto-activated environment names, then emit it.
        let all_names = build_prompt_names(&trusted_dot_flox);
        emit_prompt(&all_names, shell, &mut stdout)?;

        // Emit updated state variables.
        emit_state_vars(
            &HookStateUpdate {
                diff: &new_diff,
                active_dirs: &new_active_dirs,
                watches: &new_watches,
                suppressed_dirs: &suppressed_dirs,
                notified_dirs: &notified_dirs,
                cwd: &cwd,
                trusted_dot_flox: &trusted_dot_flox,
                prev_active_dirs: &state.active_dirs,
                activation_tracking: &new_tracking,
            },
            shell,
            &mut stdout,
        )?;

        Ok(())
    }
}

/// Fast path: CWD unchanged AND no watched files changed AND the set of
/// discovered .flox dirs hasn't changed AND trust/preference status hasn't
/// changed for any active dir → no work needed.
fn is_fast_path(
    state: &HookState,
    cwd: &Path,
    discovered: &[DotFlox],
    trust_manager: &TrustManager,
    preference_manager: &PreferenceManager,
    auto_activate_config: &AutoActivateConfig,
) -> bool {
    // Global "never" can change between prompts — if set, always re-evaluate.
    if *auto_activate_config == AutoActivateConfig::Never {
        return false;
    }

    // Filter out excluded dirs (manually-activated subshell environments)
    // so the fast-path comparison matches the filtered active_dirs.
    let exclude_dirs: Vec<PathBuf> = std::env::var(HOOK_VAR_EXCLUDE_DIRS)
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();
    let discovered_dirs: Vec<PathBuf> = discovered
        .iter()
        .filter(|d| !exclude_dirs.contains(&d.path))
        .map(|d| d.path.clone())
        .collect();
    let watches_changed = state.watches_changed();
    let trust_changed = state
        .active_dirs
        .iter()
        .any(|dir| !matches!(trust_manager.check(dir), Ok(TrustStatus::Trusted)));
    let preference_changed = state
        .active_dirs
        .iter()
        .any(|dir| !matches!(preference_manager.check(dir), Ok(PreferenceStatus::Enabled)));
    state.last_cwd.as_deref() == Some(cwd)
        && !watches_changed
        && discovered_dirs == state.active_dirs
        && !trust_changed
        && !preference_changed
}

/// Filter discovered environments by preference, trust, and suppression status.
///
/// Two-gate model: an environment must have both (a) auto-activation enabled
/// (preference gate) and (b) be trusted (security gate) to be eligible.
///
/// Returns (eligible_dot_flox, suppressed_dirs, notified_dirs).
fn filter_by_eligibility(
    state: &HookState,
    cwd: &Path,
    discovered: &[DotFlox],
    trust_manager: &TrustManager,
    preference_manager: &PreferenceManager,
    auto_activate_config: &AutoActivateConfig,
) -> (Vec<DotFlox>, Vec<PathBuf>, Vec<PathBuf>) {
    // Prune suppressed dirs: only keep those that are still ancestors of CWD.
    let suppressed_dirs: Vec<PathBuf> = state
        .suppressed_dirs
        .iter()
        .filter(|s| cwd.starts_with(s.parent().unwrap_or(s)))
        .cloned()
        .collect();

    // Prune notified dirs to those still relevant to CWD. This allows
    // the Disabled-branch notice to re-appear when the user cd's back,
    // reinforcing that `flox enable` is available.
    let mut notified_dirs: Vec<PathBuf> = state
        .notified_dirs
        .iter()
        .filter(|n| cwd.starts_with(n.parent().unwrap_or(n)))
        .cloned()
        .collect();

    let mut eligible_dot_flox: Vec<DotFlox> = Vec::new();

    // Global "never" — skip all environments with a single notice.
    if *auto_activate_config == AutoActivateConfig::Never {
        if !discovered.is_empty() {
            emit_eligibility_notice(
                cwd,
                &discovered[0].path,
                "Auto-activation is disabled globally. Run 'flox config --set auto_activate prompt' to enable.",
                &mut notified_dirs,
            );
        }
        return (eligible_dot_flox, suppressed_dirs, notified_dirs);
    }

    for dot_flox in discovered {
        if suppressed_dirs.contains(&dot_flox.path) {
            debug!(path = %dot_flox.path.display(), "suppressed, skipping");
            continue;
        }

        // Gate 1: Preference check
        let preference_ok = match preference_manager.check(&dot_flox.path) {
            Ok(PreferenceStatus::Enabled) => true,
            Ok(PreferenceStatus::Disabled) => {
                debug!(path = %dot_flox.path.display(), "preference disabled, skipping");
                emit_eligibility_notice(
                    cwd,
                    &dot_flox.path,
                    "Auto-activation is disabled. Run 'flox enable' to re-enable.",
                    &mut notified_dirs,
                );
                continue;
            },
            Ok(PreferenceStatus::Unregistered) => {
                match auto_activate_config {
                    AutoActivateConfig::Always => true,
                    AutoActivateConfig::Prompt => {
                        // Already prompted/notified this session — don't prompt again
                        if notified_dirs.contains(&dot_flox.path.to_path_buf()) {
                            continue;
                        }
                        let is_local = matches!(dot_flox.pointer, EnvironmentPointer::Path(_));
                        if prompt_auto_activate(
                            &dot_flox.path,
                            preference_manager,
                            trust_manager,
                            is_local,
                        ) {
                            true
                        } else {
                            // Non-interactive fallback or user said no
                            emit_eligibility_notice(
                                cwd,
                                &dot_flox.path,
                                "Run 'flox enable' to auto-activate this environment.",
                                &mut notified_dirs,
                            );
                            continue;
                        }
                    },
                    AutoActivateConfig::Never => unreachable!(), // handled above
                }
            },
            Err(e) => {
                debug!(path = %dot_flox.path.display(), "preference check failed: {e}");
                continue;
            },
        };

        if !preference_ok {
            continue;
        }

        // Gate 2: Trust check
        // For local environments, trust is implicit (set by `flox enable`).
        let is_local = matches!(dot_flox.pointer, EnvironmentPointer::Path(_));
        if is_local {
            eligible_dot_flox.push(dot_flox.clone());
        } else {
            // Managed/remote environments require explicit trust
            match trust_manager.check(&dot_flox.path) {
                Ok(TrustStatus::Trusted) => {
                    eligible_dot_flox.push(dot_flox.clone());
                },
                Ok(TrustStatus::Denied) => {
                    debug!(path = %dot_flox.path.display(), "trust denied, skipping");
                    emit_eligibility_notice(
                        cwd,
                        &dot_flox.path,
                        "Environment is not trusted. Run 'flox activate -t' to trust it.",
                        &mut notified_dirs,
                    );
                },
                Ok(TrustStatus::Unknown(_)) => {
                    emit_eligibility_notice(
                        cwd,
                        &dot_flox.path,
                        "Environment is not trusted. Run 'flox activate -t' to trust it.",
                        &mut notified_dirs,
                    );
                },
                Err(e) => {
                    debug!(path = %dot_flox.path.display(), "trust check failed: {e}");
                },
            }
        }
    }

    (eligible_dot_flox, suppressed_dirs, notified_dirs)
}

/// Check whether we can prompt the user from within the hook.
///
/// In all shell hooks, `hook-env` runs inside `$()` so stdout is captured.
/// But stdin is inherited from the interactive shell and stderr goes to
/// the terminal, so we can prompt via stderr and read from stdin.
fn can_prompt_from_hook() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Prompt the user whether to auto-activate an environment.
/// Returns true if the user accepted. Persists the preference on acceptance
/// or decline (as Disabled), so the decision survives across shell sessions.
fn prompt_auto_activate(
    dot_flox_path: &Path,
    preference_manager: &PreferenceManager,
    trust_manager: &TrustManager,
    is_local: bool,
) -> bool {
    if !can_prompt_from_hook() {
        return false;
    }
    let project_dir = dot_flox_path.parent().unwrap_or(dot_flox_path);
    eprint!(
        "Auto-activate environment in {}? [y/N] ",
        project_dir.display()
    );
    let _ = std::io::stderr().flush();

    let mut input = String::new();
    match std::io::stdin().read_line(&mut input) {
        Ok(_) if input.trim().eq_ignore_ascii_case("y") => {
            // Persist the preference
            let _ = preference_manager.enable(dot_flox_path);
            if is_local {
                let _ = trust_manager.trust(dot_flox_path);
            }
            true
        },
        Ok(_) => {
            // User explicitly declined — persist so it survives across sessions.
            // Moves the environment from Unregistered → Disabled, so future visits
            // show a non-interactive notice instead of re-prompting.
            let _ = preference_manager.disable(dot_flox_path);
            false
        },
        Err(_) => false,
    }
}

/// Emit an eligibility notification message for an environment,
/// deduplicating by path.
fn emit_eligibility_notice(
    cwd: &Path,
    dot_flox_path: &Path,
    message: &str,
    notified_dirs: &mut Vec<PathBuf>,
) {
    if notified_dirs.contains(&dot_flox_path.to_path_buf()) {
        return;
    }
    let _ = cwd; // reserved for future use (e.g. ancestor-aware messaging)
    eprintln!(
        "flox: environment at '{}': {message}",
        dot_flox_path.display()
    );
    notified_dirs.push(dot_flox_path.to_path_buf());
}

/// Result of `manage_activations`: tracking state, combined env vars, PATH
/// additions, FLOX_ENV_DIRS additions, and updated watch entries.
type ActivationResult = (
    ActivationTracking,
    HashMap<String, String>,
    Vec<String>,
    Vec<String>,
    Vec<WatchEntry>,
);

/// Build new env vars from all trusted environments and manage activation
/// lifecycle (PID registration, executive spawning, detach/reattach).
fn manage_activations(
    state: &HookState,
    trusted_dot_flox: &[DotFlox],
    flox: &Flox,
) -> Result<ActivationResult> {
    let shell_pid = std::os::unix::process::parent_id() as i32;
    let prev_tracking = &state.activation_tracking;
    let mut new_tracking = ActivationTracking::default();

    let mut combined_env: HashMap<String, String> = HashMap::new();
    let mut path_additions: Vec<String> = Vec::new();
    let mut env_dirs_additions: Vec<String> = Vec::new();
    let mut new_watches: Vec<WatchEntry> = Vec::new();

    for dot_flox in trusted_dot_flox {
        // Watch the manifest file for changes.
        let manifest_path = dot_flox.path.join("env").join("manifest.toml");
        new_watches.push(WatchEntry {
            path: manifest_path.clone(),
            mtime: std::fs::metadata(&manifest_path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
        });

        match resolve_env_vars(dot_flox, flox) {
            Ok(resolved) => {
                // Check if this environment is already tracked with the same store path.
                // If so, skip the auto-start subprocess (common fast path for subsequent prompts).
                let already_tracked = prev_tracking
                    .entries
                    .get(&dot_flox.path)
                    .is_some_and(|info| info.store_path == resolved.store_path);

                // Check if this environment is in the detached cache (cd-away-and-back).
                let cached = prev_tracking
                    .detached_cache
                    .get(&dot_flox.path)
                    .filter(|info| info.store_path == resolved.store_path)
                    .cloned();

                let activation_state_dir =
                    activation_state_dir_path(&flox.runtime_dir, &dot_flox.path);

                if already_tracked {
                    // Case 1: Subsequent prompt, same dir - carry forward existing info
                    let info = prev_tracking.entries[&dot_flox.path].clone();
                    // Re-apply cached on-activate diff
                    apply_on_activate_diff(&info.on_activate_diff, &mut combined_env);
                    new_tracking.entries.insert(dot_flox.path.clone(), info);
                } else if let Some(cached_info) = cached {
                    // Case 2: cd-away-and-back - re-attach PID, use cached diff
                    let auto_result =
                        spawn_auto_start(shell_pid, dot_flox, &resolved, &activation_state_dir);

                    if auto_result.as_ref().is_some_and(|r| r.is_new) {
                        // Activation was recreated (e.g. after cleanup when all PIDs
                        // detached). Use the fresh result instead of stale cached info
                        // so that services are restarted and the new start_state_dir
                        // is tracked.
                        let result = auto_result.as_ref().unwrap();
                        apply_on_activate_diff(&result.hook_env_diff, &mut combined_env);
                        new_tracking
                            .entries
                            .insert(dot_flox.path.clone(), ActivationInfo {
                                activation_state_dir: activation_state_dir.clone(),
                                store_path: resolved.store_path.clone(),
                                start_state_dir: result.start_state_dir.as_ref().map(PathBuf::from),
                                on_activate_diff: result.hook_env_diff.clone(),
                            });
                    } else {
                        // Re-apply cached on-activate diff (hooks don't re-run)
                        apply_on_activate_diff(&cached_info.on_activate_diff, &mut combined_env);
                        new_tracking
                            .entries
                            .insert(dot_flox.path.clone(), cached_info);
                    }
                } else {
                    // Case 3: Truly new or store path changed - run hooks
                    let auto_result =
                        spawn_auto_start(shell_pid, dot_flox, &resolved, &activation_state_dir);

                    let (on_activate_diff, start_state_dir) = if let Some(ref result) = auto_result
                    {
                        // Merge on-activate env diff into combined_env
                        apply_on_activate_diff(&result.hook_env_diff, &mut combined_env);
                        (
                            result.hook_env_diff.clone(),
                            result.start_state_dir.as_ref().map(PathBuf::from),
                        )
                    } else {
                        (None, None)
                    };

                    new_tracking
                        .entries
                        .insert(dot_flox.path.clone(), ActivationInfo {
                            activation_state_dir,
                            store_path: resolved.store_path.clone(),
                            start_state_dir,
                            on_activate_diff,
                        });
                }

                // Collect the FLOX_ENV value for FLOX_ENV_DIRS computation.
                env_dirs_additions.push(resolved.flox_env.clone());

                for (k, v) in resolved.vars {
                    match k.as_str() {
                        "_FLOX_PATH_ADD" | "_FLOX_SBIN_ADD" => {
                            path_additions.push(v);
                        },
                        _ => {
                            combined_env.insert(k, v);
                        },
                    }
                }
            },
            Err(e) => {
                debug!(
                    path = %dot_flox.path.display(),
                    "failed to resolve environment: {e}"
                );
                eprintln!(
                    "flox: failed to resolve environment at '{}': {e}",
                    dot_flox.path.display()
                );
            },
        }
    }

    // Detach from environments that are no longer active and cache their info.
    for (path, info) in &prev_tracking.entries {
        if !new_tracking.entries.contains_key(path) {
            spawn_auto_detach(shell_pid, &info.activation_state_dir);
            // Move to detached cache so cd-back can reuse on_activate_diff
            new_tracking
                .detached_cache
                .insert(path.clone(), info.clone());
        }
    }

    // Carry forward detached cache entries whose activation state dir still exists.
    for (path, info) in &prev_tracking.detached_cache {
        if !new_tracking.entries.contains_key(path)
            && !new_tracking.detached_cache.contains_key(path)
            && info.activation_state_dir.exists()
        {
            new_tracking
                .detached_cache
                .insert(path.clone(), info.clone());
        }
    }

    Ok((
        new_tracking,
        combined_env,
        path_additions,
        env_dirs_additions,
        new_watches,
    ))
}

/// Compute the environment diff as if `emit_revert` had already run.
///
/// Because `hook-env` writes shell commands to stdout without modifying its
/// own process environment, we simulate the post-revert state by consulting
/// `old_diff` to recover original (pre-activation) values. The resulting
/// `HookDiff` can then be applied via `emit_apply` to produce the correct
/// shell state for the new set of environments.
///
/// Also merges all PATH additions from multiple environments into a single
/// PATH entry, prepended to the reverted baseline PATH.
fn compute_new_diff(
    old_diff: &HookDiff,
    mut combined_env: HashMap<String, String>,
    path_additions: Vec<String>,
    env_dirs_additions: Vec<String>,
) -> (HookDiff, HashMap<String, String>) {
    // Merge all environment bin/sbin dirs into a single PATH.
    // Use the *reverted* PATH (what it would be after undoing the previous
    // diff) so we don't stack new additions on top of stale entries.
    if !path_additions.is_empty() {
        let base_path = reverted_env_var("PATH", old_diff).unwrap_or_default();
        let new_path = format!("{}:{}", path_additions.join(":"), base_path);
        combined_env.insert("PATH".to_string(), new_path);
    }

    // Compute FLOX_ENV_DIRS from all auto-activated environments.
    // Use the on-activate value if present (hooks may have modified it),
    // otherwise fall back to the reverted baseline.
    if !env_dirs_additions.is_empty() {
        let base_env_dirs = combined_env
            .get("FLOX_ENV_DIRS")
            .cloned()
            .or_else(|| reverted_env_var("FLOX_ENV_DIRS", old_diff))
            .unwrap_or_default();
        let new_env_dirs = if base_env_dirs.is_empty() {
            env_dirs_additions.join(":")
        } else {
            format!("{}:{}", env_dirs_additions.join(":"), base_env_dirs)
        };
        combined_env.insert("FLOX_ENV_DIRS".to_string(), new_env_dirs);

        // Compute MANPATH from the new FLOX_ENV_DIRS.
        let base_manpath = combined_env
            .get("MANPATH")
            .cloned()
            .or_else(|| reverted_env_var("MANPATH", old_diff))
            .unwrap_or_default();
        let man_dirs: Vec<String> = env_dirs_additions
            .iter()
            .map(|d| format!("{d}/share/man"))
            .collect();
        let mut new_manpath = if base_manpath.is_empty() {
            man_dirs.join(":")
        } else {
            format!("{}:{}", man_dirs.join(":"), base_manpath)
        };
        // Ensure trailing colon so the standard man page search path is
        // included, matching fix_manpath_var behavior.
        let has_trailing = new_manpath.ends_with(':');
        let has_double = new_manpath.contains("::");
        let has_leading = new_manpath.starts_with(':');
        if !(has_trailing || has_double || has_leading) {
            new_manpath.push(':');
        }
        combined_env.insert("MANPATH".to_string(), new_manpath);
    }

    // Compute the new diff against the *reverted* process env.
    // We can't use std::env::var() directly because the process env still
    // reflects the previous activation — emit_revert only writes shell
    // commands to stdout without modifying this process.
    let mut additions = HashMap::new();
    let mut modifications = HashMap::new();

    for (key, new_val) in &combined_env {
        match reverted_env_var(key, old_diff) {
            Some(orig_val) if orig_val != *new_val => {
                modifications.insert(key.clone(), orig_val);
            },
            None => {
                additions.insert(key.clone(), new_val.clone());
            },
            _ => {},
        }
    }

    // Note: deletions are not needed here because emit_revert already
    // handles restoring/unsetting vars from the previous diff before
    // emit_apply runs.
    let new_diff = HookDiff {
        additions,
        modifications,
        deletions: HashMap::new(),
    };

    (new_diff, combined_env)
}

/// Apply cached on-activate hook env diff into the combined environment.
fn apply_on_activate_diff(
    diff: &Option<OnActivateEnvDiff>,
    combined_env: &mut HashMap<String, String>,
) {
    if let Some(diff) = diff {
        for (k, v) in &diff.additions {
            combined_env.insert(k.clone(), v.clone());
        }
        for k in &diff.deletions {
            combined_env.remove(k);
        }
    }
}

/// Compute the value an environment variable would have after reverting the
/// previous diff.  `emit_revert` writes shell code but does not modify this
/// process, so we need this to know the "pristine" baseline.
fn reverted_env_var(key: &str, old_diff: &HookDiff) -> Option<String> {
    if old_diff.additions.contains_key(key) {
        // Was added by the previous activation → unset after revert.
        None
    } else if let Some(orig_val) = old_diff.modifications.get(key) {
        // Was modified → restored to original value after revert.
        Some(orig_val.clone())
    } else if let Some(orig_val) = old_diff.deletions.get(key) {
        // Was deleted → re-exported after revert.
        Some(orig_val.clone())
    } else {
        // Not touched by the old diff → current process env value.
        std::env::var(key).ok()
    }
}

/// Result of resolving an environment, containing both env vars and metadata.
struct ResolvedEnv {
    /// Environment variables to set (including special _FLOX_PATH_ADD/_FLOX_SBIN_ADD keys)
    vars: HashMap<String, String>,
    /// Nix store path for the built environment
    store_path: String,
    /// Mode link path (FLOX_ENV value)
    flox_env: String,
    /// Cache path for the environment
    env_cache: PathBuf,
    /// Log directory for the environment
    flox_env_log_dir: PathBuf,
    /// Project path for the environment
    env_project: PathBuf,
    /// Services socket path
    flox_services_socket: PathBuf,
    /// Service names to start with process-compose
    services_to_start: Vec<String>,
    /// CUDA detection setting from manifest options ("0" or "1")
    cuda_detection: String,
}

/// Resolve environment variables from a built environment.
fn resolve_env_vars(dot_flox: &DotFlox, flox: &Flox) -> Result<ResolvedEnv> {
    let mut env = UninitializedEnvironment::DotFlox(dot_flox.clone())
        .into_concrete_environment(flox, None)?;

    // Ensure the environment is locked and built.
    // These calls can trigger a Nix build on cold cache, so emit a progress
    // message so the user knows why their prompt is delayed.
    eprintln!(
        "flox: resolving environment '{}'...",
        dot_flox.pointer.name()
    );
    let lock_result = env.lockfile(flox)?;
    let lockfile: Lockfile = lock_result.into();
    let (services_to_start, cuda_detection) =
        match lockfile.manifest.migrate_typed_only(Some(&lockfile)) {
            Ok(manifest) => {
                let auto_start = manifest.options().services.auto_start.unwrap_or(false);
                let services = if auto_start {
                    let services_for_system = manifest.services().copy_for_system(&flox.system);
                    services_for_system
                        .inner()
                        .keys()
                        .cloned()
                        .collect::<Vec<String>>()
                } else {
                    Vec::new()
                };
                let cuda = manifest.options().cuda_detection;
                (services, cuda)
            },
            Err(e) => {
                debug!("failed to read services from manifest: {e}");
                (Vec::new(), None)
            },
        };

    let cuda_detection_str = match cuda_detection {
        Some(false) => "0".to_string(),
        _ => "1".to_string(),
    };

    let rendered_links = env.rendered_env_links(flox)?;
    let link = rendered_links.for_mode(&ActivateMode::Dev);

    // Collect environment metadata for activation lifecycle.
    let env_cache = env.cache_path()?.into_inner();
    let flox_env_log_dir = env.log_path()?.to_path_buf();
    let env_project = env.project_path()?;
    let flox_services_socket = env.services_socket_path(flox)?;

    // Resolve the symlink to the actual store path.
    let link_path: &std::path::Path = link.as_ref();
    let store_path = std::fs::read_link(link_path).unwrap_or_else(|_| link_path.to_path_buf());

    let mut vars = HashMap::new();

    // Set FLOX_ENV to the link path.
    let flox_env = link_path.display().to_string();
    vars.insert("FLOX_ENV".to_string(), flox_env.clone());

    // Set additional user-facing environment variables.
    vars.insert(
        "FLOX_ENV_CACHE".to_string(),
        env_cache.display().to_string(),
    );
    vars.insert(
        "FLOX_ENV_PROJECT".to_string(),
        env_project.display().to_string(),
    );
    vars.insert(
        "FLOX_ENV_DESCRIPTION".to_string(),
        dot_flox.pointer.name().to_string(),
    );
    vars.insert(
        "_FLOX_ENV_CUDA_DETECTION".to_string(),
        cuda_detection_str.clone(),
    );

    // Collect bin and sbin directories as PATH additions.
    // These are returned in the vars map under a special key and merged by the
    // caller so that multiple environments contribute to a single PATH.
    let bin = store_path.join("bin");
    if bin.exists() {
        vars.insert("_FLOX_PATH_ADD".to_string(), bin.display().to_string());
    }
    let sbin = store_path.join("sbin");
    if sbin.exists() {
        vars.insert("_FLOX_SBIN_ADD".to_string(), sbin.display().to_string());
    }

    // Parse activate.d/envrc for exported variables.
    //
    // INVARIANT: envrc is always generated by Nix (via the Flox environment
    // builder) in the exact format `export NAME="VALUE"` — one per line,
    // always double-quoted, no escaped quotes or multi-line values. This
    // regex relies on that machine-generated format. If the envrc format
    // ever changes, this parser must be updated to match.
    let envrc = store_path.join("activate.d").join("envrc");
    if envrc.exists()
        && let Ok(contents) = std::fs::read_to_string(&envrc)
    {
        static EXPORT_RE: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(r#"^export\s+([A-Za-z_][A-Za-z0-9_]*)="(.*)"$"#).expect("valid regex")
        });
        let export_re = &*EXPORT_RE;
        for line in contents.lines() {
            if let Some(caps) = export_re.captures(line) {
                let name = caps[1].to_string();
                let value = caps[2].to_string();
                if name != "PATH" {
                    vars.insert(name, value);
                }
            }
        }
    }

    Ok(ResolvedEnv {
        vars,
        store_path: store_path.display().to_string(),
        flox_env,
        env_cache,
        flox_env_log_dir,
        env_project,
        flox_services_socket,
        services_to_start,
        cuda_detection: cuda_detection_str,
    })
}

/// Emit shell commands to revert the previous HookDiff.
pub(crate) fn emit_revert(diff: &HookDiff, shell: Shell, writer: &mut impl Write) -> Result<()> {
    // Unset additions (they were added, so remove them).
    for name in diff.additions.keys() {
        UnsetVar::new(name).generate_with_newline(shell, writer)?;
    }
    // Restore modifications to their original values.
    for (name, orig_val) in &diff.modifications {
        SetVar::exported_no_expansion(name, orig_val).generate_with_newline(shell, writer)?;
    }
    // Restore deletions (they were deleted, so re-export them).
    for (name, orig_val) in &diff.deletions {
        SetVar::exported_no_expansion(name, orig_val).generate_with_newline(shell, writer)?;
    }
    // Restore the saved prompt.
    emit_prompt_restore(shell, writer)?;
    Ok(())
}

/// Emit shell commands to apply a new HookDiff.
fn emit_apply(
    diff: &HookDiff,
    new_env: &HashMap<String, String>,
    shell: Shell,
    writer: &mut impl Write,
) -> Result<()> {
    for (name, val) in &diff.additions {
        SetVar::exported_no_expansion(name, val).generate_with_newline(shell, writer)?;
    }
    for name in diff.modifications.keys() {
        if let Some(new_val) = new_env.get(name) {
            SetVar::exported_no_expansion(name, new_val).generate_with_newline(shell, writer)?;
        }
    }
    Ok(())
}

/// All state needed to emit `_FLOX_HOOK_*` variables in a single struct.
#[derive(Debug)]
struct HookStateUpdate<'a> {
    diff: &'a HookDiff,
    active_dirs: &'a [PathBuf],
    watches: &'a [WatchEntry],
    suppressed_dirs: &'a [PathBuf],
    notified_dirs: &'a [PathBuf],
    cwd: &'a Path,
    trusted_dot_flox: &'a [DotFlox],
    /// .flox dirs that were auto-activated on the *previous* hook-env call.
    prev_active_dirs: &'a [PathBuf],
    /// Per-environment activation tracking for PID lifecycle management.
    activation_tracking: &'a ActivationTracking,
}

/// Emit updated _FLOX_HOOK_* state variables.
fn emit_state_vars(
    update: &HookStateUpdate<'_>,
    shell: Shell,
    writer: &mut impl Write,
) -> Result<()> {
    let diff_encoded = update.diff.serialize()?;
    SetVar::exported_no_expansion(HOOK_VAR_DIFF, &diff_encoded)
        .generate_with_newline(shell, writer)?;

    let dirs_str = HookState::format_path_list(update.active_dirs);
    SetVar::exported_no_expansion(HOOK_VAR_DIRS, &dirs_str).generate_with_newline(shell, writer)?;

    let watches_json =
        serde_json::to_string(update.watches).context("failed to serialize watches")?;
    SetVar::exported_no_expansion(HOOK_VAR_WATCHES, &watches_json)
        .generate_with_newline(shell, writer)?;

    let suppressed_str = HookState::format_path_list(update.suppressed_dirs);
    SetVar::exported_no_expansion(HOOK_VAR_SUPPRESSED, &suppressed_str)
        .generate_with_newline(shell, writer)?;

    let notified_str = HookState::format_path_list(update.notified_dirs);
    SetVar::exported_no_expansion(HOOK_VAR_NOTIFIED, &notified_str)
        .generate_with_newline(shell, writer)?;

    SetVar::exported_no_expansion(HOOK_VAR_CWD, update.cwd.display().to_string())
        .generate_with_newline(shell, writer)?;

    // Build _FLOX_ACTIVE_ENVIRONMENTS from the auto-activated environments,
    // preserving any manually-activated environments that were already in the
    // list but are not managed by auto-activation.
    let mut active_envs = activated_environments();

    // Remove entries that were previously auto-activated OR are about to be
    // re-added.  This ensures that environments we are no longer cd'd into
    // get removed, while manually-activated environments are preserved.
    let new_auto_paths: Vec<PathBuf> = update
        .trusted_dot_flox
        .iter()
        .map(|d| d.path.clone())
        .collect();

    active_envs.retain(|env| {
        if let UninitializedEnvironment::DotFlox(d) = env {
            !new_auto_paths.contains(&d.path) && !update.prev_active_dirs.contains(&d.path)
        } else {
            true
        }
    });

    // Prepend auto-activated environments.  trusted_dot_flox is outermost-
    // first, so iterating forward and using push_front puts the innermost
    // (CWD-nearest) environment at the front, matching `last_active()`.
    for dot_flox in update.trusted_dot_flox.iter() {
        active_envs.set_last_active(
            UninitializedEnvironment::DotFlox(dot_flox.clone()),
            None,
            ActivateMode::Dev,
        );
    }

    SetVar::exported_no_expansion(FLOX_ACTIVE_ENVIRONMENTS_VAR, active_envs.to_string())
        .generate_with_newline(shell, writer)?;

    // Emit activation tracking for PID lifecycle management.
    let activations_encoded = update
        .activation_tracking
        .serialize()
        .context("failed to serialize activation tracking")?;
    SetVar::exported_no_expansion(HOOK_VAR_ACTIVATIONS, &activations_encoded)
        .generate_with_newline(shell, writer)?;

    Ok(())
}

/// Build a list of environment names for the prompt, combining manually-activated
/// (excluded) names with auto-activated names.
fn build_prompt_names(trusted_dot_flox: &[DotFlox]) -> Vec<String> {
    // Excluded env names come first (innermost, manually-activated).
    let exclude_names: Vec<String> = std::env::var(HOOK_VAR_EXCLUDE_NAMES)
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    // Auto-activated env names (innermost-first = reversed discovery order).
    let auto_names: Vec<String> = trusted_dot_flox
        .iter()
        .rev()
        .map(|d| d.pointer.name().to_string())
        .collect();

    // Combined: excluded first, then auto-activated.
    [exclude_names, auto_names].concat()
}

/// Emit shell-specific code to modify the prompt with active environment names.
/// If `env_names` is empty, only the restore is emitted (via emit_prompt_restore).
///
/// Respects:
/// - `_FLOX_SET_PROMPT`: if "false", skip prompt modification
/// - `NO_COLOR`: if set to a non-empty, non-"0" value, emit plain text
/// - `FLOX_PROMPT`: custom prefix text (default "flox")
fn emit_prompt(env_names: &[String], shell: Shell, writer: &mut impl Write) -> Result<()> {
    if env_names.is_empty() {
        return Ok(());
    }

    // Check _FLOX_SET_PROMPT: skip prompt modification if set to "false".
    if std::env::var("_FLOX_SET_PROMPT").as_deref() == Ok("false") {
        return Ok(());
    }

    // Sanitize environment names: replace any character not in [A-Za-z0-9_-]
    // with `_` to prevent shell injection via malicious env.json names.
    let sanitized: Vec<String> = env_names
        .iter()
        .map(|name| {
            name.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect()
        })
        .collect();
    let env_list = sanitized.join(" ");

    // Check NO_COLOR: suppress ANSI colors when set to a non-empty, non-"0"
    // value, matching manual activation's set-prompt.bash/zsh behavior.
    let no_color = std::env::var("NO_COLOR")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);

    // Check FLOX_PROMPT for custom prefix (default "flox").
    let flox_prompt = std::env::var("FLOX_PROMPT").unwrap_or_else(|_| "flox".to_string());

    let color1 = INDIGO_400.to_ansi256();
    let color2 = INDIGO_300.to_ansi256();

    match shell {
        Shell::Zsh => {
            if no_color {
                writeln!(
                    writer,
                    r#"if [ -z "${{_FLOX_HOOK_SAVE_PS1+x}}" ]; then _FLOX_HOOK_SAVE_PS1="$PS1"; fi;
PS1="{flox_prompt} [{env_list}] $_FLOX_HOOK_SAVE_PS1";"#,
                )?;
            } else {
                writeln!(
                    writer,
                    r#"if [ -z "${{_FLOX_HOOK_SAVE_PS1+x}}" ]; then _FLOX_HOOK_SAVE_PS1="$PS1"; fi;
PS1="%B%F{{{color1}}}{flox_prompt}%f%b %F{{{color2}}}[{env_list}]%f $_FLOX_HOOK_SAVE_PS1";"#,
                    color1 = color1,
                    color2 = color2,
                )?;
            }
        },
        Shell::Bash => {
            if no_color {
                writeln!(
                    writer,
                    r#"if [ -z "${{_FLOX_HOOK_SAVE_PS1+x}}" ]; then _FLOX_HOOK_SAVE_PS1="$PS1"; fi;
_flox_prefix="{flox_prompt} [{env_list}] ";
case "$_FLOX_HOOK_SAVE_PS1" in *\\n*) PS1="${{_FLOX_HOOK_SAVE_PS1/\\n/\\n$_flox_prefix}}" ;; *\\012*) PS1="${{_FLOX_HOOK_SAVE_PS1/\\012/\\012$_flox_prefix}}" ;; *) PS1="$_flox_prefix$_FLOX_HOOK_SAVE_PS1" ;; esac;
unset _flox_prefix;"#,
                )?;
            } else {
                writeln!(
                    writer,
                    r#"if [ -z "${{_FLOX_HOOK_SAVE_PS1+x}}" ]; then _FLOX_HOOK_SAVE_PS1="$PS1"; fi;
_flox_prefix="\[\x1b[1m\]\[\x1b[38;5;{color1}m\]{flox_prompt}\[\x1b[0m\] \[\x1b[38;5;{color2}m\][{env_list}]\[\x1b[0m\] ";
case "$_FLOX_HOOK_SAVE_PS1" in *\\n*) PS1="${{_FLOX_HOOK_SAVE_PS1/\\n/\\n$_flox_prefix}}" ;; *\\012*) PS1="${{_FLOX_HOOK_SAVE_PS1/\\012/\\012$_flox_prefix}}" ;; *) PS1="$_flox_prefix$_FLOX_HOOK_SAVE_PS1" ;; esac;
unset _flox_prefix;"#,
                    color1 = color1,
                    color2 = color2,
                )?;
            }
        },
        Shell::Fish => {
            if no_color {
                writeln!(
                    writer,
                    r#"if not set -q _FLOX_HOOK_SAVE_PROMPT; functions -q fish_prompt; and functions --copy fish_prompt _flox_hook_saved_prompt; set -g _FLOX_HOOK_SAVE_PROMPT 1; end;
function fish_prompt; echo -n '{flox_prompt} [{env_list}] '; _flox_hook_saved_prompt; end;"#,
                )?;
            } else {
                writeln!(
                    writer,
                    r#"if not set -q _FLOX_HOOK_SAVE_PROMPT; functions -q fish_prompt; and functions --copy fish_prompt _flox_hook_saved_prompt; set -g _FLOX_HOOK_SAVE_PROMPT 1; end;
function fish_prompt; set_color --bold; set_color 875fff; echo -n '{flox_prompt}'; set_color normal; echo -n ' '; set_color af87ff; echo -n '[{env_list}]'; set_color normal; echo -n ' '; _flox_hook_saved_prompt; end;"#,
                )?;
            }
        },
        Shell::Tcsh => {
            if no_color {
                writeln!(
                    writer,
                    r#"if ( ! $?_FLOX_HOOK_SAVE_PROMPT ) setenv _FLOX_HOOK_SAVE_PROMPT "$prompt";
set prompt = "{flox_prompt} [{env_list}] $_FLOX_HOOK_SAVE_PROMPT";"#,
                )?;
            } else {
                writeln!(
                    writer,
                    r#"if ( ! $?_FLOX_HOOK_SAVE_PROMPT ) setenv _FLOX_HOOK_SAVE_PROMPT "$prompt";
set prompt = "%{{\033[1m\033[38;5;{color1}m%}}{flox_prompt}%{{\033[0m%}} %{{\033[38;5;{color2}m%}}[{env_list}]%{{\033[0m%}} $_FLOX_HOOK_SAVE_PROMPT";"#,
                    color1 = color1,
                    color2 = color2,
                )?;
            }
        },
    }
    Ok(())
}

/// Emit shell-specific code to restore the prompt to its original value.
pub(crate) fn emit_prompt_restore(shell: Shell, writer: &mut impl Write) -> Result<()> {
    match shell {
        Shell::Zsh | Shell::Bash => {
            writeln!(
                writer,
                r#"if [ -n "${{_FLOX_HOOK_SAVE_PS1+x}}" ]; then PS1="$_FLOX_HOOK_SAVE_PS1"; unset _FLOX_HOOK_SAVE_PS1; fi;"#,
            )?;
        },
        Shell::Fish => {
            writeln!(
                writer,
                r#"if set -q _FLOX_HOOK_SAVE_PROMPT; functions -q _flox_hook_saved_prompt; and functions --copy _flox_hook_saved_prompt fish_prompt; functions --erase _flox_hook_saved_prompt; set -e _FLOX_HOOK_SAVE_PROMPT; end;"#,
            )?;
        },
        Shell::Tcsh => {
            writeln!(
                writer,
                r#"if ( $?_FLOX_HOOK_SAVE_PROMPT ) then; set prompt = "$_FLOX_HOOK_SAVE_PROMPT"; unsetenv _FLOX_HOOK_SAVE_PROMPT; endif;"#,
            )?;
        },
    }
    Ok(())
}

/// Re-trust an environment after a manifest change so auto-activation isn't
/// revoked. Logs on failure rather than propagating the error.
pub(crate) fn trust_or_log(data_dir: impl AsRef<Path>, dot_flox_path: impl AsRef<Path>) {
    let trust_mgr = TrustManager::new(data_dir);
    if let Err(e) = trust_mgr.trust(dot_flox_path) {
        tracing::debug!("failed to re-trust environment: {e}");
    }
}

/// Spawn `flox-activations auto-start` to register the shell PID, spawn an
/// executive, run on-activate hooks, and optionally start services.
/// Returns `AutoStartResult` on success, or `None` on failure (non-fatal).
fn spawn_auto_start(
    shell_pid: i32,
    dot_flox: &DotFlox,
    resolved: &ResolvedEnv,
    activation_state_dir: &Path,
) -> Option<AutoStartResult> {
    let ctx = AutoStartCtx {
        attach_ctx: AttachCtx {
            env: resolved.flox_env.clone(),
            env_cache: resolved.env_cache.clone(),
            env_description: dot_flox.pointer.name().to_string(),
            flox_active_environments: String::new(),
            prompt_color_1: String::new(),
            prompt_color_2: String::new(),
            flox_prompt_environments: String::new(),
            set_prompt: false,
            flox_env_cuda_detection: resolved.cuda_detection.clone(),
            interpreter_path: FLOX_INTERPRETER.clone(),
        },
        project_ctx: AttachProjectCtx {
            env_project: resolved.env_project.clone(),
            dot_flox_path: dot_flox.path.clone(),
            flox_env_log_dir: resolved.flox_env_log_dir.clone(),
            process_compose_bin: PathBuf::from(&*PROCESS_COMPOSE_BIN),
            flox_services_socket: resolved.flox_services_socket.clone(),
            services_to_start: resolved.services_to_start.clone(),
        },
        store_path: resolved.store_path.clone(),
        activation_state_dir: activation_state_dir.to_path_buf(),
        mode: ActivateMode::Dev,
        metrics_uuid: None,
    };

    // Write context to a temp file
    let temp_file = match tempfile::NamedTempFile::with_prefix_in(
        "auto_start_ctx_",
        activation_state_dir
            .parent()
            .unwrap_or(activation_state_dir),
    ) {
        Ok(f) => f,
        Err(e) => {
            error!("failed to create temp file for auto-start: {e}");
            return None;
        },
    };

    if let Err(e) = serde_json::to_writer(&temp_file, &ctx) {
        error!("failed to write auto-start context: {e}");
        return None;
    }

    let ctx_path = temp_file.path().to_path_buf();
    if let Err(e) = temp_file.keep() {
        error!("failed to persist auto-start context file: {e}");
        return None;
    }

    let mut child = match Command::new(&*FLOX_ACTIVATIONS_BIN)
        .args([
            "auto-start",
            "--pid",
            &shell_pid.to_string(),
            "--activate-data",
            &ctx_path.to_string_lossy(),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "flox: warning: failed to start activation for '{}': {e}",
                dot_flox.path.display()
            );
            let _ = std::fs::remove_file(&ctx_path);
            return None;
        },
    };

    // Read stdout/stderr in background threads to prevent pipe buffer deadlock.
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let stdout_handle = std::thread::spawn(move || {
        stdout_pipe
            .map(|mut p| {
                let mut buf = Vec::new();
                let _ = std::io::Read::read_to_end(&mut p, &mut buf);
                buf
            })
            .unwrap_or_default()
    });
    let stderr_handle = std::thread::spawn(move || {
        stderr_pipe
            .map(|mut p| {
                let mut buf = Vec::new();
                let _ = std::io::Read::read_to_end(&mut p, &mut buf);
                buf
            })
            .unwrap_or_default()
    });

    // Poll for exit with a timeout to prevent the shell prompt from freezing
    // indefinitely if auto-start hangs (e.g. on-activate hook loops, Nix
    // store lock contention, or service startup deadlock).
    const AUTO_START_TIMEOUT: Duration = Duration::from_secs(30);
    let deadline = Instant::now() + AUTO_START_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait(); // reap zombie
                    eprintln!(
                        "flox: activation timed out for '{}' ({}s limit)",
                        dot_flox.path.display(),
                        AUTO_START_TIMEOUT.as_secs()
                    );
                    let _ = std::fs::remove_file(&ctx_path);
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            },
            Err(e) => {
                eprintln!(
                    "flox: warning: failed to wait for activation of '{}': {e}",
                    dot_flox.path.display()
                );
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&ctx_path);
                return None;
            },
        }
    };

    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();

    // Clean up temp file if it still exists (auto-start normally removes it)
    let _ = std::fs::remove_file(&ctx_path);

    let status = status.unwrap();
    if status.success() {
        let stdout_str = String::from_utf8_lossy(&stdout);
        match serde_json::from_str::<AutoStartResult>(stdout_str.trim()) {
            Ok(auto_result) => {
                debug!(
                    path = %dot_flox.path.display(),
                    is_new = auto_result.is_new,
                    "auto-start completed successfully"
                );
                Some(auto_result)
            },
            Err(e) => {
                eprintln!(
                    "flox: warning: on-activate hooks and services may be skipped for '{}' (parse error)",
                    dot_flox.path.display()
                );
                debug!(
                    path = %dot_flox.path.display(),
                    "failed to parse auto-start result: {e}"
                );
                None
            },
        }
    } else {
        let stderr_str = String::from_utf8_lossy(&stderr);
        if !stderr_str.is_empty() {
            eprintln!("{}", stderr_str.trim());
        }
        debug!(
            path = %dot_flox.path.display(),
            code = ?status.code(),
            "auto-start exited with non-zero status"
        );
        None
    }
}

/// Spawn `flox-activations auto-detach` to remove the shell PID from activation
/// state. Fire-and-forget: errors are logged to debug.
pub(crate) fn spawn_auto_detach(shell_pid: i32, activation_state_dir: &Path) {
    let result = Command::new(&*FLOX_ACTIVATIONS_BIN)
        .args([
            "auto-detach",
            "--pid",
            &shell_pid.to_string(),
            "--activation-state-dir",
            &activation_state_dir.to_string_lossy(),
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status();

    match result {
        Ok(status) if status.success() => {
            debug!("auto-detach completed successfully");
        },
        Ok(status) => {
            debug!(
                code = ?status.code(),
                "auto-detach exited with non-zero status"
            );
        },
        Err(e) => {
            debug!("failed to spawn auto-detach: {e}");
        },
    }
}
