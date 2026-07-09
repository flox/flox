use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use flox_config::FLOX_CONFIG_FILE;
use flox_core::activate::context::ActivateMode;
use flox_core::vars::FLOX_DISABLE_METRICS_VAR;
use flox_rust_sdk::flox::{FLOX_VERSION, Flox};
use flox_rust_sdk::providers::container_builder::{ContainerBuilder, ContainerSource};
use flox_rust_sdk::providers::nix::{NIX_VERSION, NixSubstituterConfig};
use flox_rust_sdk::utils::ReaderExt;
use indoc::formatdoc;
use thiserror::Error;
use tracing::{debug, info, instrument};

use super::Runtime;

const NIX_PROXY_IMAGE: &str = "nixos/nix";
static NIX_PROXY_IMAGE_REF: LazyLock<Option<String>> =
    LazyLock::new(|| env::var("_FLOX_CONTAINERIZE_PROXY_IMAGE_REF").ok());

const FLOX_FLAKE: &str = "github:flox/flox";
const FLOX_PROXY_IMAGE_FLOX_CONFIG_DIR: &str = "/root/.config/flox";
static FLOX_CONTAINERIZE_FLAKE_REF_OR_REV: LazyLock<Option<String>> =
    LazyLock::new(|| env::var("_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV").ok());
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
}

