use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use flox_rust_sdk::flox::{Flox, FLOX_VERSION};
use flox_rust_sdk::providers::container_builder::{ContainerBuilder, ContainerSource};
use flox_rust_sdk::providers::nix::NIX_VERSION;
use flox_rust_sdk::utils::ReaderExt;
use indoc::formatdoc;
use thiserror::Error;
use tracing::{debug, info, instrument};

use super::Runtime;
use crate::config::{FLOX_CONFIG_FILE, FLOX_DISABLE_METRICS_VAR};

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
}

impl ContainerizeProxy {
    pub(crate) fn new(environment_path: PathBuf, container_runtime: Runtime) -> Self {
        Self {
            environment_path,
            container_runtime,
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
    fn add_cache_mount(&self, command: &mut Command, path: &str) {
        command.args([
            "--mount",
            &format!("type=volume,src={CONTAINER_NIX_CACHE_VOLUME},dst={path}"),
        ]);
    }

    /// Copy the Nix store from the container image to the cache volume.
    #[instrument(skip_all, fields(progress = "Populating proxy container cache volume"))]
    fn populate_cache_volume(&self) -> Result<(), ContainerizeProxyError> {
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
                std::io::Error::new(std::io::ErrorKind::Other, stderr.wait().to_string()),
            ));
        }

        Ok(())
    }

    /// Inception L1: Container runtime args.
    fn add_runtime_args(&self, command: &mut Command, flox: &Flox) {
        // The `--userns` flag creates a mapping of users in the container,
        // which we need. However, in order to work we also need the user
        // in the container to be `root` otherwise you run into multi-user
        // issues. The empty string `""` argument to `--userns` maps the
        // current user to `root` inside the container.
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

        self.add_cache_mount(command, "/nix");

        // Honour config from the user's flox.toml
        // This could include things like floxhub_token and floxhub_url
        let flox_toml = flox.config_dir.join(FLOX_CONFIG_FILE);
        if flox_toml.exists() {
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

        // Honour `FLOX_DISABLE_METRICS` if set. Aside from being set by the
        // user, it may also be set at runtime by  [Flox::Commands::FloxArgs]
        // from another config path like `/etc/flox.toml` which isn't mounted
        // into the proxy container.
        // TODO: it would be better to check config.flox.disable_metrics than
        // FLOX_DISABLE_METRICS if we store config on Flox struct
        // https://github.com/flox/flox/issues/1666
        if let Ok(disable_metrics) = std::env::var(FLOX_DISABLE_METRICS_VAR) {
            command.args([
                "--env",
                &format!("{}={}", FLOX_DISABLE_METRICS_VAR, disable_metrics),
            ]);
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
    }
}

impl ContainerBuilder for ContainerizeProxy {
    type Error = ContainerizeProxyError;

    /// Create a [ContainerSource] for macOS that streams the output via a proxy container.
    fn create_container_source(
        &self,
        flox: &Flox,
        // Inferred from `self.environment_path` by flox _inside_ the container.
        _name: impl AsRef<str>,
        tag: impl AsRef<str>,
    ) -> Result<ContainerSource, Self::Error> {
        self.populate_cache_volume()?;

        let mut command = self.runtime_base_command();
        self.add_runtime_args(&mut command, flox);
        self.add_nix_args(&mut command);
        self.add_flox_args(&mut command, flox, tag);

        let container_source = ContainerSource::new(command);
        Ok(container_source)
    }
}
