//! Shared plumbing for the OCI-handoff sandbox backends (`modal`, `ona`,
//! `docker-sbx`, `e2b`, `daytona`, `cognition-devin`, `anjuna`).
//!
//! These backends all follow the same shape: bake the environment into a
//! `<repo>:<hash12>` OCI image, read the manifest's egress grants, compile them
//! into a provider-specific vocabulary, render a hand-off artifact (a launcher
//! script, devcontainer, kit manifest, template, blueprint, or
//! enclave-converter config), and bail at the launch boundary. The parts that
//! genuinely differ per backend — which vocabulary, which artifact shape, where
//! it is written — stay in the backend module. The parts that were
//! byte-identical copies live here:
//!
//! - [`manifest_network_rules`] — read `[[options.sandbox.network]]` from the
//!   lockfile (was four verbatim copies).
//! - [`ensure_local_image`] — the resolve-state → `should_bake_oci` →
//!   `{RunStale | Bake | Prompt | FailFast}` ladder (was four near-identical
//!   copies; the only per-backend difference was the human-readable image label
//!   in the prompt / fail-fast messages, now a parameter).
//! - The string-literal escaping helpers ([`py_str_lit`] / [`py_str_list`],
//!   [`toml_str_lit`] / [`toml_str_list`], [`json_str_lit`] / [`json_str_list`],
//!   [`yaml_str_lit`] / [`yaml_str_list`]) used to guard single- and
//!   double-quote injection in rendered artifacts.
//!
//! # Not shared: the artifact writers
//!
//! The four backends' generated-artifact writers are *not* congruent, so they
//! stay in their own modules rather than forcing a shared abstraction:
//!
//! - `modal` writes a single script under `.flox/cache/modal-launch.py`.
//! - `docker-sbx` writes `.flox/cache/docker-sbx-kit/spec.yaml` (a kit *dir*).
//! - `ona` writes `<project>/.devcontainer/devcontainer.json` (committed to the
//!   repo root, not `.flox/cache`).
//! - `e2b` writes a *pair* (`<project>/e2b.Dockerfile` + `e2b.toml`) at the
//!   project root.
//!
//! They diverge on root (`.flox/cache` vs project), on cardinality (one file vs
//! a pair vs a dir), and on the "generated: <path>" message shape, so there is
//! no common core worth extracting beyond the escaping helpers above.

use std::path::Path;

use anyhow::{Context, Result, bail};
use flox_core::activate::sandbox_policy::SandboxNetworkRule;
use flox_manifest::interfaces::AsLatestSchema;
use flox_manifest::lockfile::Lockfile;

use super::bake::{bake_image, resolve_docker_image_state};

/// Read the manifest's `[[options.sandbox.network]]` rules from the lockfile.
///
/// Migrates the lockfile's manifest to the latest schema and returns the
/// sandbox network grants (an empty vec when the environment declares none).
pub(crate) fn manifest_network_rules(lockfile: &Lockfile) -> Result<Vec<SandboxNetworkRule>> {
    let manifest = lockfile
        .migrated_manifest()
        .context("failed to migrate the manifest for sandbox policy generation")?;
    Ok(manifest
        .as_latest_schema()
        .options
        .sandbox
        .as_ref()
        .and_then(|sandbox| sandbox.network.clone())
        .unwrap_or_default())
}

/// Ensure the `<repo>:<hash12>` image exists in the local Docker store, baking
/// it (with the shared compat layer) if absent.
///
/// Every OCI-handoff backend pushes/references this image from a registry the
/// provider can pull; baking it locally first is the content-addressed step
/// they all share. When the image is already present (cache hit) or an explicit
/// override is set, this is a no-op.
///
/// `image_label` is the human-readable name for the image in the interactive
/// prompt and the non-tty fail-fast message (e.g. `"Modal image"`, `"Ona
/// image"`, `"E2B image"`, `"Docker Sandboxes image"`) — the only part that
/// differed across the per-backend copies this consolidates.
pub(crate) fn ensure_local_image(
    repo: &str,
    env_name: &str,
    dot_flox_path: &Path,
    lockfile: &Lockfile,
    autobake: bool,
    container_builder_params: &flox_rust_sdk::providers::container_builder::ContainerBuilderParams,
    image_label: &str,
) -> Result<()> {
    use std::io::IsTerminal;

    use crate::commands::sandbox_backends::oci::{
        FLOX_SANDBOX_OCI_ALLOW_STALE_VAR,
        OciBakeDecision,
        OciImageState,
        should_bake_oci,
    };

    let state = resolve_docker_image_state(repo, lockfile);
    match state {
        OciImageState::Explicit(_) | OciImageState::Present { .. } => Ok(()),
        OciImageState::Stale {
            ref expected_ref, ..
        }
        | OciImageState::Missing { ref expected_ref } => {
            let is_missing = matches!(state, OciImageState::Missing { .. });
            let allow_stale = std::env::var(FLOX_SANDBOX_OCI_ALLOW_STALE_VAR)
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let is_tty = std::io::stdin().is_terminal();
            let decision = should_bake_oci(
                is_missing,
                allow_stale,
                autobake,
                is_tty,
                expected_ref,
                None,
            );
            match decision {
                // Running a stale image is acceptable — the operator pushes /
                // adapts whatever is present; a fresh bake is not forced here.
                OciBakeDecision::RunStale(_) => Ok(()),
                OciBakeDecision::Bake => bake_image(
                    repo,
                    env_name,
                    dot_flox_path,
                    container_builder_params,
                    lockfile,
                ),
                OciBakeDecision::Prompt => {
                    let msg = format!(
                        "{image_label} '{expected_ref}' is not baked locally.\n\
                         Bake now? (~2–5 min on first bake; later bakes reuse layers)"
                    );
                    let confirmed = inquire::Confirm::new(&msg)
                        .with_default(true)
                        .prompt()
                        .unwrap_or(false);
                    if confirmed {
                        bake_image(
                            repo,
                            env_name,
                            dot_flox_path,
                            container_builder_params,
                            lockfile,
                        )
                    } else {
                        bail!(
                            "Bake declined. To build the image manually, set \
                             FLOX_SANDBOX_OCI_AUTOBAKE=true and re-run."
                        );
                    }
                },
                OciBakeDecision::FailFast { expected_ref, .. } => {
                    bail!(
                        "{image_label} '{expected_ref}' not found in the local Docker image store.\n\
                         To bake it automatically, set FLOX_SANDBOX_OCI_AUTOBAKE=true or run on an interactive terminal."
                    );
                },
            }
        },
    }
}

