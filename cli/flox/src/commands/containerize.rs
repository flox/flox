use std::fmt::Display;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

use super::{environment_select, EnvironmentSelect};
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::message;

// Containerize an environment
#[derive(Bpaf, Clone, Debug)]
pub struct Containerize {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Output to write the container to, defaults to '{environment}-container.tar'
    #[bpaf(external(output_target), optional)]
    output: Option<OutputTarget>,

    /// Tag to apply to the container, defaults to 'latest'
    #[bpaf(short, long, argument("tag"))]
    tag: Option<String>,
}
impl Containerize {
    #[instrument(name = "containerize", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("containerize");

        let mut env = self
            .environment
            .detect_concrete_environment(&flox, "Containerize")?
            .into_dyn_environment();

        let output = self
            .output
            .unwrap_or_else(|| OutputTarget::detect_or_default(env.name().as_ref()));

        let output_tag: &str = match self.tag {
            Some(tag) => &tag.to_string(),
            None => "latest",
        };

        let builder = Dialog {
            message: &format!("Building container for environment {}...", env.name()),
            help_message: None,
            typed: Spinner::new(|| env.build_container(&flox, output_tag)),
        }
        .spin()?;

        Dialog {
            message: &format!("Writing container to {output}...",),
            help_message: None,
            typed: Spinner::new(|| {
                let mut writer = output.to_writer()?;
                builder.stream_container(&mut writer)?;
                writer.wait()?;
                anyhow::Ok(())
            }),
        }
        .spin()?;

        message::created(format!("Container written to {output}"));
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
        PathBuf,
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

impl OutputTarget {
    fn detect_or_default(env_name: impl AsRef<str>) -> Self {
        OutputTarget::File(PathBuf::from(format!(
            "{}-container.tar",
            env_name.as_ref()
        )))
    }

    fn to_writer(&self) -> Result<Box<dyn ContainerSink>> {
        let writer: Box<dyn ContainerSink> = match self {
            OutputTarget::File(path) => {
                let path = match path {
                    path if path == Path::new("-") => Path::new("/dev/stdout"),
                    path => path,
                };

                let file = fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(path)
                    .context("Could not open output file")?;

                Box::new(file)
            },
            OutputTarget::Runtime(runtime) => Box::new(runtime.to_writer()?),
        };

        Ok(writer)
    }
}

impl Display for OutputTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputTarget::File(path) => write!(f, "file '{}'", path.display()),
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

#[derive(Debug)]
struct RuntimeSink {
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
        if !status.success() {
            return Err(anyhow!("Writing to runtime was unsuccessful"));
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
    /// Get a writer to the registry,
    /// Essentially spawns a `docker load` or `podman load` process
    /// and returns a handle to its stdin.
    fn to_writer(&self) -> Result<impl ContainerSink> {
        let cmd = match self {
            Runtime::Docker => "docker",
            Runtime::Podman => "podman",
        };

        let child = Command::new(cmd)
            .arg("load")
            .stdin(Stdio::piped())
            .spawn()
            .context(format!("Failed to call runtime {cmd}"))?;

        Ok(RuntimeSink { child })
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
}
