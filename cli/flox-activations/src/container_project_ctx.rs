//! Builds the `project_ctx` for a container guest activation.
//!
//! The baked container context ships `project_ctx = null` (no project), so
//! service startup is skipped and `flox services` cannot connect to a socket.
//! At activation time this module constructs an [`AttachProjectCtx`] from the
//! bind-mounted project directory, enabling:
//!
//! - Auto-start: the guard at `activate.rs` starts process-compose when
//!   `services_to_start` is non-empty.
//! - In-guest `flox services status/start/stop/logs`: the populated project
//!   context routes these commands to the running supervisor via the pinned
//!   socket path.
//!
//! The socket path is read from `_FLOX_SERVICES_SOCKET_OVERRIDE` (set in the
//! container Env block by mkContainer.nix) so the activation entrypoint and
//! in-guest `flox services` always agree on a single path without needing to
//! re-derive a path_hash inside the guest.
//!
//! Log files are written to `/run/flox/log` rather than the bind-mounted
//! `.flox/log` because the host owns the `.flox` directory and the guest
//! (uid 10000) may lack write access to it.
//!
//! A round-trip test using [`flox_core::activate::context::AttachProjectCtx`]
//! guards the struct shape against serde drift (this crate cannot depend on
//! flox-rust-sdk, but can depend on flox-core).

use std::path::{Path, PathBuf};

use flox_core::activate::context::AttachProjectCtx;
use serde::Deserialize;

/// The environment pointer file inside a `.flox` directory.
const ENV_POINTER_FILENAME: &str = "env.json";
/// The `.flox` directory name.
const DOT_FLOX: &str = ".flox";
/// Path to the lockfile within `.flox/env/`.
const LOCKFILE_PATH: &str = "env/manifest.lock";

/// The env var that pins the services socket to a fixed guest path.
/// Set in the container image's Env block; both the activation entrypoint
/// and in-guest `flox services` read this override before deriving a
/// path_hash-based path, so they cannot diverge.
const SERVICES_SOCKET_OVERRIDE_VAR: &str = "_FLOX_SERVICES_SOCKET_OVERRIDE";

/// The env var that points to the process-compose binary. Set in the
/// container image's Env block so this crate does not need to bake it in
/// at compile time.
const PROCESS_COMPOSE_BIN_VAR: &str = "PROCESS_COMPOSE_BIN";

/// Writable guest log directory. The bind-mounted `.flox/log` is owned by the
/// host uid; the guest (uid 10000) cannot write to it. Route log files here
/// instead — `/run/flox` is chowned to the guest user in mkContainer.nix.
const GUEST_LOG_DIR: &str = "/run/flox/log";

/// Minimal env-pointer shape used only to detect managed environments
/// (those with an `owner` field).
#[derive(Debug, Deserialize)]
struct EnvPointerFile {
    #[serde(default)]
    owner: Option<String>,
}

/// Minimal lockfile shape: only the fields needed to extract service names.
///
/// The full `Lockfile` type lives in flox-manifest (which this crate cannot
/// depend on), so we deserialize only what we need using serde_json's
/// `Value`-backed field access.
#[derive(Debug, Deserialize)]
struct MinimalLockfile {
    manifest: serde_json::Value,
}

/// Build the `AttachProjectCtx` for a container guest, or `None` when the
/// project cannot be resolved.
///
/// Returns `None` for managed environments, when no `.flox` is found, when
/// the lockfile is missing or unreadable, or when neither auto-start is
/// enabled nor any services are defined.
pub fn container_project_ctx(start_dir: &Path) -> Option<AttachProjectCtx> {
    let dot_flox = find_dot_flox(start_dir)?;

    // Skip managed environments: they have an `owner` field in env.json.
    let pointer_path = dot_flox.join(ENV_POINTER_FILENAME);
    let pointer_contents = std::fs::read_to_string(&pointer_path).ok()?;
    let pointer: EnvPointerFile = serde_json::from_str(&pointer_contents).ok()?;
    if pointer.owner.is_some() {
        return None;
    }

    let lockfile_path = dot_flox.join(LOCKFILE_PATH);
    let lockfile_contents = std::fs::read_to_string(&lockfile_path).ok()?;
    let lockfile: MinimalLockfile = serde_json::from_str(&lockfile_contents).ok()?;

    let services_to_start = services_to_start_from_lockfile(&lockfile.manifest);

    // Only provide a project context when there are services to start.
    // When the services list is empty the startup gate in activate.rs is a
    // no-op, and routing through the executive for an environment with no
    // services would add overhead with no benefit.
    if services_to_start.is_empty() {
        return None;
    }

    let process_compose_bin = std::env::var(PROCESS_COMPOSE_BIN_VAR)
        .map(PathBuf::from)
        .ok()?;

    let flox_services_socket = std::env::var(SERVICES_SOCKET_OVERRIDE_VAR)
        .map(PathBuf::from)
        .ok()?;

    let env_project = dot_flox.parent().unwrap_or(Path::new("/")).to_path_buf();

    let flox_env_log_dir = PathBuf::from(GUEST_LOG_DIR);

    Some(AttachProjectCtx {
        env_project,
        dot_flox_path: dot_flox,
        flox_env_log_dir,
        process_compose_bin,
        flox_services_socket,
        services_to_start,
    })
}

