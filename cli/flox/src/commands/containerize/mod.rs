use std::convert::Infallible;
use std::fmt::Display;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::{fs, io};

use anyhow::{Context, Result, anyhow, bail};
use bpaf::Bpaf;
use flox_core::activate::context::ActivateMode;
use flox_events::EventsHub;
use flox_manifest::interfaces::AsLatestSchema;
use flox_manifest::lockfile::Lockfile;
use flox_manifest::parsed::common::ContainerizeConfig;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::providers::container_builder::{ContainerBuilder, MkContainerNix};
use flox_rust_sdk::utils::{ReaderExt, WireTap};
use indoc::indoc;
use macos_containerize_proxy::ContainerizeProxy;
use tempfile::NamedTempFile;
use tracing::{debug, info, instrument};

use super::{EnvironmentSelect, environment_select};
use crate::commands::SHELL_COMPLETION_FILE;
use crate::environment_subcommand_metric;
use crate::utils::events::env_detail_from_concrete;
use crate::utils::message;
use crate::utils::openers::first_in_path;

pub(crate) mod macos_containerize_proxy;

// Containerize an environment
#[derive(Bpaf, Clone, Debug)]
pub struct Containerize {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Container runtime to
    /// store the image (when '--file' is not specified)
    /// or build the image (when on macOS).
    /// Defaults to detecting the first available on PATH.
    #[bpaf(long, argument("docker|podman|container"))]
    runtime: Option<Runtime>,

    /// File to write the container image to.
    /// '-` to write to stdout.
    /// Defaults to '{name}-container.tar' if '--runtime' isn't specified or detected.
    #[bpaf(short, long, argument("file"), complete_shell(SHELL_COMPLETION_FILE))]
    file: Option<FileOrStdout>,

    /// Tag to apply to the container, defaults to 'latest'
    #[bpaf(short, long, argument("tag"))]
    tag: Option<String>,

    /// Set metadata for an image
    #[bpaf(long("label"), argument("key=value"))]
    labels: Vec<String>,

    /// Containerize the environment in either "dev" or "run" mode.
    /// Overrides the "options.activate.mode" setting in the manifest.
    #[bpaf(short, long)]
    mode: Option<ActivateMode>,
}
impl Containerize {
    #[instrument(name = "containerize", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        let mut env = self
            .environment
            .detect_concrete_environment(&mut flox, "Containerize")
            .await?;
        environment_subcommand_metric!("containerize", env);
        if let Err(err) =
            EventsHub::global().record_environment_containerize(env_detail_from_concrete(&env))
        {
            debug!(error = %err, "Failed to record v2 event");
        }

        // Check that a specified runtime exists.
        if let Some(runtime) = &self.runtime {
            runtime.validate_in_path()?
        }
        let runtime = self.runtime.or_else(Runtime::detect_from_path);
        let output = match (&runtime, self.file) {
            // Specified file.
            (_, Some(dest)) => OutputTarget::File(dest),
            // Or specified or detected runtime.
            (Some(runtime), None) => OutputTarget::Runtime(runtime.clone()),
            // Or default file.
            (None, None) => OutputTarget::default_file(env.name().as_ref()),
        };

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
        let lockfile: Lockfile = env.lockfile(&flox)?.into();
        let manifest = lockfile.migrated_manifest()?;
        let manifest = manifest.as_latest_schema();
        let source = if std::env::consts::OS == "linux" {
            let mode = self
                .mode
                .unwrap_or(manifest.options.activate.mode.clone().unwrap_or_default());
            let container_config = manifest
                .containerize
                .as_ref()
                .and_then(|c| c.config.clone())
                .or_else(|| should_extend_config(&self.labels).then(Default::default))
                .map(|mut c| {
                    extend_config(&self.labels, &mut c);
                    c.into()
                });
            // this method is only executed on linux
            #[cfg_attr(not(target_os = "linux"), allow(deprecated))]
            let builder =
                MkContainerNix::new(built_environment.for_mode(&mode), mode, container_config);

            builder.create_container_source(&flox, env_name.as_ref(), output_tag)?
        } else {
            let env_path = env.parent_path()?;
            let Some(proxy_runtime) = runtime else {
                bail!(indoc! {r#"
                    No container runtime found in PATH.

                    Exporting a container on macOS requires Docker, Podman, or Apple Container ('container') to be installed.
                "#});
            };
            let builder = ContainerizeProxy::new(env_path, proxy_runtime, self.labels, self.mode);
            builder.create_container_source(&flox, env_name.as_ref(), output_tag)?
        };

        let mut writer = output.to_writer()?;
        source.stream_container(&mut writer)?;
        writer.wait()?;

        message::created(format!("'{env_name}:{output_tag}' written to {output}"));
        Ok(())
    }
}

fn should_extend_config(labels: &[String]) -> bool {
    labels.is_empty()
}

fn extend_config(labels: &[String], config: &mut ContainerizeConfig) {
    if !labels.is_empty() {
        let mut label_map = config.labels.take().unwrap_or_default();

        label_map.extend(labels.iter().map(|label| {
            if let Some((key, value)) = label.split_once('=') {
                (key.to_string(), value.to_string())
            } else {
                (label.to_string(), String::new())
            }
        }));

        config.labels = Some(label_map);
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Bpaf)]
enum OutputTarget {
    File(FileOrStdout),
    Runtime(Runtime),
}

impl OutputTarget {
    fn default_file(env_name: impl AsRef<str>) -> Self {
        OutputTarget::File(FileOrStdout::File(PathBuf::from(format!(
            "{}-container.tar",
            env_name.as_ref()
        ))))
    }

