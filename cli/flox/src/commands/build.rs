use anyhow::{bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::lockfile::LockedManifest;
use flox_rust_sdk::providers::build::{FloxBuildMk, ManifestBuilder, Output};
use tracing::instrument;

use super::{environment_select, EnvironmentSelect};
use crate::commands::ConcreteEnvironment;
use crate::config::Config;
use crate::subcommand_metric;
use crate::utils::message;

#[allow(unused)] // remove when we implement the command
#[derive(Bpaf, Clone)]
pub struct Build {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Whether to print logs to stderr during build.
    /// Logs are always written to <TBD>
    #[bpaf(short('L'), long)]
    build_logs: bool,

    /// The package to build, corresponds to the entries in
    /// the 'build' table in the environment's manifest.toml.
    /// If not specified, all packages are built.
    #[bpaf(positional("build"))]
    packages: Vec<String>,
}

impl Build {
    #[instrument(name = "build", skip_all)]
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("build");

        if !config.features.unwrap_or_default().build {
            message::plain("🚧 👷 heja, a new command is in construction here, stay tuned!");
            bail!("'build' feature is not enabled.");
        }

        let env = self
            .environment
            .detect_concrete_environment(&flox, "Build")?;

        if let ConcreteEnvironment::Remote(_) = &env {
            bail!("Cannot build from a remote environment");
        };

        let mut env = env.into_dyn_environment();

        let base_dir = env.parent_path()?;
        let flox_env = env.activation_path(&flox)?;

        let packages_to_build = make_packages_to_build(&env.lockfile(&flox)?, self.packages)?;

        let builder = FloxBuildMk;
        let output = builder.build(&base_dir, &flox_env, &packages_to_build)?;

        for message in output {
            match message {
                Output::Stdout(line) => println!("{line}"),
                Output::Stderr(line) => eprintln!("{line}"),
                Output::Exit(status) if status.success() => {
                    message::created("Build completed successfully");
                    break;
                },
                Output::Exit(status) => {
                    bail!("Build failed with status: {status}");
                },
            }
        }

        Ok(())
    }
}

fn make_packages_to_build(lockfile: &LockedManifest, packages: Vec<String>) -> Result<Vec<String>> {
    let LockedManifest::Catalog(lockfile) = lockfile else {
        bail!("Build requires a v1 lockfile");
    };

    let environment_packages = &lockfile.manifest.build;

    let packages_to_build = if packages.is_empty() {
        environment_packages.keys().cloned().collect()
    } else {
        packages
    };

    for package in &packages_to_build {
        if !environment_packages.contains_key(package) {
            bail!("Package '{}' not found in environment", package);
        }
    }

    Ok(packages_to_build)
}
