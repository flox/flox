//! The `oci` sandbox backend: run the containerized environment image directly.
//!
//! Unlike the `host-native` and `srt` backends — which re-exec the host `flox`
//! binary inside an OS-level sandbox boundary — the `oci` backend runs the
//! **containerized environment image** directly. On macOS the guest is a Linux
//! VM, so the host Darwin `flox` binary cannot be exec'd inside the container;
//! instead the image's own baked entrypoint (produced by `flox containerize`)
//! handles activation, and the project directory is bind-mounted at its
//! identical absolute path so the agent's working tree is visible inside the
//! container.
//!
//! Runtime selection: Apple Container (`container`) on macOS, Podman (`podman`)
//! on Linux.

use std::convert::Infallible;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flox_core::activate::context::InvocationType;
use flox_config::Config;
use flox_core::activate::sandbox_backend::SandboxBackend;
use flox_manifest::lockfile::Lockfile;
use tracing::debug;

use super::{ActivationSandbox, SandboxLaunchCtx};

/// Environment variable that bypasses all staleness logic and uses the
/// specified image reference as-is. When set and non-empty, the hash-tag
/// scheme is skipped entirely.
pub(crate) const FLOX_SANDBOX_OCI_IMAGE_VAR: &str = "FLOX_SANDBOX_OCI_IMAGE";

/// Environment variable that opts in to running the newest existing image for
/// the environment even when the expected hash-tag is absent. A warning is
/// printed naming the stale tag. Default: off.
pub(crate) const FLOX_SANDBOX_OCI_ALLOW_STALE_VAR: &str = "FLOX_SANDBOX_OCI_ALLOW_STALE";

/// Environment variable that triggers an automatic bake when the expected
/// hash-tag is absent, without prompting (non-interactive equivalent of the
/// tty prompt). Default: off.
///
/// This name is the config env-layer spelling of the `sandbox_oci_autobake`
/// config key (see `flox_config::FloxConfig`), so the value must be
/// `true`/`false` — `1` fails config parsing as an integer. The value is read
/// via `config.flox.sandbox_oci_autobake`, not directly from the environment;
/// this constant exists for user-facing messages.
pub(crate) const FLOX_SANDBOX_OCI_AUTOBAKE_VAR: &str = "FLOX_SANDBOX_OCI_AUTOBAKE";

/// Number of leading hex characters from the lockfile content hash used as the
/// image tag suffix. Twelve characters give 48 bits of collision resistance,
/// which is more than sufficient for a local image store.
const OCI_HASH_TAG_LEN: usize = 12;

/// Prototype-only `[options]` manifest key that mainline flox does not know
/// about. It configures the host-side activation sandbox; the baked image is
/// the *inside* of that boundary, so it must not propagate into the builder's
/// view of the environment. Mainline flox parses manifest options with
/// `deny_unknown_fields` and hard-fails on it — both the in-container builder
/// and any mainline flox inside the guest.
const PROTOTYPE_ONLY_OPTION_KEYS: [&str; 1] = ["sandbox"];

pub struct OciBackend<'a> {
    dot_flox_path: PathBuf,
    env_name: String,
    invocation_type: &'a InvocationType,
    flox: &'a flox_rust_sdk::flox::Flox,
    lockfile: &'a Lockfile,
    config: &'a Config,
}

impl<'a> OciBackend<'a> {
    pub fn new(ctx: SandboxLaunchCtx<'a>) -> Self {
        Self {
            dot_flox_path: ctx.dot_flox_path,
            env_name: ctx.env_name,
            invocation_type: ctx.invocation_type,
            flox: ctx.flox,
            lockfile: ctx.lockfile,
            config: ctx.config,
        }
    }
}

impl ActivationSandbox for OciBackend<'_> {
    fn backend(&self) -> SandboxBackend {
        SandboxBackend::Oci
    }

    fn preflight(&self) -> Result<()> {
        let runtime = platform_runtime();
        if !binary_on_path(runtime) {
            #[cfg(target_os = "macos")]
            bail!(
                "The 'oci' sandbox backend requires Apple Container, which was not found on \
                 PATH.\nInstall it with 'brew install --cask container', then re-run."
            );
            #[cfg(not(target_os = "macos"))]
            bail!(
                "The 'oci' sandbox backend requires Podman, which was not found on PATH.\n\
                 Install it (e.g. 'nix profile install nixpkgs#podman'), then re-run."
            );
        }
        Ok(())
    }

    fn wrap_activation(self: Box<Self>) -> Result<Infallible> {
        wrap_oci(
            &self.dot_flox_path,
            &self.env_name,
            self.invocation_type,
            self.flox,
            self.lockfile,
            self.config,
        )
    }
}

/// Return the container runtime name for the current platform.
fn platform_runtime() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "container"
    }
    #[cfg(not(target_os = "macos"))]
    {
        "podman"
    }
}

/// `true` if an executable named `name` is on `PATH`.
fn binary_on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}

/// Run the activation inside an OCI container, then never return.
fn wrap_oci(
    dot_flox_path: &Path,
    env_name: &str,
    invocation: &InvocationType,
    flox: &flox_rust_sdk::flox::Flox,
    lockfile: &Lockfile,
    config: &Config,
) -> Result<Infallible> {
    let runtime = platform_runtime();

    let dot_flox =
        std::fs::canonicalize(dot_flox_path).unwrap_or_else(|_| dot_flox_path.to_path_buf());
    let project = dot_flox.parent().unwrap_or(&dot_flox).to_path_buf();

    // Resolve the image state using the content-hash tag scheme.
    let state = resolve_oci_image_state(runtime, env_name, lockfile);

    // Determine the final image ref to run.
    let image_ref = match state {
        OciImageState::Explicit(ref image_ref) => {
            debug!(image_ref, "using explicit FLOX_SANDBOX_OCI_IMAGE override");
            image_ref.clone()
        },
        OciImageState::Present { ref image_ref } => {
            debug!(image_ref, "cache hit: content-hash tag present");
            // Self-heal the `<env>:latest` alias on cache hits: a legacy image
            // loaded before hash-tagging existed survives as a second `latest`
            // bearer under a different stored reference (plain vs
            // registry-normalized). Converging here cleans the store without
            // requiring a rebake.
            ensure_latest_alias(runtime, env_name, &lockfile_hash12(lockfile));
            image_ref.clone()
        },
        OciImageState::Stale {
            ref expected_ref, ..
        }
        | OciImageState::Missing { ref expected_ref } => {
            let is_missing = matches!(state, OciImageState::Missing { .. });
            let stale_ref_opt = if is_missing {
                None
            } else {
                Some(stale_ref_for_state(&state))
            };

            let allow_stale = std::env::var(FLOX_SANDBOX_OCI_ALLOW_STALE_VAR)
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            // FLOX_SANDBOX_OCI_AUTOBAKE=true arrives through the config env
            // layer (same machinery as `flox config` keys), so one config read
            // covers both the env var and the config file.
            let autobake = config.flox.sandbox_oci_autobake.unwrap_or(false);
            let is_tty = std::io::stdin().is_terminal();

            let decision = should_bake_oci(
                is_missing,
                allow_stale,
                autobake,
                is_tty,
                expected_ref,
                stale_ref_opt,
            );

            match decision {
                OciBakeDecision::RunStale(ref run_ref) => {
                    eprintln!(
                        "⚠️  Running stale image '{run_ref}' (expected '{expected_ref}').\n   \
                         The environment has changed since this image was built.\n   \
                         Unset {FLOX_SANDBOX_OCI_ALLOW_STALE_VAR} and re-run to bake a fresh \
                         image."
                    );
                    run_ref.clone()
                },
                OciBakeDecision::Bake => {
                    bake_oci_image(runtime, env_name, dot_flox_path, flox, lockfile)?;
                    format!("{env_name}:{}", lockfile_hash12(lockfile))
                },
                OciBakeDecision::Prompt => {
                    let reason = if is_missing {
                        "missing"
                    } else {
                        "stale (environment has changed since last bake)"
                    };
                    let stale_note = if let Some(s) = stale_ref_opt {
                        format!("\nExisting image: {s}")
                    } else {
                        String::new()
                    };
                    let msg = format!(
                        "OCI image '{expected_ref}' is {reason}.{stale_note}\n\
                         Bake now? (~2–5 min on first bake; later bakes reuse layers)"
                    );
                    let confirmed = inquire::Confirm::new(&msg)
                        .with_default(true)
                        .prompt()
                        .unwrap_or(false);
                    if confirmed {
                        bake_oci_image(runtime, env_name, dot_flox_path, flox, lockfile)?;
                        format!("{env_name}:{}", lockfile_hash12(lockfile))
                    } else {
                        bail!(
                            "Bake declined. To build the image manually:\n  \
                             FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate --sandbox enforce \
                             --sandbox-backend oci\n  \
                             or set sandbox_oci_autobake = true in 'flox config'."
                        );
                    }
                },
                OciBakeDecision::FailFast {
                    ref expected_ref,
                    ref stale_hint,
                } => {
                    bail!(
                        "OCI image '{expected_ref}' not found in the local {runtime} image \
                         store.\n\
                         To bake and load it automatically, set \
                         {FLOX_SANDBOX_OCI_AUTOBAKE_VAR}=true \
                         or run on an interactive terminal.{stale_hint}\n\
                         To build and load the image manually:\n  \
                         flox containerize -f img.tar --runtime container\n  \
                         {runtime} image load --input img.tar\n  \
                         (then: flox activate --sandbox enforce --sandbox-backend oci)"
                    );
                },
            }
        },
    };

    let cwd = std::env::current_dir().unwrap_or_else(|_| project.clone());
    let (_, argv) = oci_run_argv(&image_ref, &project, &cwd, invocation);

    // `.exec()` replaces the current process; only returns on failure.
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new(runtime).args(&argv).exec();
    Err(anyhow::anyhow!(
        "Failed to launch the oci sandbox with '{runtime}': {err}."
    ))
}