/// Extract service names to start from the manifest embedded in the lockfile.
///
/// Returns service names that are:
/// - listed under `manifest.services` (excluding the `auto-start` key), and
/// - either have no `systems` filter, or include `aarch64-linux` (the guest
///   container system).
///
/// Returns an empty list when `auto-start` is not `true` or when no services
/// are defined for the guest system.
fn services_to_start_from_lockfile(manifest_value: &serde_json::Value) -> Vec<String> {
    let services = match manifest_value.get("services") {
        Some(serde_json::Value::Object(map)) => map,
        _ => return Vec::new(),
    };

    // Only auto-start when the manifest opts in.
    let auto_start = services
        .get("auto-start")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if !auto_start {
        return Vec::new();
    }

    // The container always runs aarch64-linux.
    let guest_system = "aarch64-linux";

    let mut names: Vec<String> = services
        .iter()
        .filter(|(key, _)| *key != "auto-start")
        .filter_map(|(name, descriptor)| {
            // A descriptor without a `systems` field runs on all systems.
            // A descriptor with a `systems` array runs only on listed systems.
            let systems = descriptor.get("systems");
            let system_matches = match systems {
                None => true,
                Some(serde_json::Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| v.as_str())
                    .any(|s| s == guest_system),
                _ => false,
            };
            system_matches.then_some(name.clone())
        })
        .collect();

    // Sort for deterministic ordering (BTreeMap iteration is ordered, but
    // serde_json::Map preserves insertion order; sort here to be safe).
    names.sort();
    names
}

