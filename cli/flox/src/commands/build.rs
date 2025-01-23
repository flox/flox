use std::env;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment};
use flox_rust_sdk::models::lockfile::Lockfile;
use flox_rust_sdk::providers::build::{build_symlink_path, FloxBuildMk, ManifestBuilder, Output};
use indoc::{formatdoc, indoc};
use tracing::instrument;

use super::{environment_select, EnvironmentSelect};
use crate::commands::activate::FLOX_INTERPRETER;
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
    pub async fn handle(self, flox: Flox) -> Result<()> {
        if !flox.features.build {
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
        let flox_env = env.rendered_env_links(&flox)?;

        let packages_to_clean = available_packages(&env.lockfile(&flox)?, packages)?;

        let builder = FloxBuildMk;
        builder.clean(&flox, &base_dir, &flox_env.development, &packages_to_clean)?;

        message::created("Clean completed successfully");

        Ok(())
    }

    #[instrument(name = "build", skip_all, fields(packages))]
    async fn build(flox: Flox, mut env: ConcreteEnvironment, packages: Vec<String>) -> Result<()> {
        subcommand_metric!("build");

        if let ConcreteEnvironment::Remote(_) = &env {
            bail!("Cannot build from a remote environment");
        };

        let base_dir = env.parent_path()?;
        let built_environments = env.build(&flox)?;

        let packages_to_build = available_packages(&env.lockfile(&flox)?, packages)?;

        let builder = FloxBuildMk;
        let output = builder.build(
            &flox,
            &base_dir,
            &built_environments,
            &FLOX_INTERPRETER,
            &packages_to_build,
        )?;

        for message in output {
            match message {
                Output::Stdout(line) => println!("{line}"),
                Output::Stderr(line) => eprintln!("{line}"),
                Output::Success { .. } => {
                    let current_dir = env::current_dir()
                        .context("could not get current directory")?
                        .canonicalize()
                        .context("could not canonicalize current directory")?;
                    let links_to_print = packages_to_build
                        .iter()
                        .map(|package| Self::check_and_display_symlink(&env, package, &current_dir))
                        .collect::<Result<Vec<_>, _>>()?;
                    if packages_to_build.len() > 1 {
                        message::created(formatdoc!(
                            "Builds completed successfully.
                            Outputs created: {}",
                            links_to_print.join(", ")
                        ));
                    } else {
                        message::created(format!(
                            "Build completed successfully. Output created: {}",
                            links_to_print[0]
                        ));
                    }
                    break;
                },
                Output::Failure(status) => {
                    bail!("Build failed with status: {status}");
                },
            }
        }

        Ok(())
    }

    /// Check if the expected symlink for a package exists.
    /// If so, shorten it if in the current directory.
    ///
    /// current_dir should be canonicalized
    fn check_and_display_symlink(
        environment: &impl Environment,
        package: &str,
        current_dir: impl AsRef<Path>,
    ) -> Result<String> {
        let symlink = build_symlink_path(environment, package)?;

        if !symlink.exists() {
            bail!("Build symlink for package '{}' does not exist", package);
        }

        let parent = symlink
            .parent()
            .ok_or(anyhow!("symlink must be in a directory"))?;

        let parent = parent
            .canonicalize()
            .context("couldn't canonicalize parent of build symlink")?;

        if parent == current_dir.as_ref() {
            Ok(format!(
                "./{}",
                symlink
                    .file_name()
                    .ok_or(anyhow!("symlink must have a file name"))?
                    .to_string_lossy()
            ))
        } else {
            Ok(symlink.to_string_lossy().to_string())
        }
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

#[cfg(test)]
mod test {
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment_in;
    use flox_rust_sdk::providers::nix::test_helpers::known_store_path;
    use tempfile::tempdir_in;

    use super::*;

    #[test]
    /// Test that check_and_display_symlink shortens the symlink when in the
    /// current directory,
    fn symlink_gets_shortened_when_in_current_dir() {
        let (flox, _temp_dir) = flox_instance();
        let dot_flox_parent_path = tempdir_in(&flox.temp_dir)
            .unwrap()
            .into_path()
            .canonicalize()
            .unwrap();
        let environment = new_path_environment_in(&flox, "version 1", &dot_flox_parent_path);
        let package = "foo";
        let symlink = dot_flox_parent_path.join(format!("result-{package}"));
        // We just want some random symlink possibly into the /nix/store
        std::os::unix::fs::symlink(known_store_path(), &symlink).unwrap();
        let displayed = Build::check_and_display_symlink(
            &environment,
            package,
            dot_flox_parent_path.canonicalize().unwrap(),
        )
        .unwrap();
        assert_eq!(displayed, format!("./result-{package}"));

        let displayed =
            Build::check_and_display_symlink(&environment, package, &flox.temp_dir).unwrap();
        assert_eq!(displayed, symlink.to_string_lossy());
    }
}
