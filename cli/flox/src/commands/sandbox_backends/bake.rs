//! Docker-resident OCI image bake pipeline shared by the Docker-ingesting
//! sandbox backends (`openshell`, `modal`).
//!
//! Both backends need the same content-addressed image: bake the environment
//! into an OCI image with the OpenShell compat layer, load it into Docker's
//! image store under a `<repo>:<hash12>` tag, and later resolve/inspect it.
//! The pipeline is parameterized by the destination `repo` (the tag namespace)
//! so each backend bakes under its own identity — `<env>-openshell:<hash12>`
//! for openshell, `<env>-modal:<hash12>` for modal — rather than one backend
//! borrowing the other's tags.
//!
//! The image *contents* are identical across these backends (they share the
//! compat layer); only the tag namespace differs, so a caller must pass the
//! repo it owns and never a peer's.

use std::path::Path;

use anyhow::{Context, Result, bail};
use flox_manifest::lockfile::Lockfile;
use flox_rust_sdk::providers::container_builder::ContainerBuilderParams;
use tracing::debug;

use crate::commands::sandbox_backends::oci::{
    FLOX_SANDBOX_OCI_IMAGE_VAR,
    OciImageState,
    classify_oci_image_state,
    lockfile_hash12,
};

// ── Docker image state resolution ─────────────────────────────────────────────

/// Resolve the Docker image state for a Docker-resident backend image.
///
/// Mirrors `oci::resolve_oci_image_state` but always uses `docker` for image
/// inspection. The `repo` argument selects the tag namespace
/// (`<env>-openshell` for the openshell backend, `<env>-modal` for the modal
/// backend), so the resolver is shared across Docker-ingesting backends while
/// keeping each backend's tags separate.
pub(crate) fn resolve_docker_image_state(repo: &str, lockfile: &Lockfile) -> OciImageState {
    let explicit = std::env::var(FLOX_SANDBOX_OCI_IMAGE_VAR)
        .ok()
        .filter(|v| !v.is_empty());

    let hash12 = lockfile_hash12(lockfile);
    let expected_ref = format!("{repo}:{hash12}");

    let expected_present = explicit.is_none() && docker_image_present(&expected_ref);
    let existing_tags = if explicit.is_none() && !expected_present {
        docker_list_repo_tags(repo)
    } else {
        Vec::new()
    };

    classify_oci_image_state(explicit, expected_present, repo, &hash12, &existing_tags)
}

/// Extract the stale ref string from a `OciImageState::Stale` variant, or
/// `None` for any other variant.
pub(crate) fn stale_ref_for_state(state: &OciImageState) -> Option<&str> {
    match state {
        OciImageState::Stale { stale_ref, .. } => Some(stale_ref.as_str()),
        _ => None,
    }
}

/// Probe whether an image reference exists in the local Docker store.
fn docker_image_present(image_ref: &str) -> bool {
    std::process::Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Id}}", image_ref])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// List all tags for `<repo>:*` in the local Docker store.
