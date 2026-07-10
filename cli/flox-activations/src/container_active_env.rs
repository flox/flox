//! Registers the guest environment as ACTIVE for container activations.
//!
//! `flox deactivate` gates on a non-empty `_FLOX_ACTIVE_ENVIRONMENTS` list
//! (it opens the front-of-stack entry and records a Deactivate hook action
//! that the next prompt turns into an `exit`). The baked container context
//! ships `flox_active_environments = "[]"`, so a guest activation is not
//! "active" and `flox deactivate` prints "No environment active!" instead
//! of leaving the session.
//!
//! This module builds a one-entry active list at activation time from the
//! bind-mounted project's `.flox` directory (present in the guest at its
//! real host path). The entry's serialized shape mirrors
//! `flox_rust_sdk::models::environment::UninitializedEnvironment::DotFlox`
//! for a path environment, so `flox deactivate`'s
//! `into_concrete_environment` opens the same `PathEnvironment` the guest
//! actually activated. A round-trip test in flox-rust-sdk guards the JSON
//! against serde drift (flox-activations cannot depend on flox-rust-sdk).
//!
//! The module also provides helpers used by the activation entrypoint to:
//! - resolve the canonical `.flox` path and env name for a path environment
//! - compute rendered-env link names that match the naming convention used
//!   by `RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system`
//!   in flox-rust-sdk (a guard test in the `flox` CLI crate enforces this
//!   invariant across crate boundaries).

use std::path::{Path, PathBuf};

use serde::Serialize;

/// The environment-pointer file inside a `.flox` directory.
const ENV_POINTER_FILENAME: &str = "env.json";
/// The `.flox` directory name.
const DOT_FLOX: &str = ".flox";
/// The rendered-environment links directory inside `.flox`.
pub const RUN_DIR_NAME: &str = "run";

/// Mirror of `flox_rust_sdk` `PathPointer` for a path environment. The real
/// type serializes `name` as a bare string (SerializeDisplay) and `version`
/// as the integer 1.
#[derive(Debug, Serialize)]
struct PathPointer {
    name: String,
    version: u8,
}

/// Mirror of the `EnvironmentPointer::Path` untagged variant: the pointer
/// fields are inlined (no `type` tag on the pointer itself).
#[derive(Debug, Serialize)]
struct DotFlox {
    #[serde(rename = "type")]
    kind: &'static str,
    path: PathBuf,
    pointer: PathPointer,
}

/// Mirror of `ActiveEnvironment`. `generation` is omitted (serde skips it
/// when None); `mode` matches `ActivateMode` snake_case ("dev").
#[derive(Debug, Serialize)]
struct ActiveEnvironment {
    environment: DotFlox,
    mode: &'static str,
}

/// The env-pointer file shape for a path environment (managed envs carry an
/// `owner` field and are handled by the [`is_none`](Option::is_none) guard).
#[derive(Debug, serde::Deserialize)]
struct EnvPointerFile {
    name: String,
    #[serde(default)]
    owner: Option<String>,
}

/// Build the `_FLOX_ACTIVE_ENVIRONMENTS` JSON for a container guest, or
/// `None` when a path environment cannot be resolved from the mounted
/// project.
///
/// Walks up from `start_dir` (the guest working directory, inside the
/// bind-mounted project) to find `.flox`, reads `.flox/env.json`, and — for
/// a path environment — emits a one-entry active list keyed to the `.flox`
/// directory at its real path. Returns `None` for managed environments
/// (owner present) or when no `.flox` is found, so the caller keeps the
/// empty list rather than baking a pointer `flox deactivate` cannot open.
pub fn container_active_environments_json(start_dir: &Path) -> Option<String> {
    let dot_flox = find_dot_flox(start_dir)?;
    let name = parse_path_env_name(&dot_flox)?;
    let entry = ActiveEnvironment {
        environment: DotFlox {
            kind: "dot-flox",
            path: dot_flox,
            pointer: PathPointer { name, version: 1 },
        },
        // Container activations default to dev mode (matching the baked
        // container context's activation mode default).
        mode: "dev",
    };
    serde_json::to_string(&[entry]).ok()
}

/// Ascend from `start_dir` looking for a directory containing `.flox`,
/// returning the canonical `.flox` path. Mirrors how `flox` discovers a
/// project environment from the working directory.
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

