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
/// `FLOX_SANDBOX_ALLOW_NET` — space-separated network destinations
/// (`host[:port]` / `ip[/cidr][:port]`) the workload may connect to.
pub const FLOX_SANDBOX_ALLOW_NET_VAR: &str = "FLOX_SANDBOX_ALLOW_NET";
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
/// `FLOX_SANDBOX_ALLOW_FOREIGN_EXE` — disables libsandbox's
/// executable-identity check. A build runs its toolchain from inside the
/// closure, so an out-of-closure process executable is a reproducibility
/// defect worth reporting or aborting on. An activation is the opposite: it
/// deliberately runs the user's shell and host tools (the coding agent, git,
/// python) from outside the closure and mediates only file/network access, so
/// the activation sets this to keep the inner shell from aborting. Builds
/// never set it.
pub const FLOX_SANDBOX_ALLOW_FOREIGN_EXE_VAR: &str = "FLOX_SANDBOX_ALLOW_FOREIGN_EXE";

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

/// Derive the ask broker's verdict-socket path from the services socket path.
///
/// The broker rides the per-activation executive and binds a Unix socket the
/// preloaded libsandbox connects to for `ask` verdicts. Two sides must agree
/// on that path with no shared mutable state: the executive (which binds and
/// listens) and the env injection in [`crate::attach_diff::double_set_envs`]
/// (which exports it as `FLOX_SANDBOX_SOCKET`). Both already carry the
/// services socket path — `runtime_dir/flox.<id>.sock` — so deriving the
/// verdict path from it as `runtime_dir/sbx.<id>.sock` keeps the agreement a
/// pure function of one value both sides hold, with no second channel to keep
/// in sync. The `<id>` substring is preserved verbatim, so the verdict socket
/// stays as short as the services socket and respects the same macOS 104-char
/// limit the services socket already cleared.
///
/// A later batch's control socket sits beside this one as `sbc.<id>.sock`,
/// derived the same way.
pub fn verdict_socket_path(services_socket: &Path) -> PathBuf {
    socket_sibling_path(services_socket, "sbx")
}

