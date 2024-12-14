use std::convert::Infallible;
use std::env;
use std::path::PathBuf;
use std::sync::LazyLock;

use dirs::home_dir;
use flox_rust_sdk::flox::{Flox, FLOX_VERSION};
use flox_rust_sdk::providers::container_builder::{ContainerBuilder, ContainerSource};

use super::Runtime;
use crate::config::FLOX_DISABLE_METRICS_VAR;

const FLOX_FLAKE: &str = "github:flox/flox";
const FLOX_PROXY_IMAGE: &str = "ghcr.io/flox/flox";
pub static FLOX_CONTAINERIZE_FLAKE_REF_OR_REV: LazyLock<Option<String>> =
    LazyLock::new(|| env::var("FLOX_CONTAINERIZE_FLAKE_REF_OR_REV").ok());

const MOUNT_ENV: &str = "/flox_env";
const MOUNT_HOME: &str = "/flox_home";

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
}

impl ContainerBuilder for ContainerizeProxy {
    type Error = Infallible;

    /// Create a [ContainerSource] for macOS that streams the output via:
    /// 1. `<container> run`
    /// 2. `nix run`
    /// 3. `flox containerize`
    fn create_container_source(
        &self,
        flox: &Flox,
        // Inferred from `self.environment_path` by flox _inside_ the container.
        _name: impl AsRef<str>,
        tag: impl AsRef<str>,
    ) -> Result<ContainerSource, Self::Error> {
        // Inception L1: Container runtime args.
        let mut command = self.container_runtime.to_command();
        command.arg("run");
        command.arg("--rm");
        command.args([
            "--mount",
            &format!(
                "type=bind,source={},target={}",
                self.environment_path.to_string_lossy(),
                MOUNT_ENV
            ),
        ]);

        // Honour config from the user's home directory on their host machine if
        // available.
        if let Some(home_dir) = home_dir() {
            command.args(["--env", &format!("HOME={}", MOUNT_HOME)]);
            command.args([
                "--mount",
                &format!(
                    "type=bind,source={},target={}",
                    home_dir.to_string_lossy(),
                    MOUNT_HOME
                ),
            ]);
        }

        // Honour `FLOX_DISABLE_METRICS` if set. Aside from being set by the
        // user, it may also be set at runtime by  [Flox::Commands::FloxArgs]
        // from another config path like `/etc/flox.toml` which isn't mounted
        // into the proxy container.
        if let Ok(disable_metrics) = std::env::var(FLOX_DISABLE_METRICS_VAR) {
            command.args([
                "--env",
                &format!("{}={}", FLOX_DISABLE_METRICS_VAR, disable_metrics),
            ]);
        }

        let flox_version = &*FLOX_VERSION;
        let flox_version_tag = format!("v{}", flox_version.base_semver());

        // Use a released Flox container of the same semantic version as a base
        // because it already has:
        //
        // - most of the dependency store paths
        // - substitutors configured
        // - correct version of nix
        let flox_container = format!("{}:{}", FLOX_PROXY_IMAGE, flox_version_tag);
        command.arg(flox_container);

        // Inception L2: Nix args.
        command.arg("nix");
        command.args(["--extra-experimental-features", "nix-command flakes"]);
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

        // Inception L3: Flox args.

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

        let container_source = ContainerSource::new(command);
        Ok(container_source)
    }
}
