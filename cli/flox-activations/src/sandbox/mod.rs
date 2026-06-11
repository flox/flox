//! Sandbox env plumbing for `flox activate --sandbox`.
//!
//! libsandbox mediates file access inside an activation. For it to engage,
//! the activation must (a) preload the library into the shell it execs and
//! (b) hand the library a policy: the mode, the allow-set, and the broker
//! rendezvous. This module owns the activation-side half of that contract —
//! locating the library on disk ([`libsandbox_path`]) and composing the env
//! vars ([`sandbox_env`]). The seed allow-set lives in [`seed`].
//!
//! The injection happens through `attach_diff::double_set_envs`, not a plain
//! pre-exec set, on purpose: macOS strips `DYLD_INSERT_LIBRARIES` when it
//! execs a SIP-protected shell such as `/bin/zsh`, so the only way the
//! preload survives to user-spawned children is to re-export it from the
//! generated rc script after the shell has started. `double_set_envs` is the
//! one channel that both sets before exec and re-exports from rc.

pub mod seed;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flox_core::activate::context::SandboxMode;

use self::seed::{SeedAllowSet, SeedContext};

/// `FLOX_VIRTUAL_SANDBOX` — the libsandbox mode (`warn`/`enforce`/`ask`).
pub const FLOX_VIRTUAL_SANDBOX_VAR: &str = "FLOX_VIRTUAL_SANDBOX";
/// `FLOX_SANDBOX_ALLOW` — space-separated fnmatch globs.
pub const FLOX_SANDBOX_ALLOW_VAR: &str = "FLOX_SANDBOX_ALLOW";
/// `FLOX_SANDBOX_ALLOW_DIRS` — space-separated directory prefixes.
pub const FLOX_SANDBOX_ALLOW_DIRS_VAR: &str = "FLOX_SANDBOX_ALLOW_DIRS";
/// `FLOX_SRC_DIR` — project working dir; the engine auto-adds it as an
/// allow-dir, so setting it is how the project tree is seeded.
pub const FLOX_SRC_DIR_VAR: &str = "FLOX_SRC_DIR";
/// `FLOX_SANDBOX_SOCKET` — verdict socket for the `ask` broker. Unset until
/// the broker lands; the libsandbox `ask` stub denies when it is unset,
/// which is the correct interim behavior.
pub const FLOX_SANDBOX_SOCKET_VAR: &str = "FLOX_SANDBOX_SOCKET";
/// `FLOX_SANDBOX_GRANTS_DIR` — directory holding persisted grants; the
/// engine's write guard routes writes here through the ask flow.
pub const FLOX_SANDBOX_GRANTS_DIR_VAR: &str = "FLOX_SANDBOX_GRANTS_DIR";

/// The preload env var name for the host platform.
#[cfg(target_os = "macos")]
pub const PRELOAD_VAR: &str = "DYLD_INSERT_LIBRARIES";
#[cfg(not(target_os = "macos"))]
pub const PRELOAD_VAR: &str = "LD_PRELOAD";

/// The libsandbox filename for the host platform. macOS loads `.dylib` via
/// `DYLD_INSERT_LIBRARIES`; everything else loads `.so` via `LD_PRELOAD`.
/// Mirrors `package-builder/flox-build.mk` PRELOAD_VARS.
#[cfg(target_os = "macos")]
const LIBSANDBOX_FILENAME: &str = "libsandbox.dylib";
#[cfg(not(target_os = "macos"))]
const LIBSANDBOX_FILENAME: &str = "libsandbox.so";

/// The platform libsandbox filename, exposed for tests in sibling modules
/// that need to stage a fake library file matching what [`libsandbox_path`]
/// looks for.
#[cfg(test)]
pub(crate) const LIBSANDBOX_FILENAME_FOR_TESTS: &str = LIBSANDBOX_FILENAME;

/// Resolve the path to flox-build.mk, whose directory also contains
/// libsandbox. Follows the same env-var-with-compile-time-fallback idiom as
/// `flox-rust-sdk` build.rs, so the dev shell's dynamic value wins and the
/// baked-in Nix store path is the fallback.
fn flox_build_mk() -> PathBuf {
    std::env::var("FLOX_BUILD_MK")
        .unwrap_or_else(|_| env!("FLOX_BUILD_MK").to_string())
        .into()
}

