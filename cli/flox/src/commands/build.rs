use std::path::Path;

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment};
use flox_rust_sdk::models::lockfile::Lockfile;
use flox_rust_sdk::models::manifest::typed::Manifest;
use flox_rust_sdk::providers::build::{
    FloxBuildMk,
    ManifestBuilder,
    Output,
    PackageTarget,
    PackageTargets,
    find_toplevel_group_nixpkgs,
    nix_expression_dir,
};
use flox_rust_sdk::providers::catalog::mock_base_catalog_url;
use indoc::formatdoc;
use tracing::instrument;

use super::{EnvironmentSelect, environment_select};
use crate::commands::activate::FLOX_INTERPRETER;
use crate::environment_subcommand_metric;
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
                environment_subcommand_metric!("build::clean", self.environment);
                let env = self
                    .environment
                    .detect_concrete_environment(&flox, "Build packages of")?;

                Self::clean(flox, env, targets).await
            },
            SubcommandOrBuildTargets::BuildTargets { targets } => {
                environment_subcommand_metric!("build", self.environment);
                let env = self
                    .environment
                    .detect_concrete_environment(&flox, "Clean build files of")?;

                Self::build(flox, env, targets).await
            },
        }
    }

    #[instrument(name = "build::clean", skip_all)]
    async fn clean(flox: Flox, mut env: ConcreteEnvironment, packages: Vec<String>) -> Result<()> {
        if let ConcreteEnvironment::Remote(_) = &env {
            bail!("Cannot build from a remote environment");
        };
        let base_dir = env.parent_path()?;
        let expression_dir = nix_expression_dir(&env); // TODO: decouple from env
        let flox_env_build_outputs = env.build(&flox)?;
        let lockfile: Lockfile = env.lockfile(&flox)?.into();

        let packages_to_clean = packages_to_build(&lockfile.manifest, &expression_dir, &packages)?;
        let target_names = packages_to_clean
            .iter()
            .map(|target| target.name())
            .collect::<Vec<_>>();

        let builder = FloxBuildMk::new(&flox, &base_dir, &expression_dir, &flox_env_build_outputs);
        builder.clean(&target_names)?;

        message::created("Clean completed successfully");

        Ok(())
    }

    #[instrument(name = "build", skip_all, fields(packages))]
    async fn build(flox: Flox, mut env: ConcreteEnvironment, packages: Vec<String>) -> Result<()> {
        if let ConcreteEnvironment::Remote(_) = &env {
            bail!("Cannot build from a remote environment");
        };

        let base_dir = env.parent_path()?;
        let built_environments = env.build(&flox)?;
        let expression_dir = nix_expression_dir(&env); // TODO: decouple from env

        let lockfile: Lockfile = env.lockfile(&flox)?.into();

        let packages_to_build = packages_to_build(&lockfile.manifest, &expression_dir, &packages)?;
        let target_names = packages_to_build
            .iter()
            .map(|target| target.name())
            .collect::<Vec<_>>();

        let builder = FloxBuildMk::new(&flox, &base_dir, &expression_dir, &built_environments);
        let output = builder.build(
            &mock_base_catalog_url().as_flake_ref()?,
            find_toplevel_group_nixpkgs(&lockfile)
                .map(|catalog_ref| catalog_ref.as_flake_ref())
                .transpose()?
                .as_ref(),
            &FLOX_INTERPRETER,
            &target_names,
            None,
        )?;

        for message in output {
            match message {
                Output::Stdout(line) => println!("{line}"),
                Output::Stderr(line) => eprintln!("{line}"),
                Output::Success(_) => {
                    message::created(formatdoc!(
                        "Build completed successfully."
                    ));
                    break;
                },
                Output::Failure(status) => {
                    bail!("Build failed with status: {status}");
                },
            }
        }

        Ok(())
    }
}

pub(crate) fn packages_to_build<'o>(
    manifest: &'o Manifest,
    expression_dir: &'o Path,
    packages: &[impl AsRef<str>],
) -> Result<Vec<PackageTarget>> {
    let available_targets = PackageTargets::new(manifest, expression_dir)?;

    if available_targets.is_empty() {
        bail!(formatdoc! {"
            No packages found to build.

            Add a build by modifying the '[build]' section of the manifest with 'flox edit'
            or add expression files in '{expression_dir}'.
            ", expression_dir = expression_dir.display()
        });
    }

    let selected = if !packages.is_empty() {
        available_targets.select(packages)?
    } else {
        available_targets.all()
    };

    Ok(selected)
}

#[cfg(test)]
mod test {
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::{
        new_path_environment,
        new_path_environment_in,
    };
    use flox_rust_sdk::providers::build::test_helpers::prepare_nix_expressions_in;
    use flox_rust_sdk::providers::nix::test_helpers::known_store_path;
    use tempfile::tempdir_in;

    use super::*;

    /// Test that check_and_display_symlink shortens the symlink when in the
    /// current directory,
    #[test]
    fn symlink_gets_shortened_when_in_current_dir() {
        let (flox, _temp_dir) = flox_instance();
        let dot_flox_parent_path = tempdir_in(&flox.temp_dir)
            .unwrap()
            .keep()
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

    /// Test that conflicting build names are detected if builds are defined via the manifest and nix expressions.
    #[test]
    fn conflicting_build_names() {
        let pname = "conflict".to_string();

        let (flox, tempdir) = flox_instance();

        // Create a manifest (may be empty)
        let manifest = formatdoc! {r#"
            version = 1

            [build]
            conflict.command = ""
        "#};

        let mut env = new_path_environment(&flox, &manifest);

        // Create expressions
        let expressions_dir =
            prepare_nix_expressions_in(&tempdir, &[(&[&pname], &formatdoc! {r#"
                {{runCommand}}: runCommand "{pname}" {{}} ""
            "#})]);

        let lockfile: Lockfile = env.lockfile(&flox).unwrap().into();
        let result = packages_to_build(&lockfile.manifest, &expressions_dir, &Vec::<String>::new());
        assert!(result.is_err());
    }
}
