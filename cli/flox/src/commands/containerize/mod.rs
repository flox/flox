use std::convert::Infallible;
use std::fmt::Display;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::{fs, io};

use anyhow::{Context, Result, anyhow, bail};
use bpaf::Bpaf;
use flox_core::activate::context::ActivateMode;
use flox_events::{CliEnvironmentPayload, EventKind, EventsHub};
use flox_manifest::interfaces::AsLatestSchema;
use flox_manifest::lockfile::Lockfile;
use flox_manifest::parsed::common::ContainerizeConfig;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::providers::container_builder::{ContainerBuilder, MkContainerNix};
use flox_rust_sdk::utils::{ReaderExt, WireTap};
use indoc::indoc;
use macos_containerize_proxy::{ContainerizeProxy, convert_to_oci_archive};
use tracing::{debug, info, instrument};

use super::{EnvironmentSelect, environment_select};
use crate::commands::SHELL_COMPLETION_FILE;
use crate::environment_subcommand_metric;
use crate::utils::events::env_detail_from_concrete;
use crate::utils::message;
use crate::utils::openers::first_in_path;
use crate::utils::platform::macos_major_version;

mod macos_containerize_proxy;

// Containerize an environment
#[derive(Bpaf, Clone, Debug)]
pub struct Containerize {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Container runtime to
    /// store the image (when '--file' is not specified)
    /// or build the image (when on macOS).
    /// One of 'docker', 'podman', or 'container' (Apple's runtime, macOS only).
    /// Defaults to detecting the first available on PATH,
    /// except on macOS 26 and later, where 'container' is preferred.
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
        if let Err(err) = EventsHub::global().record_event(EventKind::CliEnvironmentContainerize(
            CliEnvironmentPayload::new(env_detail_from_concrete(&flox, &env)),
        )) {
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

                    Exporting a container on macOS requires Docker, Podman, or Apple's 'container' to be installed.
                "#});
            };
            let builder = ContainerizeProxy::new(env_path, proxy_runtime, self.labels, self.mode);
            builder.create_container_source(&flox, env_name.as_ref(), output_tag)?
        };

        let mut writer = output.to_writer(&flox.cache_dir, format!("{env_name}:{output_tag}"))?;
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

    fn to_writer(
        &self,
        cache_dir: &Path,
        reference: String,
    ) -> Result<Box<dyn ContainerSink + '_>> {
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
            OutputTarget::Runtime(runtime) => runtime.to_writer(cache_dir, reference)?,
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

const DOCKER_ARCHIVE_NAME: &str = "image.tar";
const OCI_ARCHIVE_NAME: &str = "image-oci.tar";

/// A sink for Apple's `container`, which neither streams nor speaks
/// docker-archive.
///
/// `container image load` reads `--input <file>` rather than stdin, so the
/// image is spooled to disk, and it accepts only an OCI layout, so the
/// docker-archive is converted before it is loaded. Both archives live in a
/// working directory under the Flox cache rather than `$TMPDIR`, because they
/// routinely run to gigabytes and the conversion needs to bind-mount the
/// directory into a container.
#[derive(Debug)]
struct AppleContainerSink {
    work_dir: PathBuf,
    image: fs::File,
    reference: String,
    runtime: Runtime,
}

impl AppleContainerSink {
    fn new(runtime: Runtime, cache_dir: &Path, reference: String) -> Result<Self> {
        let work_dir = cache_dir.join("containerize-image");
        fs::create_dir_all(&work_dir)
            .context("Could not create the container image working directory")?;

        let image = fs::File::create(work_dir.join(DOCKER_ARCHIVE_NAME))
            .context("Could not create the container image file")?;

        Ok(Self {
            work_dir,
            image,
            reference,
            runtime,
        })
    }
}

impl Write for AppleContainerSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.image.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.image.flush()
    }
}

impl ContainerSink for AppleContainerSink {
    fn wait(&mut self) -> Result<()> {
        self.flush()?;
        self.image.sync_all()?;

        convert_to_oci_archive(
            &self.runtime,
            &self.work_dir,
            DOCKER_ARCHIVE_NAME,
            OCI_ARCHIVE_NAME,
            &self.reference,
        )?;

        let output = self
            .runtime
            .to_command()
            .args(["image", "load", "--input"])
            .arg(self.work_dir.join(OCI_ARCHIVE_NAME))
            .output()
            .context(format!("Failed to call runtime {}", self.runtime.to_cmd()))?;

        String::from_utf8_lossy(&output.stdout)
            .lines()
            .for_each(|line| info!("{line}"));

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(anyhow!("Writing to runtime was unsuccessful").context(stderr));
        }

        Ok(())
    }
}

/// The macOS release in which Apple's `container` gained the networking and
/// runtime support that make it usable as a default. It runs on earlier
/// releases with limitations, so there it stays a fallback behind Docker and
/// Podman rather than the preferred choice.
const APPLE_CONTAINER_PREFERRED_MACOS_MAJOR: u32 = 26;