/// Locate the libsandbox preload library.
///
/// The library installs next to flox-build.mk in the package-builder
/// libexec dir, so it is resolved as `dirname(FLOX_BUILD_MK)/libsandbox.*`.
/// Returns an error rather than a missing path so the caller can fail the
/// activation loudly instead of silently activating unsandboxed.
pub fn libsandbox_path() -> Result<PathBuf> {
    let build_mk = flox_build_mk();
    let libexec = build_mk.parent().with_context(|| {
        format!(
            "could not determine the package-builder libexec directory from FLOX_BUILD_MK ({})",
            build_mk.display(),
        )
    })?;
    let lib = libexec.join(LIBSANDBOX_FILENAME);
    if !lib.exists() {
        bail!(
            "the sandbox library was not found at {}.\n\
             Build it with 'just build' (or 'make -C package-builder') and retry.",
            lib.display(),
        );
    }
    Ok(lib)
}

/// Compose the preload value, appending the libsandbox path to any preload
/// the caller already had set rather than clobbering it. Mirrors how
/// flox-build.mk composes PRELOAD_VARS atop an existing value: a colon-
/// separated list with the existing entries first.
fn compose_preload(existing: Option<&str>, libsandbox: &Path) -> String {
    let lib = libsandbox.to_string_lossy();
    match existing.map(str::trim).filter(|s| !s.is_empty()) {
        Some(prev) => format!("{prev}:{lib}"),
        None => lib.into_owned(),
    }
}