/// Extract the stale ref string from an `OciImageState::Stale` variant.
/// Panics if called on any other variant.
fn stale_ref_for_state(state: &OciImageState) -> &str {
    match state {
        OciImageState::Stale { stale_ref, .. } => stale_ref.as_str(),
        _ => panic!("stale_ref_for_state called on non-Stale state"),
    }
}

// ── Image state resolution ────────────────────────────────────────────────────

/// Resolution of the OCI image ref for the sandbox backend.
///
/// Resolution precedence (highest wins):
/// 1. `FLOX_SANDBOX_OCI_IMAGE` — explicit override, no staleness logic.
/// 2. `<env>:<hash12>` — content-addressed tag derived from the lockfile.
/// 3. Stale: any `<env>:*` tag other than the expected hash tag.
/// 4. Missing: no image for this environment exists at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OciImageState {
    /// `FLOX_SANDBOX_OCI_IMAGE` was set; use this ref as-is.
    Explicit(String),
    /// The expected hash-tag exists in the local store. Ready to run.
    Present { image_ref: String },
    /// A different `<env>:*` tag exists but not the expected hash tag.
    Stale {
        /// The tag we expected but did not find.
        expected_ref: String,
        /// The newest stale tag found (used with `FLOX_SANDBOX_OCI_ALLOW_STALE`).
        stale_ref: String,
    },
    /// No image for this environment exists in the local store.
    Missing { expected_ref: String },
}

/// Resolve the OCI image state for the oci sandbox backend.
///
/// Thin I/O wrapper around [`classify_oci_image_state`]: reads the override
/// env var and probes the local container store, then delegates classification
/// to the pure core.
fn resolve_oci_image_state(runtime: &str, env_name: &str, lockfile: &Lockfile) -> OciImageState {
    let explicit = std::env::var(FLOX_SANDBOX_OCI_IMAGE_VAR)
        .ok()
        .filter(|v| !v.is_empty());

    let hash12 = lockfile_hash12(lockfile);
    let expected_ref = format!("{env_name}:{hash12}");

    // Skip probing entirely when the override is set: it bypasses all staleness
    // logic, so the probe results would be discarded.
    let expected_present = explicit.is_none() && oci_image_present(runtime, &expected_ref);
    let existing_tags = if explicit.is_none() && !expected_present {
        oci_list_env_tags(runtime, env_name)
    } else {
        Vec::new()
    };

    classify_oci_image_state(
        explicit,
        expected_present,
        env_name,
        &hash12,
        &existing_tags,
    )
}

/// Pure core of [`resolve_oci_image_state`]. Extracted for unit testing.
///
/// See [`OciImageState`] for the resolution precedence.
pub(crate) fn classify_oci_image_state(
    explicit_override: Option<String>,
    expected_present: bool,
    env_name: &str,
    hash12: &str,
    existing_tags: &[String],
) -> OciImageState {
    if let Some(explicit) = explicit_override {
        return OciImageState::Explicit(explicit);
    }

    let expected_ref = format!("{env_name}:{hash12}");
    if expected_present {
        return OciImageState::Present {
            image_ref: expected_ref,
        };
    }

    // Prefer the `latest` alias if present, otherwise take the first
    // non-expected tag found. This heuristic is "good enough" for the stale
    // path — the user is warned and the exact version matters less than getting
    // them unblocked.
    let stale_tag = existing_tags
        .iter()
        .find(|t| t.as_str() == "latest")
        .or_else(|| existing_tags.iter().find(|t| t.as_str() != hash12))
        .cloned();

    match stale_tag {
        Some(tag) => OciImageState::Stale {
            expected_ref,
            stale_ref: format!("{env_name}:{tag}"),
        },
        None => OciImageState::Missing { expected_ref },
    }
}

// ── Bake decision ─────────────────────────────────────────────────────────────

/// Decide whether to bake an OCI image for a missing/stale state.
///
/// Returns `true` when the caller should proceed with a bake, `false` when it
/// should fail fast or run the stale image. The decision is extracted from
/// `wrap_oci` so it can be unit-tested without a tty.
///
/// Decision matrix:
///
/// | state   | allow_stale | autobake | tty   | action       |
/// |---------|-------------|----------|-------|--------------|
/// | missing | *           | true     | *     | bake         |
/// | missing | *           | false    | true  | prompt       |
/// | missing | *           | false    | false | fail fast    |
/// | stale   | true        | *        | *     | run stale    |
/// | stale   | false       | true     | *     | bake         |
/// | stale   | false       | false    | true  | prompt       |
/// | stale   | false       | false    | false | fail fast    |
pub(crate) fn should_bake_oci(
    is_missing: bool,
    allow_stale: bool,
    autobake: bool,
    is_tty: bool,
    expected_ref: &str,
    stale_ref: Option<&str>,
) -> OciBakeDecision {
    // Stale + allow_stale: run the existing image with a warning.
    if !is_missing && allow_stale {
        let stale = stale_ref.unwrap_or("(unknown)");
        return OciBakeDecision::RunStale(stale.to_string());
    }

    // Autobake active (env var or config): bake without prompting.
    if autobake {
        return OciBakeDecision::Bake;
    }

    // Interactive tty: prompt the user.
    if is_tty {
        return OciBakeDecision::Prompt;
    }

    // Non-tty, no autobake: fail fast with guidance.
    let stale_hint = if let Some(s) = stale_ref {
        format!("\nA stale image exists ({s}); set {FLOX_SANDBOX_OCI_ALLOW_STALE_VAR}=1 to run it.")
    } else {
        String::new()
    };
    OciBakeDecision::FailFast {
        expected_ref: expected_ref.to_string(),
        stale_hint,
    }
}

/// Outcome of the bake decision function.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OciBakeDecision {
    /// Proceed with a bake.
    Bake,
    /// Show the interactive tty prompt (caller must resolve).
    Prompt,
    /// Run the named stale image with a warning.
    RunStale(String),
    /// Fail with a guidance message.
    FailFast {
        expected_ref: String,
        stale_hint: String,
    },
}

