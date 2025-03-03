use std::convert::Infallible;
use std::fmt::Display;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::{fs, io};

use anyhow::{anyhow, bail, Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::providers::container_builder::{ContainerBuilder, MkContainerNix};
use flox_rust_sdk::utils::{ReaderExt, WireTap};
use indoc::indoc;
use macos_containerize_proxy::ContainerizeProxy;
use tracing::{debug, info, instrument};

use super::{environment_select, EnvironmentSelect};
use crate::utils::message;
use crate::utils::openers::first_in_path;
use crate::{environment_subcommand_metric, subcommand_metric};

mod macos_containerize_proxy;

// Containerize an environment
#[derive(Bpaf, Clone, Debug)]
pub struct Containerize {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Output to write the container to.
    /// Defaults to loading into container storage if docker or podman is present
    /// or otherwise writes to a file '{name}-container.tar'
    #[bpaf(external(output_target), optional)]
    output: Option<OutputTarget>,

    /// Tag to apply to the container, defaults to 'latest'
    #[bpaf(short, long, argument("tag"))]
    tag: Option<String>,
}
impl Containerize {
    #[instrument(name = "containerize", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        environment_subcommand_metric!("containerize", self.environment);

        let mut env = self
            .environment
            .detect_concrete_environment(&flox, "Containerize")?;

        let output = self
            .output
            .unwrap_or_else(|| OutputTarget::detect_or_default(env.name().as_ref()));

        let output_tag: &str = match self.tag {
            Some(tag) => &tag.to_string(),
            None => "latest",
        };

        let _span = tracing::info_span!(
            "building and writing container",
            progress = format!("Creating container image and writing to {output}")
        );

        let built_environment = env.build(&flox)?;
        let env_name = env.name();
        let manifest = env.lockfile(&flox)?.manifest;
        let mode = manifest.options.activate.mode.unwrap_or_default();

        let source = if std::env::consts::OS == "linux" {
            let container_config = manifest
                .containerize
                .and_then(|c| c.config)
                .map(|c| c.into());
            // this method is only executed on linux
            #[cfg_attr(not(target_os = "linux"), allow(deprecated))]
            let builder =
                MkContainerNix::new(built_environment.for_mode(&mode), mode, container_config);

            builder.create_container_source(&flox, env_name.as_ref(), output_tag)?
        } else {
            let env_path = env.parent_path()?;
            let Some(container_runtime) = Runtime::detect_from_path() else {
                bail!(indoc! {r#"
                    No container runtime found in PATH.

                    Exporting a container on macOS requires Docker or Podman to be installed.
                "#});
            };
            let builder = ContainerizeProxy::new(env_path, container_runtime);
            builder.create_container_source(&flox, env_name.as_ref(), output_tag)?
        };

        let mut writer = output.to_writer()?;
        source.stream_container(&mut writer)?;
        writer.wait()?;

        message::created(format!("'{env_name}:{output_tag}' written to {output}"));
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Bpaf)]
enum OutputTarget {
    File(
        #[bpaf(
            long("file"),
            short('f'),
            argument("file"),
            help("File to write the container image to. '-' to write to stdout.")
        )]
        FileOrStdout,
    ),
    Runtime(
        #[bpaf(
            long("runtime"),
            argument("runtime"),
            help("Container runtime to load the image into. 'docker' or 'podman'")
        )]
        Runtime,
    ),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FileOrStdout {
    File(PathBuf),
    Stdout,
}

impl FromStr for FileOrStdout {
    type Err = Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        if s == "-" {
            Ok(FileOrStdout::Stdout)
        } else {
            Ok(FileOrStdout::File(PathBuf::from(s)))
        }
    }
}

impl OutputTarget {
    fn detect_or_default(env_name: impl AsRef<str>) -> Self {
        let default_to_file = OutputTarget::File(FileOrStdout::File(PathBuf::from(format!(
            "{}-container.tar",
            env_name.as_ref()
        ))));

        let Some(runtime) = Runtime::detect_from_path() else {
            debug!("No container runtime found in PATH, defaulting to file");
            return default_to_file;
        };

        OutputTarget::Runtime(runtime)
    }

    fn to_writer(&self) -> Result<Box<dyn ContainerSink>> {
        let writer: Box<dyn ContainerSink> = match self {
            OutputTarget::File(FileOrStdout::File(path)) => {
                let file = fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(path)
                    .context("Could not open output file")?;

                Box::new(file)
            },
            OutputTarget::File(FileOrStdout::Stdout) => Box::new(io::stdout()),
            OutputTarget::Runtime(runtime) => Box::new(runtime.to_writer()?),
        };

        Ok(writer)
    }
}

impl Display for OutputTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputTarget::File(FileOrStdout::File(path)) => write!(f, "file '{}'", path.display()),
            OutputTarget::File(FileOrStdout::Stdout) => write!(f, "stdout"),
            OutputTarget::Runtime(runtime) => write!(f, "{runtime}"),
        }
    }
}

/// A sink for writing container tarballs
///
/// This trait extends the `Write` trait with a `wait` method,
/// which blocks until all data has been written to the sink
/// and returns any errors the sink may have encountered
/// that are not strictly I/O errors (e.g. process exit status).
///
/// In case of sinks that are subprocesses,
/// the `wait` method should also wait for the subprocess to exit,
/// in order not to orphan the process.
trait ContainerSink: Write + Send {
    fn wait(&mut self) -> Result<()>;
}

impl ContainerSink for fs::File {
    fn wait(&mut self) -> Result<()> {
        self.sync_all()?;
        Ok(())
    }
}