    fn to_writer(&self) -> Result<Box<dyn ContainerSink + '_>> {
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
            OutputTarget::Runtime(runtime) => runtime.to_writer()?,
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
pub(crate) trait ContainerSink: Write + Send {
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

/// Sink for Apple Container (`container image load`).
///
/// Apple Container's `image load` does not read from stdin; it requires a
/// file path via `--input`. This sink accumulates the OCI archive in a
/// temporary file and invokes `container image load --input <file>` when
/// `wait()` is called.
#[derive(Debug)]
struct AppleContainerSink {
    /// The temporary file holding the OCI archive.
    /// Wrapped in Option so it can be taken in `wait()`.
    tmp: Option<NamedTempFile>,
}

impl Write for AppleContainerSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.tmp
            .as_mut()
            .expect("tmp file is present before wait()")
            .write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.tmp
            .as_mut()
            .expect("tmp file is present before wait()")
            .flush()
    }
}

impl ContainerSink for AppleContainerSink {
    fn wait(&mut self) -> Result<()> {
        self.flush()?;
        let tmp = self
            .tmp
            .take()
            .expect("tmp file is present and wait() is called only once");

        let tmp_path = tmp.path().to_owned();
        debug!(path = %tmp_path.display(), "loading OCI archive into Apple Container");

        let mut load_cmd = Command::new("container");
        load_cmd.args(["image", "load", "--input"]);
        load_cmd.arg(&tmp_path);
        load_cmd.stderr(Stdio::piped());
        load_cmd.stdout(Stdio::piped());

        let mut child = load_cmd
            .spawn()
            .context("Failed to spawn 'container image load'")?;

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

        let status = child.wait()?;
        let stderr = stderr_tap.wait();

        if !status.success() {
            return Err(anyhow!("'container image load' was unsuccessful").context(stderr));
        }

        Ok(())
    }
}

/// The container runtime used to load or build container images.
///
/// Detection order is docker → podman → container, so existing users
/// with Docker or Podman installed see no behavior change; Apple Container
/// is used only when it is the only runtime present (or explicitly selected).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Runtime {
    Docker,
    Podman,
    /// Apple Container (the `container` CLI, macOS-only).
    /// Requires OCI archive format rather than docker-archive.
    AppleContainer,
}

impl Runtime {
    /// Detect the container runtime from the PATH environment variable.
    ///
    /// Search order: docker, podman, container — append-only so existing
    /// users with Docker or Podman installed see no behavior change.
    fn detect_from_path() -> Option<Self> {
        let path_var = match std::env::var("PATH") {
            Err(e) => {
                debug!("Could not read PATH variable: {e}");
                return None;
            },
            Ok(path) => path,
        };

        let Some((_, runtime)) = first_in_path(
            ["docker", "podman", "container"],
            std::env::split_paths(&path_var),
        ) else {
            debug!("No container runtime found in PATH");
            return None;
        };

        debug!(runtime, "Detected container runtime");
        let runtime =
            Runtime::from_str(runtime).expect("Should search for valid runtime names only");

        Some(runtime)
    }

