use std::collections::BTreeMap;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use serde_with::skip_serializing_none;
use thiserror::Error;
use tracing::{debug, info, instrument};

use super::buildenv::BuiltStorePath;
use crate::flox::Flox;
use crate::models::manifest::typed::ManifestContainerizeConfig;
use crate::providers::build::BUILDTIME_NIXPKGS_URL;
use crate::providers::nix::nix_base_command;
use crate::utils::gomap::GoMap;
use crate::utils::{CommandExt, ReaderExt};

static MK_CONTAINER_NIX: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("FLOX_MK_CONTAINER_NIX")
        .unwrap_or_else(|_| env!("FLOX_MK_CONTAINER_NIX").to_string())
        .into()
});

pub trait ContainerBuilder {
    type Error: std::error::Error;
    fn create_container_source(
        &self,
        flox: &Flox,
        name: impl AsRef<str>,
        tag: impl AsRef<str>,
    ) -> Result<ContainerSource, Self::Error>;
}

/// OCI representation of our more user-friendly `ManifestContainerizeConfig`.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "PascalCase")]
pub struct OCIConfig {
    user: Option<String>,
    exposed_ports: Option<GoMap>,
    cmd: Option<Vec<String>>,
    volumes: Option<GoMap>,
    working_dir: Option<String>,
    labels: Option<BTreeMap<String, String>>,
    stop_signal: Option<String>,
}

impl From<ManifestContainerizeConfig> for OCIConfig {
    fn from(config: ManifestContainerizeConfig) -> Self {
        Self {
            user: config.user,
            exposed_ports: config.exposed_ports.map(GoMap::from),
            cmd: config.cmd,
            volumes: config.volumes.map(GoMap::from),
            working_dir: config.working_dir,
            labels: config.labels,
            stop_signal: config.stop_signal,
        }
    }
}

/// An implementation of [ContainerBuilder] that uses a Nix script
/// to build a native [ContainerSource] from a [BuiltStorePath].
///
/// [MK_CONTAINER_NIX] is a Nix script that uses nixpkgs' `dockerTools.streamLayeredImage`
/// which produces a script which that composes store paths
/// as layers of a container image, and writes the resulting image to `stdout`.
/// [MkContainerNix::create_container_source] performs the build of that script,
/// which in turn builds all the required layers.
/// The resulting [ContainerSource] will execute the script
/// and stream the container image to a given sink via [ContainerSource::stream_container].
///
/// Note: that this implementation is not suitable for building containers on macOS.
///
/// 1. Containers are native _linux_ systems, for which packages can't be built on macOS.
/// 2. The script generated by `dockerTools.streamLayeredImage` is not compatible with macOS.
#[derive(Debug)]
pub struct MkContainerNix {
    store_path: BuiltStorePath,
    container_config: Option<OCIConfig>,
}

#[derive(Debug, Error)]
pub enum MkContainerNixError {
    #[error("failed to call nix")]
    CallNixError(#[source] std::io::Error),

    #[error("failed to build container: {0}")]
    BuildContainerError(String),

    #[error("failed to parse nix build output")]
    ParseBuildOutout(#[source] serde_json::Error),

    #[error("couldn't serialize container config")]
    SerializeContainerConfig(#[source] serde_json::Error),
}

impl MkContainerNix {
    /// Create a new [MkContainerNix] instance that will build a container
    /// from the given [BuiltStorePath].
    /// Generally, this should be the output of a [crate::providers::buildenv::BuildEnv::build].
    ///
    /// Note: this constructor is only available on Linux.
    /// On macOS, use a macOS-specific implementation of [ContainerBuilder].
    #[cfg_attr(
        not(target_os = "linux"),
        deprecated(note = "MkContainerNix is not supported on this platform")
    )]
    pub fn new(store_path: BuiltStorePath, container_config: Option<OCIConfig>) -> Self {
        Self {
            store_path,
            container_config,
        }
    }
}

impl ContainerBuilder for MkContainerNix {
    type Error = MkContainerNixError;

