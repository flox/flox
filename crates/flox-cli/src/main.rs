use anyhow::{anyhow, Result};
use clap::Parser;
use flox_rust_sdk::environment::{build_flox_env, FLOX_SH};
use flox_rust_sdk::providers::initializers;
use std::env;
use std::process::{exit, ExitStatus};
use tokio::process::Command;

mod build;
mod config;
mod utils;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub(crate) struct FloxArgs {
    #[clap(subcommand, help = "Initialize a flox project")]
    init: InitializeAction,
}

#[derive(clap::Subcommand, Debug)]
pub(crate) enum InitializeAction {
    Init {
        #[clap(value_parser, help = "The package name you are trying to initialize")]
        package_name: String,
        #[clap(value_parser, help = "The builder you would like to use.")]
        builder: String,
    },
}

pub async fn run_in_flox(args: &[String]) -> ExitStatus {
    Command::new(FLOX_SH)
        .args(args)
        .envs(&build_flox_env().unwrap())
        .spawn()
        .expect("failed to spawn flox")
        .wait()
        .await
        .unwrap()
}

#[tokio::main]
async fn main() -> Result<()> {
    // TODO pick out the commands we want to implement in Rust
    let raw_args: Vec<String> = env::args().collect();

    exit(run_in_flox(&raw_args[1..]).await.code().unwrap());

    let args = FloxArgs::parse();
    println!("{:?}", args);

    match args.init {
        InitializeAction::Init {
            package_name,
            builder,
        } => {
            initializers::get_provider()
                .await?
                .init(&package_name, &builder.into())
                .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_flox_help() {
        // TODO check the output
        assert_eq!(
            run_in_flox(&["--help".to_string()]).await.code().unwrap(),
            0,
        )
    }
}