/// Rewrite a `flox.<id>.sock` services socket path into a sibling socket in
/// the same directory with a different short prefix (`sbx` for the verdict
/// socket). When the file name does not match the expected `flox.*.sock`
/// shape — which should not happen in practice — fall back to appending the
/// prefix as an extra extension so the result is still deterministic and
/// unique per services socket.
fn socket_sibling_path(services_socket: &Path, prefix: &str) -> PathBuf {
    let dir = services_socket.parent().unwrap_or_else(|| Path::new("."));
    let file = services_socket
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let rewritten = match file.strip_prefix("flox.") {
        Some(rest) => format!("{prefix}.{rest}"),
        None => format!("{file}.{prefix}.sock"),
    };
    dir.join(rewritten)
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

/// Compose `FLOX_SANDBOX_ALLOW_NET` from any operator-supplied value plus the
/// flox seeds, space-separated and deduplicated with the operator entries
/// first. Unlike the other allow-sets, the network policy honors an inherited
/// value so a CI step (or a one-off `FLOX_SANDBOX_ALLOW_NET=host flox
/// activate`) can extend it without losing flox's own service hosts.
fn merge_allow_net(existing: Option<&str>, seed: &SeedAllowSet) -> String {
    let mut entries: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut push = |entry: &str| {
        let entry = entry.trim();
        if !entry.is_empty() && seen.insert(entry.to_string()) {
            entries.push(entry.to_string());
        }
    };
    if let Some(existing) = existing {
        for entry in existing.split_whitespace() {
            push(entry);
        }
    }
    for entry in &seed.allow_net {
        push(entry);
    }
    entries.join(" ")
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
///
/// `existing_allow_net` is any `FLOX_SANDBOX_ALLOW_NET` the caller already
/// had set (e.g. a CI step pre-seeding extra hosts before `flox activate`).
/// Its entries are merged ahead of the flox seeds rather than discarded, so an
/// operator can extend the network policy from outside the session. The
/// filesystem allow-set is seed-only because its inputs are derived, not
/// operator-supplied; the network policy is the one allow-set a human is
/// likely to want to pre-populate, so it honors an inherited value.
pub fn sandbox_env(
    mode: SandboxMode,
    seed_ctx: &SeedContext,
    project_working_dir: &Path,
    grants_dir: &Path,
    verdict_socket: &Path,
    existing_preload: Option<&str>,
    existing_allow_net: Option<&str>,
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
        FLOX_SANDBOX_ALLOW_NET_VAR.to_string(),
        merge_allow_net(existing_allow_net, &seed),
    );
    env.insert(
        FLOX_SRC_DIR_VAR.to_string(),
        project_working_dir.to_string_lossy().into_owned(),
    );
    env.insert(
        FLOX_SANDBOX_GRANTS_DIR_VAR.to_string(),
        grants_dir.to_string_lossy().into_owned(),
    );
    // Disable the executable-identity check for every active mode. An
    // activation runs the user's shell and host tools from outside the
    // closure on purpose; without this the inner shell would abort under
    // enforce/ask before the user's command ran. Builds never reach this
    // path, so build behaviour is untouched.
    env.insert(
        FLOX_SANDBOX_ALLOW_FOREIGN_EXE_VAR.to_string(),
        "1".to_string(),
    );
    env.insert(
        PRELOAD_VAR.to_string(),
        compose_preload(existing_preload, &libsandbox),
    );

    // The verdict socket is the libsandbox `ask` RPC rendezvous. Only `ask`
    // runs a broker, so only `ask` exports the socket; warn/enforce never
    // contact a broker and leaving the var unset for them keeps their wire
    // behavior unchanged. The path is a pure function of the services socket
    // (see `verdict_socket_path`), so the broker binds the same path the
    // engine connects to without a second channel to synchronize. If the
    // broker is absent (e.g. a container activation with no executive), the
    // socket simply never appears and the engine fail-closes — the same
    // outcome as an unreachable broker.
    if mode == SandboxMode::Ask {
        env.insert(
            FLOX_SANDBOX_SOCKET_VAR.to_string(),
            verdict_socket.to_string_lossy().into_owned(),
        );
    }

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
                Path::new("/run/sbx.abc.sock"),
                None,
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
        // An active mode exempts the inner shell from the executable-identity
        // check, which is a build-only heuristic: an activation runs host
        // tools from outside the closure on purpose.
        assert_eq!(env.get(FLOX_SANDBOX_ALLOW_FOREIGN_EXE_VAR).unwrap(), "1");
        // The network allow-list is seeded with loopback and flox's own hosts.
        let allow_net = env.get(FLOX_SANDBOX_ALLOW_NET_VAR).unwrap();
        assert!(
            allow_net.contains("hub.flox.dev") && allow_net.contains("api.flox.dev"),
            "expected flox service hosts in FLOX_SANDBOX_ALLOW_NET, got {allow_net:?}"
        );
        // The preload var points at the fake libsandbox inside the libexec.
        let expected_lib = libexec.path().join(LIBSANDBOX_FILENAME);
        assert_eq!(
            env.get(PRELOAD_VAR).unwrap(),
            expected_lib.to_str().unwrap()
        );
        // The verdict socket is set only for `ask`; enforce never contacts a
        // broker, so it stays unset here.
        assert!(!env.contains_key(FLOX_SANDBOX_SOCKET_VAR));
    }

    #[test]
    fn ask_mode_exports_verdict_socket() {
        let libexec = fake_libexec();
        let build_mk = libexec.path().join("flox-build.mk");
        let seed_tmp = TempDir::new().unwrap();
        let seed_ctx = minimal_seed_ctx(&seed_tmp);

        let env = temp_env::with_var("FLOX_BUILD_MK", Some(build_mk.as_os_str()), || {
            sandbox_env(
                SandboxMode::Ask,
                &seed_ctx,
                Path::new("/project/dir"),
                Path::new("/project/dir/.flox/cache/sandbox"),
                Path::new("/run/sbx.abc.sock"),
                None,
                None,
            )
            .unwrap()
        });

        // Ask runs a broker, so the verdict socket is exported and matches the
        // path the broker is expected to bind.
        assert_eq!(
            env.get(FLOX_SANDBOX_SOCKET_VAR).unwrap(),
            "/run/sbx.abc.sock"
        );
    }

    #[test]
    fn verdict_socket_path_is_a_sibling_of_the_services_socket() {
        // The verdict socket preserves the `<id>` and lives next to the
        // services socket, swapping only the `flox` prefix for `sbx` so both
        // the broker and the env injection compute the identical path.
        assert_eq!(
            verdict_socket_path(Path::new("/run/user/1000/flox.deadbeef.sock")),
            PathBuf::from("/run/user/1000/sbx.deadbeef.sock"),
        );
        // An unexpected services socket shape still yields a deterministic,
        // unique sibling rather than panicking.
        assert_eq!(
            verdict_socket_path(Path::new("/run/custom.sock")),
            PathBuf::from("/run/custom.sock.sbx.sock"),
        );
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
                Path::new("/run/sbx.abc.sock"),
                None,
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
            Path::new("/run/sbx.abc.sock"),
            None,
            None,
        )
        .unwrap();
        assert!(env.is_empty());
        // In particular the foreign-exe exemption is absent when off, so the
        // executable-identity check keeps its build-time behaviour.
        assert!(!env.contains_key(FLOX_SANDBOX_ALLOW_FOREIGN_EXE_VAR));
    }

    #[test]
    fn active_mode_merges_inherited_allow_net() {
        let libexec = fake_libexec();
        let build_mk = libexec.path().join("flox-build.mk");
        let seed_tmp = TempDir::new().unwrap();
        let seed_ctx = minimal_seed_ctx(&seed_tmp);

        let env = temp_env::with_var("FLOX_BUILD_MK", Some(build_mk.as_os_str()), || {
            sandbox_env(
                SandboxMode::Enforce,
                &seed_ctx,
                Path::new("/project"),
                Path::new("/grants"),
                Path::new("/run/sbx.abc.sock"),
                None,
                Some("example.com 10.0.0.0/8"),
            )
            .unwrap()
        });

        // The operator-supplied entries come first and the flox seeds follow,
        // so a pre-set FLOX_SANDBOX_ALLOW_NET extends rather than replaces.
        let allow_net = env.get(FLOX_SANDBOX_ALLOW_NET_VAR).unwrap();
        assert!(allow_net.contains("example.com"), "got {allow_net:?}");
        assert!(allow_net.contains("10.0.0.0/8"), "got {allow_net:?}");
        assert!(allow_net.contains("api.flox.dev"), "got {allow_net:?}");
    }

    #[test]
    fn merge_allow_net_dedups_and_orders_existing_first() {
        let seed = SeedAllowSet {
            allow_net: vec!["api.flox.dev".to_string(), "hub.flox.dev".to_string()],
            ..Default::default()
        };
        // Empty/absent existing yields just the seeds.
        assert_eq!(merge_allow_net(None, &seed), "api.flox.dev hub.flox.dev");
        // Existing entries come first; a duplicate (api.flox.dev) is collapsed.
        assert_eq!(
            merge_allow_net(Some("example.com api.flox.dev"), &seed),
            "example.com api.flox.dev hub.flox.dev"
        );
        // Whitespace-only existing is treated as absent.
        assert_eq!(
            merge_allow_net(Some("   "), &seed),
            "api.flox.dev hub.flox.dev"
        );
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
