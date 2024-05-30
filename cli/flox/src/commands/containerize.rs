use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

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

    /// Path to write the container to (pass '-' to write to stdout)
    #[bpaf(short, long, argument("path"))]
    output: Option<PathBuf>,
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
                .join(format!("{}-container.tar.gz", env.name())),
        };

        let (output, output_name): (Box<dyn Write + Send>, String) =
            if output_path == Path::new("-") {
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
            typed: Spinner::new(|| env.build_container(&flox)),
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