/// The container registry to load the container into
/// Supports Docker, Podman, and Apple's `container` (macOS only).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Runtime {
    Docker,
    Podman,
    AppleContainer,
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

        let prefer_apple_container = macos_major_version()
            .is_some_and(|major| major >= APPLE_CONTAINER_PREFERRED_MACOS_MAJOR);

        Self::detect_in_paths(
            prefer_apple_container,
            std::env::split_paths(&path_var).collect(),
        )
    }

    /// Detection split from environment lookup so both sides of the
    /// version-dependent preference are testable on any host.
    ///
    /// The Docker/Podman lookup is deliberately left as a single
    /// [first_in_path] call: that function iterates PATH entries on the
    /// outside, so which of the two wins is decided by PATH order, and
    /// splitting it into per-runtime lookups would silently change that.
    /// Apple's runtime therefore gets its own stage on one side or the other
    /// rather than joining the candidate list.
    fn detect_in_paths(prefer_apple_container: bool, paths: Vec<PathBuf>) -> Option<Self> {
        let apple_container = cfg!(target_os = "macos")
            .then(|| {
                first_in_path([Runtime::AppleContainer.to_cmd()], paths.iter().cloned())
                    .map(|_| Runtime::AppleContainer)
            })
            .flatten();

        let detected = if prefer_apple_container && apple_container.is_some() {
            apple_container
        } else {
            first_in_path(["docker", "podman"], paths)
                .map(|(_, runtime)| {
                    Runtime::from_str(runtime).expect("Should search for valid runtime names only")
                })
                .or(apple_container)
        };

        match &detected {
            Some(runtime) => debug!(runtime = runtime.to_cmd(), "Detected container runtime"),
            None => debug!("No container runtime found in PATH"),
        }

        detected
    }

    /// Get the unqualified command name for the runtime.
    fn to_cmd(&self) -> &str {
        match self {
            Runtime::Docker => "docker",
            Runtime::Podman => "podman",
            Runtime::AppleContainer => "container",
        }
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

    /// Get a writer that loads the image into the runtime.
    ///
    /// Docker and Podman read the archive from `load`'s stdin, so the image
    /// streams straight through. Apple's runtime needs it on disk and in a
    /// different archive format; see [AppleContainerSink].
    fn to_writer(&self, cache_dir: &Path, reference: String) -> Result<Box<dyn ContainerSink>> {
        if self == &Runtime::AppleContainer {
            return Ok(Box::new(AppleContainerSink::new(
                self.clone(),
                cache_dir,
                reference,
            )?));
        }

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
    }

    fn to_command(&self) -> Command {
        Command::new(self.to_cmd())
    }
}

impl Display for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Runtime::Docker => write!(f, "Docker runtime"),
            Runtime::Podman => write!(f, "Podman runtime"),
            Runtime::AppleContainer => write!(f, "Apple container runtime"),
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
        assert_eq!("docker".parse::<Runtime>().unwrap(), Runtime::Docker);
        assert_eq!("podman".parse::<Runtime>().unwrap(), Runtime::Podman);
        assert_eq!(
            "container".parse::<Runtime>().unwrap(),
            Runtime::AppleContainer
        );
        assert!("invalid".parse::<Runtime>().is_err());
    }

    /// Apple's runtime is preferred on macOS 26 and later, where it is fully
    /// supported, and is a last resort everywhere else. Both branches are
    /// exercised here rather than only the one matching the test host.
    #[test]
    #[cfg(target_os = "macos")]
    fn apple_container_preference_depends_on_macos_version() {
        let tempdir = tempfile::tempdir().unwrap();

        let container_bin = tempdir.path().join("container-bin");
        let podman_bin = tempdir.path().join("podman-bin");
        fs::create_dir(&container_bin).unwrap();
        fs::create_dir(&podman_bin).unwrap();
        fs::write(container_bin.join("container"), "").unwrap();
        fs::write(podman_bin.join("podman"), "").unwrap();

        // Podman deliberately sits *earlier* in PATH: preference must come from
        // the version check, not from PATH order.
        let paths = vec![podman_bin, container_bin];

        assert_eq!(
            Runtime::detect_in_paths(true, paths.clone()),
            Some(Runtime::AppleContainer),
            "macOS 26+ should prefer Apple's runtime over an earlier Podman"
        );
        assert_eq!(
            Runtime::detect_in_paths(false, paths),
            Some(Runtime::Podman),
            "older macOS should fall back to Apple's runtime only when nothing else is present"
        );
    }

    /// Apple's runtime is macOS-only, and adding it must not disturb which of
    /// Docker and Podman wins — that is decided by PATH order, in both
    /// preference modes.
    #[test]
    fn docker_podman_path_order_is_unchanged_by_apple_container() {
        let tempdir = tempfile::tempdir().unwrap();

        let docker_bin = tempdir.path().join("docker-bin");
        let podman_bin = tempdir.path().join("podman-bin");
        fs::create_dir(&docker_bin).unwrap();
        fs::create_dir(&podman_bin).unwrap();
        fs::write(docker_bin.join("docker"), "").unwrap();
        fs::write(podman_bin.join("podman"), "").unwrap();

        for prefer_apple_container in [true, false] {
            assert_eq!(
                Runtime::detect_in_paths(prefer_apple_container, vec![
                    podman_bin.clone(),
                    docker_bin.clone()
                ]),
                Some(Runtime::Podman)
            );
            assert_eq!(
                Runtime::detect_in_paths(prefer_apple_container, vec![
                    docker_bin.clone(),
                    podman_bin.clone()
                ]),
                Some(Runtime::Docker)
            );
        }
    }

    #[test]
    fn detect_runtime_in_path() {
        let tempdir = tempfile::tempdir().unwrap();

        let docker_target = Runtime::Docker;
        let podman_target = Runtime::Podman;

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

        let docker_first_path =
            Some(std::env::join_paths([&docker_bin, &podman_bin, &combined_bin]).unwrap());
        let podman_first_path =
            Some(std::env::join_paths([&podman_bin, &docker_bin, &combined_bin]).unwrap());
        let combined_path =
            Some(std::env::join_paths([&combined_bin, &podman_bin, &docker_bin]).unwrap());
        let neither_path = Some(std::env::join_paths([neither_bin]).unwrap());

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
    }
}
