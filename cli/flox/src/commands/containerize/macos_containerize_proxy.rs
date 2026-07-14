use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use flox_config::FLOX_CONFIG_FILE;
use flox_core::activate::context::ActivateMode;
use flox_core::vars::FLOX_DISABLE_METRICS_VAR;
use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::providers::container_builder::{
    ContainerBuilder,
    ContainerBuilderParams,
    ContainerSource,
};
use flox_rust_sdk::providers::nix::{NIX_VERSION, NixSubstituterConfig};
use flox_rust_sdk::utils::ReaderExt;
use indoc::formatdoc;
use thiserror::Error;
use tracing::{debug, info, instrument};

use super::Runtime;
use flox_config::FLOX_CONFIG_FILE;

use crate::commands::sandbox_backends::openshell::OPENSHELL_COMPAT_ENV;

const NIX_PROXY_IMAGE: &str = "nixos/nix";
static NIX_PROXY_IMAGE_REF: LazyLock<Option<String>> =
    LazyLock::new(|| env::var("_FLOX_CONTAINERIZE_PROXY_IMAGE_REF").ok());

const FLOX_FLAKE: &str = "github:flox/flox";
const FLOX_PROXY_IMAGE_FLOX_CONFIG_DIR: &str = "/root/.config/flox";
static FLOX_CONTAINERIZE_FLAKE_REF_OR_REV: LazyLock<Option<String>> = LazyLock::new(|| {
    env::var("_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV")
        .ok()
        .filter(|v| !v.is_empty())
});
const CONTAINER_NIX_CACHE_VOLUME: &str = "flox-nix";

/// Default VM memory for Apple Container builder runs.
///
/// Compiling uncached Rust crates inside the nixos/nix proxy image requires
/// more memory than the Apple Container VM default. Override with
/// `_FLOX_CONTAINERIZE_VM_MEMORY` (e.g. "4g", "16g").
const DEFAULT_VM_MEMORY: &str = "8g";

/// `--memory` value passed to `container run` for Apple Container builds.
/// Unset means fall through to `DEFAULT_VM_MEMORY`; set to empty string to
/// omit the flag entirely (not recommended for builds).
static FLOX_CONTAINERIZE_VM_MEMORY: LazyLock<Option<String>> =
    LazyLock::new(|| env::var("_FLOX_CONTAINERIZE_VM_MEMORY").ok());

/// `--cpus` value passed to `container run` for Apple Container builds.
/// When unset the flag is omitted and the Apple Container default applies.
static FLOX_CONTAINERIZE_VM_CPUS: LazyLock<Option<String>> =
    LazyLock::new(|| env::var("_FLOX_CONTAINERIZE_VM_CPUS").ok());

const MOUNT_ENV: &str = "/flox_env";

/// Escape a string for safe interpolation into a `bash -c` script as a
/// single-quoted word. Internal single quotes become `'\''` (close quote,
/// escaped literal quote, reopen quote).
fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[derive(Debug, Error)]
pub enum ContainerizeProxyError {
    #[error("failed to populate proxy container cache volume")]
    PopulateCacheVolume(#[source] std::io::Error),
}

/// An implementation of [ContainerBuilder] for macOS that uses `flox
/// containerize` within a proxy container of a given [Runtime].
#[derive(Debug, Clone)]
pub(crate) struct ContainerizeProxy {
    environment_path: PathBuf,
    container_runtime: Runtime,
    labels: Vec<String>,
    mode: Option<ActivateMode>,
    /// Whether to bake a real flox binary into the guest image. Set only
    /// by the sandbox activation bake; the general `flox containerize`
    /// command leaves it false. Passed as a direct bool rather than via a
    /// process env var: a runtime `std::env::set_var` write is not reliably
    /// observed under flox's multi-threaded runtime (Rust 2024 made
    /// `set_var` unsafe for exactly this reason).
    include_guest_flox: bool,
    /// Explicit builder flake-ref or revision override for the in-container
    /// `nix run github:flox/flox/<ref>` invocation. Consumed only when the
    /// ambient `_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV` env var is unset (the
    /// env var wins so the user-facing override always takes effect).
    /// Precedence: env var > this field > default version-based ref.
    /// Callers at the composition root supply the pin computed for the bake;
    /// the general `flox containerize` command passes `None`.
    flake_ref_override: Option<String>,
    /// Whether to enable the OpenShell compat layer in `mkContainer.nix`
    /// (sandbox user/group uid/gid 1000660000, iproute2, /bin/sh symlink).
    /// Set only by the OpenShell backend bake; all other callers leave it
    /// false so the oci backend and plain `flox containerize` are unaffected.
    openshell_compat: bool,
}

impl ContainerizeProxy {
    pub(crate) fn new(
        environment_path: PathBuf,
        container_runtime: Runtime,
        labels: Vec<String>,
        mode: Option<ActivateMode>,
        include_guest_flox: bool,
        flake_ref_override: Option<String>,
    ) -> Self {
        Self {
            environment_path,
            container_runtime,
            labels,
            mode,
            include_guest_flox,
            flake_ref_override,
            openshell_compat: false,
        }
    }

    /// Variant of [`new`] that also enables the OpenShell compat layer.
    ///
    /// Used exclusively by the `openshell` sandbox backend bake to add the
    /// `sandbox` user/group, `iproute2`, and `/bin/sh` to the guest image.
    /// All other callers use [`new`] to leave `openshell_compat` false and
    /// keep the `oci` backend and plain `flox containerize` unaffected.
    pub(crate) fn new_with_openshell_compat(
        environment_path: PathBuf,
        container_runtime: Runtime,
        labels: Vec<String>,
        mode: Option<ActivateMode>,
        include_guest_flox: bool,
        flake_ref_override: Option<String>,
        openshell_compat: bool,
    ) -> Self {
        Self {
            environment_path,
            container_runtime,
            labels,
            mode,
            include_guest_flox,
            flake_ref_override,
            openshell_compat,
        }
    }

