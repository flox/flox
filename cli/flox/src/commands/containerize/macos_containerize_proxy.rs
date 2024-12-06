use std::convert::Infallible;
use std::path::PathBuf;

use dirs::home_dir;
use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::providers::container_builder::{ContainerBuilder, ContainerSource};

use super::Runtime;

const FLOX_FLAKE: &str = "github:flox/flox";
const FLOX_PROXY_IMAGE: &str = "ghcr.io/flox/flox";

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

    fn create_container_source(
        &self,
        _name: impl AsRef<str>, // Inferred from `self.environment_path`
        tag: impl AsRef<str>,
    ) -> Result<ContainerSource, Self::Error> {
        let env_mount = "/flox_env";
        let home_mount = "/flox_home";

        // Inception L1: Container runtime args.
        let mut command = self.container_runtime.to_command();
        command.arg("run");
        command.arg("--rm");
        command.args([
            "--mount",
            &format!(
                "type=bind,source={},target={}",
                self.environment_path.to_string_lossy(),
                env_mount
            ),
        ]);

        // Honour config from the user's home directory on their host machine if
        // available.
        if let Some(home_dir) = home_dir() {
            command.args(["--env", &format!("HOME={}", home_mount)]);
            command.args([
                "--mount",
                &format!(
                    "type=bind,source={},target={}",
                    home_dir.to_string_lossy(),
                    home_mount
                ),
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
            flox_version
                .commit_sha()
                // TODO: Replace fallback when 0a8e8d1d has been released.
                // .unwrap_or(flox_version_tag)
                .unwrap_or_else(|| match flox_version_tag.as_str() {
                    "v1.3.6" => String::from("0a8e8d1d368d85e5afa7a35394f9a212cbc18aa4"),
                    _ => flox_version_tag,
                })
        );
        command.args(["run", &flox_flake, "--"]);

        // Inception L3: Flox args.
        command.arg("containerize");
        command.args(["--dir", env_mount]);
        command.args(["--tag", tag.as_ref()]);
        command.args(["--file", "-"]);

        let container_source = ContainerSource::new(command);
        Ok(container_source)
    }
}