impl ContainerSink for io::Stdout {
    fn wait(&mut self) -> Result<()> {
        self.flush()?;
        Ok(())
    }
}

#[derive(Debug)]
struct RuntimeSink {
    /// An optional collector for the runtime's stderr
    /// to be displayed to the user in case of errors.
    /// This is an Option, due to [ContainerSink::wait]
    /// taking a mutable reference.
    stderr: Option<WireTap<String>>,
    child: Child,
}

impl Write for RuntimeSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.child.stdin.as_mut().unwrap().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.child.stdin.as_mut().unwrap().flush()
    }
}

impl ContainerSink for RuntimeSink {
    fn wait(&mut self) -> Result<()> {
        self.flush()?;
        drop(self.child.stdin.take());
        let status = self.child.wait()?;
        let stderr = self
            .stderr
            .take()
            .expect("stderr is tapped and `ContainerSink::wait` is called only once")
            .wait();
        if !status.success() {
            return Err(anyhow!("Writing to runtime was unsuccessful").context(stderr));
        }

        Ok(())
    }
}

/// The container registry to load the container into
/// Currently only supports Docker and Podman
#[derive(Debug, Clone, PartialEq, Eq)]
enum Runtime {
    Docker,
    Podman,
}

impl Runtime {
    /// Detect the container runtime from the PATH environment variable.
    fn detect_from_path() -> Option<Self> {
        let path_var = match std::env::var("PATH") {
            Err(e) => {
                debug!("Could not read PATH variable: {e}");
                return None;
            },
            Ok(path) => path,
        };

        let Some((_, runtime)) =
            first_in_path(["docker", "podman"], std::env::split_paths(&path_var))
        else {
            debug!("No container runtime found in PATH");
            return None;
        };

        debug!(runtime, "Detected container runtime");
        let runtime =
            Runtime::from_str(runtime).expect("Should search for valid runtime names only");

        Some(runtime)
    }

    /// Get a writer to the registry,
    /// Essentially spawns a `docker load` or `podman load` process
    /// and returns a handle to its stdin.
    fn to_writer(&self) -> Result<impl ContainerSink> {
        let cmd = match self {
            Runtime::Docker => "docker",
            Runtime::Podman => "podman",
        };

        let mut child = Command::new(cmd)
            .arg("load")
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .context(format!("Failed to call runtime {cmd}"))?;

        let stderr_tap = child
            .stderr
            .take()
            .expect("Stderr is piped")
            .tap_lines(|line| info!("{line}"));

        child
            .stdout
            .take()
            .expect("Stdout is piped")
            .tap_lines(|line| info!("{line}"));

        Ok(RuntimeSink {
            child,
            stderr: Some(stderr_tap),
        })
    }

    fn to_command(&self) -> Command {
        let cmd = match self {
            Runtime::Docker => "docker",
            Runtime::Podman => "podman",
        };

        Command::new(cmd)
    }
}

impl Display for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Runtime::Docker => write!(f, "Docker runtime"),
            Runtime::Podman => write!(f, "Podman runtime"),
        }
    }
}

impl FromStr for Runtime {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "docker" => Ok(Runtime::Docker),
            "podman" => Ok(Runtime::Podman),
            _ => Err(anyhow!("Registry must be 'docker' or 'podman'")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_parse() {
        "docker".parse::<Runtime>().unwrap();
        "podman".parse::<Runtime>().unwrap();
        assert!("invalid".parse::<Runtime>().is_err());
    }

    /// Test that the default output target is one of the supported runtimes
    /// which is found first in the PATH, or a file with the environment name,
    /// if no supported runtime is found in the PATH.
    #[test]
    fn detect_runtime_in_path() {
        let tempdir = tempfile::tempdir().unwrap();

        let default_target =
            OutputTarget::File(FileOrStdout::File(PathBuf::from("test-container.tar")));
        let docker_target = OutputTarget::Runtime(Runtime::Docker);
        let podman_target = OutputTarget::Runtime(Runtime::Podman);

        let docker_bin = tempdir.path().join("docker-bin");
        let podman_bin = tempdir.path().join("podman-bin");
        let combined_bin = tempdir.path().join("combined-bin");
        let neither_bin = tempdir.path().join("neither-bin");

        fs::create_dir(&docker_bin).unwrap();
        fs::create_dir(&podman_bin).unwrap();
        fs::create_dir(&combined_bin).unwrap();
        fs::create_dir(&neither_bin).unwrap();

        fs::write(docker_bin.join("docker"), "").unwrap();
        fs::write(podman_bin.join("podman"), "").unwrap();
        fs::write(combined_bin.join("docker"), "").unwrap();
        fs::write(combined_bin.join("podman"), "").unwrap();

        let target = temp_env::with_var(
            "PATH",
            Some(std::env::join_paths([&docker_bin, &podman_bin, &combined_bin]).unwrap()),
            || OutputTarget::detect_or_default("test"),
        );
        assert_eq!(target, docker_target);

        let target = temp_env::with_var(
            "PATH",
            Some(std::env::join_paths([&podman_bin, &docker_bin, &combined_bin]).unwrap()),
            || OutputTarget::detect_or_default("test"),
        );
        assert_eq!(target, podman_target);

        let target = temp_env::with_var(
            "PATH",
            Some(std::env::join_paths([&combined_bin, &podman_bin, &docker_bin]).unwrap()),
            || OutputTarget::detect_or_default("test"),
        );
        assert_eq!(target, docker_target);

        let target = temp_env::with_var(
            "PATH",
            Some(std::env::join_paths([neither_bin]).unwrap()),
            || OutputTarget::detect_or_default("test"),
        );
        assert_eq!(target, default_target);
    }
}