/// Ascend from `start_dir` looking for a `.flox` directory.
/// Mirrors the same function in `container_active_env`.
pub fn find_dot_flox(start_dir: &Path) -> Option<PathBuf> {
    let mut dir = Some(start_dir);
    while let Some(current) = dir {
        let candidate = current.join(DOT_FLOX);
        if candidate.is_dir() {
            return std::fs::canonicalize(&candidate).ok();
        }
        dir = current.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use std::fs;

    use flox_core::activate::context::AttachProjectCtx;
    use tempfile::TempDir;

    use super::*;

    /// Create a minimal `.flox` directory with the given env.json and
    /// manifest.lock contents. Returns the canonical path to `.flox`.
    fn write_dot_flox(dir: &Path, env_json: &str, lockfile_json: &str) -> PathBuf {
        let dot_flox = dir.join(DOT_FLOX);
        let env_dir = dot_flox.join("env");
        fs::create_dir_all(&env_dir).unwrap();
        fs::write(dot_flox.join(ENV_POINTER_FILENAME), env_json).unwrap();
        fs::write(env_dir.join("manifest.lock"), lockfile_json).unwrap();
        std::fs::canonicalize(&dot_flox).unwrap()
    }

    /// Minimal lockfile JSON with one service that has auto-start enabled.
    fn lockfile_with_autostart_service(service_name: &str) -> String {
        serde_json::json!({
            "lockfile-version": 1,
            "manifest": {
                "version": 1,
                "services": {
                    "auto-start": true,
                    service_name: {
                        "command": "my-daemon"
                    }
                }
            },
            "packages": []
        })
        .to_string()
    }

    /// Minimal lockfile JSON with a service restricted to aarch64-linux.
    fn lockfile_with_system_filtered_service() -> String {
        serde_json::json!({
            "lockfile-version": 1,
            "manifest": {
                "version": 1,
                "services": {
                    "auto-start": true,
                    "linux-only": {
                        "command": "linux-daemon",
                        "systems": ["aarch64-linux"]
                    },
                    "mac-only": {
                        "command": "mac-daemon",
                        "systems": ["aarch64-darwin"]
                    }
                }
            },
            "packages": []
        })
        .to_string()
    }

    /// Minimal lockfile JSON with no services.
    fn lockfile_no_services() -> String {
        serde_json::json!({
            "lockfile-version": 1,
            "manifest": {
                "version": 1
            },
            "packages": []
        })
        .to_string()
    }

    /// Minimal lockfile JSON with services but auto-start = false.
    fn lockfile_autostart_false() -> String {
        serde_json::json!({
            "lockfile-version": 1,
            "manifest": {
                "version": 1,
                "services": {
                    "auto-start": false,
                    "my-service": {
                        "command": "daemon"
                    }
                }
            },
            "packages": []
        })
        .to_string()
    }

    #[test]
    fn builds_project_ctx_for_path_env_with_autostart() {
        let tmp = TempDir::new().unwrap();
        write_dot_flox(
            tmp.path(),
            r#"{"name":"sandbox-demo","version":1}"#,
            &lockfile_with_autostart_service("postgres"),
        );

        temp_env::with_vars(
            [
                (
                    PROCESS_COMPOSE_BIN_VAR,
                    Some("/nix/store/abc-pc/bin/process-compose"),
                ),
                (
                    SERVICES_SOCKET_OVERRIDE_VAR,
                    Some("/run/flox/runtime/services.sock"),
                ),
            ],
            || {
                let ctx = container_project_ctx(tmp.path())
                    .expect("path env with autostart should yield a project ctx");

                assert_eq!(
                    ctx.flox_services_socket,
                    PathBuf::from("/run/flox/runtime/services.sock")
                );
                assert_eq!(
                    ctx.process_compose_bin,
                    PathBuf::from("/nix/store/abc-pc/bin/process-compose")
                );
                assert_eq!(ctx.flox_env_log_dir, PathBuf::from(GUEST_LOG_DIR));
                assert_eq!(ctx.services_to_start, vec!["postgres".to_string()]);
            },
        );
    }

    #[test]
    fn filters_services_by_system() {
        let tmp = TempDir::new().unwrap();
        write_dot_flox(
            tmp.path(),
            r#"{"name":"proj","version":1}"#,
            &lockfile_with_system_filtered_service(),
        );

        temp_env::with_vars(
            [
                (
                    PROCESS_COMPOSE_BIN_VAR,
                    Some("/nix/store/abc-pc/bin/process-compose"),
                ),
                (
                    SERVICES_SOCKET_OVERRIDE_VAR,
                    Some("/run/flox/runtime/services.sock"),
                ),
            ],
            || {
                let ctx = container_project_ctx(tmp.path())
                    .expect("should have services for aarch64-linux");
                // Only the linux-only service matches the guest system.
                assert_eq!(ctx.services_to_start, vec!["linux-only".to_string()]);
            },
        );
    }

    #[test]
    fn returns_none_when_no_services() {
        let tmp = TempDir::new().unwrap();
        write_dot_flox(
            tmp.path(),
            r#"{"name":"empty","version":1}"#,
            &lockfile_no_services(),
        );

        temp_env::with_vars(
            [
                (
                    PROCESS_COMPOSE_BIN_VAR,
                    Some("/nix/store/abc-pc/bin/process-compose"),
                ),
                (
                    SERVICES_SOCKET_OVERRIDE_VAR,
                    Some("/run/flox/runtime/services.sock"),
                ),
            ],
            || {
                assert!(container_project_ctx(tmp.path()).is_none());
            },
        );
    }

    #[test]
    fn returns_none_when_autostart_false() {
        let tmp = TempDir::new().unwrap();
        write_dot_flox(
            tmp.path(),
            r#"{"name":"no-auto","version":1}"#,
            &lockfile_autostart_false(),
        );

        temp_env::with_vars(
            [
                (
                    PROCESS_COMPOSE_BIN_VAR,
                    Some("/nix/store/abc-pc/bin/process-compose"),
                ),
                (
                    SERVICES_SOCKET_OVERRIDE_VAR,
                    Some("/run/flox/runtime/services.sock"),
                ),
            ],
            || {
                assert!(container_project_ctx(tmp.path()).is_none());
            },
        );
    }

    #[test]
    fn returns_none_for_managed_env() {
        let tmp = TempDir::new().unwrap();
        write_dot_flox(
            tmp.path(),
            r#"{"name":"prod","owner":"acme","version":1}"#,
            &lockfile_with_autostart_service("web"),
        );

        temp_env::with_vars(
            [
                (
                    PROCESS_COMPOSE_BIN_VAR,
                    Some("/nix/store/abc-pc/bin/process-compose"),
                ),
                (
                    SERVICES_SOCKET_OVERRIDE_VAR,
                    Some("/run/flox/runtime/services.sock"),
                ),
            ],
            || {
                assert!(container_project_ctx(tmp.path()).is_none());
            },
        );
    }

    #[test]
    fn returns_none_when_no_dot_flox() {
        let tmp = TempDir::new().unwrap();

        temp_env::with_vars(
            [
                (
                    PROCESS_COMPOSE_BIN_VAR,
                    Some("/nix/store/abc-pc/bin/process-compose"),
                ),
                (
                    SERVICES_SOCKET_OVERRIDE_VAR,
                    Some("/run/flox/runtime/services.sock"),
                ),
            ],
            || {
                assert!(container_project_ctx(tmp.path()).is_none());
            },
        );
    }

    #[test]
    fn returns_none_when_socket_override_not_set() {
        let tmp = TempDir::new().unwrap();
        write_dot_flox(
            tmp.path(),
            r#"{"name":"proj","version":1}"#,
            &lockfile_with_autostart_service("svc"),
        );

        temp_env::with_vars(
            [
                (
                    PROCESS_COMPOSE_BIN_VAR,
                    Some("/nix/store/abc-pc/bin/process-compose"),
                ),
                (SERVICES_SOCKET_OVERRIDE_VAR, None),
            ],
            || {
                // Without the socket override, we can't build the project ctx.
                assert!(container_project_ctx(tmp.path()).is_none());
            },
        );
    }

    /// Verify that the socket path produced by the container path and the
    /// `_FLOX_SERVICES_SOCKET_OVERRIDE` env var agree. Both the entrypoint
    /// (`container_project_ctx`) and in-guest `flox services` read the same
    /// env var, so they cannot diverge.
    #[test]
    fn socket_override_env_var_is_read_by_container_project_ctx() {
        let expected_socket = "/run/flox/runtime/services.sock";

        let tmp = TempDir::new().unwrap();
        write_dot_flox(
            tmp.path(),
            r#"{"name":"proj","version":1}"#,
            &lockfile_with_autostart_service("svc"),
        );

        temp_env::with_vars(
            [
                (
                    PROCESS_COMPOSE_BIN_VAR,
                    Some("/nix/store/abc-pc/bin/process-compose"),
                ),
                (SERVICES_SOCKET_OVERRIDE_VAR, Some(expected_socket)),
            ],
            || {
                let ctx = container_project_ctx(tmp.path()).expect("should build ctx");
                assert_eq!(ctx.flox_services_socket, PathBuf::from(expected_socket));
            },
        );
    }

    /// The pinned socket path `/run/flox/runtime/services.sock` is well under
    /// the Linux 108-char `sun_path` limit. This documents the constraint so it
    /// won't be silently violated if the path changes.
    #[test]
    fn pinned_socket_path_is_within_sun_path_limit() {
        let socket = "/run/flox/runtime/services.sock";
        // 108 minus null terminator = 107 usable chars on Linux.
        assert!(
            socket.len() <= 107,
            "socket path '{}' exceeds 107-char Linux sun_path limit (len={})",
            socket,
            socket.len()
        );
    }

    /// Guard the [`AttachProjectCtx`] field names against serde drift.
    /// `container_project_ctx` builds this struct in flox-activations and
    /// `flox-activations activate` receives it as a serialised JSON value —
    /// if the struct fields change in flox-core, the round-trip would silently
    /// break without this test.
    #[test]
    fn attach_project_ctx_round_trip() {
        let ctx = AttachProjectCtx {
            env_project: PathBuf::from("/project"),
            dot_flox_path: PathBuf::from("/project/.flox"),
            flox_env_log_dir: PathBuf::from("/run/flox/log"),
            process_compose_bin: PathBuf::from("/nix/store/abc-pc/bin/process-compose"),
            flox_services_socket: PathBuf::from("/run/flox/runtime/services.sock"),
            services_to_start: vec!["postgres".to_string(), "redis".to_string()],
        };

        let json = serde_json::to_string(&ctx).unwrap();
        let decoded: AttachProjectCtx = serde_json::from_str(&json).unwrap();
        assert_eq!(ctx.env_project, decoded.env_project);
        assert_eq!(ctx.dot_flox_path, decoded.dot_flox_path);
        assert_eq!(ctx.flox_env_log_dir, decoded.flox_env_log_dir);
        assert_eq!(ctx.process_compose_bin, decoded.process_compose_bin);
        assert_eq!(ctx.flox_services_socket, decoded.flox_services_socket);
        assert_eq!(ctx.services_to_start, decoded.services_to_start);
    }

    #[test]
    fn discovers_dot_flox_from_subdirectory() {
        let tmp = TempDir::new().unwrap();
        write_dot_flox(
            tmp.path(),
            r#"{"name":"proj","version":1}"#,
            &lockfile_with_autostart_service("web"),
        );
        let subdir = tmp.path().join("src").join("deep");
        fs::create_dir_all(&subdir).unwrap();

        temp_env::with_vars(
            [
                (
                    PROCESS_COMPOSE_BIN_VAR,
                    Some("/nix/store/abc-pc/bin/process-compose"),
                ),
                (
                    SERVICES_SOCKET_OVERRIDE_VAR,
                    Some("/run/flox/runtime/services.sock"),
                ),
            ],
            || {
                let ctx =
                    container_project_ctx(&subdir).expect("should ascend to the project .flox");
                assert_eq!(ctx.services_to_start, vec!["web".to_string()]);
            },
        );
    }
}