    /// Base command for the container runtime.
    fn runtime_base_command(&self) -> Command {
        let mut command = self.container_runtime.to_command();
        command.arg("run");
        command.arg("--rm");
        if self.container_runtime == Runtime::AppleContainer {
            self.add_vm_resource_args(&mut command);
        }
        command
    }

    /// Add `--memory` and optionally `--cpus` to an Apple Container `run`
    /// invocation.
    ///
    /// The nixos/nix builder VM needs more memory than the Apple Container
    /// default when compiling uncached Rust crates. `_FLOX_CONTAINERIZE_VM_MEMORY`
    /// overrides the default; `_FLOX_CONTAINERIZE_VM_CPUS` adds `--cpus` when
    /// set (the flag is omitted when unset so the VM default applies).
    fn add_vm_resource_args(&self, command: &mut Command) {
        let memory = FLOX_CONTAINERIZE_VM_MEMORY
            .as_deref()
            .unwrap_or(DEFAULT_VM_MEMORY);
        if !memory.is_empty() {
            command.args(["--memory", memory]);
        }
        if let Some(cpus) = FLOX_CONTAINERIZE_VM_CPUS.as_deref()
            && !cpus.is_empty()
        {
            command.args(["--cpus", cpus]);
        }
    }

    /// Resolve the flake ref used for `nix run github:flox/flox/<ref>`.
    ///
    /// Precedence (highest to lowest):
    /// 1. `env_override` — the ambient `_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV`
    ///    env var captured at startup. Passed in rather than read from the
    ///    `LazyLock` directly so callers in tests can exercise all branches
    ///    without mutating process state.
    /// 2. `self.flake_ref_override` — an explicit pin supplied at construction
    ///    time by callers such as the OCI sandbox bake.
    /// 3. The version-derived default: commit SHA if available, otherwise the
    ///    semver tag (e.g. `v1.4.0`).
    fn resolve_flake_ref(&self, env_override: Option<&str>) -> String {
        let flox_version = &*FLOX_VERSION;
        let flox_version_tag = format!("v{}", flox_version.base_semver());
        env_override
            .map(|s| s.to_string())
            .or_else(|| self.flake_ref_override.clone())
            .unwrap_or_else(|| flox_version.commit_sha().unwrap_or(flox_version_tag))
    }

    // Use a Nix container that matches the version of Nix that this Flox
    // has been built with because it's smaller and changes less frequently
    // than a Flox container of the corresponding version, which result in less
    // container image pulls. It also prevents the chicken-and-egg problem when
    // we bump `VERSION` in Flox but haven't published the container image yet.
    fn container_image(&self) -> String {
        format!(
            "{}:{}",
            NIX_PROXY_IMAGE,
            NIX_PROXY_IMAGE_REF
                .clone()
                .unwrap_or(NIX_VERSION.to_string())
        )
    }

    /// Add a named-volume mount to the container runtime command.
    ///
    /// All three runtimes (Docker, Podman, Apple Container) support named
    /// volumes, but with different syntax:
    ///
    /// - Docker / Podman: `--mount type=volume,src=<name>,dst=<path>` — the
    ///   `type=volume` selector distinguishes named volumes from bind mounts.
    /// - Apple Container: `--volume <name>:<path>` — the CLI uses the same
    ///   short `-v` flag family; the source is interpreted as a named volume
    ///   when it does not begin with `/` or `.`.
    fn add_cache_mount(&self, command: &mut Command, path: &str) {
        if self.container_runtime == Runtime::AppleContainer {
            command.args(["--volume", &format!("{CONTAINER_NIX_CACHE_VOLUME}:{path}")]);
        } else {
            command.args([
                "--mount",
                &format!("type=volume,src={CONTAINER_NIX_CACHE_VOLUME},dst={path}"),
            ]);
        }
    }

    /// Create the named cache volume for Apple Container if it does not exist.
    ///
    /// Apple Container volumes are VM-attached disk images. `container volume
    /// create` is idempotent — if the volume already exists the command exits
    /// non-zero but that is expected and ignored here.
    fn ensure_apple_container_cache_volume(&self) -> Result<(), ContainerizeProxyError> {
        debug!(
            volume = CONTAINER_NIX_CACHE_VOLUME,
            "ensuring Apple Container cache volume exists"
        );
        // Capture output rather than inheriting it: the expected
        // "volume already exists" error would otherwise leak into
        // every bake's output.
        let output = self
            .container_runtime
            .to_command()
            .args(["volume", "create", CONTAINER_NIX_CACHE_VOLUME])
            .output()
            .map_err(ContainerizeProxyError::PopulateCacheVolume)?;
        if !output.status.success() {
            // A non-zero exit is expected when the volume already exists;
            // treat it as success so repeated builds are idempotent.
            debug!(
                volume = CONTAINER_NIX_CACHE_VOLUME,
                stderr = %String::from_utf8_lossy(&output.stderr),
                "volume create returned non-zero (likely already exists)"
            );
        }
        Ok(())
    }

