use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_core::activate::mode::ActivateMode;
use flox_core::activate::vars::FLOX_ACTIVE_ENVIRONMENTS_VAR;
use flox_core::hook_state::{
    HOOK_VAR_CWD,
    HOOK_VAR_DIFF,
    HOOK_VAR_DIRS,
    HOOK_VAR_NOTIFIED,
    HOOK_VAR_SUPPRESSED,
    HOOK_VAR_WATCHES,
    HookDiff,
    HookState,
    WatchEntry,
};
use flox_core::trust::{TrustManager, TrustStatus};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{
    DotFlox,
    Environment,
    UninitializedEnvironment,
    find_all_dot_flox,
};
use regex::Regex;
use shell_gen::{GenerateShell, SetVar, Shell, UnsetVar};
use tracing::debug;

use crate::utils::active_environments::activated_environments;
use crate::utils::colors::{INDIGO_300, INDIGO_400};

#[derive(Bpaf, Clone, Debug)]
pub struct HookEnv {
    /// Shell to emit hook-env code for (bash, zsh, fish, tcsh)
    #[bpaf(long("shell"), argument("SHELL"))]
    shell: String,
}

impl HookEnv {
    pub fn handle(self, flox: Flox) -> Result<()> {
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

        // Fast path: CWD unchanged AND no watched files changed AND the set of
        // discovered .flox dirs hasn't changed → exit with no output.
        // We must check discovered dirs so that a new `flox init` in the
        // current directory is detected without requiring a `cd` away and back.
        let discovered_dirs: Vec<PathBuf> = discovered.iter().map(|d| d.path.clone()).collect();
        let watches_changed = state.watches_changed();
        if state.last_cwd.as_ref() == Some(&cwd)
            && !watches_changed
            && discovered_dirs == state.active_dirs
        {
            return Ok(());
        }

        let trust_manager = TrustManager::new(&flox.data_dir);

        // Prune suppressed dirs: only keep those that are still ancestors of CWD.
        let suppressed_dirs: Vec<PathBuf> = state
            .suppressed_dirs
            .iter()
            .filter(|s| cwd.starts_with(s.parent().unwrap_or(s)))
            .cloned()
            .collect();

        // Filter discovered envs by trust and suppression.
        let mut trusted_dot_flox: Vec<DotFlox> = Vec::new();
        let mut notified_dirs = state.notified_dirs.clone();

        for dot_flox in &discovered {
            if suppressed_dirs.contains(&dot_flox.path) {
                debug!(path = %dot_flox.path.display(), "suppressed, skipping");
                continue;
            }

            match trust_manager.check(&dot_flox.path) {
                Ok(TrustStatus::Trusted) => {
                    trusted_dot_flox.push(dot_flox.clone());
                },
                Ok(TrustStatus::Denied) => {
                    debug!(path = %dot_flox.path.display(), "denied, skipping");
                },
                Ok(TrustStatus::Unknown(_)) => {
                    if !notified_dirs.contains(&dot_flox.path) {
                        let is_ancestor =
                            cwd.starts_with(dot_flox.path.parent().unwrap_or(&dot_flox.path));
                        if is_ancestor {
                            eprintln!(
                                "flox: environment at '{}' is not trusted. Run 'flox trust' to auto-activate it.",
                                dot_flox.path.display()
                            );
                        } else {
                            eprintln!(
                                "flox: environment at '{}' is not trusted. Run 'flox trust --path {}' to auto-activate it.",
                                dot_flox.path.display(),
                                dot_flox.path.display()
                            );
                        }
                        notified_dirs.push(dot_flox.path.clone());
                    }
                },
                Err(e) => {
                    debug!(path = %dot_flox.path.display(), "trust check failed: {e}");
                },
            }
        }

        let new_active_dirs: Vec<PathBuf> =
            trusted_dot_flox.iter().map(|d| d.path.clone()).collect();

        // Check if the set of active dirs actually changed.
        let dirs_changed = new_active_dirs != state.active_dirs;

        if !dirs_changed && !watches_changed && state.last_cwd.as_ref() == Some(&cwd) {
            // Nothing changed, just update CWD tracking.
            return Ok(());
        }

        let mut stdout = std::io::stdout().lock();

        // Step 1: Revert previous diff.
        emit_revert(&state.diff, shell, &mut stdout)?;

        // Step 2: Build new env vars from all trusted environments.
        let mut combined_env: HashMap<String, String> = HashMap::new();
        let mut path_additions: Vec<String> = Vec::new();
        let mut new_watches: Vec<WatchEntry> = Vec::new();

        for dot_flox in &trusted_dot_flox {
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

            match resolve_env_vars(dot_flox, &flox) {
                Ok(env_vars) => {
                    for (k, v) in env_vars {
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

        // Merge all environment bin/sbin dirs into a single PATH.
        // Use the *reverted* PATH (what it would be after undoing the previous
        // diff) so we don't stack new additions on top of stale entries.
        if !path_additions.is_empty() {
            let base_path = reverted_env_var("PATH", &state.diff).unwrap_or_default();
            let new_path = format!("{}:{}", path_additions.join(":"), base_path);
            combined_env.insert("PATH".to_string(), new_path);
        }

        // Step 3: Compute the new diff against the *reverted* process env
        // (what the env would look like after emit_revert runs in the shell).
        // We can't use std::env::var() directly because the process env still
        // reflects the previous activation — emit_revert only writes shell
        // commands to stdout without modifying this process.
        let new_diff = {
            let mut additions = HashMap::new();
            let mut modifications = HashMap::new();

            for (key, new_val) in &combined_env {
                match reverted_env_var(key, &state.diff) {
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
            // emit_apply runs.  The new diff only needs to track what the new
            // activation adds or modifies relative to the pristine state.
            HookDiff {
                additions,
                modifications,
                deletions: HashMap::new(),
            }
        };

        // Step 4: Emit new exports.
        emit_apply(&new_diff, &combined_env, shell, &mut stdout)?;

        // Step 5: Emit prompt modification.
        let env_names: Vec<String> = trusted_dot_flox
            .iter()
            .map(|d| d.pointer.name().to_string())
            .collect();
        emit_prompt(&env_names, shell, &mut stdout)?;

        // Step 6: Emit updated state variables.
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
            },
            shell,
            &mut stdout,
        )?;

        Ok(())
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

/// Resolve environment variables from a built environment.
fn resolve_env_vars(dot_flox: &DotFlox, flox: &Flox) -> Result<HashMap<String, String>> {
    let mut env = UninitializedEnvironment::DotFlox(dot_flox.clone())
        .into_concrete_environment(flox, None)?;

    // Ensure the environment is locked and built.
    env.lockfile(flox)?;
    let rendered_links = env.rendered_env_links(flox)?;
    let link = rendered_links.for_mode(&ActivateMode::Dev);

    // Resolve the symlink to the actual store path.
    let link_path: &std::path::Path = link.as_ref();
    let store_path = std::fs::read_link(link_path).unwrap_or_else(|_| link_path.to_path_buf());

    let mut vars = HashMap::new();

    // Set FLOX_ENV to the link path.
    vars.insert("FLOX_ENV".to_string(), link_path.display().to_string());

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

    Ok(vars)
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

    Ok(())
}

/// Emit shell-specific code to modify the prompt with active environment names.
/// If `env_names` is empty, only the restore is emitted (via emit_prompt_restore).
fn emit_prompt(env_names: &[String], shell: Shell, writer: &mut impl Write) -> Result<()> {
    if env_names.is_empty() {
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
    let color1 = INDIGO_400.to_ansi256();
    let color2 = INDIGO_300.to_ansi256();

    match shell {
        Shell::Zsh => {
            writeln!(
                writer,
                r#"if [ -z "${{_FLOX_HOOK_SAVE_PS1+x}}" ]; then _FLOX_HOOK_SAVE_PS1="$PS1"; fi;
PS1="%B%F{{{color1}}}flox%f%b %F{{{color2}}}[{env_list}]%f $_FLOX_HOOK_SAVE_PS1";"#,
                color1 = color1,
                color2 = color2,
                env_list = env_list,
            )?;
        },
        Shell::Bash => {
            writeln!(
                writer,
                r#"if [ -z "${{_FLOX_HOOK_SAVE_PS1+x}}" ]; then _FLOX_HOOK_SAVE_PS1="$PS1"; fi;
PS1="\[\x1b[1m\]\[\x1b[38;5;{color1}m\]flox\[\x1b[0m\] \[\x1b[38;5;{color2}m\][{env_list}]\[\x1b[0m\] $_FLOX_HOOK_SAVE_PS1";"#,
                color1 = color1,
                color2 = color2,
                env_list = env_list,
            )?;
        },
        Shell::Fish => {
            writeln!(
                writer,
                r#"if not set -q _FLOX_HOOK_SAVE_PROMPT; functions -q fish_prompt; and functions --copy fish_prompt _flox_hook_saved_prompt; set -g _FLOX_HOOK_SAVE_PROMPT 1; end;
function fish_prompt; set_color --bold; set_color 875fff; echo -n 'flox'; set_color normal; echo -n ' '; set_color af87ff; echo -n '[{env_list}]'; set_color normal; echo -n ' '; _flox_hook_saved_prompt; end;"#,
                env_list = env_list,
            )?;
        },
        Shell::Tcsh => {
            writeln!(
                writer,
                r#"if ( ! $?_FLOX_HOOK_SAVE_PROMPT ) setenv _FLOX_HOOK_SAVE_PROMPT "$prompt";
set prompt = "%{{\033[1m\033[38;5;{color1}m%}}flox%{{\033[0m%}} %{{\033[38;5;{color2}m%}}[{env_list}]%{{\033[0m%}} $_FLOX_HOOK_SAVE_PROMPT";"#,
                color1 = color1,
                color2 = color2,
                env_list = env_list,
            )?;
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
