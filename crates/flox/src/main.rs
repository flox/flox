#[macro_use]
extern crate anyhow;

use anyhow::Result;
use flox_rust_sdk::environment::build_flox_env;
use log::{debug, info};
use std::env;
use std::fmt::Debug;
use std::process::ExitStatus;

use tokio::process::Command;

mod build;
mod commands;
mod config;
mod utils;
pub static FLOX_SH: &str = env!("FLOX_SH");
static FLOX_VERSION: &str = env!("FLOX_VERSION");

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let args = commands::flox_args().run();
    args.handle(config::Config::parse()?).await?;

    Ok(())
}

pub async fn flox_forward() -> Result<()> {
    info!("`FLOX_PREVIEW_ENABLE` unset or not \"true\", falling back to legacy flox");
    run_in_flox(&env::args_os().collect::<Vec<_>>()[1..]).await?;
    Ok(())
}

pub async fn run_in_flox(args: &[impl AsRef<std::ffi::OsStr> + Debug]) -> Result<ExitStatus> {
    debug!("Running in flox with arguments: {:?}", args);
    let status = Command::new(FLOX_SH)
        .args(args)
        .envs(&build_flox_env())
        .spawn()
        .expect("failed to spawn flox")
        .wait()
        .await?;

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_flox_help() {
        // TODO check the output
        assert_eq!(run_in_flox(&["--help"]).await.unwrap().code().unwrap(), 0,)
    }
}
