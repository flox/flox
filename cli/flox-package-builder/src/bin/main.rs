use std::path::PathBuf;

use anyhow::Result;
use bpaf::{Bpaf, Parser};
use flox_rust_sdk::models::lockfile::Lockfile;
use flox_rust_sdk::providers::buildenv::BuildEnvOutputs;
use serde::de::DeserializeOwned;

#[derive(Debug, Bpaf, Clone)]
struct Args {
    packages: Vec<String>,
    lockfile: PathBuf,
    #[bpaf(argument::<String>("json"), parse(json_from_str::<BuildEnvOutputs>))]
    built_lockfile: BuildEnvOutputs,
    cache_dir: PathBuf,
    results_file: PathBuf,
    clean: bool,
}

fn main() -> Result<()> {
    let parsed = args().to_options().run();

    let lockfile_content = std::fs::read_to_string(&parsed.lockfile).unwrap();
    let lockfile: Lockfile = serde_json::from_str(&lockfile_content).unwrap();

    if !parsed.clean {
        let results = flox_package_builder::build_all(
            parsed.packages,
            &lockfile,
            &parsed.built_lockfile,
            &parsed.cache_dir,
        )?;

        let results_json = serde_json::to_string(&results).unwrap();
        std::fs::write(&parsed.results_file, results_json)?;
    } else {
        flox_package_builder::clean_all(parsed.packages, &lockfile, &parsed.cache_dir)?;
    }

    Ok(())
}

fn json_from_str<T: DeserializeOwned>(s: String) -> Result<T, impl ToString> {
    serde_json::from_str(&s)
}
