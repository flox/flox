//! Seed allow-set for sandboxed activations.
//!
//! When an activation runs under `--sandbox`, libsandbox mediates file
//! access against an allow-set carried in `FLOX_SANDBOX_ALLOW` (globs) and
//! `FLOX_SANDBOX_ALLOW_DIRS` (directory prefixes). Without a starting set,
//! the very first interactive session drowns in receipts: the user's shell
//! reads its rc files, the terminal driver reads terminfo, libc reads the
//! locale archive, and every one of those is "out of policy".
//!
//! This module computes a quiet baseline. The entries fall into two engine
//! buckets:
//!
//! - `FLOX_SANDBOX_ALLOW` — fnmatch globs, matched against the resolved
//!   realpath of each access. Single files (the shell binary, the
//!   interpreter) and recursive trees (flox's own config/cache, written as
//!   `<dir>/**`) live here.
//! - `FLOX_SANDBOX_ALLOW_DIRS` — directory prefixes, also compared against
//!   realpaths. System config and data directories (`/etc`, terminfo,
//!   locale) live here.
//!
//! The engine auto-adds the project working directory from `FLOX_SRC_DIR`
//! and the Nix closure, so those are not repeated here. libsandbox does not
//! expand `~` or environment references, so every path this module emits is
//! already absolute with `$HOME` resolved. The allow-list entries are
//! tokenized on spaces by the engine, so a path containing a space cannot be
//! expressed; such paths are dropped rather than corrupting the list.
//!
//! Why the dotfiles are seeded at all: under `ask` the engine deliberately
//! stops waving through everything under `$HOME/.` (the build-purity
//! carve-out is backwards for an interactive agent threat model). Seeding
//! the shell's own rc/history files keeps the first prompt quiet without
//! re-opening that carve-out wholesale.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The computed allow-set for one sandboxed activation.
///
/// `allow` populates `FLOX_SANDBOX_ALLOW` (fnmatch globs); `allow_dirs`
/// populates `FLOX_SANDBOX_ALLOW_DIRS` (directory prefixes); `allow_net`
/// populates `FLOX_SANDBOX_ALLOW_NET` (network destinations). All render to
/// space-separated strings via [`Self::allow_value`],
/// [`Self::allow_dirs_value`], and [`Self::allow_net_value`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SeedAllowSet {
    /// fnmatch globs for `FLOX_SANDBOX_ALLOW`.
    pub allow: Vec<String>,
    /// Directory prefixes for `FLOX_SANDBOX_ALLOW_DIRS`.
    pub allow_dirs: Vec<String>,
    /// Network destinations (`host[:port]` / `ip[/cidr][:port]`) for
    /// `FLOX_SANDBOX_ALLOW_NET`.
    pub allow_net: Vec<String>,
}

/// Network destinations seeded into `FLOX_SANDBOX_ALLOW_NET` on every
/// sandboxed activation.
///
/// Four kinds of entry:
///
/// - Loopback. libsandbox already treats loopback as always-allowed without
///   consulting the policy, so these are belt-and-suspenders — harmless to
///   list and a clear signal of intent for anyone reading the rendered env
///   var.
/// - Flox's own service hosts. flox commands run *inside* an activation
///   (`flox install`, `flox pull`, catalog resolution) reach out to FloxHub
///   and the catalog API. Without these seeds an `enforce`/`ask` session
///   would block flox's own network calls, which is never the user's intent
///   — the sandbox is meant to mediate the *workload's* egress, not flox's.
/// - Git hosting and release-download hosts. An agent that clones, fetches,
///   or downloads a release archive needs GitHub and its CDN reachable, or
///   `enforce` blocks routine version-control work on the first fetch.
/// - Language package registries. `npm`/`pip`/`cargo` install from these
///   hosts; seeding them lets an agent install dependencies without a manual
///   grant. These are best-effort hostname matches via the engine's
///   getaddrinfo attribution cache (a CDN that resolves through an
///   unobserved path falls back to the per-session grant) — acceptable for
///   the prototype's autonomy goal.
///
/// The flox hosts mirror the SDK defaults: `hub.flox.dev`
/// (`flox-rust-sdk::flox::DEFAULT_FLOXHUB_URL`) and `api.flox.dev`
/// (`flox-catalog::DEFAULT_CATALOG_URL`). All entries are hostname entries (no
/// port), so libsandbox matches them against the getaddrinfo attribution cache
/// for any port. If a deployment overrides a URL, the operator extends
/// `FLOX_SANDBOX_ALLOW_NET` or grants the host interactively; seeding the
/// defaults keeps the common case quiet without hard-coding a port.
const NET_SEEDS: &[&str] = &[
    // Loopback (also auto-allowed by the engine; listed for clarity).
    "127.0.0.1",
    "::1",
    // FloxHub — environment push/pull and auth.
    "hub.flox.dev",
    // Flox Catalog API — package search and resolution.
    "api.flox.dev",
    // Git hosting and release downloads (clone/fetch/pull, release archives).
    "github.com",
    "codeload.github.com",
    "objects.githubusercontent.com",
    "raw.githubusercontent.com",
    // npm registry.
    "registry.npmjs.org",
    // Python package index and its file CDN.
    "pypi.org",
    "files.pythonhosted.org",
    // Rust crates registry (index + downloads).
    "crates.io",
    "static.crates.io",
    "index.crates.io",
];