// ── Image lifecycle helpers ───────────────────────────────────────────────────

/// Compute the first `OCI_HASH_TAG_LEN` hex characters of the blake3 hash of
/// the canonical JSON serialization of a lockfile.
///
/// Hashing the canonical JSON (sorted keys, deterministic) means the tag is
/// stable across re-serialization and across machines. Changing any package
/// pin, hook, or manifest field changes the hash and thus the tag, correctly
/// marking the cached image stale.
pub(crate) fn lockfile_hash12(lockfile: &Lockfile) -> String {
    // serde_json serializes BTreeMap keys in sorted order, so the output is
    // canonical across different serialization passes of the same value.
    let json = serde_json::to_vec(lockfile).expect("serializing a Lockfile to JSON cannot fail");
    let mut hex = blake3::hash(&json).to_hex();
    hex.truncate(OCI_HASH_TAG_LEN);
    hex.to_string()
}

/// Probe whether an image reference exists in the local container store.
fn oci_image_present(runtime: &str, image_ref: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        let _ = runtime;
        // Apple Container requires the fully-qualified `name:tag` form;
        // a bare name (e.g. `myenv`) returns a non-zero exit code.
        std::process::Command::new("container")
            .args(["image", "inspect", image_ref])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::process::Command::new(runtime)
            .args(["image", "exists", image_ref])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// A `<env_name>:*` image entry in the local container store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OciImageEntry {
    /// The full stored reference (may be registry-normalized).
    reference: String,
    /// The tag portion of the reference.
    tag: String,
    /// The image digest.
    digest: String,
}

/// Parse the Apple Container `container image ls --format json` output.
///
/// Each array element carries the full reference in `configuration.name`
/// (e.g. `myenv:abc123`) and the content digest in `id`. Apple Container
/// has no server-side name filter, so we receive all images and filter
/// here. Registry-prefixed names like `docker.io/library/<name>` are
/// matched on the last path segment.
// Only the macOS cfg branch calls this in production; tests exercise it on
// every platform.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_apple_container_entries(json: &[u8], env_name: &str) -> Vec<OciImageEntry> {
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(json) else {
        return Vec::new();
    };
    parsed
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    let reference = e.pointer("/configuration/name")?.as_str()?;
                    let digest = e.get("id")?.as_str()?;
                    let (name, tag) = reference.rsplit_once(':')?;
                    // Apple Container normalizes alias refs with the
                    // default registry prefix (`docker.io/library/<name>`);
                    // match on the last path segment as well.
                    let matches = name == env_name
                        || name
                            .rsplit_once('/')
                            .is_some_and(|(_, last)| last == env_name);
                    matches.then(|| OciImageEntry {
                        reference: reference.to_string(),
                        tag: tag.to_string(),
                        digest: digest.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse the podman `podman images --format json` output.
///
/// Each array element carries `Id` (content digest) and `Names` (array of
/// full references like `<name>:<tag>`). podman accepts a server-side
/// `--filter reference=<env>` so the caller pre-filters; we still check
/// the name segment to guard against partial matches and registry prefixes.
// Only the Linux cfg branch calls this in production; tests exercise it on
// every platform.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_podman_entries(json: &[u8], env_name: &str) -> Vec<OciImageEntry> {
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(json) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for e in parsed.as_array().into_iter().flatten() {
        let digest = e.get("Id").and_then(|v| v.as_str()).unwrap_or_default();
        for name in e
            .get("Names")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str())
        {
            if let Some((refname, tag)) = name.rsplit_once(':') {
                let matches = refname == env_name
                    || refname
                        .rsplit_once('/')
                        .is_some_and(|(_, last)| last == env_name);
                if matches {
                    entries.push(OciImageEntry {
                        reference: name.to_string(),
                        tag: tag.to_string(),
                        digest: digest.to_string(),
                    });
                }
            }
        }
    }
    entries
}

/// List all `<env_name>:*` entries in the local container store, including
/// both hash-tagged images and `latest` aliases.
///
/// The two runtimes expose different JSON schemas; parsing is delegated to
/// [`parse_apple_container_entries`] (macOS) and [`parse_podman_entries`]
/// (Linux) so the logic can be unit-tested without spawning a process.
fn oci_list_env_entries(runtime: &str, env_name: &str) -> Vec<OciImageEntry> {
    #[cfg(target_os = "macos")]
    {
        let _ = runtime;
        let Ok(out) = std::process::Command::new("container")
            .args(["image", "ls", "--format", "json"])
            .output()
        else {
            return Vec::new();
        };
        parse_apple_container_entries(&out.stdout, env_name)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let Ok(out) = std::process::Command::new(runtime)
            .args([
                "images",
                "--format",
                "json",
                "--filter",
                &format!("reference={env_name}"),
            ])
            .output()
        else {
            return Vec::new();
        };
        parse_podman_entries(&out.stdout, env_name)
    }
}

/// List all `<env_name>:*` tags in the local container store.
///
/// Returns the tag strings (the `<hash>` portion, not the full ref).
fn oci_list_env_tags(runtime: &str, env_name: &str) -> Vec<String> {
    oci_list_env_entries(runtime, env_name)
        .into_iter()
        .map(|e| e.tag)
        .collect()
}

