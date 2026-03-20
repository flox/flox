use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_core::activate::mode::ActivateMode;
use flox_core::hook_state::{HookDiff, HookState, WatchEntry};
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

        // Fast path: CWD unchanged AND no watched files changed → exit with no output.
        if state.last_cwd.as_ref() == Some(&cwd) && !state.watches_changed() {
            return Ok(());
        }

        // Discover .flox dirs in ancestor chain.
        let discovered = find_all_dot_flox(&cwd).unwrap_or_else(|e| {
            debug!("find_all_dot_flox failed: {e}");
            Vec::new()
        });

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
                        eprintln!(
                            "flox: environment at '{}' is not trusted. Run 'flox trust --path {}' to auto-activate it.",
                            dot_flox.path.display(),
                            dot_flox.path.display()
                        );
                        notified_dirs.push(dot_flox.path.clone());
                    }
                },
                Err(e) => {
                    debug!(path = %dot_flox.path.display(), "trust check failed: {e}");
                },
            }
        }

        let new_active_dirs: Vec<PathBuf> = trusted_dot_flox
            .iter()
            .map(|d| d.path.clone())
            .collect();

        // Check if the set of active dirs actually changed.
        let dirs_changed = new_active_dirs != state.active_dirs;
        let watches_changed = state.watches_changed();

        if !dirs_changed && !watches_changed && state.last_cwd.as_ref() == Some(&cwd) {
            // Nothing changed, just update CWD tracking.
            return Ok(());
        }

        let mut stdout = std::io::stdout().lock();

        // Step 1: Revert previous diff.
        emit_revert(&state.diff, shell, &mut stdout)?;

        // Step 2: Build new env vars from all trusted environments.
        let mut combined_env: HashMap<String, String> = HashMap::new();
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
                        combined_env.insert(k, v);
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

        // Step 3: Compute the new diff (pristine = current env, new = combined).
        let pristine: HashMap<String, String> = std::env::vars().collect();
        let mut new_env = pristine.clone();
        for (k, v) in &combined_env {
            new_env.insert(k.clone(), v.clone());
        }
        let new_diff = HookDiff::compute(&pristine, &new_env);

        // Step 4: Emit new exports.
        emit_apply(&new_diff, &new_env, shell, &mut stdout)?;

        // Step 5: Emit updated state variables.
        emit_state_vars(
            &new_diff,
            &new_active_dirs,
            &new_watches,
            &suppressed_dirs,
            &notified_dirs,
            &cwd,
            shell,
            &mut stdout,
        )?;

        Ok(())
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

    // Add bin and sbin directories to PATH.
    let mut path_additions = Vec::new();
    let bin = store_path.join("bin");
    if bin.exists() {
        path_additions.push(bin.display().to_string());
    }
    let sbin = store_path.join("sbin");
    if sbin.exists() {
        path_additions.push(sbin.display().to_string());
    }
    if !path_additions.is_empty() {
        let current_path = std::env::var("PATH").unwrap_or_default();
        let new_path = format!("{}:{}", path_additions.join(":"), current_path);
        vars.insert("PATH".to_string(), new_path);
    }

    // Parse activate.d/envrc for exported variables.
    let envrc = store_path.join("activate.d").join("envrc");
    if envrc.exists() {
        if let Ok(contents) = std::fs::read_to_string(&envrc) {
            let export_re = Regex::new(r#"^export\s+([A-Za-z_][A-Za-z0-9_]*)="(.*)"$"#)
                .expect("valid regex");
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
    }

    Ok(vars)
}

/// Emit shell commands to revert the previous HookDiff.
fn emit_revert(diff: &HookDiff, shell: Shell, writer: &mut impl Write) -> Result<()> {
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
    for (name, _orig_val) in &diff.modifications {
        if let Some(new_val) = new_env.get(name) {
            SetVar::exported_no_expansion(name, new_val).generate_with_newline(shell, writer)?;
        }
    }
    Ok(())
}

/// Emit updated _FLOX_HOOK_* state variables.
fn emit_state_vars(
    diff: &HookDiff,
    active_dirs: &[PathBuf],
    watches: &[WatchEntry],
    suppressed_dirs: &[PathBuf],
    notified_dirs: &[PathBuf],
    cwd: &std::path::Path,
    shell: Shell,
    writer: &mut impl Write,
) -> Result<()> {
    let diff_encoded = diff.serialize()?;
    SetVar::exported_no_expansion("_FLOX_HOOK_DIFF", &diff_encoded)
        .generate_with_newline(shell, writer)?;

    let dirs_str = HookState::format_path_list(active_dirs);
    SetVar::exported_no_expansion("_FLOX_HOOK_DIRS", &dirs_str)
        .generate_with_newline(shell, writer)?;

    let watches_json = serde_json::to_string(watches).context("failed to serialize watches")?;
    SetVar::exported_no_expansion("_FLOX_HOOK_WATCHES", &watches_json)
        .generate_with_newline(shell, writer)?;

    let suppressed_str = HookState::format_path_list(suppressed_dirs);
    SetVar::exported_no_expansion("_FLOX_HOOK_SUPPRESSED", &suppressed_str)
        .generate_with_newline(shell, writer)?;

    let notified_str = HookState::format_path_list(notified_dirs);
    SetVar::exported_no_expansion("_FLOX_HOOK_NOTIFIED", &notified_str)
        .generate_with_newline(shell, writer)?;

    SetVar::exported_no_expansion("_FLOX_HOOK_CWD", &cwd.display().to_string())
        .generate_with_newline(shell, writer)?;

    Ok(())
}
