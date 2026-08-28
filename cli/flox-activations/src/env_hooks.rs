//! Dispatch of `[plugin-hooks].env` executables.
//!
//! An env hook contributes environment variables at activation start and
//! at every attach: it is invoked with the shared hook env-var protocol
//! (`FLOX_HOOK=env`, a `0600` ctx file via `FLOX_HOOK_CTX`), inherits the
//! current environment, and prints a JSON object `{"VAR": "value"}` on
//! stdout. The contract is fail-closed — a non-zero exit, malformed JSON,
//! a non-string value, or a `_FLOX_*`-prefixed key (core control state
//! must not be forgeable through this channel) fails the activation or
//! attach. Hooks were resolved, validated, and lexically ordered by the
//! CLI at activation time ([`PluginHookExec`]); later hooks win on key
//! collisions. Design: docs/plugin-lifecycle-hooks.md §3.4.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use flox_core::activate::context::{AttachProjectCtx, PluginHookExec};
use flox_core::activate::hooks::{
    FLOX_HOOK_CTX_VAR,
    FLOX_HOOK_JQ_VAR,
    FLOX_HOOK_VAR,
    FLOX_PLUGIN_NAME_VAR,
    JQ_BIN,
};
use serde::Serialize;
use tracing::debug;

/// Which dispatch point is invoking the hook.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EnvHookPhase {
    /// Activation start: the activate script and executive-supervised
    /// processes (services) are about to run.
    Start,
    /// A shell attaching to the activation (including the first one).
    Attach,
}

/// The versioned context an env hook receives. Serialized to a `0600`
/// file whose path is passed via `FLOX_HOOK_CTX`.
#[derive(Debug, Serialize)]
struct EnvHookCtx<'a> {
    ctx_version: u32,
    hook: &'a str,
    phase: EnvHookPhase,
    dot_flox_path: Option<&'a Path>,
    rendered_env: &'a Path,
    /// The activation's runtime dir (parent of the services socket), so
    /// injected processes can find a sibling sidecar's sockets.
    runtime_dir: Option<PathBuf>,
    services_socket: Option<&'a Path>,
    session_root_pid: i32,
    plugin_table: &'a serde_json::Value,
}