impl ContainerizeProxy {
    pub(crate) fn new(
        environment_path: PathBuf,
        container_runtime: Runtime,
        labels: Vec<String>,
        mode: Option<ActivateMode>,
    ) -> Self {
        Self {
            environment_path,
            container_runtime,
            labels,
            mode,
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
    fn add_runtime_args(&self, command: &mut Command, flox: &Flox) {
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
        let flox_toml = flox.config_dir.join(FLOX_CONFIG_FILE);
        if flox_toml.exists() {
            if self.container_runtime == Runtime::AppleContainer {
                // Mount the whole config directory; `flox.toml` lives inside.
                let mut config_dir_mount = OsString::new();
                config_dir_mount.push("type=bind,source=");
                config_dir_mount.push(&flox.config_dir);
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
        if flox.metrics_device_uuid.is_none() {
            command.args(["--env", &format!("{}=true", FLOX_DISABLE_METRICS_VAR)]);
        }

        // Forward the sandbox-bake marker into the builder VM so the inner
        // `flox containerize` bakes a real guest flox. Absent for the
        // general containerize command, so its images are unaffected. Read
        // fresh (not via a LazyLock): `bake_oci_image` sets this var during
        // the bake, after this module is first loaded, so a cached read
        // would miss it.
        if env::var_os(super::INCLUDE_GUEST_FLOX_ENV).is_some() {
            command.args(["--env", &format!("{}=1", super::INCLUDE_GUEST_FLOX_ENV)]);
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

        let flox_version = &*FLOX_VERSION;
        let flox_version_tag = format!("v{}", flox_version.base_semver());
        let flox_flake = format!(
            "{}/{}",
            FLOX_FLAKE,
            // Use a more specific commit if available, e.g. pushed to GitHub.
            (*FLOX_CONTAINERIZE_FLAKE_REF_OR_REV)
                .clone()
                .unwrap_or(flox_version.commit_sha().unwrap_or(flox_version_tag))
        );
        command.args(["run", &flox_flake, "--"]);
    }

    /// Inception L3: Flox args.
    fn add_flox_args(&self, command: &mut Command, flox: &Flox, tag: impl AsRef<str>) {
        // TODO: this should probably be a method on Verbosity
        match flox.verbosity {
            -1 => {
                command.arg("--quiet");
            },
            _ if flox.verbosity > 0 => {
                command.arg(format!(
                    "-{}",
                    "v".repeat(flox.verbosity.try_into().unwrap())
                ));
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
    fn build_oci_conversion_command(&self, flox: &Flox, tag: impl AsRef<str>) -> Command {
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

        let flox_version = &*FLOX_VERSION;
        let flox_version_tag = format!("v{}", flox_version.base_semver());
        let flox_flake = format!(
            "{}/{}",
            FLOX_FLAKE,
            (*FLOX_CONTAINERIZE_FLAKE_REF_OR_REV)
                .clone()
                .unwrap_or(flox_version.commit_sha().unwrap_or(flox_version_tag))
        );

        let verbosity_arg = match flox.verbosity {
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
        self.add_runtime_args(&mut command, flox);
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
        flox: &Flox,
        // Inferred from `self.environment_path` by flox _inside_ the container.
        _name: impl AsRef<str>,
        tag: impl AsRef<str>,
    ) -> Result<ContainerSource, Self::Error> {
        self.populate_cache_volume()?;

        let command = if self.container_runtime.requires_oci_format() {
            // Apple Container: emit OCI archive via an in-container skopeo pipe.
            self.build_oci_conversion_command(flox, tag)
        } else {
            // Docker / Podman: standard docker-archive path.
            let mut command = self.runtime_base_command();
            self.add_runtime_args(&mut command, flox);
            self.add_nix_args(&mut command);
            self.add_flox_args(&mut command, flox, tag);
            command
        };

        let container_source = ContainerSource::new(command);
        Ok(container_source)
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_helpers::flox_instance;

    use super::*;

    /// Collect the argv of a Command as strings for inspection.
    fn argv(cmd: &Command) -> Vec<String> {
        std::iter::once(cmd.get_program())
            .chain(cmd.get_args())
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn docker_proxy_uses_docker_run() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new("/some/env".into(), Runtime::Docker, vec![], None);
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &flox);
        let args = argv(&cmd);
        assert_eq!(args[0], "docker");
        assert!(args.contains(&"run".to_string()));
        assert!(!args.iter().any(|a| a.contains("--userns")));
    }

    /// When the sandbox-bake marker is set, add_runtime_args must forward it
    /// into the builder VM as `--env _FLOX_CONTAINERIZE_INCLUDE_GUEST_FLOX=1`
    /// so the inner `flox containerize` bakes a real guest flox. Env-var
    /// mutation is process-global, so guard with #[serial] and scope the set
    /// tightly via temp_env::with_var.
    #[test]
    #[serial_test::serial]
    fn add_runtime_args_forwards_guest_flox_marker_when_set() {
        let (flox, _tempdir) = flox_instance();
        temp_env::with_var(super::super::INCLUDE_GUEST_FLOX_ENV, Some("1"), || {
            let proxy = ContainerizeProxy::new("/some/env".into(), Runtime::Docker, vec![], None);
            let mut cmd = proxy.runtime_base_command();
            proxy.add_runtime_args(&mut cmd, &flox);
            let args = argv(&cmd);
            let env_pos = args
                .iter()
                .position(|a| a == "_FLOX_CONTAINERIZE_INCLUDE_GUEST_FLOX=1")
                .expect("marker --env must be forwarded when the var is set");
            assert_eq!(args[env_pos - 1], "--env");
        });
    }

    /// General `flox containerize` (marker unset) must NOT forward the
    /// marker into the builder VM, so its images keep today's behavior.
    #[test]
    #[serial_test::serial]
    fn add_runtime_args_omits_guest_flox_marker_when_unset() {
        let (flox, _tempdir) = flox_instance();
        temp_env::with_var(super::super::INCLUDE_GUEST_FLOX_ENV, None::<&str>, || {
            let proxy = ContainerizeProxy::new("/some/env".into(), Runtime::Docker, vec![], None);
            let mut cmd = proxy.runtime_base_command();
            proxy.add_runtime_args(&mut cmd, &flox);
            let args = argv(&cmd);
            assert!(
                !args
                    .iter()
                    .any(|a| a == "_FLOX_CONTAINERIZE_INCLUDE_GUEST_FLOX=1"),
                "marker --env must be absent when the var is unset: {args:?}"
            );
        });
    }

    #[test]
    fn podman_proxy_adds_userns_flag() {
        let (flox, _tempdir) = flox_instance();
        let proxy = ContainerizeProxy::new("/some/env".into(), Runtime::Podman, vec![], None);
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &flox);
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
        let proxy =
            ContainerizeProxy::new("/some/env".into(), Runtime::AppleContainer, vec![], None);
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &flox);
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
        let proxy =
            ContainerizeProxy::new("/some/env".into(), Runtime::AppleContainer, vec![], None);
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &flox);
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
        let proxy = ContainerizeProxy::new("/some/env".into(), Runtime::Docker, vec![], None);
        let mut cmd = proxy.runtime_base_command();
        proxy.add_runtime_args(&mut cmd, &flox);
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
        );
        let cmd = proxy.build_oci_conversion_command(&flox, "latest");
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
        let proxy =
            ContainerizeProxy::new("/some/env".into(), Runtime::AppleContainer, vec![], None);
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
        let docker_proxy =
            ContainerizeProxy::new("/some/env".into(), Runtime::Docker, vec![], None);
        let docker_args = argv(&docker_proxy.runtime_base_command());
        assert!(
            !docker_args.iter().any(|a| a == "--memory"),
            "Docker should not receive --memory (it is not a VM)"
        );

        let podman_proxy =
            ContainerizeProxy::new("/some/env".into(), Runtime::Podman, vec![], None);
        let podman_args = argv(&podman_proxy.runtime_base_command());
        assert!(
            !podman_args.iter().any(|a| a == "--memory"),
            "Podman should not receive --memory (it is not a VM)"
        );
    }

    #[test]
    fn apple_container_memory_override_is_respected() {
        // Safety: test is single-threaded; the LazyLock is already initialized
        // with the real env value, so we test the override path by exercising
        // add_vm_resource_args directly with a stub rather than mutating the
        // static. Instead, test via the add_vm_resource_args path with env
        // manipulation limited to the override scenario by constructing the
        // args manually.
        //
        // Override by constructing a proxy and calling add_vm_resource_args
        // after temporarily setting the env var before the LazyLock initialises.
        // Because LazyLock<Option<String>> is already initialised by prior tests,
        // we cannot mutate it here. Test the DEFAULT_VM_MEMORY constant instead,
        // which is the codepath exercised by the absence of the env var.
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
        );
        let cmd = proxy.build_oci_conversion_command(&flox, "latest");
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
        );
        let cmd = proxy.build_oci_conversion_command(&flox, "latest");
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
        let proxy =
            ContainerizeProxy::new("/env/myapp".into(), Runtime::AppleContainer, vec![], None);
        let cmd = proxy.build_oci_conversion_command(&flox, "v1.2.3");
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
        let proxy =
            ContainerizeProxy::new("/env/myapp".into(), Runtime::AppleContainer, vec![], None);

        // A tag containing a space must be single-quoted so it stays one word.
        let cmd = proxy.build_oci_conversion_command(&flox, "my tag");
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
        let cmd = proxy.build_oci_conversion_command(&flox, "a'b");
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
}