    /// Get the unqualified command name for the runtime.
    fn to_cmd(&self) -> &str {
        match self {
            Runtime::Docker => "docker",
            Runtime::Podman => "podman",
            Runtime::AppleContainer => "container",
        }
    }

    /// Returns true when the runtime requires OCI archive format rather than
    /// docker-archive. Apple Container's `image load` only accepts OCI archives.
    fn requires_oci_format(&self) -> bool {
        matches!(self, Runtime::AppleContainer)
    }

    /// Validate that the container runtime is available in the PATH.
    fn validate_in_path(&self) -> Result<()> {
        let path_var = std::env::var("PATH").context("Could not read PATH variable")?;
        let paths = std::env::split_paths(path_var.as_str());
        let cmd = self.to_cmd();
        match first_in_path([cmd], paths) {
            Some(_) => Ok(()),
            None => Err(anyhow!(format!(
                "Container runtime '{cmd}' not found in PATH.",
            ))),
        }
    }

    /// Get a writer to the registry.
    ///
    /// For Docker and Podman this spawns `docker load` / `podman load` and
    /// returns a handle to its stdin.
    ///
    /// For Apple Container, `container image load` does not read from stdin;
    /// it requires a file path via `--input`. The returned sink writes to a
    /// temporary OCI archive on disk and invokes `container image load` when
    /// flushed.
    pub(crate) fn to_writer(&self) -> Result<Box<dyn ContainerSink>> {
        match self {
            Runtime::Docker | Runtime::Podman => {
                let cmd = self.to_cmd();
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

                Ok(Box::new(RuntimeSink {
                    child,
                    stderr: Some(stderr_tap),
                }))
            },
            Runtime::AppleContainer => {
                // `container image load` requires a file path; it cannot read
                // from stdin. Write the OCI archive to a temp file, then invoke
                // `container image load --input <file>` when the sink is flushed.
                let tmp = tempfile::NamedTempFile::new()
                    .context("Failed to create temporary file for OCI archive")?;
                Ok(Box::new(AppleContainerSink { tmp: Some(tmp) }))
            },
        }
    }

    fn to_command(&self) -> Command {
        let cmd = self.to_cmd();
        Command::new(cmd)
    }
}

impl Display for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Runtime::Docker => write!(f, "Docker runtime"),
            Runtime::Podman => write!(f, "Podman runtime"),
            Runtime::AppleContainer => write!(f, "Apple Container runtime"),
        }
    }
}