/// Inputs the seed needs that are not derivable from the process
/// environment alone. Keeping these explicit makes the seed unit-testable
/// without a live activation.
#[derive(Debug, Clone)]
pub struct SeedContext {
    /// The user's login shell binary (`$SHELL`), if known. Seeded so the
    /// shell that the activation execs does not trip the out-of-closure-exe
    /// warning on its own binary.
    pub shell_binary: Option<PathBuf>,
    /// The activation interpreter directory (`interpreter_path`). Its
    /// realpath is seeded so the activate scripts and their assets read
    /// quietly.
    pub interpreter_path: PathBuf,
    /// The user's home directory. Dotfiles and flox state are anchored here.
    pub home_dir: Option<PathBuf>,
    /// The per-activation runtime directory (`$FLOX_RUNTIME_DIR`), if known.
    /// Seeded so flox's own bookkeeping reads do not generate receipts.
    pub runtime_dir: Option<PathBuf>,
}

/// Shell rc, profile, and history files seeded under `$HOME`.
///
/// Needed because `ask` flips the `$HOME`-dotfile carve-out: without these,
/// the first interactive shell would queue a receipt for reading its own
/// startup files. Covers the zsh and bash families plus the shared
/// `.profile`/`.inputrc`.
const SHELL_DOTFILES: &[&str] = &[
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zsh_history",
    ".bashrc",
    ".bash_profile",
    ".bash_history",
    ".profile",
    ".inputrc",
];

/// Routine, non-sensitive developer config files seeded under `$HOME` as
/// ALLOW globs.
///
/// Under `ask` the engine flips the `$HOME`-dotfile carve-out, so without
/// these every `git`, `npm`, `pip`, or `cargo` invocation would queue a
/// receipt for reading its own config. These are deliberately *non-sensitive*:
/// they hold tool preferences (editor, registry URL, build profile), never
/// credentials. Secrets live in the sensitive set the engine denies even under
/// `enforce` (`~/.ssh`, `~/.aws`, `~/.netrc`, `~/.config/gh`, `**/.env`, ...),
/// which is why none of those appears here — seeding a credential path would
/// defeat the denial. flox's own state (`~/.config/flox/**`,
/// `~/.cache/flox/**`) is seeded separately in [`SeedAllowSet::compute`].
const DEV_CONFIG_GLOBS: &[&str] = &[
    // git: user config and the XDG config dir (excludes ~/.config/gh, which is
    // sensitive and seeded nowhere).
    ".gitconfig",
    ".config/git/**",
    // npm.
    ".npmrc",
    ".config/npm/**",
    // cargo: both the legacy and current config filenames.
    ".cargo/config",
    ".cargo/config.toml",
    // pip.
    ".config/pip/**",
    ".pip/**",
    // rustup toolchain selection.
    ".rustup/settings.toml",
];