/// Read `.flox/env.json` and return the environment name for a path (ownerless)
/// environment, or `None` for a managed environment or on read/parse error.
///
/// The caller must pass the canonical `.flox` directory path (as returned by
/// [`find_dot_flox`]).
pub fn parse_path_env_name(dot_flox: &Path) -> Option<String> {
    let pointer_path = dot_flox.join(ENV_POINTER_FILENAME);
    let contents = std::fs::read_to_string(&pointer_path).ok()?;
    let pointer: EnvPointerFile = serde_json::from_str(&contents).ok()?;
    // Only path environments: managed envs carry an `owner` field.
    if pointer.owner.is_some() {
        return None;
    }
    Some(pointer.name)
}

/// Compute the file-name prefix for a guest environment's rendered-env links.
///
/// The prefix is `{system}.{name}` where `system` is derived from
/// `std::env::consts::ARCH` and `std::env::consts::OS` at runtime (e.g.
/// `aarch64-linux`, `x86_64-linux`). Appending `-dev` or `-run` to the
/// prefix yields the exact link names that
/// `RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system` in
/// `flox-rust-sdk` would create for the same inputs. A guard test in the
/// `flox` CLI crate (which depends on both crates) asserts this invariance
/// so that naming drift breaks CI rather than the demo.
///
/// This helper lives in `flox-activations` rather than `flox-rust-sdk`
/// because `flox-activations` must NOT depend on `flox-rust-sdk` (the
/// dependency would be circular).
pub fn guest_env_link_prefix(name: &str) -> String {
    let system = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);
    format!("{system}.{name}")
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn write_env(dir: &Path, contents: &str) -> PathBuf {
        let dot_flox = dir.join(DOT_FLOX);
        fs::create_dir_all(&dot_flox).unwrap();
        fs::write(dot_flox.join(ENV_POINTER_FILENAME), contents).unwrap();
        std::fs::canonicalize(&dot_flox).unwrap()
    }

    #[test]
    fn builds_one_entry_active_list_for_path_env() {
        let tmp = TempDir::new().unwrap();
        let canonical_dot_flox = write_env(tmp.path(), r#"{"name":"sandbox-demo","version":1}"#);

        let json = container_active_environments_json(tmp.path())
            .expect("path env should yield an active list");

        let expected = format!(
            "[{{\"environment\":{{\"type\":\"dot-flox\",\"path\":\"{}\",\
             \"pointer\":{{\"name\":\"sandbox-demo\",\"version\":1}}}},\"mode\":\"dev\"}}]",
            canonical_dot_flox.display()
        );
        assert_eq!(json, expected);
    }

    #[test]
    fn discovers_dot_flox_from_a_subdirectory() {
        let tmp = TempDir::new().unwrap();
        write_env(tmp.path(), r#"{"name":"proj","version":1}"#);
        let subdir = tmp.path().join("src").join("nested");
        fs::create_dir_all(&subdir).unwrap();

        let json = container_active_environments_json(&subdir)
            .expect("should ascend to the project .flox");
        assert!(json.contains("\"name\":\"proj\""), "got: {json}");
    }

    #[test]
    fn returns_none_for_managed_env() {
        let tmp = TempDir::new().unwrap();
        write_env(tmp.path(), r#"{"name":"prod","owner":"acme","version":1}"#);
        assert_eq!(container_active_environments_json(tmp.path()), None);
    }

    #[test]
    fn returns_none_when_no_dot_flox() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(container_active_environments_json(tmp.path()), None);
    }

    #[test]
    fn parse_path_env_name_returns_name_for_path_env() {
        let tmp = TempDir::new().unwrap();
        let dot_flox = write_env(tmp.path(), r#"{"name":"my-env","version":1}"#);
        assert_eq!(parse_path_env_name(&dot_flox), Some("my-env".to_string()));
    }

    #[test]
    fn parse_path_env_name_returns_none_for_managed_env() {
        let tmp = TempDir::new().unwrap();
        let dot_flox = write_env(tmp.path(), r#"{"name":"prod","owner":"acme","version":1}"#);
        assert_eq!(parse_path_env_name(&dot_flox), None);
    }

    #[test]
    fn guest_env_link_prefix_format() {
        // The prefix must be "{ARCH}-{OS}.{name}" so that appending "-dev"/"-run"
        // yields names identical to RenderedEnvironmentLinks (see guard test in
        // the flox CLI crate).
        let prefix = guest_env_link_prefix("my-env");
        let expected_system = format!("{}-{}", std::env::consts::ARCH, std::env::consts::OS);
        assert_eq!(prefix, format!("{expected_system}.my-env"));
        assert!(
            prefix.ends_with(".my-env"),
            "prefix must end with '.{{name}}'"
        );
    }
}