// ── String-literal escaping helpers ─────────────────────────────────────────────
//
// Each rendered artifact embeds arbitrary hosts / argv members in a quoted
// scalar of its target language. These helpers escape the quote character (and
// backslash) so an adversarial grant cannot break out of the literal. The
// `split_endpoint` charset check already forbids quotes and newlines in hosts,
// but the escaping is the belt-and-suspenders guard the artifacts depend on.

/// Render a Python single-quoted string literal, escaping backslashes and
/// single quotes so arbitrary argv members are safe to embed.
pub(crate) fn py_str_lit(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('\'', "\\'");
    format!("'{escaped}'")
}

/// Render a Python list of single-quoted string literals.
pub(crate) fn py_str_list(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|s| py_str_lit(s))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

/// Render a TOML basic-string literal, escaping backslashes and double quotes
/// so arbitrary hosts are safe to embed.
pub(crate) fn toml_str_lit(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Render a TOML array of double-quoted string literals.
pub(crate) fn toml_str_list(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|s| toml_str_lit(s))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

/// Render a JSON double-quoted string literal, escaping backslashes and double
/// quotes so arbitrary hosts are safe to embed.
pub(crate) fn json_str_lit(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Render a JSON array of double-quoted string literals.
pub(crate) fn json_str_list(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|s| json_str_lit(s))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

/// Render a YAML double-quoted flow scalar, escaping backslashes and double
/// quotes so arbitrary hosts are safe to embed.
///
/// YAML double-quoted scalars share the JSON escape set, so the output matches
/// [`json_str_lit`] / [`toml_str_lit`]; the distinct name marks the YAML
/// artifacts (enclave config, blueprint) that reach for it.
pub(crate) fn yaml_str_lit(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Render a YAML flow-sequence of double-quoted scalars, e.g.
/// `["a.com", "b.com"]`.
pub(crate) fn yaml_str_list(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|s| yaml_str_lit(s))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── py_str_lit / py_str_list ──────────────────────────────────────────────

    #[test]
    fn py_str_lit_escapes_single_quotes_and_backslashes() {
        assert_eq!(py_str_lit("plain"), "'plain'");
        assert_eq!(py_str_lit("print('hi')"), "'print(\\'hi\\')'");
        assert_eq!(py_str_lit("a\\b"), "'a\\\\b'");
    }

    #[test]
    fn py_str_list_joins_escaped_literals() {
        assert_eq!(py_str_list(&[]), "[]");
        assert_eq!(
            py_str_list(&["a".to_string(), "b".to_string()]),
            "['a', 'b']"
        );
    }

    // ── toml_str_lit / toml_str_list ──────────────────────────────────────────

    #[test]
    fn toml_str_lit_escapes_double_quotes_and_backslashes() {
        assert_eq!(toml_str_lit("plain"), "\"plain\"");
        assert_eq!(toml_str_lit("a\"b"), "\"a\\\"b\"");
        assert_eq!(toml_str_lit("a\\b"), "\"a\\\\b\"");
    }

    #[test]
    fn toml_str_list_joins_escaped_literals() {
        assert_eq!(toml_str_list(&[]), "[]");
        assert_eq!(
            toml_str_list(&["api.github.com".to_string(), "*.anthropic.com".to_string()]),
            "[\"api.github.com\", \"*.anthropic.com\"]"
        );
    }

    // ── json_str_lit / json_str_list ──────────────────────────────────────────

    #[test]
    fn json_str_lit_escapes_double_quotes_and_backslashes() {
        assert_eq!(json_str_lit("plain"), "\"plain\"");
        assert_eq!(json_str_lit("a\"b"), "\"a\\\"b\"");
        assert_eq!(json_str_lit("a\\b"), "\"a\\\\b\"");
    }

    #[test]
    fn json_str_list_joins_escaped_literals() {
        assert_eq!(json_str_list(&[]), "[]");
        assert_eq!(
            json_str_list(&["api.github.com".to_string(), "*.anthropic.com".to_string()]),
            "[\"api.github.com\", \"*.anthropic.com\"]"
        );
    }

    // ── yaml_str_lit / yaml_str_list ──────────────────────────────────────────

    #[test]
    fn yaml_str_lit_escapes_double_quotes_and_backslashes() {
        assert_eq!(yaml_str_lit("plain"), "\"plain\"");
        assert_eq!(yaml_str_lit("a\"b"), "\"a\\\"b\"");
        assert_eq!(yaml_str_lit("a\\b"), "\"a\\\\b\"");
    }

    #[test]
    fn yaml_str_list_joins_escaped_literals() {
        assert_eq!(yaml_str_list(&[]), "[]");
        assert_eq!(
            yaml_str_list(&["api.github.com".to_string(), "*.anthropic.com".to_string()]),
            "[\"api.github.com\", \"*.anthropic.com\"]"
        );
        assert_eq!(yaml_str_list(&["a\"b".to_string()]), "[\"a\\\"b\"]");
    }
}