/// Run every recorded env hook in order, returning the contributed
/// variables as ordered pairs (later entries override earlier ones when
/// merged). An empty hook list short-circuits without side effects.
pub fn run_env_hooks(
    hooks: &[PluginHookExec],
    project: Option<&AttachProjectCtx>,
    rendered_env: &Path,
    phase: EnvHookPhase,
    session_root_pid: i32,
) -> Result<Vec<(String, String)>> {
    if hooks.is_empty() {
        return Ok(Vec::new());
    }

    let mut pairs = Vec::new();
    for hook in hooks {
        let ctx = EnvHookCtx {
            ctx_version: 1,
            hook: "env",
            phase,
            dot_flox_path: project.map(|p| p.dot_flox_path.as_path()),
            rendered_env,
            runtime_dir: project
                .and_then(|p| p.flox_services_socket.parent())
                .map(Path::to_path_buf),
            services_socket: project.map(|p| p.flox_services_socket.as_path()),
            session_root_pid,
            plugin_table: &hook.plugin_table,
        };
        // NamedTempFile is created 0600; the hook returns (unlike
        // session-wrap), so the file is cleaned up when this drops.
        let ctx_file =
            tempfile::NamedTempFile::new().context("could not create the env hook ctx file")?;
        serde_json::to_writer_pretty(&ctx_file, &ctx)
            .context("could not serialize the env hook ctx")?;

        debug!(
            plugin = hook.plugin_name,
            ?phase,
            hook = %hook.hook_path.display(),
            "running env hook"
        );
        let output = Command::new(&hook.hook_path)
            .env(FLOX_HOOK_CTX_VAR, ctx_file.path())
            .env(FLOX_HOOK_VAR, "env")
            .env(FLOX_PLUGIN_NAME_VAR, &hook.plugin_name)
            .env(FLOX_HOOK_JQ_VAR, &*JQ_BIN)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .output()
            .with_context(|| {
                format!(
                    "failed to run the env hook for plugin '{}'",
                    hook.plugin_name
                )
            })?;
        if !output.status.success() {
            bail!(
                "The env hook for plugin '{}' failed with {}.",
                hook.plugin_name,
                output.status
            );
        }

        let contributed: serde_json::Map<String, serde_json::Value> =
            serde_json::from_slice(&output.stdout).with_context(|| {
                format!(
                    "The env hook for plugin '{}' did not print a JSON object on stdout.",
                    hook.plugin_name
                )
            })?;
        for (key, value) in contributed {
            if key.starts_with("_FLOX_") {
                bail!(
                    "The env hook for plugin '{}' attempted to set '{key}', but '_FLOX_'-prefixed variables are reserved.",
                    hook.plugin_name
                );
            }
            let serde_json::Value::String(value) = value else {
                bail!(
                    "The env hook for plugin '{}' set '{key}' to a non-string value.",
                    hook.plugin_name
                );
            };
            pairs.push((key, value));
        }
    }
    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn hook_with_script(dir: &Path, script: &str) -> PluginHookExec {
        let path = dir.join("test-plugin");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(script.as_bytes()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        PluginHookExec {
            plugin_name: "test-plugin".to_string(),
            hook_path: path,
            plugin_table: serde_json::json!({"key": "value"}),
        }
    }

    fn run_one(script: &str) -> Result<Vec<(String, String)>> {
        let dir = tempfile::tempdir().unwrap();
        let hook = hook_with_script(dir.path(), script);
        run_env_hooks(
            &[hook],
            None,
            Path::new("/nix/store/fake-env"),
            EnvHookPhase::Attach,
            4242,
        )
    }

    #[test]
    fn contributed_vars_are_returned_in_order() {
        let pairs = run_one("#!/bin/sh\nprintf '{\"B\": \"2\", \"A\": \"1\"}'\n").unwrap();
        // serde_json::Map iterates in sorted key order.
        assert_eq!(pairs, vec![
            ("A".to_string(), "1".to_string()),
            ("B".to_string(), "2".to_string())
        ]);
    }

    #[test]
    fn hook_reads_its_ctx_fields() {
        let pairs = run_one(concat!(
            "#!/bin/sh\n",
            "phase=$(\"$FLOX_HOOK_JQ\" -r .phase \"$FLOX_HOOK_CTX\")\n",
            "table=$(\"$FLOX_HOOK_JQ\" -r .plugin_table.key \"$FLOX_HOOK_CTX\")\n",
            "printf '{\"PHASE\": \"%s\", \"TABLE\": \"%s\"}' \"$phase\" \"$table\"\n",
        ))
        .unwrap();
        assert_eq!(pairs, vec![
            ("PHASE".to_string(), "attach".to_string()),
            ("TABLE".to_string(), "value".to_string())
        ]);
    }

    #[test]
    fn reserved_keys_fail_closed() {
        let err = run_one("#!/bin/sh\nprintf '{\"_FLOX_SESSION_WRAPPED\": \"forged\"}'\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("reserved"), "unexpected error: {err}");
    }

    #[test]
    fn nonzero_exit_fails_closed() {
        let err = run_one("#!/bin/sh\nexit 3\n").unwrap_err().to_string();
        assert!(err.contains("failed with"), "unexpected error: {err}");
    }

    #[test]
    fn malformed_output_fails_closed() {
        let err = run_one("#!/bin/sh\nprintf 'not json'\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("JSON object"), "unexpected error: {err}");
    }

    #[test]
    fn non_string_values_fail_closed() {
        let err = run_one("#!/bin/sh\nprintf '{\"PORT\": 8080}'\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("non-string"), "unexpected error: {err}");
    }

    #[test]
    fn later_hooks_win_on_collision() {
        let dir = tempfile::tempdir().unwrap();
        let first = hook_with_script(dir.path(), "#!/bin/sh\nprintf '{\"X\": \"first\"}'\n");
        let dir2 = tempfile::tempdir().unwrap();
        let second = hook_with_script(dir2.path(), "#!/bin/sh\nprintf '{\"X\": \"second\"}'\n");
        let pairs = run_env_hooks(
            &[first, second],
            None,
            Path::new("/nix/store/fake-env"),
            EnvHookPhase::Start,
            1,
        )
        .unwrap();
        let mut merged = std::collections::HashMap::new();
        merged.extend(pairs);
        assert_eq!(merged.get("X"), Some(&"second".to_string()));
    }
}
