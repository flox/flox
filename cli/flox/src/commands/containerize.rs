use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use log::debug;
use tracing::instrument;

use super::{environment_select, EnvironmentSelect};
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::message;

fn is_accepted_registry(reg: &Option<String>) -> bool {
    return match reg.as_deref() {
        Some("docker") | Some("podman") => true,
        Some(_) => false,
        None => true, // Since it's optional, it's fine to not have it
    };
}

// Containerize an environment
#[derive(Bpaf, Clone, Debug)]
pub struct Containerize {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Path to write the container to (pass '-' to write to stdout)
    #[bpaf(short, long, argument("path"))]
    output: Option<PathBuf>,

    /// Tag to apply to the container, defaults to 'latest'
    #[bpaf(short, long, argument("tag"))]
    tag: Option<String>,

    /// Which container registry to load the container into
    #[bpaf(
        short,
        long("load-into-registry"),
        argument("registry"),
        guard(is_accepted_registry, "one of docker or podman is required")
    )]
    load_into_registry: Option<String>,
}
impl Containerize {
    #[instrument(name = "containerize", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("containerize");

        let mut env = self
            .environment
            .detect_concrete_environment(&flox, "Upgrade")?
            .into_dyn_environment();

        let output_path = match self.output {
            Some(output) => output,
            None => std::env::current_dir()
                .context("Could not get current directory")?
                .join(format!("{}-container.tar", env.name())),
        };

        let output_tag: &str = match self.tag {
            Some(tag) => &tag.to_string(),
            None => "latest",
        };

        let (output, output_name): (Box<dyn Write + Send>, String) =
            if let Some(registry) = self.load_into_registry {
                let command = Command::new(&registry)
                    .arg("load")
                    .stdin(Stdio::piped())
                    .spawn()
                    .context("Could not start registry load command")?;

                (Box::new(command.stdin.unwrap()), registry.to_string())
            } else if output_path == Path::new("-") {
                debug!("output=stdout");

                (Box::new(std::io::stdout()), "stdout".to_string())
            } else {
                debug!("output={}", output_path.display());

                let file = fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&output_path)
                    .context("Could not open output file")?;

                (Box::new(file), output_path.display().to_string())
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