/// Tag an OCI image with a new name:tag in the local container store.
///
/// Uses `container tag` on macOS, `podman tag` on Linux.
fn oci_tag_image(runtime: &str, source_ref: &str, dest_ref: &str) -> Result<()> {
    // Apple Container nests image operations under `image` (`container image
    // tag`); the docker-style `container tag` shortcut does not exist. Podman
    // supports the shortcut.
    #[cfg(target_os = "macos")]
    let (cmd_name, subcmd): (&str, &[&str]) = {
        let _ = runtime;
        ("container", &["image", "tag"])
    };
    #[cfg(not(target_os = "macos"))]
    let (cmd_name, subcmd): (&str, &[&str]) = (runtime, &["tag"]);

    // Capture output rather than inheriting it: Apple Container's `image tag`
    // prints the new ref to stdout, which would leak a stray line into every
    // bake's output.
    let output = std::process::Command::new(cmd_name)
        .args(subcmd)
        .args([source_ref, dest_ref])
        .output()
        .with_context(|| format!("failed to run '{cmd_name} {}'", subcmd.join(" ")))?;
    if !output.status.success() {
        bail!(
            "'{cmd_name} {} {source_ref} {dest_ref}' failed: {}",
            subcmd.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Remove an OCI image tag from the local container store.
///
/// Non-fatal: if the removal fails (e.g. the tag does not exist), the error is
/// logged but not propagated. Pruning is best-effort.
fn oci_remove_tag(runtime: &str, image_ref: &str) {
    // Apple Container: `container image rm`; podman: `rmi` shortcut.
    #[cfg(target_os = "macos")]
    let (cmd_name, subcmd): (&str, &[&str]) = {
        let _ = runtime;
        ("container", &["image", "rm"])
    };
    #[cfg(not(target_os = "macos"))]
    let (cmd_name, subcmd): (&str, &[&str]) = (runtime, &["rmi"]);

    let result = std::process::Command::new(cmd_name)
        .args(subcmd)
        .arg(image_ref)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match result {
        Ok(s) if s.success() => debug!(tag = %image_ref, "pruned stale OCI tag"),
        Ok(_) => debug!(tag = %image_ref, "prune of stale OCI tag returned non-zero (ignored)"),
        Err(e) => debug!(tag = %image_ref, err = %e, "prune of stale OCI tag failed (ignored)"),
    }
}

/// Refs to prune after a successful bake: every `<env>:*` tag except the fresh
/// content-hash tag and the `latest` alias. Extracted for unit testing.
pub(crate) fn oci_prune_set(env_name: &str, existing_tags: &[String], hash12: &str) -> Vec<String> {
    existing_tags
        .iter()
        .filter(|t| t.as_str() != hash12 && t.as_str() != "latest")
        .map(|t| format!("{env_name}:{t}"))
        .collect()
}

/// Latest-alias repair actions for an environment, given the store state.
///
/// Returns `(refs_to_remove, need_retag)`:
/// - `refs_to_remove`: exact stored references of `latest` bearers whose
///   digest differs from the expected image.
/// - `need_retag`: true when no `latest` bearer with the expected digest
///   remains.
///
/// No action is taken when the expected `<env>:<hash12>` image is absent.
/// Hash-tag bearers are never removal candidates; superseded hash tags are
/// handled by [`oci_prune_set`].
pub(crate) fn latest_alias_actions(entries: &[OciImageEntry], hash12: &str) -> (Vec<String>, bool) {
    let Some(expected_digest) = entries
        .iter()
        .find(|e| e.tag == hash12)
        .map(|e| e.digest.clone())
    else {
        return (Vec::new(), false);
    };

    let to_remove = entries
        .iter()
        .filter(|e| e.tag == "latest" && e.digest != expected_digest)
        .map(|e| e.reference.clone())
        .collect();
    let has_good_latest = entries
        .iter()
        .any(|e| e.tag == "latest" && e.digest == expected_digest);

    (to_remove, !has_good_latest)
}

/// Converge the `<env>:latest` alias onto the expected `<env>:<hash12>` image:
/// remove conflicting `latest` bearers, then (re)create the alias if needed.
/// Non-fatal throughout; the alias is a convenience.
fn ensure_latest_alias(runtime: &str, env_name: &str, hash12: &str) {
    let entries = oci_list_env_entries(runtime, env_name);
    let (mut to_remove, need_retag) = latest_alias_actions(&entries, hash12);
    if to_remove.is_empty() && !need_retag {
        return;
    }

    const MAX_REMOVAL_PASSES: usize = 4;
    for _ in 0..MAX_REMOVAL_PASSES {
        if to_remove.is_empty() {
            break;
        }
        for reference in &to_remove {
            oci_remove_tag(runtime, reference);
        }
        let entries = oci_list_env_entries(runtime, env_name);
        (to_remove, _) = latest_alias_actions(&entries, hash12);
    }
    if !to_remove.is_empty() {
        debug!(
            survivors = ?to_remove,
            "conflicting latest bearer(s) survived removal passes (ignored)"
        );
    }

    // (Re)create the alias if no bearer with the expected digest remains
    // (removal passes may have peeled the correct alias on the way to a legacy
    // bearer).
    let entries = oci_list_env_entries(runtime, env_name);
    let (_, need_retag) = latest_alias_actions(&entries, hash12);
    if need_retag {
        let hash_tag = format!("{env_name}:{hash12}");
        let latest_tag = format!("{env_name}:latest");
        if let Err(e) = oci_tag_image(runtime, &hash_tag, &latest_tag) {
            debug!(err = %e, "could not tag {hash_tag} as {latest_tag} (non-fatal)");
        }
    }
}

// ── Builder flake-ref selection ───────────────────────────────────────────────

/// Select the flake ref for the `flox containerize` builder inside the proxy
/// container. Thin wrapper around [`select_builder_pin`]; reads the
/// `_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV` override and host version facts.
fn oci_builder_flake_ref(lockfile: &Lockfile, frozen_fallback: &str) -> Result<String> {
    use flox_manifest::parsed::common::KnownSchemaVersion;
    use flox_rust_sdk::flox::FLOX_VERSION;

    let override_ref = std::env::var("_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV")
        .ok()
        .filter(|v| !v.is_empty());

    let version = &*FLOX_VERSION;
    let release_tag = format!("v{}", version.base_semver());
    let lockfile_schema = lockfile.manifest_schema_version();
    let schema_is_latest = lockfile_schema == KnownSchemaVersion::latest();

    let pin = select_builder_pin(
        override_ref.as_deref(),
        version.commit_sha().as_deref(),
        &release_tag,
        schema_is_latest,
        &lockfile_schema.to_string(),
        frozen_fallback,
    )?;
    debug!(flake_ref = %pin, "selected OCI builder pin");
    Ok(pin)
}

/// Pure core of [`oci_builder_flake_ref`]. Extracted for unit testing.
///
/// Precedence:
/// 1. `override_ref` — explicit `_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV`.
/// 2. Release host (`host_commit_sha` is `None`, i.e. no `-g<sha>` suffix in
///    the version) → pin at the host's release tag.
/// 3. Dev host → the frozen fallback pin, gated by a schema preflight: if the
///    lockfile uses a schema newer than the latest this binary knows about, the
///    frozen pin cannot parse it — fail fast with guidance.
pub(crate) fn select_builder_pin(
    override_ref: Option<&str>,
    host_commit_sha: Option<&str>,
    host_release_tag: &str,
    schema_is_latest: bool,
    lockfile_schema: &str,
    frozen_fallback: &str,
) -> Result<String> {
    if let Some(override_ref) = override_ref {
        return Ok(format!("github:flox/flox/{override_ref}"));
    }

    if host_commit_sha.is_none() {
        return Ok(format!("github:flox/flox/{host_release_tag}"));
    }

    if !schema_is_latest {
        bail!(
            "OCI bake schema mismatch: lockfile uses schema '{lockfile_schema}' \
             but the frozen builder pin '{frozen_fallback}' predates this schema.\n\
             Set _FLOX_CONTAINERIZE_FLAKE_REF_OR_REV to a flox revision that \
             supports schema '{lockfile_schema}' and retry."
        );
    }

    Ok(format!("github:flox/flox/{frozen_fallback}"))
}

// ── Run argv construction ─────────────────────────────────────────────────────

/// Build the container run argv for the `oci` sandbox backend.
///
/// macOS uses Apple Container (`container run`); Linux uses Podman (`podman
/// run`). The project directory is mounted at its identical absolute path so
/// the guest sees the same paths as the host. The workdir is set to the host
/// cwd when it is under the project, otherwise the project root itself.
///
/// Returns `(runtime_cmd, argv)` where `runtime_cmd` is `"container"` or
/// `"podman"` and `argv` is the full argument list (excluding the binary
/// itself) to pass to `.exec()`.
pub(crate) fn oci_run_argv(
    image_ref: &str,
    project: &Path,
    cwd: &Path,
    invocation: &InvocationType,
) -> (String, Vec<String>) {
    #[cfg(target_os = "macos")]
    let (runtime, vol_flag, workdir_flag) = ("container", "--volume", "--workdir");
    #[cfg(not(target_os = "macos"))]
    let (runtime, vol_flag, workdir_flag) = ("podman", "-v", "-w");

    let mount = format!("{}:{}", project.display(), project.display());
    let effective_cwd = if cwd.starts_with(project) {
        cwd
    } else {
        project
    };

    let mut argv: Vec<String> = vec!["run".to_string(), "--rm".to_string()];

    argv.push(vol_flag.to_string());
    argv.push(mount);
    argv.push(workdir_flag.to_string());
    argv.push(effective_cwd.display().to_string());

    match invocation {
        InvocationType::Interactive => {
            // Gate -t on stdin being a tty so the backend also works in
            // non-interactive pipelines.
            if std::io::stdin().is_terminal() {
                argv.push("-it".to_string());
            } else {
                argv.push("-i".to_string());
            }
            argv.push(image_ref.to_string());
            // No trailing command: the image entrypoint starts an activated shell.
        },
        InvocationType::ExecCommand(cmd) => {
            argv.push(image_ref.to_string());
            // `--` separates the image ref from the command on Apple Container.
            // Podman ignores a bare `--` before the command too.
            argv.push("--".to_string());
            argv.extend(cmd.iter().cloned());
        },
        InvocationType::ShellCommand(shell_cmd) => {
            // Shell-command form: wrap in `sh -c` so that shell builtins,
            // pipelines, and redirects work as the user expects.
            argv.push(image_ref.to_string());
            argv.push("--".to_string());
            argv.push("sh".to_string());
            argv.push("-c".to_string());
            argv.push(shell_cmd.clone());
        },
        InvocationType::InPlace => {
            // In-place activations are rejected before dispatch reaches this
            // function (ensure_sandbox_not_in_place). This arm is unreachable
            // in practice but required for exhaustive match.
            unreachable!(
                "in-place invocation cannot reach the oci backend (blocked by \
                 ensure_sandbox_not_in_place)"
            );
        },
    }

    (runtime.to_string(), argv)
}

// ── Bake implementation ───────────────────────────────────────────────────────

/// Bake an OCI image for the given environment and load it into the local
/// container store, then tag it with the content-hash tag and `<env>:latest`,
/// and prune superseded `<env>:*` tags.
///
/// This reuses Part 1's `ContainerizeProxy` / `AppleContainerSink` pipeline
/// programmatically rather than shelling out to `flox containerize`.
///
/// On Linux the podman path mirrors the macOS path symmetrically, but Linux
/// end-to-end validation is deferred to a Linux host.
fn bake_oci_image(
    runtime: &str,
    env_name: &str,
    dot_flox_path: &Path,
    flox: &flox_rust_sdk::flox::Flox,
    lockfile: &Lockfile,
) -> Result<()> {
    use flox_rust_sdk::providers::container_builder::ContainerBuilder;

    use crate::commands::containerize::Runtime;
    use crate::commands::containerize::macos_containerize_proxy::ContainerizeProxy;

    // The frozen fallback rev: a known-good prototype-branch commit containing
    // the container-guest fixes (hook enable/disable, prompt label), the CF-7
    // guest-flox baking (real flox in the guest, active-env registration,
    // deterministic marker), the CF-7b in-guest services (project_ctx +
    // process-compose auto-start + pinned socket), the CF-7c demo-feedback
    // fixes (guest metrics disabled, state-dir alignment, guest rendered-env
    // links so deactivate/services never rebuild), and the packaging fixes
    // that let the branch build for aarch64-linux. Used when the host is a
    // dev build. Not CI-cached: the builder compiles it in-VM against the
    // persistent `flox-nix` cache volume, so only the first bake pays the
    // compile.
    //
    // NOTE: Update this rev when builder-side behavior changes (mkContainer,
    // flox-activations, package-builder); host-side-only commits don't need it.
    const FROZEN_FALLBACK_REV: &str = "3c374021c8df69441895a04be9c3c59da4bddec7";

    let hash12 = lockfile_hash12(lockfile);
    let hash_tag = format!("{env_name}:{hash12}");

    // Resolve the builder flake ref (may fail on schema mismatch).
    let flake_ref = oci_builder_flake_ref(lockfile, FROZEN_FALLBACK_REV)?;
    // Strip the `github:flox/flox/` prefix — the proxy uses just the ref/rev.
    let ref_or_rev = flake_ref
        .strip_prefix("github:flox/flox/")
        .unwrap_or(&flake_ref)
        .to_string();

    eprintln!("⚙️  Baking OCI image '{hash_tag}' (builder pin: {ref_or_rev})…");
    eprintln!(
        "   First bake: ~2–5 min (downloads builder + cross-compiles). Later bakes reuse layers."
    );

    // The proxy expects the project directory (the directory containing `.flox`),
    // matching `env.parent_path()` in the containerize command.
    let env_path = {
        let dot_flox =
            std::fs::canonicalize(dot_flox_path).unwrap_or_else(|_| dot_flox_path.to_path_buf());
        dot_flox.parent().unwrap_or(&dot_flox).to_path_buf()
    };

    // Temporarily override the flake ref so ContainerizeProxy picks it up.
    // SAFETY: single-process, no concurrent readers of this var during bake.
    unsafe {
        std::env::set_var("_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV", &ref_or_rev);
    }

    let container_runtime = {
        #[cfg(target_os = "macos")]
        {
            let _ = runtime;
            Runtime::AppleContainer
        }
        #[cfg(not(target_os = "macos"))]
        {
            runtime.parse::<Runtime>().unwrap_or(Runtime::Podman)
        }
    };

    // Builder view: when the manifest declares prototype-only sandbox options,
    // mount a sanitized temp copy instead of the real project — the mainline
    // builder cannot parse those keys. The tag was derived above from the
    // ORIGINAL lockfile: the image identity is the environment as declared,
    // sanitization only shapes the builder's view. Hold the temp dir until the
    // bake completes.
    let sanitized_view =
        sanitized_project_view(&env_path).context("failed to prepare sanitized builder view")?;
    let builder_project = match &sanitized_view {
        Some((_, mount_path)) => {
            debug!(
                view = %mount_path.display(),
                "mounting sanitized builder view (prototype-only options stripped)"
            );
            mount_path.clone()
        },
        None => env_path,
    };

    // include_guest_flox = true: the sandbox bake bakes a real flox into the
    // guest so `flox list` works inside the sandboxed session.
    let proxy = ContainerizeProxy::new(
        builder_project,
        container_runtime.clone(),
        vec![],
        None,
        true,
    );
    let container_source = proxy.create_container_source(flox, env_name, &hash12)?;

    let mut sink = container_runtime.to_writer()?;
    container_source.stream_container(&mut sink)?;
    {
        use tracing::info_span;
        let _span = info_span!(
            "load_image",
            progress = "[3/3] Loading image into container store"
        )
        .entered();
        sink.wait()?;
    }

    // Clean up the env override now that the bake is done.
    unsafe {
        std::env::remove_var("_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV");
    }

    eprintln!("✅  Image '{hash_tag}' loaded into {runtime} store.");

    // Move the `<env>:latest` alias, removing any conflicting bearer first.
    ensure_latest_alias(runtime, env_name, &hash12);

    // Prune superseded `<env>:*` tags (keep current hash tag and latest alias).
    let existing_tags = oci_list_env_tags(runtime, env_name);
    for full_ref in oci_prune_set(env_name, &existing_tags, &hash12) {
        oci_remove_tag(runtime, &full_ref);
    }

    Ok(())
}

// ── Manifest sanitization ─────────────────────────────────────────────────────

/// Strip the prototype-only `sandbox` key from `[options]` in manifest TOML
/// text.
///
/// Returns `Some(sanitized)` when the key was removed, `None` when the manifest
/// declares none of them.
pub(crate) fn sanitize_manifest_toml(toml_text: &str) -> Result<Option<String>> {
    let mut doc = toml_text
        .parse::<toml_edit::DocumentMut>()
        .context("failed to parse manifest for builder sanitization")?;
    let mut changed = false;
    if let Some(options) = doc.get_mut("options").and_then(|i| i.as_table_like_mut()) {
        for key in PROTOTYPE_ONLY_OPTION_KEYS {
            if options.remove(key).is_some() {
                changed = true;
            }
        }
    }
    Ok(changed.then(|| doc.to_string()))
}

/// Strip the prototype-only `sandbox` key from the embedded manifest(s) in
/// lockfile JSON text.
///
/// Returns `Some(sanitized)` when at least one key was removed, `None`
/// otherwise.
pub(crate) fn sanitize_lockfile_json(json_text: &str) -> Result<Option<String>> {
    let mut value: serde_json::Value = serde_json::from_str(json_text)
        .context("failed to parse lockfile for builder sanitization")?;
    let mut changed = false;
    for pointer in ["/manifest/options", "/compose/composer/options"] {
        if let Some(options) = value.pointer_mut(pointer).and_then(|v| v.as_object_mut()) {
            for key in PROTOTYPE_ONLY_OPTION_KEYS {
                if options.remove(key).is_some() {
                    changed = true;
                }
            }
        }
    }
    if changed {
        let mut out = serde_json::to_string_pretty(&value)
            .context("failed to serialize sanitized lockfile")?;
        out.push('\n');
        Ok(Some(out))
    } else {
        Ok(None)
    }
}

/// Minimal recursive copy for the sanitized builder view.
fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// Build a sanitized temp view of the project for the in-container builder,
/// with prototype-only `[options]` keys stripped from the manifest and from the
/// lockfile's embedded manifest.
///
/// Returns `None` when the manifest declares none of the keys. Otherwise
/// returns the temp dir (keep it alive for the duration of the bake) and the
/// path to mount.
fn sanitized_project_view(
    project_dir: &Path,
) -> Result<Option<(tempfile::TempDir, std::path::PathBuf)>> {
    let manifest_path = project_dir.join(".flox/env/manifest.toml");
    let manifest_text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let Some(sanitized_manifest) = sanitize_manifest_toml(&manifest_text)? else {
        return Ok(None);
    };

    let lockfile_path = project_dir.join(".flox/env/manifest.lock");
    let lockfile_text = std::fs::read_to_string(&lockfile_path)
        .with_context(|| format!("failed to read {}", lockfile_path.display()))?;
    let sanitized_lockfile = sanitize_lockfile_json(&lockfile_text)?.unwrap_or(lockfile_text);

    let project_name = project_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "flox-env".to_string());

    // /tmp rather than $TMPDIR: the container runtime bind-mounts the view,
    // and /tmp is shared with the VM while /var/folders is not.
    let view = tempfile::Builder::new()
        .prefix("flox-bake-view-")
        .tempdir_in("/tmp")
        .context("failed to create sanitized builder view")?;
    let mount_path = view.path().join(&project_name);
    let dot_flox_dst = mount_path.join(".flox");
    let env_dst = dot_flox_dst.join("env");

    copy_dir_recursive(&project_dir.join(".flox/env"), &env_dst)
        .context("failed to copy environment into sanitized builder view")?;
    std::fs::copy(
        project_dir.join(".flox/env.json"),
        dot_flox_dst.join("env.json"),
    )
    .context("failed to copy env.json into sanitized builder view")?;
    std::fs::write(env_dst.join("manifest.toml"), sanitized_manifest)
        .context("failed to write sanitized manifest")?;
    std::fs::write(env_dst.join("manifest.lock"), sanitized_lockfile)
        .context("failed to write sanitized lockfile")?;

    Ok(Some((view, mount_path)))
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Exercises every row of the `should_bake_oci` decision matrix.
    mod should_bake_oci_matrix {
        use super::*;

        const EXPECTED: &str = "myenv:abc123def456";
        const STALE: &str = "myenv:latest";

        #[test]
        fn missing_autobake_bakes_regardless_of_tty() {
            for tty in [true, false] {
                let d = should_bake_oci(true, false, true, tty, EXPECTED, None);
                assert_eq!(d, OciBakeDecision::Bake);
            }
        }

        #[test]
        fn missing_no_autobake_tty_prompts() {
            let d = should_bake_oci(true, false, false, true, EXPECTED, None);
            assert_eq!(d, OciBakeDecision::Prompt);
        }

        #[test]
        fn missing_no_autobake_non_tty_fails_fast_without_stale_hint() {
            let d = should_bake_oci(true, false, false, false, EXPECTED, None);
            match d {
                OciBakeDecision::FailFast {
                    expected_ref,
                    stale_hint,
                } => {
                    assert_eq!(expected_ref, EXPECTED);
                    assert!(stale_hint.is_empty());
                },
                other => panic!("expected FailFast, got {other:?}"),
            }
        }

        #[test]
        fn stale_allow_stale_runs_stale_regardless_of_other_flags() {
            for autobake in [true, false] {
                for tty in [true, false] {
                    let d = should_bake_oci(false, true, autobake, tty, EXPECTED, Some(STALE));
                    assert_eq!(d, OciBakeDecision::RunStale(STALE.to_string()));
                }
            }
        }

        #[test]
        fn stale_autobake_bakes() {
            let d = should_bake_oci(false, false, true, false, EXPECTED, Some(STALE));
            assert_eq!(d, OciBakeDecision::Bake);
        }

        #[test]
        fn stale_no_autobake_tty_prompts() {
            let d = should_bake_oci(false, false, false, true, EXPECTED, Some(STALE));
            assert_eq!(d, OciBakeDecision::Prompt);
        }

        #[test]
        fn stale_no_autobake_non_tty_fails_fast_with_stale_hint() {
            let d = should_bake_oci(false, false, false, false, EXPECTED, Some(STALE));
            match d {
                OciBakeDecision::FailFast {
                    expected_ref,
                    stale_hint,
                } => {
                    assert_eq!(expected_ref, EXPECTED);
                    assert!(stale_hint.contains(STALE));
                    assert!(stale_hint.contains(FLOX_SANDBOX_OCI_ALLOW_STALE_VAR));
                },
                other => panic!("expected FailFast, got {other:?}"),
            }
        }

        #[test]
        fn missing_allow_stale_has_no_effect() {
            let d = should_bake_oci(true, true, false, false, EXPECTED, None);
            assert!(matches!(d, OciBakeDecision::FailFast { .. }));
        }
    }

    /// Tag derivation, resolution precedence, pin selection, and prune-set
    /// behavior for the oci sandbox backend image lifecycle.
    mod oci_image_lifecycle {
        use flox_test_utils::GENERATED_DATA;

        use super::*;

        fn fixture_lockfile(env: &str) -> Lockfile {
            let path = GENERATED_DATA.join(format!("envs/{env}/manifest.lock"));
            let content = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            content
                .parse()
                .unwrap_or_else(|e| panic!("parse {}: {e:?}", path.display()))
        }

        #[test]
        fn hash12_is_deterministic_and_12_hex() {
            let lf = fixture_lockfile("hello");
            let h1 = lockfile_hash12(&lf);
            let h2 = lockfile_hash12(&lf);
            assert_eq!(h1, h2);
            assert_eq!(h1.len(), OCI_HASH_TAG_LEN);
            assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
        }

        #[test]
        fn hash12_differs_for_different_lockfiles() {
            let a = lockfile_hash12(&fixture_lockfile("hello"));
            let b = lockfile_hash12(&fixture_lockfile("bash"));
            assert_ne!(a, b);
        }

        #[test]
        fn classify_explicit_override_wins_over_everything() {
            let tags = vec!["latest".to_string()];
            let s = classify_oci_image_state(
                Some("custom:v2".to_string()),
                true,
                "env",
                "abc123def456",
                &tags,
            );
            assert_eq!(s, OciImageState::Explicit("custom:v2".to_string()));
        }

        #[test]
        fn classify_expected_present_is_cache_hit() {
            let s = classify_oci_image_state(None, true, "env", "abc123def456", &[]);
            assert_eq!(s, OciImageState::Present {
                image_ref: "env:abc123def456".to_string(),
            });
        }

        #[test]
        fn classify_absent_prefers_latest_alias_as_stale() {
            let tags = vec!["oldhash".to_string(), "latest".to_string()];
            let s = classify_oci_image_state(None, false, "env", "abc123def456", &tags);
            assert_eq!(s, OciImageState::Stale {
                expected_ref: "env:abc123def456".to_string(),
                stale_ref: "env:latest".to_string(),
            });
        }

        #[test]
        fn classify_absent_with_other_tag_is_stale() {
            let tags = vec!["oldhash".to_string()];
            let s = classify_oci_image_state(None, false, "env", "abc123def456", &tags);
            assert_eq!(s, OciImageState::Stale {
                expected_ref: "env:abc123def456".to_string(),
                stale_ref: "env:oldhash".to_string(),
            });
        }

        #[test]
        fn classify_absent_with_no_tags_is_missing() {
            let s = classify_oci_image_state(None, false, "env", "abc123def456", &[]);
            assert_eq!(s, OciImageState::Missing {
                expected_ref: "env:abc123def456".to_string(),
            });
        }

        #[test]
        fn pin_override_wins() {
            let pin = select_builder_pin(
                Some("deadbeef"),
                Some("abc"),
                "v1.13.1",
                false,
                "1",
                "frozen",
            )
            .unwrap();
            assert_eq!(pin, "github:flox/flox/deadbeef");
        }

        #[test]
        fn pin_release_host_uses_release_tag_without_preflight() {
            let pin = select_builder_pin(None, None, "v1.13.1", false, "1", "frozen").unwrap();
            assert_eq!(pin, "github:flox/flox/v1.13.1");
        }

        #[test]
        fn pin_dev_host_schema_ok_uses_frozen_fallback() {
            let pin =
                select_builder_pin(None, Some("abc"), "v1.13.1", true, "1", "frozen").unwrap();
            assert_eq!(pin, "github:flox/flox/frozen");
        }

        #[test]
        fn pin_dev_host_schema_mismatch_fails_with_guidance() {
            let err = select_builder_pin(None, Some("abc"), "v1.13.1", false, "1.14.0", "frozen")
                .unwrap_err()
                .to_string();
            assert!(err.contains("1.14.0"));
            assert!(err.contains("_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV"));
        }

        #[test]
        fn prune_set_keeps_hash_and_latest() {
            let tags = vec![
                "oldhash".to_string(),
                "latest".to_string(),
                "abc123def456".to_string(),
            ];
            let pruned = oci_prune_set("env", &tags, "abc123def456");
            assert_eq!(pruned, vec!["env:oldhash".to_string()]);
        }

        #[test]
        fn prune_set_empty_when_no_tags() {
            let pruned = oci_prune_set("env", &[], "abc123def456");
            assert!(pruned.is_empty());
        }
    }

    /// Latest-alias repair decisions.
    mod latest_alias {
        use super::*;

        const HASH: &str = "a7f880489710";
        const GOOD_DIGEST: &str = "2f2d460d82cd";
        const OLD_DIGEST: &str = "e7daad51f6d2";

        fn entry(reference: &str, digest: &str) -> OciImageEntry {
            let tag = reference
                .rsplit_once(':')
                .map(|(_, t)| t)
                .unwrap_or("")
                .to_string();
            OciImageEntry {
                reference: reference.to_string(),
                tag,
                digest: digest.to_string(),
            }
        }

        #[test]
        fn removes_legacy_plain_latest_with_different_digest() {
            let entries = vec![
                entry("env:a7f880489710", GOOD_DIGEST),
                entry("env:latest", OLD_DIGEST),
            ];
            let (to_remove, need_retag) = latest_alias_actions(&entries, HASH);
            assert_eq!(to_remove, vec!["env:latest".to_string()]);
            assert!(need_retag, "no good latest remains; must retag");
        }

        #[test]
        fn keeps_normalized_good_latest() {
            let entries = vec![
                entry("env:a7f880489710", GOOD_DIGEST),
                entry("docker.io/library/env:latest", GOOD_DIGEST),
            ];
            let (to_remove, need_retag) = latest_alias_actions(&entries, HASH);
            assert!(to_remove.is_empty());
            assert!(!need_retag);
        }

        #[test]
        fn removes_stray_and_keeps_good_when_both_present() {
            let entries = vec![
                entry("docker.io/library/env:latest", GOOD_DIGEST),
                entry("env:a7f880489710", GOOD_DIGEST),
                entry("env:latest", OLD_DIGEST),
            ];
            let (to_remove, need_retag) = latest_alias_actions(&entries, HASH);
            assert_eq!(to_remove, vec!["env:latest".to_string()]);
            assert!(!need_retag, "good alias already present");
        }

        #[test]
        fn retags_when_no_latest_exists() {
            let entries = vec![entry("env:a7f880489710", GOOD_DIGEST)];
            let (to_remove, need_retag) = latest_alias_actions(&entries, HASH);
            assert!(to_remove.is_empty());
            assert!(need_retag);
        }

        #[test]
        fn noop_when_expected_hash_absent() {
            let entries = vec![entry("env:latest", OLD_DIGEST)];
            let (to_remove, need_retag) = latest_alias_actions(&entries, HASH);
            assert!(to_remove.is_empty());
            assert!(!need_retag, "no expected hash present; no safe action");
        }

        #[test]
        fn never_removes_hash_tag_bearers() {
            let entries = vec![
                entry("env:a7f880489710", GOOD_DIGEST),
                entry("env:0834718d65c2", OLD_DIGEST),
            ];
            let (to_remove, _) = latest_alias_actions(&entries, HASH);
            assert!(to_remove.is_empty());
        }

        #[test]
        fn prune_set_ignores_legacy_latest_only_store() {
            let tags = vec!["latest".to_string()];
            let pruned = oci_prune_set("env", &tags, "a7f880489710");
            assert!(pruned.is_empty());
        }
    }

    /// Builder-view sanitization tests.
    mod bake_sanitization {
        use flox_test_utils::GENERATED_DATA;

        use super::*;

        const MANIFEST_WITH_SANDBOX: &str = r#"version = 1

[install]
hello.pkg-path = "hello"

[options]
systems = ["aarch64-darwin"]

[options.sandbox]
backend = "oci"
"#;

        const MANIFEST_WITHOUT_SANDBOX: &str = r#"version = 1

[install]
hello.pkg-path = "hello"

[options]
systems = ["aarch64-darwin"]
"#;

        #[test]
        fn toml_removes_exactly_the_prototype_keys() {
            let sanitized = sanitize_manifest_toml(MANIFEST_WITH_SANDBOX)
                .unwrap()
                .expect("sandbox table present; must sanitize");
            assert!(
                !sanitized.contains("sandbox"),
                "no sandbox key may survive: {sanitized}"
            );
            assert!(sanitized.contains("systems"), "other options survive");
            assert!(sanitized.contains("hello"), "install table survives");
            assert!(sanitize_manifest_toml(&sanitized).unwrap().is_none());
        }

        #[test]
        fn toml_noop_without_prototype_keys() {
            assert!(
                sanitize_manifest_toml(MANIFEST_WITHOUT_SANDBOX)
                    .unwrap()
                    .is_none()
            );
        }

        #[test]
        fn toml_removes_sandbox_table() {
            let manifest = r#"version = 1

[options.sandbox]
mode = "warn"
"#;
            let sanitized = sanitize_manifest_toml(manifest).unwrap().unwrap();
            assert!(!sanitized.contains("sandbox"));
        }

        fn lockfile_json_with_sandbox() -> String {
            let fixture = GENERATED_DATA.join("envs/hello/manifest.lock");
            let text = std::fs::read_to_string(&fixture).unwrap();
            let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
            if let Some(v) = value.pointer_mut("/manifest/schema-version") {
                *v = "1.13.0".into();
            }
            let options = value
                .pointer_mut("/manifest/options")
                .and_then(|v| v.as_object_mut())
                .expect("fixture has manifest.options");
            options.insert("sandbox".into(), serde_json::json!({"backend": "oci"}));
            serde_json::to_string_pretty(&value).unwrap()
        }

        #[test]
        fn lockfile_removes_exactly_the_prototype_keys() {
            let with_sandbox = lockfile_json_with_sandbox();
            let sanitized = sanitize_lockfile_json(&with_sandbox)
                .unwrap()
                .expect("sandbox table present; must sanitize");
            let value: serde_json::Value = serde_json::from_str(&sanitized).unwrap();
            let options = value.pointer("/manifest/options").unwrap();
            assert!(options.get("sandbox").is_none(), "sandbox key must be gone");
            let original: serde_json::Value = serde_json::from_str(&with_sandbox).unwrap();
            let mut expected = original.clone();
            if let Some(o) = expected
                .pointer_mut("/manifest/options")
                .and_then(|v| v.as_object_mut())
            {
                o.remove("sandbox");
            }
            assert_eq!(value, expected);
        }

        #[test]
        fn lockfile_noop_without_prototype_keys() {
            let fixture = GENERATED_DATA.join("envs/hello/manifest.lock");
            let text = std::fs::read_to_string(&fixture).unwrap();
            assert!(sanitize_lockfile_json(&text).unwrap().is_none());
        }

        #[test]
        fn lockfile_strips_composer_options_when_composed() {
            let json = r#"{
                "lockfile-version": 1,
                "manifest": {"version": 1, "options": {"sandbox": {"mode": "enforce"}}},
                "packages": [],
                "compose": {"composer": {"version": 1, "options": {"sandbox": {"backend": "oci"}}}}
            }"#;
            let sanitized = sanitize_lockfile_json(json).unwrap().unwrap();
            let value: serde_json::Value = serde_json::from_str(&sanitized).unwrap();
            assert!(value.pointer("/compose/composer/options/sandbox").is_none());
            assert!(value.pointer("/manifest/options/sandbox").is_none());
        }

        #[test]
        fn tag_derivation_is_unchanged_by_sanitization() {
            let with_sandbox = lockfile_json_with_sandbox();
            let original: Lockfile = with_sandbox.parse().unwrap();
            let hash_before = lockfile_hash12(&original);

            let sanitized_text = sanitize_lockfile_json(&with_sandbox).unwrap().unwrap();
            let sanitized: Lockfile = sanitized_text.parse().unwrap();

            assert_eq!(lockfile_hash12(&original), hash_before);
            assert_ne!(lockfile_hash12(&sanitized), hash_before);
        }
    }

    /// `oci_run_argv` construction tests.
    mod oci_run_argv_tests {
        use std::path::Path;

        use super::*;

        #[test]
        fn oci_run_argv_exec_command_passes_argv_verbatim() {
            let project = Path::new("/home/user/myproject");
            let cwd = Path::new("/home/user/myproject/subdir");
            let invocation = InvocationType::ExecCommand(vec![
                "bash".to_string(),
                "-c".to_string(),
                "echo hello".to_string(),
            ]);
            let (_runtime, argv) = oci_run_argv("myenv:latest", project, cwd, &invocation);
            let ref_pos = argv.iter().position(|a| a == "myenv:latest").unwrap();
            let sep_pos = argv.iter().position(|a| a == "--").unwrap();
            assert!(sep_pos > ref_pos, "-- must follow the image ref");
            let cmd_start = sep_pos + 1;
            assert_eq!(&argv[cmd_start..], &["bash", "-c", "echo hello"]);
        }

        #[test]
        fn oci_run_argv_interactive_has_no_trailing_command() {
            let project = Path::new("/tmp/project");
            let cwd = Path::new("/tmp/project");
            let invocation = InvocationType::Interactive;
            let (_runtime, argv) = oci_run_argv("env:latest", project, cwd, &invocation);
            let ref_pos = argv.iter().rposition(|a| a == "env:latest").unwrap();
            assert_eq!(
                ref_pos,
                argv.len() - 1,
                "image ref must be the last argv element for interactive"
            );
        }

        #[test]
        fn oci_run_argv_mounts_project_at_identical_path() {
            let project = Path::new("/home/user/myproject");
            let cwd = project;
            let invocation = InvocationType::ExecCommand(vec!["true".to_string()]);
            let (_runtime, argv) = oci_run_argv("img:latest", project, cwd, &invocation);
            let expected_mount = format!("{}:{}", project.display(), project.display());
            assert!(
                argv.contains(&expected_mount),
                "argv must contain mount '{expected_mount}', got: {argv:?}",
            );
        }

        #[test]
        fn oci_run_argv_workdir_is_cwd_when_under_project() {
            let project = Path::new("/home/user/proj");
            let cwd = Path::new("/home/user/proj/src");
            let invocation = InvocationType::ExecCommand(vec!["ls".to_string()]);
            let (_runtime, argv) = oci_run_argv("img:latest", project, cwd, &invocation);
            assert!(
                argv.contains(&cwd.display().to_string()),
                "argv must contain workdir '{cwd:?}', got: {argv:?}",
            );
        }

        #[test]
        fn oci_run_argv_workdir_falls_back_to_project_when_cwd_outside() {
            let project = Path::new("/home/user/proj");
            let cwd = Path::new("/tmp/other");
            let invocation = InvocationType::ExecCommand(vec!["ls".to_string()]);
            let (_runtime, argv) = oci_run_argv("img:latest", project, cwd, &invocation);
            assert!(
                argv.contains(&project.display().to_string()),
                "argv must contain project as workdir '{project:?}', got: {argv:?}",
            );
            assert!(
                !argv.contains(&cwd.display().to_string()),
                "argv must not contain external cwd '{cwd:?}', got: {argv:?}",
            );
        }
    }

    /// Unit tests for the production JSON-parsing functions used by
    /// `oci_list_env_entries`.
    ///
    /// Tests call [`parse_apple_container_entries`] and
    /// [`parse_podman_entries`] directly so the suite covers the exact
    /// production code on both platforms regardless of which cfg branch the
    /// host activates.
    mod oci_list_env_entries_parsing {
        use super::*;

        // ── Apple Container JSON parsing ──────────────────────────────────

        #[test]
        fn apple_container_extracts_matching_entry() {
            let json = br#"[
                {
                    "id": "sha256:abc123",
                    "configuration": { "name": "myenv:deadbeef1234" }
                }
            ]"#;
            let entries = parse_apple_container_entries(json, "myenv");
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].reference, "myenv:deadbeef1234");
            assert_eq!(entries[0].tag, "deadbeef1234");
            assert_eq!(entries[0].digest, "sha256:abc123");
        }

        #[test]
        fn apple_container_filters_out_other_envs() {
            let json = br#"[
                {
                    "id": "sha256:aaa",
                    "configuration": { "name": "myenv:deadbeef1234" }
                },
                {
                    "id": "sha256:bbb",
                    "configuration": { "name": "otherenv:deadbeef1234" }
                }
            ]"#;
            let entries = parse_apple_container_entries(json, "myenv");
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].reference, "myenv:deadbeef1234");
        }

        #[test]
        fn apple_container_matches_registry_prefixed_name() {
            // Apple Container normalizes some refs to `docker.io/library/<name>`.
            let json = br#"[
                {
                    "id": "sha256:ccc",
                    "configuration": { "name": "docker.io/library/myenv:latest" }
                }
            ]"#;
            let entries = parse_apple_container_entries(json, "myenv");
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].reference, "docker.io/library/myenv:latest");
            assert_eq!(entries[0].tag, "latest");
        }

        #[test]
        fn apple_container_empty_list_returns_empty() {
            let entries = parse_apple_container_entries(b"[]", "myenv");
            assert!(entries.is_empty());
        }

        // ── Podman JSON parsing ───────────────────────────────────────────

        #[test]
        fn podman_extracts_matching_entry() {
            let json = br#"[
                {
                    "Id": "sha256:def456",
                    "Names": ["myenv:deadbeef1234"]
                }
            ]"#;
            let entries = parse_podman_entries(json, "myenv");
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].reference, "myenv:deadbeef1234");
            assert_eq!(entries[0].tag, "deadbeef1234");
            assert_eq!(entries[0].digest, "sha256:def456");
        }

        #[test]
        fn podman_extracts_multiple_tags_for_same_image() {
            let json = br#"[
                {
                    "Id": "sha256:def456",
                    "Names": ["myenv:deadbeef1234", "myenv:latest"]
                }
            ]"#;
            let entries = parse_podman_entries(json, "myenv");
            assert_eq!(entries.len(), 2);
            let tags: Vec<&str> = entries.iter().map(|e| e.tag.as_str()).collect();
            assert!(tags.contains(&"deadbeef1234"));
            assert!(tags.contains(&"latest"));
        }

        #[test]
        fn podman_filters_out_other_envs() {
            let json = br#"[
                {
                    "Id": "sha256:aaa",
                    "Names": ["myenv:deadbeef1234"]
                },
                {
                    "Id": "sha256:bbb",
                    "Names": ["otherenv:deadbeef1234"]
                }
            ]"#;
            let entries = parse_podman_entries(json, "myenv");
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].digest, "sha256:aaa");
        }

        #[test]
        fn podman_matches_registry_prefixed_name() {
            let json = br#"[
                {
                    "Id": "sha256:ccc",
                    "Names": ["localhost/myenv:latest"]
                }
            ]"#;
            let entries = parse_podman_entries(json, "myenv");
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].reference, "localhost/myenv:latest");
            assert_eq!(entries[0].tag, "latest");
        }

        #[test]
        fn podman_empty_list_returns_empty() {
            let entries = parse_podman_entries(b"[]", "myenv");
            assert!(entries.is_empty());
        }
    }
}