/// System directories seeded as allow-dirs on every platform.
///
/// `/etc` and its macOS realpath `/private/etc` hold the host config that
/// libc and name resolution read on startup.
///
/// `/nix/store` is the immutable, content-addressed, world-readable package
/// store. Under a build sandbox an out-of-closure store read is a
/// reproducibility violation, but for an activation it is benign: a host tool
/// run from outside the environment closure (git from another env, the coding
/// agent, python) reads its own package files, and those reads are the
/// dominant noise class an enforce session would otherwise surface. Allowing
/// every store read carries no exfiltration or destruction risk — the store
/// holds public packages, not user data — so it is seeded unconditionally.
/// Builds compose their own allow-dirs via flox-build.mk and never consult
/// this seed, so build behaviour is unaffected.
const SYSTEM_ALLOW_DIRS: &[&str] = &["/etc", "/private/etc", "/nix/store"];

/// Best-effort terminfo and locale locations.
///
/// These are not guaranteed to exist on a given host, so each is included
/// only if present. Terminal libraries (ncurses, readline) read terminfo on
/// the first prompt; glibc reads the locale archive. Missing entries are
/// simply skipped — their absence means nothing would read them anyway.
const TERMINFO_AND_LOCALE_DIRS: &[&str] = &[
    "/usr/share/terminfo",
    "/lib/terminfo",
    "/etc/terminfo",
    "/usr/share/locale",
    "/usr/lib/locale",
    "/run/current-system/sw/share/terminfo",
];

impl SeedAllowSet {
    /// Compute the seed allow-set for an activation.
    ///
    /// All filesystem probing is best-effort: a path that cannot be resolved
    /// or does not exist is dropped, never fatal. The result is
    /// deduplicated and stable-ordered so the rendered env var is
    /// deterministic (which keeps the activation diff and tests stable).
    pub fn compute(ctx: &SeedContext) -> Self {
        let mut allow: Vec<String> = Vec::new();
        let mut allow_dirs: Vec<String> = Vec::new();

        // The shell binary the activation execs: seed its realpath so the
        // engine's out-of-closure-exe check does not fire on, e.g.,
        // /bin/zsh.
        if let Some(shell) = ctx.shell_binary.as_deref() {
            push_realpath(&mut allow, shell);
        }

        // The interpreter directory holds the activate scripts and assets
        // the shell sources on entry.
        push_realpath(&mut allow, &ctx.interpreter_path);

        // System config dirs (realpath-compared by the engine).
        for dir in SYSTEM_ALLOW_DIRS {
            push_realpath_dir(&mut allow_dirs, Path::new(dir));
        }
        for dir in TERMINFO_AND_LOCALE_DIRS {
            push_realpath_dir(&mut allow_dirs, Path::new(dir));
        }

        if let Some(home) = ctx.home_dir.as_deref() {
            // Shell rc/history files: seeded literally (home-expanded) so the
            // dotfile-carve-out flip under ask does not bury the first
            // prompt in receipts. These are globs, not allow-dirs, so a
            // missing file is harmless — fnmatch simply never matches it.
            for dotfile in SHELL_DOTFILES {
                push_glob(&mut allow, home.join(dotfile));
            }

            // Routine, non-sensitive dev tool configs (git, npm, pip, cargo,
            // rustup) so an `ask` session does not prompt for them. Credential
            // paths are deliberately excluded — they stay denied via the
            // engine's sensitive set.
            for config in DEV_CONFIG_GLOBS {
                push_glob(&mut allow, home.join(config));
            }

            // Flox's own config and cache trees, written as recursive globs
            // so flox commands run inside the session (including
            // `flox sandbox`) read their own state without generating
            // receipts.
            push_glob(&mut allow, home.join(".config/flox/**"));
            push_glob(&mut allow, home.join(".cache/flox/**"));
        }

        // The per-activation runtime dir holds activation state and (in a
        // later batch) the broker sockets; flox reads it constantly.
        if let Some(runtime) = ctx.runtime_dir.as_deref() {
            push_glob(&mut allow, runtime.join("**"));
        }

        // Network destinations are fixed seeds (loopback + flox's own service
        // hosts); they do not depend on the filesystem, so no probing is
        // needed. Grants and overrides are layered on top later by the broker
        // batch — this is just the quiet baseline.
        let allow_net: Vec<String> = NET_SEEDS.iter().map(|s| s.to_string()).collect();

        Self {
            allow: dedup_preserving_order(allow),
            allow_dirs: dedup_preserving_order(allow_dirs),
            allow_net: dedup_preserving_order(allow_net),
        }
    }