impl FromStr for Runtime {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "docker" => Ok(Runtime::Docker),
            "podman" => Ok(Runtime::Podman),
            "container" => Ok(Runtime::AppleContainer),
            _ => Err(anyhow!(
                "Runtime must be 'docker', 'podman', or 'container'"
            )),
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
        "container".parse::<Runtime>().unwrap();
        assert!("invalid".parse::<Runtime>().is_err());
        assert_eq!(
            "container".parse::<Runtime>().unwrap(),
            Runtime::AppleContainer
        );
    }

    #[test]
    fn apple_container_requires_oci_format() {
        assert!(Runtime::AppleContainer.requires_oci_format());
        assert!(!Runtime::Docker.requires_oci_format());
        assert!(!Runtime::Podman.requires_oci_format());
    }

    #[test]
    fn detect_runtime_in_path() {
        let tempdir = tempfile::tempdir().unwrap();

        let docker_target = Runtime::Docker;
        let podman_target = Runtime::Podman;
        let apple_container_target = Runtime::AppleContainer;

        let docker_bin = tempdir.path().join("docker-bin");
        let podman_bin = tempdir.path().join("podman-bin");
        let combined_bin = tempdir.path().join("combined-bin");
        let neither_bin = tempdir.path().join("neither-bin");
        let apple_container_bin = tempdir.path().join("apple-container-bin");
        let all_bin = tempdir.path().join("all-bin");

        fs::create_dir(&docker_bin).unwrap();
        fs::create_dir(&podman_bin).unwrap();
        fs::create_dir(&combined_bin).unwrap();
        fs::create_dir(&neither_bin).unwrap();
        fs::create_dir(&apple_container_bin).unwrap();
        fs::create_dir(&all_bin).unwrap();

        fs::write(docker_bin.join("docker"), "").unwrap();
        fs::write(podman_bin.join("podman"), "").unwrap();
        fs::write(combined_bin.join("docker"), "").unwrap();
        fs::write(combined_bin.join("podman"), "").unwrap();
        fs::write(apple_container_bin.join("container"), "").unwrap();
        fs::write(all_bin.join("docker"), "").unwrap();
        fs::write(all_bin.join("podman"), "").unwrap();
        fs::write(all_bin.join("container"), "").unwrap();

        let docker_first_path =
            Some(std::env::join_paths([&docker_bin, &podman_bin, &combined_bin]).unwrap());
        let podman_first_path =
            Some(std::env::join_paths([&podman_bin, &docker_bin, &combined_bin]).unwrap());
        let combined_path =
            Some(std::env::join_paths([&combined_bin, &podman_bin, &docker_bin]).unwrap());
        let neither_path = Some(std::env::join_paths([&neither_bin]).unwrap());
        let apple_container_only_path = Some(std::env::join_paths([&apple_container_bin]).unwrap());
        // When all three runtimes are present, docker takes precedence.
        let all_runtimes_path = Some(std::env::join_paths([&all_bin]).unwrap());
        // When only podman and container are present, podman takes precedence.
        let podman_and_container_path =
            Some(std::env::join_paths([&podman_bin, &apple_container_bin]).unwrap());

        // Check that a Runtime can be detected in PATH.
        let target = temp_env::with_var("PATH", docker_first_path.as_ref(), || {
            Runtime::detect_from_path()
        });
        assert_eq!(target, Some(docker_target.clone()));

        let target = temp_env::with_var("PATH", podman_first_path.as_ref(), || {
            Runtime::detect_from_path()
        });
        assert_eq!(target, Some(podman_target.clone()));

        let target = temp_env::with_var("PATH", combined_path.as_ref(), || {
            Runtime::detect_from_path()
        });
        assert_eq!(target, Some(docker_target.clone()));

        let target = temp_env::with_var("PATH", neither_path.as_ref(), || {
            Runtime::detect_from_path()
        });
        assert_eq!(target, None);

        // Apple Container is detected when it is the only runtime available.
        let target = temp_env::with_var("PATH", apple_container_only_path.as_ref(), || {
            Runtime::detect_from_path()
        });
        assert_eq!(target, Some(apple_container_target.clone()));

        // Docker takes precedence over Apple Container when both are present.
        let target = temp_env::with_var("PATH", all_runtimes_path.as_ref(), || {
            Runtime::detect_from_path()
        });
        assert_eq!(target, Some(docker_target.clone()));

        // Podman takes precedence over Apple Container when docker is absent.
        let target = temp_env::with_var("PATH", podman_and_container_path.as_ref(), || {
            Runtime::detect_from_path()
        });
        assert_eq!(target, Some(podman_target.clone()));

        // Check that a specified Runtime is in PATH.
        assert!(temp_env::with_var("PATH", docker_first_path, || {
            docker_target.validate_in_path().is_ok()
        }));
        assert!(temp_env::with_var("PATH", podman_first_path, || {
            docker_target.validate_in_path().is_ok()
        }));
        assert!(temp_env::with_var("PATH", combined_path, || {
            docker_target.validate_in_path().is_ok()
        }));
        assert!(temp_env::with_var("PATH", neither_path, || {
            docker_target.validate_in_path().is_err()
        }));
        assert!(temp_env::with_var(
            "PATH",
            apple_container_only_path,
            || { apple_container_target.validate_in_path().is_ok() }
        ));
        assert!(temp_env::with_var("PATH", all_runtimes_path, || {
            apple_container_target.validate_in_path().is_ok()
        }));
    }
}
