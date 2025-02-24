use std::convert::Infallible;
use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;

use flox_rust_sdk::flox::{Flox, FLOX_VERSION};
use flox_rust_sdk::providers::container_builder::{ContainerBuilder, ContainerSource};
use flox_rust_sdk::providers::nix::NIX_VERSION;

use super::Runtime;
use crate::config::{FLOX_CONFIG_FILE, FLOX_DISABLE_METRICS_VAR};

const NIX_PROXY_IMAGE: &str = "nixos/nix";
static NIX_PROXY_IMAGE_REF: LazyLock<Option<String>> =
    LazyLock::new(|| env::var("_FLOX_CONTAINERIZE_PROXY_IMAGE_REF").ok());

const FLOX_FLAKE: &str = "github:flox/flox";
const FLOX_PROXY_IMAGE_FLOX_CONFIG_DIR: &str = "/root/.config/flox";
static FLOX_CONTAINERIZE_FLAKE_REF_OR_REV: LazyLock<Option<String>> =
    LazyLock::new(|| env::var("_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV").ok());
const CONTAINER_VOLUME_PREFIX: &str = "flox-nix-";

const MOUNT_ENV: &str = "/flox_env";

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

        let flox_version = &*FLOX_VERSION;
        let flox_version_tag = format!("v{}", flox_version.base_semver());

        // The cache volume must be unique per Flox version, otherwise store
        // paths in the container will be shadowed by the cache.
        let volume_name = format!("{}{}", CONTAINER_VOLUME_PREFIX, flox_version_tag);
        command.args([
            // From https://docs.docker.com/engine/storage/volumes
            // If you mount an empty volume into a directory in the container in
            // which files or directories exist, these files or directories are
            // propagated (copied) into the volume by default. Similarly, if you
            // start a container and specify a volume which does not already
            // exist, an empty volume is created for you.
            //
            // From https://docs.podman.io/en/v5.1.1/markdown/podman-run.1.html
            // If no such named volume exists, Podman creates one.
            //
            // I confirmed manually that Podman has the same propagation
            // behavior as Docker for an auto created volume.
            //
            // This gives us precisely the behavior we want;
            // /nix is bootstrapped from FLOX_PROXY_IMAGE,
            // and then subsequently CONTAINER_VOLUME_NAME acts as a cache of
            // /nix.
            //
            // There are no tests for this behavior since that would just be
            // testing podman and Docker work as expected.
            "--mount",
            &format!("type=volume,src={},dst=/nix", volume_name),
        ]);

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

        // Use a Nix container that matches the version of Nix that this Flox
        // has been built with because it's smaller and changes less frequently
        // than a Flox container of the corresponding version, which result in less
        // container image pulls. It also prevents the chicken-and-egg problem when
        // we bump `VERSION` in Flox but haven't published the container image yet.
        let nix_container = format!(
            "{}:{}",
            NIX_PROXY_IMAGE,
            NIX_PROXY_IMAGE_REF
                .clone()
                .unwrap_or(NIX_VERSION.to_string())
        );
        command.arg(nix_container);
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
            // TODO: Doesn't always work: https://github.com/flox/flox/issues/2502
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
    type Error = Infallible;

    /// Create a [ContainerSource] for macOS that streams the output via a proxy container.
    fn create_container_source(
        &self,
        flox: &Flox,
        // Inferred from `self.environment_path` by flox _inside_ the container.
        _name: impl AsRef<str>,
        tag: impl AsRef<str>,
    ) -> Result<ContainerSource, Self::Error> {
        let mut command = self.runtime_base_command();
        self.add_runtime_args(&mut command, flox);
        self.add_nix_args(&mut command);
        self.add_flox_args(&mut command, flox, tag);

        let container_source = ContainerSource::new(command);
        Ok(container_source)
    }
}
