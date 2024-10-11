use std::fmt::Display;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::str::FromStr;

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use log::debug;
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

    /// Tag to apply to the container, defaults to 'latest'
    #[bpaf(short, long, argument("tag"))]
    tag: Option<String>,

    #[bpaf(
        external(output_file_or_registry),
        fallback(OutputFileOrRegistry::OutputFile(None))
    )]
    output: OutputFileOrRegistry,
}

#[derive(Debug, Clone, Bpaf)]
enum OutputFileOrRegistry {
    OutputFile(
        #[bpaf(
            short('o'),
            long("output"),
            argument("path"),
            help("Path to write the container to (pass '-' to write to stdout)")
        )]
        Option<PathBuf>,
    ),
    Registry(
        #[bpaf(
            short('l'),
            long("load-into-registry"),
            argument("registry"),
            help("Which container registry to load the container into")
        )]
        Registry,
    ),
}

/// The container registry to load the container into
/// Currently only supports Docker and Podman
#[derive(Debug, Clone)]
enum Registry {
    Docker,
    Podman,
}

impl Registry {
    /// Get a writer to the registry,
    /// Essentially spawns a `docker load` or `podman load` process
    /// and returns a handle to its stdin.
    fn to_writer(&self) -> Result<impl Write> {
        let cmd = match self {
            Registry::Docker => "docker",
            Registry::Podman => "podman",
        };

        let mut child = Command::new(cmd)
            .arg("load")
            .stdin(Stdio::piped())
            .spawn()?;

        Ok(child.stdin.take().expect("stdin is piped"))
    }
}

impl Display for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Registry::Docker => write!(f, "Docker registry"),
            Registry::Podman => write!(f, "Podman registry"),
        }
    }
}

impl FromStr for Registry {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "docker" => Ok(Registry::Docker),
            "podman" => Ok(Registry::Podman),
            _ => Err("Registry must be 'docker' or 'podman'".to_string()),
        }
    }
}

impl Containerize {
    #[instrument(name = "containerize", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("containerize");

        let mut env = self
            .environment
            .detect_concrete_environment(&flox, "Upgrade")?
            .into_dyn_environment();

        let output_tag: &str = match self.tag {
            Some(tag) => &tag.to_string(),
            None => "latest",
        };

        let (output, output_name): (Box<dyn Write + Send>, String) = match self.output {
            // If the output file is '-', we write to stdout
            OutputFileOrRegistry::OutputFile(Some(file)) if file == Path::new("-") => {
                debug!("output=stdout");

                (Box::new(std::io::stdout()), "stdout".to_string())
            },
            // If the output file is None, we write to a file in the current directory
            // with the name of the environment.
            // Otherwise, we write to the specified file.
            OutputFileOrRegistry::OutputFile(output_path) => {
                let output_path = match output_path {
                    Some(output) => output,
                    None => std::env::current_dir()
                        .context("Could not get current directory")?
                        .join(format!("{}-container.tar", env.name())),
                };
                let file = fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&output_path)
                    .context("Could not open output file")?;

                (Box::new(file), output_path.display().to_string())
            },
            // If the registry is specified, we write to the registry directly instead of a file
            OutputFileOrRegistry::Registry(registry) => {
                let writer = registry.to_writer()?;
                (Box::new(writer), registry.to_string())
            },
        };

        let builder = Dialog {
            message: &format!("Building container for environment {}...", env.name()),
            help_message: None,
            typed: Spinner::new(|| env.build_container(&flox, output_tag)),
        }
        .spin()?;

        Dialog {
            message: &format!("Writing container to '{output_name}'"),
            help_message: None,
            typed: Spinner::new(|| builder.stream_container(output)),
        }
        .spin()?;

        message::created(format!("Container written to '{output_name}'"));
        Ok(())
    }
}
