use anyhow::{bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::lockfile::Lockfile;
use flox_rust_sdk::providers::build::{FloxBuildMk, ManifestBuilder, Output};
use indoc::indoc;
use tracing::instrument;

use super::{environment_select, EnvironmentSelect};
use crate::commands::activate::FLOX_INTERPRETER;
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

    #[bpaf(external(subcommand_or_build_targets))]
    subcommand_or_targets: SubcommandOrBuildTargets,
}

#[derive(Debug, Bpaf, Clone)]
enum SubcommandOrBuildTargets {
    /// Clean the build directory
    ///
    /// Remove builds artifacts and temporary files.
    #[bpaf(command, footer("Run 'man flox-build-clean' for more details."))]
    Clean {
        /// The package(s) to clean.
        /// Corresponds to entries in the 'build' table in the environment's manifest.toml.
        /// If not specified, all packages are cleaned up.
        #[bpaf(positional("package"))]
        targets: Vec<String>,
    },
    BuildTargets {
        /// The package to build.
        /// Corresponds to entries in the 'build' table in the environment's manifest.toml.
        /// If not specified, all packages are built.
        #[bpaf(positional("package"))]
        targets: Vec<String>,
    },
}

impl Build {
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        if !config.features.unwrap_or_default().build {
            message::plain("🚧 👷 heja, a new command is in construction here, stay tuned!");
            bail!("'build' feature is not enabled.");
        }

        match self.subcommand_or_targets {
            SubcommandOrBuildTargets::Clean { targets } => {
                let env = self
                    .environment
                    .detect_concrete_environment(&flox, "Build packages of")?;

                Self::clean(flox, env, targets).await
            },
            SubcommandOrBuildTargets::BuildTargets { targets } => {
                let env = self
                    .environment
                    .detect_concrete_environment(&flox, "Clean build files of")?;

                Self::build(flox, env, targets).await
            },
        }
    }

    #[instrument(name = "build::clean", skip_all)]
    async fn clean(flox: Flox, env: ConcreteEnvironment, packages: Vec<String>) -> Result<()> {
        subcommand_metric!("build::clean");

        if let ConcreteEnvironment::Remote(_) = &env {
            bail!("Cannot build from a remote environment");
        };

        let mut env = env.into_dyn_environment();

        let base_dir = env.parent_path()?;
        let flox_env = env.rendered_env_path(&flox)?;

        let packages_to_clean = available_packages(&env.lockfile(&flox)?, packages)?;

        let builder = FloxBuildMk;
        builder.clean(&base_dir, &flox_env, &packages_to_clean)?;

        message::created("Clean completed successfully");

        Ok(())
    }

    #[instrument(name = "build", skip_all, fields(packages))]
    async fn build(flox: Flox, env: ConcreteEnvironment, packages: Vec<String>) -> Result<()> {
        subcommand_metric!("build");

        if let ConcreteEnvironment::Remote(_) = &env {
            bail!("Cannot build from a remote environment");
        };

        let mut env = env.into_dyn_environment();

        let base_dir = env.parent_path()?;
        let flox_env = env.rendered_env_path(&flox)?;

        let packages_to_build = available_packages(&env.lockfile(&flox)?, packages)?;

        let builder = FloxBuildMk;
        let output = builder.build(
            &flox,
            &base_dir,
            &flox_env,
            &FLOX_INTERPRETER,
            &packages_to_build,
        )?;

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

fn available_packages(lockfile: &Lockfile, packages: Vec<String>) -> Result<Vec<String>> {
    let environment_packages = &lockfile.manifest.build;

    if environment_packages.is_empty() {
        bail!(indoc! {"
        No builds found.

        Add a build by modifying the '[build]' section of the manifest with 'flox edit'
        "});
    }

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
