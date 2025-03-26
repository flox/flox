use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::Parser;
use generate::{Config, execute_jobs, generate_jobs};

pub mod generate;

type Error = anyhow::Error;

#[derive(Debug, Clone, Parser)]
#[command(about = "Generate mock test data from a config file")]
pub struct Cli {
    #[arg(value_name = "PATH")]
    #[arg(help = "The path to the config file")]
    pub spec: PathBuf,

    #[arg(short, long)]
    #[arg(help = "Regenerate all data and overwrite existing data")]
    pub force: bool,

    #[arg(short, long)]
    #[arg(
        help = "The path to the directory in which to store the output [default: $PWD/generated]"
    )]
    pub output: Option<PathBuf>,

    #[arg(short, long)]
    #[arg(help = "The path to the directory containing the input data [default: $PWD/input_data]")]
    pub input: Option<PathBuf>,

    #[arg(short, long)]
    #[arg(help = "Don't show a spinner")]
    pub quiet: bool,
}

fn main() -> Result<(), Error> {
    tracing_subscriber::fmt::init();
    let args = Cli::parse();
    if !args.spec.exists() {
        bail!("spec file does not exist")
    }
    let spec_contents = std::fs::read_to_string(&args.spec).context("failed to read spec file")?;
    let config: Config =
        toml::from_str(&spec_contents).context("couldn't deserialize spec file")?;
    let output_dir =
        generate::get_output_dir(&args).context("failed to determine output directory")?;
    let input_dir =
        generate::get_input_dir(&args).context("failed to determine input directory")?;
    generate::create_output_dir(&output_dir).context("failed to create output directory")?;
    let jobs = generate_jobs(&config, &output_dir, args.force)
        .context("failed to generate jobs from config")?;
    execute_jobs(jobs, &config.vars, &input_dir, args.quiet)
        .context("failed while executing jobs")?;
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn validate_cli() {
        use clap::CommandFactory;
        Cli::command().debug_assert()
    }
}