    /// Create a [ContainerSource] that will assemble a container image.
    /// The container image will be built from the provided [BuiltStorePath],
    /// which is generally the output of a [crate::providers::buildenv::BuildEnv::build].
    /// The streaming script will be built via `nix build`.
    #[instrument(skip_all, fields(
        name = name.as_ref(),
        tag = tag.as_ref(),
        progress = "Building container layers"))]
    fn create_container_source(
        &self,
        _flox: &Flox,
        name: impl AsRef<str>,
        tag: impl AsRef<str>,
    ) -> Result<ContainerSource, Self::Error> {
        let mut command = nix_base_command();
        command.args(["--option", "pure-eval", "true"]);
        command.arg("build");
        command.arg("--json");
        command.arg("--no-link");
        command.arg("--file").arg(&*MK_CONTAINER_NIX);
        command.args(["--argstr", "nixpkgsFlakeRef", &*BUILDTIME_NIXPKGS_URL]);
        command.args(["--argstr", "containerSystem", env!("NIX_TARGET_SYSTEM")]);
        command.args(["--argstr", "system", env!("NIX_TARGET_SYSTEM")]);
        command.args([
            "--argstr",
            "environmentOutPath",
            self.store_path.to_string_lossy().as_ref(),
        ]);
        command.args(["--argstr", "containerName", name.as_ref()]);
        command.args(["--argstr", "containerTag", tag.as_ref()]);
        if let Some(container_config) = &self.container_config {
            command.args([
                "--argstr",
                "containerConfig",
                &serde_json::to_string(container_config)
                    .map_err(MkContainerNixError::SerializeContainerConfig)?,
            ]);
        }
        debug!(cmd=%command.display(), "building container");

        let output = command
            .output()
            .map_err(MkContainerNixError::CallNixError)?;
        if !output.status.success() {
            return Err(MkContainerNixError::BuildContainerError(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        // defined inline as an implementation detail
        #[derive(Debug, Clone, Deserialize)]
        struct MkContainerOutputs {
            out: PathBuf,
        }

        #[derive(Debug, Clone, Deserialize)]
        struct MkContainerResultRaw {
            outputs: MkContainerOutputs,
        }

        let [raw @ MkContainerResultRaw { .. }] = serde_json::from_slice(&output.stdout)
            .map_err(MkContainerNixError::ParseBuildOutout)?;
        let container_builder_script_path = raw.outputs.out;
        let container_source = ContainerSource::new(Command::new(container_builder_script_path));

        Ok(container_source)
    }
}

/// Type representing a container source,
/// i.e. a command that writes a container tarball to stdout.
/// This is typically created by [ContainerBuilder::create_container_source].
#[derive(Debug)]
pub struct ContainerSource {
    source_command: Command,
}

impl ContainerSource {
    pub fn new(source_command: Command) -> Self {
        Self { source_command }
    }

    /// Run the container builder script
    /// and write the container tarball to the given sink
    #[instrument(skip_all, fields(command = ?self.source_command, progress = "Writing container"))]
    pub fn stream_container(self, sink: &mut impl Write) -> Result<(), ContainerSourceError> {
        let mut container_source_command = self.source_command;

        // ensure the command writes to stdout
        container_source_command.stdout(Stdio::piped());
        container_source_command.stderr(Stdio::piped());

        debug!(
            "running container source command: {}",
            container_source_command.display()
        );

        let mut handle = container_source_command
            .spawn()
            .map_err(ContainerSourceError::CallContainerSourceCommand)?;

        let tap = handle
            .stderr
            .take()
            .expect("stderr set to piped")
            .tap_lines(|line| info!("{line}"));

        let mut stdout = handle.stdout.take().expect("stdout set to piped");

        io::copy(&mut stdout, sink).map_err(ContainerSourceError::StreamContainer)?;

        let status = handle
            .wait()
            .map_err(ContainerSourceError::CallContainerSourceCommand)?;

        if !status.success() {
            let stderr_content = tap.wait();
            return Err(ContainerSourceError::CommandUnsuccessful(stderr_content));
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ContainerSourceError {
    #[error("failed to call container source command")]
    CallContainerSourceCommand(#[source] std::io::Error),
    #[error("failed to stream container to sink")]
    StreamContainer(#[source] std::io::Error),
    #[error(
        "container source command unsuccessful\n\
        \n\
        stderr:\n\
        {0}"
    )]
    CommandUnsuccessful(String),
}

#[cfg(test)]
mod container_source_tests {
    use std::collections::BTreeSet;
    use std::fs::{self, File};
    use std::os::unix::fs::PermissionsExt;

    use indoc::indoc;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn oci_config_from_manifest() {
        let manifest_config = ManifestContainerizeConfig {
            user: Some("root".to_string()),
            exposed_ports: Some(BTreeSet::from(["80/tcp".to_string()])),
            volumes: Some(BTreeSet::from(["/app".to_string()])),
            working_dir: Some("/app".to_string()),
            ..Default::default()
        };
        let oci_config: OCIConfig = manifest_config.into();
        let json = serde_json::to_string_pretty(&oci_config).unwrap();

        // Selection of fields that verify From + Serialize:
        // - omits fields that are `None`
        // - renames keys from kebab-case to PascalCase
        // - converts values from `BTreeSet` to `GoMap`
        assert_eq!(json, indoc! {r#"{
          "User": "root",
          "ExposedPorts": {
            "80/tcp": {}
          },
          "Volumes": {
            "/app": {}
          },
          "WorkingDir": "/app"
        }"#});
    }

    /// OS error 26 is "Text file busy",
    /// which can happen when executing a script
    /// that is has been written to immediately before.
    /// We typically see this in tests, where we write
    /// a new script and immediately execute it.
    /// In production use, this should not happen as the script
    /// will be written by a different process (`nix`).
    ///
    /// <https://github.com/rust-lang/rust/issues/114554>
    const ERR_TEXT_FILE_BUSY: i32 = 26;

    const TEST_BUILDER: &str = indoc! {r#"
        #!/usr/bin/env bash
        echo "hello world"
    "#};

    fn create_test_script() -> (TempDir, PathBuf) {
        let tempdir = tempfile::tempdir().unwrap();
        let path = tempdir.path().join("flox-test-container-builder");
        std::fs::write(&path, TEST_BUILDER).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (tempdir, path)
    }

    #[test]
    fn test_writes_output_to_writer() {
        let (_tempdir, test_script) = create_test_script();

        let mut buf = Vec::new();

        let mut tries = 0;
        loop {
            if tries >= 3 {
                panic!("Test flaked with 'Text file busy' and can be re-run")
            }
            let container_builder = ContainerSource::new(Command::new(&test_script));
            match container_builder.stream_container(&mut buf) {
                Err(ContainerSourceError::CallContainerSourceCommand(e))
                    if e.raw_os_error() == Some(ERR_TEXT_FILE_BUSY) =>
                {
                    dbg!("Text file busy -- ignored");
                    tries += 1;
                    continue;
                },
                result => break result.unwrap(),
            }
        }
        assert_eq!(buf, b"hello world\n");
    }

    #[test]
    fn test_allows_forwarding_to_file() {
        let (tempdir, test_script) = create_test_script();
        let output_path = tempdir.path().join("output");

        let mut file = File::create(&output_path).unwrap();

        // looping to ignore "Text file busy" errors
        // see the comment on `ERR_TEXT_FILE_BUSY` for more information
        let mut tries = 0;
        loop {
            if tries >= 3 {
                panic!("Test flaked with 'Text file busy' and can be re-run")
            }
            let container_builder = ContainerSource::new(Command::new(&test_script));
            match container_builder.stream_container(&mut file) {
                Err(ContainerSourceError::CallContainerSourceCommand(e))
                    if e.raw_os_error() == Some(ERR_TEXT_FILE_BUSY) =>
                {
                    dbg!("Text file busy -- ignored");
                    tries += 1;
                    continue;
                },
                result => break result.unwrap(),
            }
        }
        drop(file);

        let output = fs::read_to_string(&output_path).unwrap();
        assert_eq!(output, "hello world\n");
    }
}