    /// Copy the Nix store from the container image into the named cache volume.
    ///
    /// The volume is mounted at `/cache/nix` alongside the image's own `/nix`
    /// so that `nix copy --all` can treat it as a local store root:
    /// https://nix.dev/manual/nix/2.24/command-ref/new-cli/nix3-help-stores#local-store
    ///
    /// This is idempotent: `nix copy` skips store paths that already exist in
    /// the destination, so repeated runs are fast after the first cold fill.
    ///
    /// For Apple Container the named volume is created (if absent) before
    /// mounting, because Apple Container does not auto-create volumes on first
    /// use the way Docker does.
    #[instrument(
        skip_all,
        fields(progress = "[1/3] Filling build cache (downloads + cross-compile on first bake)")
    )]
    fn populate_cache_volume(&self) -> Result<(), ContainerizeProxyError> {
        if self.container_runtime == Runtime::AppleContainer {
            self.ensure_apple_container_cache_volume()?;
        }

        let mut command = self.runtime_base_command();

        // Mount alongside /nix at a cache root so nix copy can treat it as a
        // local store. The real /nix is not touched during populate.
        let cache_root = "/cache";
        self.add_cache_mount(&mut command, &format!("{cache_root}/nix"));

        command.arg(self.container_image());
        // We have to additionally copy some parts of `/nix/var/nix` so that the
        // container isn't broken when the cache volume shadows its `/nix`.
        command.args(["bash", "-c", &formatdoc! {"
            set -euo pipefail
            nix --extra-experimental-features nix-command copy --all --no-check-sigs --to {cache_root}
            cp -R /nix/var/nix/profiles /nix/var/nix/gcroots {cache_root}/nix/var/nix/
        "}]);

        debug!(?command, "running populate cache volume command");
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(ContainerizeProxyError::PopulateCacheVolume)?;

        let stderr = child
            .stderr
            .take()
            .expect("STDERR is piped")
            .tap_lines(|line| info!("{line}"));

        child
            .stdout
            .take()
            .expect("STDOUT is piped")
            .tap_lines(|line| info!("{line}"));

        let status = child
            .wait()
            .map_err(ContainerizeProxyError::PopulateCacheVolume)?;

        if !status.success() {
            return Err(ContainerizeProxyError::PopulateCacheVolume(
                std::io::Error::other(stderr.wait().to_string()),
            ));
        }

        Ok(())
    }

    /// Inception L1: Container runtime args.
    ///
    /// Builds the `docker run` / `podman run` / `container run` invocation
    /// that launches the nixos/nix proxy container. Adapts CLI flags for each
    /// runtime:
    ///
    /// - Docker / Podman: `--mount type=bind,...` for bind mounts, `--mount
    ///   type=volume,...` for named volumes; Podman also needs `--userns ""`
    ///   to map the host user to root inside the container.
    /// - Apple Container: `--mount type=bind,...` for bind mounts, `--volume
    ///   name:dst` for named volumes; no `--userns` flag (runs as the host
    ///   user by default in its VM).
    fn add_runtime_args(&self, command: &mut Command, params: &ContainerBuilderParams) {
        // Podman needs --userns to map the current user to root inside the
        // container; otherwise multi-user Nix operations fail. Docker and Apple
        // Container do not need this flag.
        if self.container_runtime == Runtime::Podman {
            command.args(["--userns", ""]);
        }
        command.args([
            "--mount",
            &format!(
                "type=bind,source={},target={}",
                self.environment_path.to_string_lossy(),
                MOUNT_ENV
            ),
        ]);

        // Mount the populated cache volume over /nix so that store paths built
        // in one run are reused in subsequent runs. The populate step runs
        // before this and fills the volume from the image's own /nix; mounting
        // the volume here then shadows that path, but by that point all
        // required store content is already in the volume.
        //
        // For Apple Container the volume is mounted via --volume name:/nix
        // (see add_cache_mount); for Docker and Podman via --mount type=volume.
        self.add_cache_mount(command, "/nix");

        // Honour config from the user's flox.toml
        // This could include things like floxhub_token and floxhub_url.
        //
        // Docker and Podman support binding individual files; Apple Container
        // requires the bind-mount source to be a directory. When the runtime
        // is Apple Container we mount the entire config directory
        // (~/.config/flox) to avoid the "path is not a directory" error.
        let flox_toml = params.config_dir.join(FLOX_CONFIG_FILE);
        if flox_toml.exists() {
            if self.container_runtime == Runtime::AppleContainer {
                // Mount the whole config directory; `flox.toml` lives inside.
                let mut config_dir_mount = OsString::new();
                config_dir_mount.push("type=bind,source=");
                config_dir_mount.push(&params.config_dir);
                config_dir_mount.push(format!(",target={}", FLOX_PROXY_IMAGE_FLOX_CONFIG_DIR));
                command.arg("--mount");
                command.arg(config_dir_mount);
            } else {
                let mut flox_toml_mount = OsString::new();
                flox_toml_mount.push("type=bind,source=");
                flox_toml_mount.push(flox_toml);
                flox_toml_mount.push(format!(
                    ",target={}/{}",
                    FLOX_PROXY_IMAGE_FLOX_CONFIG_DIR, FLOX_CONFIG_FILE
                ));
                command.arg("--mount");
                command.arg(flox_toml_mount);
            }
        }

        // If metrics are disabled (no device UUID), propagate that into the
        // proxy container. This covers both the env var and config file paths
        // (e.g. /etc/flox.toml) that may not be mounted into the container.
        if params.metrics_disabled {
            command.args(["--env", &format!("{}=true", FLOX_DISABLE_METRICS_VAR)]);
        }

        // Forward the guest-flox request into the builder VM as an env var
        // for the non-OCI (Docker/Podman) path, where the inner `nix run` is
        // built as exec args rather than a shell script. The OCI path
        // (Apple Container) instead exports the marker inside the builder
        // shell script (see `build_oci_conversion_command`), because Apple
        // Container's `run --env` forwarding into the VM is unreliable; the
        // shell export is the deterministic channel there. Gating the
        // `--env` to non-OCI runtimes avoids a redundant, unreliable
        // host→VM env crossing on the OCI path.
        if self.include_guest_flox && !self.container_runtime.requires_oci_format() {
            command.args(["--env", &format!("{}=1", super::INCLUDE_GUEST_FLOX_ENV)]);
        }
        // Forward the OpenShell compat marker on non-OCI paths. The OCI
        // (Apple Container) path carries it inside the builder shell script
        // (see `build_oci_conversion_command`) for the same reliability
        // reason as the guest-flox marker.
        if self.openshell_compat && !self.container_runtime.requires_oci_format() {
            command.args(["--env", &format!("{}=1", OPENSHELL_COMPAT_ENV)]);
        }

        // Propagate the host's nix substituters and trusted public keys into
        // the proxy container so that all nix invocations can fetch packages
        // from the same caches available on the host.
        match NixSubstituterConfig::from_nix_config() {
            Ok(config) => {
                let config_str = config.to_string();
                if !config_str.is_empty() {
                    command.args(["--env", &format!("NIX_CONFIG={config_str}")]);
                }
            },
            Err(err) => {
                tracing::warn!(%err, "failed to read nix substituter config, continuing without extra substituters");
            },
        }

        command.arg(self.container_image());
    }

    /// Inception L2: Nix args.
    fn add_nix_args(&self, command: &mut Command) {
        command.arg("nix");
        command.args([
            "--extra-experimental-features",
            "nix-command flakes",
            "--accept-flake-config",
        ]);

        // Precedence: env var > explicit field > default (see resolve_flake_ref).
        let flox_flake = format!(
            "{}/{}",
            FLOX_FLAKE,
            self.resolve_flake_ref((*FLOX_CONTAINERIZE_FLAKE_REF_OR_REV).as_deref()),
        );
        command.args(["run", &flox_flake, "--"]);
    }

    /// Inception L3: Flox args.
    fn add_flox_args(&self, command: &mut Command, verbosity: i32, tag: impl AsRef<str>) {
        // TODO: this should probably be a method on Verbosity
        match verbosity {
            -1 => {
                command.arg("--quiet");
            },
            v if v > 0 => {
                command.arg(format!("-{}", "v".repeat(v.try_into().unwrap())));
            },
            _ => {},
        }
        command.arg("containerize");
        command.args(["--dir", MOUNT_ENV]);
        command.args(["--tag", tag.as_ref()]);
        command.args(["--file", "-"]);
        command.args(self.labels.iter().flat_map(|l| ["--label", l]));
        if let Some(mode) = &self.mode {
            command.args(["--mode", &mode.to_string()]);
        }
    }

    /// Build the full container run command for the OCI-conversion path.
    ///
    /// Apple Container's `image load` only accepts OCI archives, but nixpkgs
    /// `dockerTools.streamLayeredImage` emits docker-archive format. This
    /// command runs a two-phase conversion inside the nixos/nix builder
    /// container:
    ///
    /// 1. `nix run github:flox/flox -- containerize` writes a docker-archive
    ///    to a temp file.
    /// 2. nixpkgs `skopeo copy` converts it to an OCI archive in a second
    ///    temp file, which `cat` then streams to stdout.
    ///
    /// skopeo comes from nixpkgs inside the builder container, so no
    /// host-side tooling is required. Apple Container image refs require an
    /// explicit tag (`name:latest` works where bare `name` fails), so the
    /// tag is embedded in the skopeo destination reference.
    fn build_oci_conversion_command(
        &self,
        params: &ContainerBuilderParams,
        tag: impl AsRef<str>,
    ) -> Command {
        let tag_str = tag.as_ref();

        // Derive the image name from the environment directory name, matching
        // what the inner `flox containerize` would use.
        let env_name = self
            .environment_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "flox-env".to_string());

        // Apple Container requires an explicit tag in the image reference;
        // bare `name` (without `:tag`) causes `image load` to fail.
        let image_ref = format!("{env_name}:{tag_str}");

        // Precedence: env var > explicit field > default (see resolve_flake_ref).
        let flox_flake = format!(
            "{}/{}",
            FLOX_FLAKE,
            self.resolve_flake_ref((*FLOX_CONTAINERIZE_FLAKE_REF_OR_REV).as_deref()),
        );

        let verbosity_arg = match params.verbosity {
            -1 => " --quiet".to_string(),
            v if v > 0 => format!(" -{}", "v".repeat(v.try_into().unwrap())),
            _ => String::new(),
        };

        // User-controlled values (tag, image name, labels) are single-quote
        // escaped so they cannot break out of the bash -c script.
        let tag_quoted = shell_single_quote(tag_str);
        let image_ref_quoted = shell_single_quote(&image_ref);
        let label_args: String = self
            .labels
            .iter()
            .map(|l| format!(" --label {}", shell_single_quote(l)))
            .collect();

        // Carry the guest-flox marker as a shell export inside the builder
        // script, so the inner `flox containerize` (launched by this same
        // bash) inherits it directly. Apple Container's `run --env`
        // forwarding into the VM is unreliable — the same commit sometimes
        // bakes flox in and sometimes not — so the OCI path uses the shell
        // export as the deterministic channel and `add_runtime_args` omits
        // the `--env` for OCI runtimes.
        let guest_flox_export = if self.include_guest_flox {
            format!("export {}=1\n", super::INCLUDE_GUEST_FLOX_ENV)
        } else {
            String::new()
        };
        // Carry the OpenShell compat marker the same way: as a shell export
        // inside the builder script for the OCI (Apple Container) path.
        let openshell_compat_export = if self.openshell_compat {
            format!("export {}=1\n", OPENSHELL_COMPAT_ENV)
        } else {
            String::new()
        };

        // Sequential phases (not a pipe): running flox containerize and
        // skopeo concurrently can exceed the Apple Container VM's memory
        // and trigger the kernel OOM killer.
        //
        // `>&2` on both nix invocations: only `cat "$oci_tmp"` may write to
        // stdout — anything else corrupts the OCI archive stream read by
        // the host-side sink.
        //
        // skopeo runs with --insecure-policy because the nixos/nix image
        // ships no /etc/containers/policy.json.
        //
        // The skopeo destination concatenates a double-quoted shell variable
        // with the single-quoted image ref: "oci-archive:$oci_tmp:"'name:tag'.
        let shell_cmd = format!(
            "set -euo pipefail\n\
            {guest_flox_export}\
            {openshell_compat_export}\
            docker_tmp=$(mktemp /tmp/flox-docker-XXXXXX.tar)\n\
            oci_tmp=$(mktemp /tmp/flox-oci-XXXXXX.tar)\n\
            nix --extra-experimental-features 'nix-command flakes' --accept-flake-config \
            run '{flox_flake}' --{verbosity_arg} containerize --dir {MOUNT_ENV} --tag {tag_quoted} --file \"$docker_tmp\"{label_args} >&2\n\
            nix --extra-experimental-features 'nix-command flakes' \
            run 'nixpkgs#skopeo' -- --insecure-policy copy \
            \"docker-archive:$docker_tmp\" \"oci-archive:$oci_tmp:\"{image_ref_quoted} >&2\n\
            rm -f \"$docker_tmp\"\n\
            cat \"$oci_tmp\"\n\
            rm -f \"$oci_tmp\""
        );

        let mut command = self.runtime_base_command();
        self.add_runtime_args(&mut command, params);
        // `bash` (unqualified): Apple Container resolves the entrypoint via
        // the image's PATH env var and does not follow absolute Nix profile
        // symlink paths.
        command.args(["bash", "-c", &shell_cmd]);
        command
    }
}

impl ContainerBuilder for ContainerizeProxy {
    type Error = ContainerizeProxyError;

    /// Create a [ContainerSource] for macOS that streams the output via a proxy container.
    ///
    /// When the target runtime is Apple Container, the output stream is an OCI
    /// archive (converted inside the builder container using nixpkgs skopeo).
    /// For Docker and Podman, the output remains docker-archive format.
    fn create_container_source(
        &self,
        params: &ContainerBuilderParams,
        // Inferred from `self.environment_path` by flox _inside_ the container.
        _name: impl AsRef<str>,
        tag: impl AsRef<str>,
    ) -> Result<ContainerSource, Self::Error> {
        self.populate_cache_volume()?;

        let command = if self.container_runtime.requires_oci_format() {
            // Apple Container: emit OCI archive via an in-container skopeo pipe.
            self.build_oci_conversion_command(params, tag)
        } else {
            // Docker / Podman: standard docker-archive path.
            let mut command = self.runtime_base_command();
            self.add_runtime_args(&mut command, params);
            self.add_nix_args(&mut command);
            self.add_flox_args(&mut command, params.verbosity, tag);
            command
        };

        let container_source = ContainerSource::new(command);
        Ok(container_source)
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::Flox;
    use flox_rust_sdk::flox::test_helpers::flox_instance;

    use super::*;

    /// Collect the argv of a Command as strings for inspection.
    fn argv(cmd: &Command) -> Vec<String> {
        std::iter::once(cmd.get_program())
            .chain(cmd.get_args())
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    /// Build a [`ContainerBuilderParams`] from a [`Flox`] test instance.
    fn params_from_flox(flox: &Flox) -> ContainerBuilderParams {
        ContainerBuilderParams {
            config_dir: flox.config_dir.clone(),
            metrics_disabled: flox.metrics_device_uuid.is_none(),
            verbosity: flox.verbosity,
        }
    }

    #[test]
    fn docker_proxy_uses_docker_run() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env".into(),
            Runtime::Docker,
            vec![],
            None,
            false,
            None,
        );
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &params_from_flox(&flox));
        let args = argv(&cmd);
        assert_eq!(args[0], "docker");
        assert!(args.contains(&"run".to_string()));
        assert!(!args.iter().any(|a| a.contains("--userns")));
    }

    /// On the non-OCI (Docker/Podman) path, add_runtime_args forwards the
    /// marker into the builder VM as `--env
    /// _FLOX_CONTAINERIZE_INCLUDE_GUEST_FLOX=1` so the inner `flox
    /// containerize` (exec args, not a shell script) bakes a real guest
    /// flox. Driven by the constructor bool, so it is deterministic and
    /// parallel-safe.
    #[test]
    fn add_runtime_args_forwards_guest_flox_marker_for_non_oci() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env".into(),
            Runtime::Docker,
            vec![],
            None,
            true,
            None,
        );
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &params_from_flox(&flox));
        let args = argv(&cmd);
        let env_pos = args
            .iter()
            .position(|a| a == "_FLOX_CONTAINERIZE_INCLUDE_GUEST_FLOX=1")
            .expect("marker --env must be forwarded for non-OCI when include_guest_flox is true");
        assert_eq!(args[env_pos - 1], "--env");
    }

    /// On the OCI (Apple Container) path, add_runtime_args must NOT forward
    /// the marker via `--env` — the marker is carried by the builder shell
    /// script instead (Apple Container `run --env` is unreliable). This
    /// keeps the host→VM env crossings minimal.
    #[test]
    fn add_runtime_args_omits_guest_flox_env_for_oci() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
            true,
            None,
        );
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &params_from_flox(&flox));
        let args = argv(&cmd);
        assert!(
            !args
                .iter()
                .any(|a| a == "_FLOX_CONTAINERIZE_INCLUDE_GUEST_FLOX=1"),
            "OCI path must not forward the marker via --env (uses shell export): {args:?}"
        );
    }

    /// General `flox containerize` (include_guest_flox = false) must NOT
    /// forward the marker into the builder VM, so its images keep today's
    /// behavior.
    #[test]
    fn add_runtime_args_omits_guest_flox_marker_when_not_requested() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env".into(),
            Runtime::Docker,
            vec![],
            None,
            false,
            None,
        );
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &params_from_flox(&flox));
        let args = argv(&cmd);
        assert!(
            !args
                .iter()
                .any(|a| a == "_FLOX_CONTAINERIZE_INCLUDE_GUEST_FLOX=1"),
            "marker --env must be absent when include_guest_flox is false: {args:?}"
        );
    }

    /// The OCI builder shell script must `export` the guest-flox marker
    /// (before the inner `flox containerize` line) when include_guest_flox
    /// is true, so the inner flox inherits it deterministically without
    /// relying on Apple Container `--env` forwarding.
    #[test]
    fn oci_conversion_command_exports_guest_flox_marker_when_requested() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env/.flox/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
            true,
            None,
        );
        let cmd = proxy.build_oci_conversion_command(&params_from_flox(&flox), "latest");
        let args = argv(&cmd);
        let script = args.last().expect("script is the last arg");
        let export = "export _FLOX_CONTAINERIZE_INCLUDE_GUEST_FLOX=1";
        assert!(
            script.contains(export),
            "OCI builder script must export the guest-flox marker: {script}"
        );
        // The export must precede the inner `flox containerize` invocation
        // so the inner flox inherits it.
        let export_pos = script.find(export).unwrap();
        let containerize_pos = script
            .find("containerize")
            .expect("script must invoke containerize");
        assert!(
            export_pos < containerize_pos,
            "export must precede the containerize invocation: {script}"
        );
    }

    /// The OCI builder shell script must NOT export the marker when
    /// include_guest_flox is false (general containerize, unaffected).
    #[test]
    fn oci_conversion_command_omits_guest_flox_marker_when_not_requested() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env/.flox/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
            false,
            None,
        );
        let cmd = proxy.build_oci_conversion_command(&params_from_flox(&flox), "latest");
        let args = argv(&cmd);
        let script = args.last().expect("script is the last arg");
        assert!(
            !script.contains("_FLOX_CONTAINERIZE_INCLUDE_GUEST_FLOX"),
            "OCI builder script must not mention the marker when not requested: {script}"
        );
    }

    // ── OpenShell compat marker tests ─────────────────────────────────────────

    /// On the non-OCI (Docker) path, add_runtime_args must forward the
    /// OpenShell compat marker via `--env _FLOX_CONTAINERIZE_OPENSHELL_COMPAT=1`
    /// when openshell_compat is true.
    #[test]
    fn add_runtime_args_forwards_openshell_compat_marker_for_non_oci() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new_with_openshell_compat(
            "/some/env".into(),
            Runtime::Docker,
            vec![],
            None,
            false,
            true,
        );
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &flox);
        let args = argv(&cmd);
        let env_pos = args
            .iter()
            .position(|a| a == "_FLOX_CONTAINERIZE_OPENSHELL_COMPAT=1")
            .expect(
                "openshell compat --env must be forwarded for Docker when openshell_compat is true",
            );
        assert_eq!(args[env_pos - 1], "--env");
    }

    /// On the OCI (Apple Container) path, add_runtime_args must NOT forward
    /// the OpenShell compat marker via `--env` — the marker is carried by the
    /// builder shell script instead (Apple Container `run --env` is unreliable).
    #[test]
    fn add_runtime_args_omits_openshell_compat_marker_for_oci() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new_with_openshell_compat(
            "/some/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
            false,
            true,
        );
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &flox);
        let args = argv(&cmd);
        assert!(
            !args
                .iter()
                .any(|a| a == "_FLOX_CONTAINERIZE_OPENSHELL_COMPAT=1"),
            "OCI path must not forward openshell compat via --env (uses shell export): {args:?}"
        );
    }

    /// The OCI builder shell script must `export` the OpenShell compat marker
    /// when openshell_compat is true, so the inner `flox containerize` inherits
    /// it deterministically without relying on Apple Container `--env` forwarding.
    #[test]
    fn oci_conversion_command_exports_openshell_compat_marker_when_requested() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new_with_openshell_compat(
            "/some/env/.flox/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
            false,
            true,
        );
        let cmd = proxy.build_oci_conversion_command(&flox, "latest");
        let args = argv(&cmd);
        let script = args.last().expect("script is the last arg");
        let export = "export _FLOX_CONTAINERIZE_OPENSHELL_COMPAT=1";
        assert!(
            script.contains(export),
            "OCI builder script must export the openshell compat marker: {script}"
        );
        // The export must precede the inner `flox containerize` invocation.
        let export_pos = script.find(export).unwrap();
        let containerize_pos = script
            .find("containerize")
            .expect("script must invoke containerize");
        assert!(
            export_pos < containerize_pos,
            "export must precede the containerize invocation: {script}"
        );
    }

    /// The OCI builder shell script must NOT export the OpenShell compat marker
    /// when openshell_compat is false (general containerize and oci backend
    /// are unaffected).
    #[test]
    fn oci_conversion_command_omits_openshell_compat_marker_when_not_requested() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env/.flox/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
            false,
        );
        let cmd = proxy.build_oci_conversion_command(&flox, "latest");
        let args = argv(&cmd);
        let script = args.last().expect("script is the last arg");
        assert!(
            !script.contains(OPENSHELL_COMPAT_ENV),
            "OCI builder script must not mention openshell compat when not requested: {script}"
        );
    }

    /// The `oci` backend bake (include_guest_flox=true, openshell_compat=false)
    /// must not set the OpenShell compat marker — only the openshell backend
    /// bake triggers the compat layer.
    #[test]
    fn oci_backend_bake_does_not_set_openshell_compat() {
        let (flox, _tempdir) = flox_instance();
        // OCI backend bake uses include_guest_flox=true but openshell_compat=false
        let proxy = ContainerizeProxy::new(
            "/some/env/.flox/env".into(),
            Runtime::Docker,
            vec![],
            None,
            true,
        );
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &flox);
        let args = argv(&cmd);
        assert!(
            !args.iter().any(|a| a.contains(OPENSHELL_COMPAT_ENV)),
            "OCI backend must not set openshell compat marker: {args:?}"
        );
    }

    #[test]
    fn podman_proxy_adds_userns_flag() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env".into(),
            Runtime::Podman,
            vec![],
            None,
            false,
            None,
        );
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &params_from_flox(&flox));
        let args = argv(&cmd);
        assert_eq!(args[0], "podman");
        // --userns must appear for Podman to map the host user to root inside the container.
        let userns_pos = args
            .iter()
            .position(|a| a == "--userns")
            .expect("--userns should be present for Podman");
        assert_eq!(args[userns_pos + 1], "");
    }

    #[test]
    fn apple_container_proxy_omits_userns_flag() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
            false,
            None,
        );
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &params_from_flox(&flox));
        let args = argv(&cmd);
        assert_eq!(args[0], "container");
        // Apple Container does not need --userns
        assert!(
            !args.iter().any(|a| a == "--userns"),
            "Apple Container should not have --userns"
        );
    }

    #[test]
    fn apple_container_uses_volume_flag_for_cache() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
            false,
            None,
        );
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &params_from_flox(&flox));
        let args = argv(&cmd);
        // Apple Container uses --volume name:dst, not --mount type=volume,...
        let vol_pos = args
            .iter()
            .position(|a| a == "--volume")
            .expect("Apple Container should have --volume for the Nix store cache");
        assert_eq!(
            args[vol_pos + 1],
            format!("{CONTAINER_NIX_CACHE_VOLUME}:/nix"),
            "cache mount must be named-volume syntax for Apple Container"
        );
        // The --mount type=volume form must not appear on Apple Container.
        assert!(
            !args
                .iter()
                .any(|a| a.contains("type=volume") && a.contains("dst=/nix")),
            "Apple Container must not use --mount type=volume for /nix"
        );
    }

    #[test]
    fn docker_uses_mount_flag_for_cache() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env".into(),
            Runtime::Docker,
            vec![],
            None,
            false,
            None,
        );
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &params_from_flox(&flox));
        let args = argv(&cmd);
        // Docker uses --mount type=volume,...
        let mount_pos = args
            .iter()
            .position(|a| a.contains("type=volume") && a.contains("dst=/nix"))
            .expect("Docker should have --mount type=volume for the Nix store cache");
        assert!(
            args[mount_pos].contains(CONTAINER_NIX_CACHE_VOLUME),
            "Docker cache mount must reference the named volume"
        );
        // --volume flag must not appear for the cache on Docker.
        assert!(
            !args.iter().any(|a| a == "--volume"),
            "Docker must not use --volume for the cache mount"
        );
    }

    #[test]
    fn oci_conversion_command_includes_cache_volume_mount() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env/.flox/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
            false,
            None,
        );
        let cmd = proxy.build_oci_conversion_command(&params_from_flox(&flox), "latest");
        let args = argv(&cmd);
        // The OCI conversion command also mounts the cache so the builder VM
        // can reuse stored paths during the conversion phase.
        let vol_pos = args
            .iter()
            .position(|a| a == "--volume")
            .expect("OCI conversion command should include --volume for Apple Container cache");
        assert_eq!(
            args[vol_pos + 1],
            format!("{CONTAINER_NIX_CACHE_VOLUME}:/nix"),
            "OCI conversion cache mount must be named-volume syntax"
        );
    }

    #[test]
    fn apple_container_builder_gets_default_memory() {
        let proxy = ContainerizeProxy::new(
            "/some/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
            false,
            None,
        );
        let cmd = proxy.runtime_base_command();
        let args = argv(&cmd);
        // Default 8g memory flag must be present so the builder VM does not
        // OOM-kill Rust compilation.
        let mem_pos = args
            .iter()
            .position(|a| a == "--memory")
            .expect("--memory should be present for Apple Container");
        assert_eq!(args[mem_pos + 1], DEFAULT_VM_MEMORY);
        // --cpus must NOT be present when _FLOX_CONTAINERIZE_VM_CPUS is unset.
        assert!(
            !args.iter().any(|a| a == "--cpus"),
            "Apple Container should not have --cpus when the env var is unset"
        );
    }

    #[test]
    fn docker_and_podman_do_not_get_memory_flag() {
        let docker_proxy = ContainerizeProxy::new(
            "/some/env".into(),
            Runtime::Docker,
            vec![],
            None,
            false,
            None,
        );
        let docker_args = argv(&docker_proxy.runtime_base_command());
        assert!(
            !docker_args.iter().any(|a| a == "--memory"),
            "Docker should not receive --memory (it is not a VM)"
        );

        let podman_proxy = ContainerizeProxy::new(
            "/some/env".into(),
            Runtime::Podman,
            vec![],
            None,
            false,
            None,
        );
        let podman_args = argv(&podman_proxy.runtime_base_command());
        assert!(
            !podman_args.iter().any(|a| a == "--memory"),
            "Podman should not receive --memory (it is not a VM)"
        );
    }

    #[test]
    fn default_vm_memory_is_8g() {
        assert_eq!(DEFAULT_VM_MEMORY, "8g");
    }

    #[test]
    fn oci_conversion_command_includes_memory_flag() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env/.flox/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
            false,
            None,
        );
        let cmd = proxy.build_oci_conversion_command(&params_from_flox(&flox), "latest");
        let args = argv(&cmd);
        // The OCI conversion command uses runtime_base_command, so it must
        // also carry the memory flag.
        let mem_pos = args
            .iter()
            .position(|a| a == "--memory")
            .expect("OCI conversion command should include --memory for Apple Container");
        assert_eq!(args[mem_pos + 1], DEFAULT_VM_MEMORY);
    }

    #[test]
    fn oci_conversion_command_uses_bash_pipeline() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/some/env/.flox/env".into(),
            Runtime::AppleContainer,
            vec![],
            None,
            false,
            None,
        );
        let cmd = proxy.build_oci_conversion_command(&params_from_flox(&flox), "latest");
        let args = argv(&cmd);

        // Command is `container run ...`
        assert_eq!(args[0], "container");
        // Apple Container resolves `bash` via the image's PATH env var.
        let bash_pos = args
            .iter()
            .position(|a| a == "bash")
            .expect("bash should be in argv");
        assert_eq!(args[bash_pos + 1], "-c");
        let shell_script = &args[bash_pos + 2];
        // Pipeline should include flox containerize piped to skopeo
        assert!(shell_script.contains("flox"), "pipeline should run flox");
        assert!(
            shell_script.contains("containerize"),
            "pipeline should invoke containerize"
        );
        assert!(
            shell_script.contains("skopeo"),
            "pipeline should use skopeo for OCI conversion"
        );
        assert!(
            shell_script.contains("oci-archive"),
            "output should be OCI archive format"
        );
        assert!(
            shell_script.contains("docker-archive"),
            "input should be docker-archive format"
        );
        // Image ref must include explicit tag for Apple Container compatibility.
        // Bare `name` (without `:tag`) causes `container image load` to fail.
        assert!(
            shell_script.contains(":latest"),
            "OCI image ref must include explicit tag"
        );

        // Load-bearing pipeline structure:
        // skopeo needs --insecure-policy (no policy.json in nixos/nix image).
        assert!(
            shell_script.contains("--insecure-policy"),
            "skopeo must run with --insecure-policy"
        );
        // Both nix invocations redirect stdout to stderr so only `cat`
        // writes the OCI archive to stdout.
        assert_eq!(
            shell_script.matches(">&2").count(),
            2,
            "both nix invocations must redirect stdout to stderr"
        );
        // Two-phase conversion: two temp files, no pipe between
        // containerize and skopeo (concurrent execution can OOM the VM).
        assert_eq!(
            shell_script.matches("mktemp").count(),
            2,
            "two temp files: one docker-archive, one OCI archive"
        );
        assert!(
            !shell_script.contains(" | "),
            "containerize and skopeo must run sequentially, not piped"
        );
    }

    #[test]
    fn oci_conversion_embeds_custom_tag() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/env/myapp".into(),
            Runtime::AppleContainer,
            vec![],
            None,
            false,
            None,
        );
        let cmd = proxy.build_oci_conversion_command(&params_from_flox(&flox), "v1.2.3");
        let args = argv(&cmd);
        // Custom tag must appear in the OCI destination reference
        assert!(
            args.iter().any(|a| a.contains("v1.2.3")),
            "custom tag 'v1.2.3' should appear in the OCI conversion command"
        );
    }

    #[test]
    fn oci_conversion_escapes_hostile_tags() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new(
            "/env/myapp".into(),
            Runtime::AppleContainer,
            vec![],
            None,
            false,
            None,
        );

        // A tag containing a space must be single-quoted so it stays one word.
        let cmd = proxy.build_oci_conversion_command(&params_from_flox(&flox), "my tag");
        let args = argv(&cmd);
        let script = args.last().expect("script is last arg");
        assert!(
            script.contains("--tag 'my tag'"),
            "tag with space must be single-quoted: {script}"
        );
        assert!(
            script.contains("'myapp:my tag'"),
            "image ref with space must be single-quoted: {script}"
        );

        // A tag containing a single quote must not be able to close the
        // quoting and inject shell syntax. `a'b` becomes `'a'\''b'`.
        let cmd = proxy.build_oci_conversion_command(&params_from_flox(&flox), "a'b");
        let args = argv(&cmd);
        let script = args.last().expect("script is last arg");
        assert!(
            script.contains("--tag 'a'\\''b'"),
            "tag with single quote must be escaped: {script}"
        );
        assert!(
            script.contains("'myapp:a'\\''b'"),
            "image ref with single quote must be escaped: {script}"
        );
        // The raw unescaped tag must not appear as a bare word.
        assert!(
            !script.contains("--tag a'b"),
            "unescaped tag must not reach the shell: {script}"
        );
    }

    #[test]
    fn shell_single_quote_escapes() {
        assert_eq!(shell_single_quote("plain"), "'plain'");
        assert_eq!(shell_single_quote("has space"), "'has space'");
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_single_quote("$(rm -rf /)"), "'$(rm -rf /)'");
    }

    /// Tests for `resolve_flake_ref` precedence.
    ///
    /// Precedence: env var > explicit field > version-based default.
    /// `resolve_flake_ref` takes the env value as a parameter so all three
    /// branches can be exercised without mutating the `LazyLock` static.
    /// This mirrors the `select_builder_pin` pattern in `oci.rs`.
    mod flake_ref_precedence {
        use super::*;

        fn proxy_with_override(pin: Option<&str>) -> ContainerizeProxy {
            ContainerizeProxy::new(
                "/some/env".into(),
                Runtime::Docker,
                vec![],
                None,
                false,
                pin.map(str::to_string),
            )
        }

        /// When only the explicit field is set (no env var), the field is used.
        #[test]
        fn explicit_field_used_when_no_env_var() {
            let proxy = proxy_with_override(Some("deadbeef1234"));
            let resolved = proxy.resolve_flake_ref(None);
            assert_eq!(
                resolved, "deadbeef1234",
                "explicit field must be used when env var is absent"
            );
        }

        /// When neither env var nor field is set, the version-based default
        /// is used. The exact value is version-dependent; check the prefix.
        #[test]
        fn default_used_when_neither_env_nor_field_set() {
            let proxy = proxy_with_override(None);
            let resolved = proxy.resolve_flake_ref(None);
            assert!(!resolved.is_empty(), "default flake ref must not be empty");
            // The resolved value is a tag or SHA — both are non-empty strings.
            // We cannot assert the exact value without pinning FLOX_VERSION in
            // tests, but we can assert it is neither of the explicit inputs.
        }

        /// When both the env var and the explicit field are set, the env var
        /// wins (user-facing override takes priority over the bake pin).
        #[test]
        fn env_var_wins_over_explicit_field() {
            let proxy = proxy_with_override(Some("bake-pin-abc123"));
            let resolved = proxy.resolve_flake_ref(Some("user-env-override"));
            assert_eq!(
                resolved, "user-env-override",
                "env var must win over the explicit flake_ref_override field"
            );
        }
    }
}