///
/// Returns the tag strings (the part after `:`).
fn docker_list_repo_tags(repo: &str) -> Vec<String> {
    let output = std::process::Command::new("docker")
        .args(["image", "ls", "--format", "{{.Repository}}:{{.Tag}}", repo])
        .output();
    let stdout = match output {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    String::from_utf8_lossy(&stdout)
        .lines()
        .filter_map(|line| line.rsplit_once(':').map(|(_, tag)| tag.to_string()))
        .collect()
}

// ── Docker image inspection ───────────────────────────────────────────────────

/// Read the image ENTRYPOINT from Docker image inspect output.
///
/// Returns the entrypoint as a `Vec<String>` (the JSON array from
/// `Config.Entrypoint`). Returns an empty vec when the image has no
/// configured entrypoint.
pub(crate) fn docker_image_entrypoint(image_ref: &str) -> Result<Vec<String>> {
    let output = std::process::Command::new("docker")
        .args([
            "image",
            "inspect",
            "--format",
            "{{json .Config.Entrypoint}}",
            image_ref,
        ])
        .output()
        .with_context(|| format!("failed to run 'docker image inspect' for '{image_ref}'"))?;
    if !output.status.success() {
        bail!(
            "'docker image inspect' for '{image_ref}' failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let raw = raw.trim();
    if raw == "null" || raw.is_empty() {
        return Ok(Vec::new());
    }
    let parsed: Vec<String> = serde_json::from_str(raw)
        .with_context(|| format!("failed to parse Entrypoint JSON from '{image_ref}': {raw}"))?;
    Ok(parsed)
}

/// Read the image `Config.Env` from Docker image inspect output.
///
/// Returns the entries as `Vec<String>` in `KEY=VALUE` format. Entries that
/// do not match the `[A-Za-z_][A-Za-z0-9_]*` name pattern required by
/// OpenShell, or that begin with the reserved `OPENSHELL_` prefix, are
/// silently dropped.
pub(crate) fn docker_image_env(image_ref: &str) -> Result<Vec<String>> {
    let output = std::process::Command::new("docker")
        .args([
            "image",
            "inspect",
            "--format",
            "{{json .Config.Env}}",
            image_ref,
        ])
        .output()
        .with_context(|| format!("failed to run 'docker image inspect' for '{image_ref}'"))?;
    if !output.status.success() {
        bail!(
            "'docker image inspect' (Env) for '{image_ref}' failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let raw = raw.trim();
    if raw == "null" || raw.is_empty() {
        return Ok(Vec::new());
    }
    let all: Vec<String> = serde_json::from_str(raw)
        .with_context(|| format!("failed to parse Env JSON from '{image_ref}': {raw}"))?;
    Ok(all.into_iter().filter(|e| env_entry_valid(e)).collect())
}

/// Return `true` when an env entry has a valid name for OpenShell.
///
/// OpenShell rejects env names that do not match `[A-Za-z_][A-Za-z0-9_]*`
/// and reserves the `OPENSHELL_` prefix.
pub(crate) fn env_entry_valid(entry: &str) -> bool {
    let name = match entry.split_once('=') {
        Some((name, _)) => name,
        None => entry,
    };
    if name.starts_with("OPENSHELL_") {
        return false;
    }
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    first_ok && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// ── Bake implementation ───────────────────────────────────────────────────────

/// Bake an OCI image for a Docker-ingesting backend, with the compat layer
/// applied, and load it into Docker's image store under `<repo>:<hash12>`.
///
/// The compat layer (`_FLOX_CONTAINERIZE_OPENSHELL_COMPAT=1`) causes
/// `mkContainer.nix` to add the `sandbox` user/group, `iproute2`, and
/// `/bin/sh`, which the OpenShell supervisor (and Modal's remote runtime)
/// requires. The image is loaded into Docker's image store (not Apple
/// Container or Podman).
///
/// `repo` is the destination tag namespace and must be the repo the calling
/// backend owns (`<env>-openshell` or `<env>-modal`) so the loaded image lands
/// where that backend's [`resolve_docker_image_state`] will look for it.
pub(crate) fn bake_image(
    repo: &str,
    env_name: &str,
    dot_flox_path: &Path,
    builder_params: &ContainerBuilderParams,
    lockfile: &Lockfile,
) -> Result<()> {
    use flox_rust_sdk::providers::container_builder::ContainerBuilder;

    use crate::commands::containerize::Runtime;
    use crate::commands::containerize::macos_containerize_proxy::ContainerizeProxy;

    let hash12 = lockfile_hash12(lockfile);
    let hash_tag = format!("{repo}:{hash12}");

    // Pin the builder to a rev on this branch that contains the
    // openshell compat layer (mkContainer openshellCompat + the
    // _FLOX_CONTAINERIZE_OPENSHELL_COMPAT marker plumbing) AND the
    // [[options.sandbox.network]] manifest schema — the baked guest
    // flox parses the live-mounted project lockfile, so a pre-schema
    // guest breaks in-guest commands like 'flox services status'.
    // The oci backend keeps its own, older pin — the compat layer is
    // gated off there and its builder does not need it.
    const FROZEN_FALLBACK_REV: &str = "525741aacf2659a5b88834fe601e59cb143723d4";

    let flake_ref = crate::commands::sandbox_backends::oci::oci_builder_flake_ref(
        lockfile,
        FROZEN_FALLBACK_REV,
    )?;
    let ref_or_rev = flake_ref
        .strip_prefix("github:flox/flox/")
        .unwrap_or(&flake_ref)
        .to_string();

    // No released flox contains the OpenShell compat layer, so a bake routed
    // to a release tag produces an image whose sandbox crashes at create
    // (missing `sandbox` user, iproute2, /var/run). A plain release version
    // — e.g. a flox built with `cargo build` instead of `just build`, which
    // drops the `-g<sha>` suffix — routes there silently; fail loudly
    // instead of baking a doomed image.
    if ref_or_rev.starts_with('v')
        && std::env::var_os("_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV").is_none()
    {
        bail!(
            "The openshell bake would use the release builder '{ref_or_rev}', which lacks the OpenShell compat layer.\nThis flox reports a plain release version; rebuild it with 'just build' so the version carries a '-g<sha>' suffix, or set _FLOX_CONTAINERIZE_FLAKE_REF_OR_REV to a rev containing the compat layer."
        );
    }

    eprintln!("⚙️  Baking OpenShell image '{hash_tag}' (builder pin: {ref_or_rev})…");
    eprintln!(
        "   First bake: ~2–5 min (downloads builder + cross-compiles). \
         Later bakes reuse layers."
    );

    let env_path = {
        let dot_flox =
            std::fs::canonicalize(dot_flox_path).unwrap_or_else(|_| dot_flox_path.to_path_buf());
        dot_flox.parent().unwrap_or(&dot_flox).to_path_buf()
    };

    // Use Docker for image loading (openshell requires Docker, not
    // Apple Container or Podman).
    let container_runtime = Runtime::Docker;

    // Sanitize the project view (strip prototype-only manifest keys).
    let sanitized_view = crate::commands::sandbox_backends::oci::sanitized_project_view(&env_path)
        .context("failed to prepare sanitized builder view")?;
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

    // include_guest_flox = true: bake a real flox into the guest so `flox
    // list` works inside the sandboxed session.
    // flake_ref_override = Some(ref_or_rev): pass the computed builder pin
    // as an explicit constructor argument so the proxy embeds it directly
    // without touching the process environment.
    // openshell_compat = true: add the sandbox user/group and iproute2.
    let proxy = ContainerizeProxy::new_with_openshell_compat(
        builder_project.clone(),
        container_runtime.clone(),
        vec![],
        None,
        true,
        Some(ref_or_rev),
        true,
    );
    // NOTE: create_container_source ignores the `name` argument — the inner
    // `flox containerize` derives the image name from the environment directory
    // name, so the archive always loads as `<env_name>:<hash12>`. After loading
    // we retag to `<repo>:<hash12>` and remove the bare tag so
    // resolve_docker_image_state can find the image under the destination repo.
    let container_source = proxy.create_container_source(builder_params, repo, &hash12)?;

    let mut sink = container_runtime.to_writer()?;
    container_source.stream_container(&mut sink)?;
    {
        use tracing::info_span;
        let _span = info_span!(
            "load_image",
            progress = "[3/3] Loading image into Docker store"
        )
        .entered();
        sink.wait()?;
    }

    // The inner builder derives the image name from the environment directory,
    // so the image loads as `<dir_name>:<hash12>` rather than the destination
    // repo. Retag it into `<repo>:<hash12>` and remove the bare tag to keep the
    // oci namespace clean.
    let bare_name = builder_project
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| env_name.to_string());
    let bare_tag = format!("{bare_name}:{hash12}");
    docker_retag_image(&bare_tag, &hash_tag)
        .with_context(|| format!("failed to retag '{bare_tag}' → '{hash_tag}'"))?;

    eprintln!("✅  Image '{hash_tag}' loaded into Docker store.");
    Ok(())
}

// ── Post-load retag ───────────────────────────────────────────────────────────

/// Retag a loaded Docker image from its builder-assigned name into the
/// destination repository, then remove the bare tag.
///
/// The inner `flox containerize` builder derives the image name from the
/// environment directory name and has no way to set it to the destination
/// repo. After `docker load` completes the image sits at `<env>:<hash12>`;
/// this function moves it to `<repo>:<hash12>` so that
/// [`resolve_docker_image_state`] can find it.
///
/// `docker tag` failure is fatal — without the retag, the image is effectively
/// invisible to the backend. `docker rmi` failure is non-fatal (the bare tag
/// may already be absent or shared with another image); a debug log is emitted
/// instead of propagating the error.
pub(crate) fn docker_retag_image(bare_tag: &str, suffixed_tag: &str) -> Result<()> {
    // Step 1: tag into the destination repo.
    let status = std::process::Command::new("docker")
        .args(["tag", bare_tag, suffixed_tag])
        .status()
        .with_context(|| format!("failed to run 'docker tag {bare_tag} {suffixed_tag}'"))?;
    if !status.success() {
        bail!(
            "'docker tag {bare_tag} {suffixed_tag}' exited with {status}; \
             the source tag may be missing or Docker is unavailable"
        );
    }
    debug!(from = bare_tag, to = suffixed_tag, "retagged baked image");

    // Step 2: unlink the bare tag (best-effort; ignore if already gone).
    let rmi_status = std::process::Command::new("docker")
        .args(["rmi", bare_tag])
        .status();
    match rmi_status {
        Ok(s) if s.success() => {
            debug!(tag = bare_tag, "removed bare baked image tag");
        },
        Ok(s) => {
            debug!(
                tag = bare_tag,
                exit_status = %s,
                "docker rmi of bare tag failed (non-fatal)"
            );
        },
        Err(e) => {
            debug!(
                tag = bare_tag,
                err = %e,
                "docker rmi of bare tag errored (non-fatal)"
            );
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── env_entry_valid ───────────────────────────────────────────────────────

    #[test]
    fn valid_env_entries_are_accepted() {
        for entry in [
            "HOME=/home/flox",
            "XDG_RUNTIME_DIR=/run/flox/runtime",
            "_FLOX_SERVICES_SOCKET_OVERRIDE=/run/flox/runtime/services.sock",
            "PATH=/usr/bin:/bin",
            "A=1",
        ] {
            assert!(env_entry_valid(entry), "should be valid: {entry}");
        }
    }

    #[test]
    fn invalid_name_starts_with_digit_rejected() {
        assert!(!env_entry_valid("1INVALID=val"));
    }

    #[test]
    fn openshell_prefix_rejected() {
        assert!(!env_entry_valid("OPENSHELL_TOKEN=secret"));
        assert!(!env_entry_valid("OPENSHELL_=val"));
    }

    #[test]
    fn name_with_dash_rejected() {
        // Dashes are not in [A-Za-z0-9_]
        assert!(!env_entry_valid("MY-VAR=val"));
    }

    #[test]
    fn empty_name_rejected() {
        assert!(!env_entry_valid("=val"));
    }
}