    /// Render `FLOX_SANDBOX_ALLOW` as the engine expects it: entries
    /// separated by single spaces.
    pub fn allow_value(&self) -> String {
        self.allow.join(" ")
    }

    /// Render `FLOX_SANDBOX_ALLOW_DIRS` as the engine expects it: entries
    /// separated by single spaces.
    pub fn allow_dirs_value(&self) -> String {
        self.allow_dirs.join(" ")
    }

    /// Render `FLOX_SANDBOX_ALLOW_NET` as the engine expects it: entries
    /// separated by single spaces.
    pub fn allow_net_value(&self) -> String {
        self.allow_net.join(" ")
    }
}

/// Push the canonicalized form of `path` as an ALLOW glob, if it resolves
/// and contains no space (the engine tokenizes on spaces). Paths that fail
/// to canonicalize are dropped — they do not exist, so nothing reads them.
fn push_realpath(out: &mut Vec<String>, path: &Path) {
    if let Ok(real) = std::fs::canonicalize(path) {
        push_str(out, real);
    }
}

/// Push the canonicalized form of `dir` as an allow-dir prefix, if it
/// resolves and contains no space. Allow-dirs are realpath-compared by the
/// engine, so canonicalization is required for a match.
fn push_realpath_dir(out: &mut Vec<String>, dir: &Path) {
    if let Ok(real) = std::fs::canonicalize(dir) {
        push_str(out, real);
    }
}

/// Push a glob entry verbatim (no canonicalization — a `**` suffix is not a
/// real path), dropping it if it contains a space.
fn push_glob(out: &mut Vec<String>, path: PathBuf) {
    push_str(out, path);
}

/// Shared tail for the push helpers: stringify, reject space-containing
/// entries (unrepresentable in the space-tokenized list), and append.
fn push_str(out: &mut Vec<String>, path: PathBuf) {
    let Some(s) = path.to_str() else { return };
    if s.contains(' ') {
        return;
    }
    out.push(s.to_string());
}