/// Build the sandbox environment variables for an activation.
///
/// Returns an empty map when `mode` is `Off`, so callers can unconditionally
/// extend their env diff. For any active mode this sets the policy vars, the
/// seeded allow-set, the grants dir, and the platform preload. The verdict
/// socket is intentionally left unset in this batch (no broker yet).
///
/// `existing_preload` is the caller's current `LD_PRELOAD` /
/// `DYLD_INSERT_LIBRARIES`, preserved by appending rather than replacing.
pub fn sandbox_env(
    mode: SandboxMode,
    seed_ctx: &SeedContext,
    project_working_dir: &Path,
    grants_dir: &Path,
    existing_preload: Option<&str>,
) -> Result<HashMap<String, String>> {
    if mode == SandboxMode::Off {
        return Ok(HashMap::new());
    }

    let libsandbox = libsandbox_path()?;
    let seed = SeedAllowSet::compute(seed_ctx);

    let mut env = HashMap::new();
    env.insert(FLOX_VIRTUAL_SANDBOX_VAR.to_string(), mode.to_string());
    env.insert(FLOX_SANDBOX_ALLOW_VAR.to_string(), seed.allow_value());
    env.insert(
        FLOX_SANDBOX_ALLOW_DIRS_VAR.to_string(),
        seed.allow_dirs_value(),
    );
    env.insert(
        FLOX_SRC_DIR_VAR.to_string(),
        project_working_dir.to_string_lossy().into_owned(),
    );
    env.insert(
        FLOX_SANDBOX_GRANTS_DIR_VAR.to_string(),
        grants_dir.to_string_lossy().into_owned(),
    );
    env.insert(
        PRELOAD_VAR.to_string(),
        compose_preload(existing_preload, &libsandbox),
    );

    // TODO(next batch): set FLOX_SANDBOX_SOCKET to the broker verdict socket
    // path once the broker rides the executive. Until then the var stays
    // unset; the libsandbox `ask` stub denies when it is absent, which is the
    // correct fail-closed behavior before approvals exist.

    Ok(env)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;

    /// Create a fake package-builder libexec dir containing `flox-build.mk`
    /// and the platform libsandbox file, returning the dir so the caller can
    /// point `FLOX_BUILD_MK` at the makefile inside it.
    fn fake_libexec() -> TempDir {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("flox-build.mk"), b"# fake\n").unwrap();
        std::fs::write(tmp.path().join(LIBSANDBOX_FILENAME), b"\x7fELF").unwrap();
        tmp
    }

    /// A minimal seed context anchored in a tempdir so canonicalization has
    /// something real to resolve.
    fn minimal_seed_ctx(tmp: &TempDir) -> SeedContext {
        let interpreter = tmp.path().join("interpreter");
        std::fs::create_dir_all(&interpreter).unwrap();
        SeedContext {
            shell_binary: None,
            interpreter_path: interpreter,
            home_dir: None,
            runtime_dir: None,
        }
    }

    #[test]
    fn active_mode_sets_all_policy_vars() {
        let libexec = fake_libexec();
        let build_mk = libexec.path().join("flox-build.mk");
        let seed_tmp = TempDir::new().unwrap();
        let seed_ctx = minimal_seed_ctx(&seed_tmp);

        let env = temp_env::with_var("FLOX_BUILD_MK", Some(build_mk.as_os_str()), || {
            sandbox_env(
                SandboxMode::Enforce,
                &seed_ctx,
                Path::new("/project/dir"),
                Path::new("/project/dir/.flox/cache/sandbox"),
                None,
            )
            .unwrap()
        });

        assert_eq!(env.get(FLOX_VIRTUAL_SANDBOX_VAR).unwrap(), "enforce");
        assert_eq!(env.get(FLOX_SRC_DIR_VAR).unwrap(), "/project/dir");
        assert_eq!(
            env.get(FLOX_SANDBOX_GRANTS_DIR_VAR).unwrap(),
            "/project/dir/.flox/cache/sandbox"
        );
        assert!(env.contains_key(FLOX_SANDBOX_ALLOW_VAR));
        assert!(env.contains_key(FLOX_SANDBOX_ALLOW_DIRS_VAR));
        // The preload var points at the fake libsandbox inside the libexec.
        let expected_lib = libexec.path().join(LIBSANDBOX_FILENAME);
        assert_eq!(
            env.get(PRELOAD_VAR).unwrap(),
            expected_lib.to_str().unwrap()
        );
        // The verdict socket is NOT set in this batch (no broker yet).
        assert!(!env.contains_key(FLOX_SANDBOX_SOCKET_VAR));
    }

    #[test]
    fn missing_library_is_a_loud_error() {
        let tmp = TempDir::new().unwrap();
        // FLOX_BUILD_MK points into a dir with no libsandbox file.
        let build_mk = tmp.path().join("flox-build.mk");
        std::fs::write(&build_mk, b"# fake\n").unwrap();
        let seed_ctx = minimal_seed_ctx(&tmp);

        let result = temp_env::with_var("FLOX_BUILD_MK", Some(build_mk.as_os_str()), || {
            sandbox_env(
                SandboxMode::Warn,
                &seed_ctx,
                Path::new("/project"),
                Path::new("/grants"),
                None,
            )
        });
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("sandbox library was not found"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn off_mode_injects_nothing() {
        let ctx = SeedContext {
            shell_binary: None,
            interpreter_path: PathBuf::from("/interpreter"),
            home_dir: None,
            runtime_dir: None,
        };
        let env = sandbox_env(
            SandboxMode::Off,
            &ctx,
            Path::new("/project"),
            Path::new("/grants"),
            None,
        )
        .unwrap();
        assert!(env.is_empty());
    }

    #[test]
    fn compose_preload_appends_to_existing() {
        let lib = Path::new("/nix/store/x/libexec/libsandbox.so");
        assert_eq!(
            compose_preload(None, lib),
            "/nix/store/x/libexec/libsandbox.so"
        );
        assert_eq!(
            compose_preload(Some("/other/lib.so"), lib),
            "/other/lib.so:/nix/store/x/libexec/libsandbox.so"
        );
        // Whitespace-only existing values are treated as absent.
        assert_eq!(
            compose_preload(Some("   "), lib),
            "/nix/store/x/libexec/libsandbox.so"
        );
    }

    #[test]
    fn libsandbox_filename_matches_platform() {
        if cfg!(target_os = "macos") {
            assert_eq!(LIBSANDBOX_FILENAME, "libsandbox.dylib");
            assert_eq!(PRELOAD_VAR, "DYLD_INSERT_LIBRARIES");
        } else {
            assert_eq!(LIBSANDBOX_FILENAME, "libsandbox.so");
            assert_eq!(PRELOAD_VAR, "LD_PRELOAD");
        }
    }
}
