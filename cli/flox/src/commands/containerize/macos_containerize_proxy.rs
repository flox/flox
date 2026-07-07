use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

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
use crate::config::FLOX_CONFIG_FILE;

const NIX_PROXY_IMAGE: &str = "nixos/nix";
static NIX_PROXY_IMAGE_REF: LazyLock<Option<String>> =
    LazyLock::new(|| env::var("_FLOX_CONTAINERIZE_PROXY_IMAGE_REF").ok());

const FLOX_FLAKE: &str = "github:flox/flox";
const FLOX_PROXY_IMAGE_FLOX_CONFIG_DIR: &str = "/root/.config/flox";
static FLOX_CONTAINERIZE_FLAKE_REF_OR_REV: LazyLock<Option<String>> =
    LazyLock::new(|| env::var("_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV").ok());
const CONTAINER_NIX_CACHE_VOLUME: &str = "flox-nix";

const MOUNT_ENV: &str = "/flox_env";

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
        command
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

    /// Add a cache volume mount to the container runtime command.
    ///
    /// Docker and Podman support named volumes (`type=volume`), which persist
    /// the Nix store across builds and speed up subsequent runs.
    ///
    /// Not used for Apple Container: see the comment in `add_runtime_args` for
    /// the reason it is skipped.
    fn add_cache_mount(&self, command: &mut Command, path: &str) {
        command.args([
            "--mount",
            &format!("type=volume,src={CONTAINER_NIX_CACHE_VOLUME},dst={path}"),
        ]);
    }

    /// Copy the Nix store from the container image to the cache volume.
    ///
    /// Skipped for Apple Container because the cache volume population step
    /// uses a shell pipeline (`bash -c '...'`) that is not available in the
    /// NixOS container on Apple's Virtualization.framework without additional
    /// setup. The main build still works; it just won't benefit from the
    /// pre-populated cache on first run.
    #[instrument(skip_all, fields(progress = "Populating proxy container cache volume"))]
    fn populate_cache_volume(&self) -> Result<(), ContainerizeProxyError> {
        if self.container_runtime == Runtime::AppleContainer {
            // The cache-volume population shell pipeline is not compatible with
            // Apple Container's runtime environment. Skip it; the build will
            // proceed without the warm cache.
            debug!("Skipping cache volume population for Apple Container runtime");
            return Ok(());
        }

        let mut command = self.runtime_base_command();

        // The cache volume has to be mounted in parallel to the container's own
        // `/nix` and at a prefix where it can be treated as a new local root:
        // https://nix.dev/manual/nix/2.24/command-ref/new-cli/nix3-help-stores#local-store
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
    /// - Docker / Podman: `--mount type=bind,...` for all mounts; Podman also
    ///   needs `--userns ""` to map the host user to root inside the container.
    /// - Apple Container: uses the same `--mount type=bind,...` syntax;
    ///   no `--userns` flag needed (Apple Container runs as the host user by
    ///   default in its VM).
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

        // The Nix store cache volume is mounted over /nix so that store paths
        // built in one run are reused in subsequent runs.
        //
        // For Apple Container, mounting a volume over /nix shadows the nixos/nix
        // image's own Nix store, removing bash and nix from PATH before the
        // container starts. The populate_cache_volume step (which copies image
        // store content into the volume) uses a shell pipeline that also fails
        // for the same reason.
        //
        // Skip the cache mount for Apple Container: the build works without it,
        // relying on the substituter cache configured via NIX_CONFIG. This is
        // slower on first run but avoids the bootstrap problem.
        if self.container_runtime != Runtime::AppleContainer {
            self.add_cache_mount(command, "/nix");
        }

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
    /// When the target runtime is Apple Container, `container image load` only
    /// accepts OCI archives, but nixpkgs `dockerTools.streamLayeredImage`
    /// emits docker-archive format. This method builds a shell pipeline that:
    ///
    /// 1. Runs `nix run github:flox/flox -- flox containerize --file -` to
    ///    emit a docker-archive on stdout.
    /// 2. Pipes it through nixpkgs `skopeo copy` to convert to OCI archive on
    ///    stdout.
    ///
    /// The conversion runs entirely inside the nixos/nix builder container
    /// using nixpkgs' skopeo, so no additional host-side tooling is required.
    ///
    /// Apple Container image refs require an explicit tag (`name:latest` works
    /// where bare `name` fails), so the tag is embedded in the skopeo
    /// destination reference.
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

        let label_args: String = self
            .labels
            .iter()
            .map(|l| format!(" --label '{}'", l.replace('\'', "'\\''")))
            .collect();

        let mode_arg = self
            .mode
            .as_ref()
            .map(|m| format!(" --mode {}", m))
            .unwrap_or_default();

        // OCI conversion pipeline: flox containerize | skopeo copy
        //
        // Apple Container resolves unqualified binary names (e.g. `bash`)
        // using the image's OCI `Env` PATH, but does NOT follow absolute
        // symlink paths like `/nix/var/nix/profiles/default/bin/bash` at
        // container startup. We therefore pass `bash` as the executable name
        // and let Apple Container find it via the nixos/nix image's PATH.
        //
        // The shell pipeline:
        // 1. Runs `nix run github:flox/flox -- flox containerize --file -`
        //    to emit a docker-archive on stdout.
        // 2. Pipes it through `nix run nixpkgs#skopeo -- copy` to convert
        //    to OCI archive on stdout.
        //
        // Both tools are fetched from nixpkgs inside the builder container,
        // so no new host-side dependencies are introduced.
        // `nix` is on PATH via `/root/.nix-profile/bin` (set in the OCI image
        // Env config) once the shell starts.
        // Two-phase OCI conversion:
        //
        // Phase 1: Build the docker-archive to a temp file using flox containerize.
        // Phase 2: Convert with skopeo (sequential, not a simultaneous pipeline).
        //
        // We avoid the pipe approach because Apple Container VMs have limited
        // memory; running flox containerize (which builds container layers) and
        // skopeo (which reads and converts the archive) simultaneously can trigger
        // the kernel OOM killer, causing one process to be killed with SIGKILL.
        // Sequential execution keeps peak memory use lower.
        //
        // We write the OCI archive to a second temp file and cat it to the
        // container's stdout (which `ContainerSource::stream_container` pipes to
        // the host-side sink). Apple Container streams container stdout back to
        // the host via virtio-serial, which handles sequential reads fine.
        //
        // `nix run github:flox/flox -- containerize` (not `flox containerize`):
        // `nix run` makes the flox binary the process; `containerize` is the
        // subcommand passed as the first argument.
        // The nix run and skopeo commands write progress to stderr.
        // We redirect stdout of the nix/skopeo setup to stderr (>&2) to
        // prevent any nix evaluation output from leaking into the OCI archive
        // stream. Only `cat "$oci_tmp"` writes the archive to stdout.
        let shell_cmd = format!(
            "set -euo pipefail\n\
            docker_tmp=$(mktemp /tmp/flox-docker-XXXXXX.tar)\n\
            oci_tmp=$(mktemp /tmp/flox-oci-XXXXXX.tar)\n\
            nix --extra-experimental-features 'nix-command flakes' --accept-flake-config \
            run '{flox_flake}' --{verbosity_arg} containerize --dir {MOUNT_ENV} --tag {tag_str} --file \"$docker_tmp\"{label_args}{mode_arg} >&2\n\
            nix --extra-experimental-features 'nix-command flakes' \
            run 'nixpkgs#skopeo' -- --insecure-policy copy \
            \"docker-archive:$docker_tmp\" \"oci-archive:$oci_tmp:{image_ref}\" >&2\n\
            rm -f \"$docker_tmp\"\n\
            cat \"$oci_tmp\"\n\
            rm -f \"$oci_tmp\""
        );

        let mut command = self.runtime_base_command();
        self.add_runtime_args(&mut command, flox);
        // `bash` is found by Apple Container using the image's PATH env var.
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
}