/// Deduplicate while preserving first-seen order. Order stability keeps the
/// rendered env var deterministic across activations.
fn dedup_preserving_order(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|item| seen.insert(item.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// Build a context whose paths all live under one tempdir so the
    /// canonicalization-based helpers have something real to resolve.
    fn ctx_in(tmp: &TempDir) -> (SeedContext, PathBuf) {
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let interpreter = tmp.path().join("interpreter");
        fs::create_dir_all(&interpreter).unwrap();
        let shell = tmp.path().join("bin").join("zsh");
        fs::create_dir_all(shell.parent().unwrap()).unwrap();
        fs::write(&shell, b"#!/bin/sh\n").unwrap();
        let runtime = tmp.path().join("runtime");
        fs::create_dir_all(&runtime).unwrap();
        (
            SeedContext {
                shell_binary: Some(shell),
                interpreter_path: interpreter.clone(),
                home_dir: Some(home),
                runtime_dir: Some(runtime),
            },
            interpreter,
        )
    }

    #[test]
    fn seeds_shell_interpreter_and_flox_state() {
        let tmp = TempDir::new().unwrap();
        let (ctx, interpreter) = ctx_in(&tmp);
        let home = ctx.home_dir.clone().unwrap();
        let runtime = ctx.runtime_dir.clone().unwrap();
        let seed = SeedAllowSet::compute(&ctx);

        let interpreter_real = fs::canonicalize(&interpreter).unwrap();
        let shell_real = fs::canonicalize(ctx.shell_binary.as_ref().unwrap()).unwrap();

        // Shell binary and interpreter are present as ALLOW globs (realpaths).
        assert!(
            seed.allow
                .contains(&shell_real.to_str().unwrap().to_string())
        );
        assert!(
            seed.allow
                .contains(&interpreter_real.to_str().unwrap().to_string())
        );

        // Each shell dotfile is present, home-expanded, exactly once.
        for dotfile in SHELL_DOTFILES {
            let expected = home.join(dotfile).to_str().unwrap().to_string();
            assert!(
                seed.allow.contains(&expected),
                "missing dotfile seed: {expected}"
            );
        }

        // Flox state trees and the runtime dir are recursive globs.
        assert!(
            seed.allow
                .contains(&home.join(".config/flox/**").to_str().unwrap().to_string())
        );
        assert!(
            seed.allow
                .contains(&home.join(".cache/flox/**").to_str().unwrap().to_string())
        );
        assert!(
            seed.allow
                .contains(&runtime.join("**").to_str().unwrap().to_string())
        );
    }

    #[test]
    fn seeds_loopback_and_flox_service_hosts_for_net() {
        let tmp = TempDir::new().unwrap();
        let (ctx, _) = ctx_in(&tmp);
        let seed = SeedAllowSet::compute(&ctx);

        // Loopback is listed (also auto-allowed by the engine, but seeded for
        // an explicit, readable policy).
        assert!(seed.allow_net.contains(&"127.0.0.1".to_string()));
        assert!(seed.allow_net.contains(&"::1".to_string()));

        // Flox's own service hosts must be present so flox commands run inside
        // the activation are not blocked under enforce/ask.
        assert!(
            seed.allow_net.contains(&"hub.flox.dev".to_string()),
            "FloxHub host must be seeded, got {:?}",
            seed.allow_net
        );
        assert!(
            seed.allow_net.contains(&"api.flox.dev".to_string()),
            "catalog API host must be seeded, got {:?}",
            seed.allow_net
        );

        // The net seeds do not depend on the filesystem context, so they are
        // present even with a minimal context.
        let minimal = SeedContext {
            shell_binary: None,
            interpreter_path: ctx.interpreter_path.clone(),
            home_dir: None,
            runtime_dir: None,
        };
        let minimal_seed = SeedAllowSet::compute(&minimal);
        assert_eq!(minimal_seed.allow_net, seed.allow_net);
    }

    #[test]
    fn allow_net_value_is_space_separated_and_quiet_for_empty() {
        let tmp = TempDir::new().unwrap();
        let (ctx, _) = ctx_in(&tmp);
        let seed = SeedAllowSet::compute(&ctx);

        let net = seed.allow_net_value();
        assert!(!net.contains("  "));
        assert_eq!(net.split(' ').count(), seed.allow_net.len());

        assert_eq!(SeedAllowSet::default().allow_net_value(), "");
    }

    #[test]
    fn allow_dirs_include_etc_when_present() {
        let tmp = TempDir::new().unwrap();
        let (ctx, _) = ctx_in(&tmp);
        let seed = SeedAllowSet::compute(&ctx);

        // /etc exists on every supported host; its realpath must be present.
        let etc_real = fs::canonicalize("/etc").unwrap();
        assert!(
            seed.allow_dirs
                .contains(&etc_real.to_str().unwrap().to_string()),
            "expected /etc realpath in allow_dirs, got {:?}",
            seed.allow_dirs
        );
    }

    #[test]
    fn allow_dirs_include_nix_store() {
        let tmp = TempDir::new().unwrap();
        let (ctx, _) = ctx_in(&tmp);
        let seed = SeedAllowSet::compute(&ctx);

        // /nix/store is the dominant read source for host tools run inside an
        // activation; seeding it as an allow-dir erases that noise class. It
        // exists on every Flox host, so its realpath must be present.
        let store_real = fs::canonicalize("/nix/store").unwrap();
        assert!(
            seed.allow_dirs
                .contains(&store_real.to_str().unwrap().to_string()),
            "expected /nix/store realpath in allow_dirs, got {:?}",
            seed.allow_dirs
        );
    }

    #[test]
    fn net_seeds_include_git_hosts_and_language_registries() {
        let tmp = TempDir::new().unwrap();
        let (ctx, _) = ctx_in(&tmp);
        let seed = SeedAllowSet::compute(&ctx);

        // Git hosting and release downloads, plus the npm/pip/cargo registries
        // an agent needs so `enforce` does not block routine clone/install
        // work. The flox hosts stay present alongside them.
        let expected = [
            "github.com",
            "codeload.github.com",
            "objects.githubusercontent.com",
            "raw.githubusercontent.com",
            "registry.npmjs.org",
            "pypi.org",
            "files.pythonhosted.org",
            "crates.io",
            "static.crates.io",
            "index.crates.io",
            "hub.flox.dev",
            "api.flox.dev",
        ];
        for host in expected {
            assert!(
                seed.allow_net.contains(&host.to_string()),
                "expected {host} in allow_net, got {:?}",
                seed.allow_net
            );
        }
    }

    #[test]
    fn allow_seeds_non_sensitive_dev_configs_and_omits_sensitive_paths() {
        let tmp = TempDir::new().unwrap();
        let (ctx, _) = ctx_in(&tmp);
        let home = ctx.home_dir.clone().unwrap();
        let seed = SeedAllowSet::compute(&ctx);

        // Routine, non-sensitive dev configs are present (home-expanded) so an
        // `ask` session does not prompt for them.
        let expected_present = [
            ".gitconfig",
            ".config/git/**",
            ".npmrc",
            ".config/npm/**",
            ".cargo/config",
            ".cargo/config.toml",
            ".config/pip/**",
            ".pip/**",
            ".rustup/settings.toml",
        ];
        for config in expected_present {
            let expected = home.join(config).to_str().unwrap().to_string();
            assert!(
                seed.allow.contains(&expected),
                "missing dev-config seed: {expected}"
            );
        }

        // No sensitive path is ever seeded: those must stay denied by the
        // engine's sensitive set even under `enforce`. Seeding any of them
        // would defeat the denial.
        let forbidden_fragments = [
            "/.ssh",
            "/.aws",
            "/.gnupg",
            "/.kube",
            "/.netrc",
            "/.config/gh",
            ".env",
        ];
        for fragment in forbidden_fragments {
            assert!(
                seed.allow.iter().all(|e| !e.contains(fragment)),
                "sensitive fragment {fragment:?} leaked into the seed: {:?}",
                seed.allow
            );
        }
    }

    #[test]
    fn rendered_values_are_space_separated_and_quiet_for_empty() {
        let tmp = TempDir::new().unwrap();
        let (ctx, _) = ctx_in(&tmp);
        let seed = SeedAllowSet::compute(&ctx);

        // Space-separated, and every token is non-empty (no double spaces).
        let allow = seed.allow_value();
        assert!(!allow.contains("  "));
        assert_eq!(allow.split(' ').count(), seed.allow.len());

        let empty = SeedAllowSet::default();
        assert_eq!(empty.allow_value(), "");
        assert_eq!(empty.allow_dirs_value(), "");
    }

    #[test]
    fn missing_optional_inputs_do_not_panic_or_emit_empty_entries() {
        let tmp = TempDir::new().unwrap();
        let interpreter = tmp.path().join("interpreter");
        fs::create_dir_all(&interpreter).unwrap();
        let ctx = SeedContext {
            shell_binary: None,
            interpreter_path: interpreter,
            home_dir: None,
            runtime_dir: None,
        };
        let seed = SeedAllowSet::compute(&ctx);

        // No home means no dotfile or flox-state entries.
        assert!(seed.allow.iter().all(|e| !e.contains("/.zshrc")));
        // No empty strings ever leak into the list.
        assert!(seed.allow.iter().all(|e| !e.is_empty()));
        assert!(seed.allow_dirs.iter().all(|e| !e.is_empty()));
    }

    #[test]
    fn entries_are_deduplicated() {
        // A shell binary that resolves to the same realpath as the
        // interpreter would otherwise appear twice; dedup keeps one.
        let tmp = TempDir::new().unwrap();
        let shared = tmp.path().join("shared");
        fs::create_dir_all(&shared).unwrap();
        let ctx = SeedContext {
            shell_binary: Some(shared.clone()),
            interpreter_path: shared.clone(),
            home_dir: None,
            runtime_dir: None,
        };
        let seed = SeedAllowSet::compute(&ctx);
        let shared_real = fs::canonicalize(&shared)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(
            seed.allow.iter().filter(|e| **e == shared_real).count(),
            1,
            "duplicate realpath should be collapsed"
        );
    }
}
