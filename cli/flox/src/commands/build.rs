use std::env;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
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
    PackageTargetKind,
    PackageTargets,
    find_toplevel_group_nixpkgs,
    nix_expression_dir,
};
use flox_rust_sdk::providers::catalog::{BaseCatalogInfo, BaseCatalogUrl, ClientTrait};
use futures::TryFutureExt;
use indoc::formatdoc;
use itertools::Itertools;
use tracing::{debug, info, instrument};
use url::Url;

use super::{DirEnvironmentSelect, dir_environment_select};
use crate::commands::activate::FLOX_INTERPRETER;
use crate::environment_subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Clone)]
pub struct Build {
    #[bpaf(external(dir_environment_select), fallback(Default::default()))]
    environment: DirEnvironmentSelect,

    #[bpaf(external(subcommand_or_build_targets))]
    subcommand_or_targets: SubcommandOrBuildTargets,
}

#[derive(Debug, Clone, Bpaf)]
enum BaseCatalogUrlSelect {
    NixpkgsUrl(#[bpaf(long("nixpkgs-url"), argument("url"), hide)] Url),
    Stability(
        #[bpaf(
            long("stability"),
            argument("stability"),
            help(
                "Perform a nix expression build using a base package set of the given stability\n\
                as tracked by the catalog server.\n\
                Can not be used with manifest base builds."
            )
        )]
        String,
    ),
}

#[derive(Debug, Bpaf, Clone)]
enum SubcommandOrBuildTargets {
    /// Clean the build directory
    ///
    /// Removes build artifacts and temporary files.
    #[bpaf(command, footer("Run 'man flox-build-clean' for more details."))]
    Clean {
        /// The package(s) to clean.
        /// Corresponds to entries in the 'build' table in the environment's manifest.toml.
        /// If not specified, all packages are cleaned up.
        #[bpaf(positional("package"))]
        targets: Vec<String>,
    },
    BuildTargets {
        #[bpaf(external(base_catalog_url_select), optional)]
        base_catalog_url_select: Option<BaseCatalogUrlSelect>,

        /// The package to build.
        /// Corresponds to entries in the 'build' table in the environment's manifest.toml.
        /// If not specified, all packages are built.
        #[bpaf(positional("package"))]
        targets: Vec<String>,
    },
}

impl Build {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        match self.subcommand_or_targets {
            SubcommandOrBuildTargets::Clean { targets } => {
                let env = self
                    .environment
                    .detect_concrete_environment(&flox, "Clean build files of")?;
                environment_subcommand_metric!("build::clean", env);

                Self::clean(flox, env, targets).await
            },
            SubcommandOrBuildTargets::BuildTargets {
                targets,
                base_catalog_url_select,
            } => {
                let env = self
                    .environment
                    .detect_concrete_environment(&flox, "Build packages of")?;
                environment_subcommand_metric!("build", env);

                Self::build(flox, env, targets, base_catalog_url_select).await
            },
        }
    }

    #[instrument(name = "build::clean", skip_all)]
    async fn clean(flox: Flox, mut env: ConcreteEnvironment, packages: Vec<String>) -> Result<()> {
        match &env {
            ConcreteEnvironment::Path(_) => (),
            ConcreteEnvironment::Managed(_) => {
                bail!("Cannot build from an environment on FloxHub.")
            },
            ConcreteEnvironment::Remote(_) => {
                unreachable!("Cannot build from a remote environment")
            },
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
    async fn build(
        flox: Flox,
        mut env: ConcreteEnvironment,
        packages: Vec<String>,
        nixpkgs_url_select: Option<BaseCatalogUrlSelect>,
    ) -> Result<()> {
        match &env {
            ConcreteEnvironment::Path(_) => (),
            ConcreteEnvironment::Managed(_) => {
                bail!("Cannot build from an environment on FloxHub.")
            },
            ConcreteEnvironment::Remote(_) => {
                unreachable!("Cannot build from a remote environment")
            },
        };

        let base_dir = env.parent_path()?;
        let built_environments = env.build(&flox)?;
        let expression_dir = nix_expression_dir(&env); // TODO: decouple from env

        let lockfile: Lockfile = env.lockfile(&flox)?.into();

        let packages_to_build = packages_to_build(&lockfile.manifest, &expression_dir, &packages)?;

        let base_catalog_info_fut = flox.catalog_client.get_base_catalog_info().map_err(|err| {
            anyhow!(err).context("could not get information about the base catalog")
        });

        check_stability_compatibility(&packages_to_build, nixpkgs_url_select.is_some())?;

        let toplevel_derived_url = find_toplevel_group_nixpkgs(&lockfile);

        let base_nixpkgs_url = match nixpkgs_url_select {
            Some(BaseCatalogUrlSelect::NixpkgsUrl(url)) => {
                debug!(%url, "using provided nixpkgs flake");
                url
            },
            Some(BaseCatalogUrlSelect::Stability(stability)) => {
                let url = base_catalog_url_for_stability_arg(
                    Some(&stability),
                    base_catalog_info_fut,
                    toplevel_derived_url.as_ref(),
                )
                .await?;
                url.as_flake_ref()?
            },
            None => {
                let url = base_catalog_url_for_stability_arg(
                    None,
                    base_catalog_info_fut,
                    toplevel_derived_url.as_ref(),
                )
                .await?;
                url.as_flake_ref()?
            },
        };

        let target_names = packages_to_build
            .iter()
            .map(|target| target.name())
            .collect::<Vec<_>>();

        let builder = FloxBuildMk::new(&flox, &base_dir, &expression_dir, &built_environments);
        let output = builder.build(&base_nixpkgs_url, &FLOX_INTERPRETER, &target_names, None)?;

        for message in output {
            match message {
                Output::Stdout(line) => println!("{line}"),
                Output::Stderr(line) => eprintln!("{line}"),
                Output::Success(results) => {
                    let current_dir = env::current_dir()
                        .context("could not get current directory")?
                        .canonicalize()
                        .context("could not canonicalize current directory")?;

                    let links_to_print = results
                        .iter()
                        .map(|package| {
                            Self::format_result_links(package.result_links.keys(), &current_dir)
                        })
                        .flatten_ok()
                        .collect::<Result<Vec<_>, _>>()?;

                    if links_to_print.len() > 1 {
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

    /// If so, shorten symlink for a package it if in the current directory.
    ///
    /// current_dir should be canonicalized
    fn format_result_links(
        package_result_links: impl IntoIterator<Item = impl AsRef<Path>>,
        current_dir: impl AsRef<Path>,
    ) -> Result<Vec<String>> {
        package_result_links
            .into_iter()
            .map(|result_link| {
                let result_link = result_link.as_ref();
                let parent = result_link
                    .parent()
                    .expect("symlink must be in a directory");

                let parent = parent
                    .canonicalize()
                    .context("couldn't canonicalize parent of build symlink")?;

                if parent == current_dir.as_ref() {
                    Ok(format!(
                        "./{}",
                        result_link
                            .file_name()
                            .expect("symlink must have a file name")
                            .to_string_lossy()
                    ))
                } else {
                    Ok(result_link.display().to_string())
                }
            })
            .collect::<Result<Vec<_>>>()
    }
}

/// Check that all packages are compatible with the selected Nixpkgs URL selection.
pub(crate) fn check_stability_compatibility<'p>(
    packages: impl IntoIterator<Item = &'p PackageTarget>,
    nixpkgs_overridden: bool,
) -> Result<()> {
    if !nixpkgs_overridden {
        return Ok(());
    }

    for package in packages {
        if package.kind() == PackageTargetKind::ExpressionBuild {
            continue;
        }
        bail!(formatdoc! {"
            The '--stability' option only applies to nix expression builds.
            '{name}' is a manifest build.
            Omit '--stability' to build with nixpkgs compatible with the environment,
            or pass exclusively nix expression builds.
            ", name = package.name()
        })
    }
    Ok(())
}

/// Derive the nixpkgs url to be used for builds.
/// If a stability is provided, try to retrieve a url for that stability from the catalog.
/// Else, if we can derive a stability from the toplevel group of the environment, use that.
/// Otherwise attrr
pub(crate) async fn base_catalog_url_for_stability_arg(
    stability: Option<&str>,
    base_catalog_info_fut: impl IntoFuture<Output = Result<BaseCatalogInfo>>,
    toplevel_derived_url: Option<&BaseCatalogUrl>,
) -> Result<BaseCatalogUrl> {
    let url = match (stability, toplevel_derived_url) {
        (Some(stability), _) => {
            let base_catalog_info = base_catalog_info_fut.await?;
            let make_error_message = || {
                let available_stabilities = base_catalog_info.available_stabilities().join(", ");
                formatdoc! {"
                    Stability '{stability}' does not exist (or has not yet been populated).
                    Available stabilities are: {available_stabilities}
                "}
            };

            let url = base_catalog_info
                .url_for_latest_page_with_stability(stability)
                .with_context(make_error_message)?;

            info!(%url, %stability, "using page from user provided stability");
            url
        },
        (None, Some(toplevel_derived_url)) => {
            info!(url=%toplevel_derived_url, "using nixpkgs derived from toplevel group");
            toplevel_derived_url.clone()
        },
        (None, None) => {
            let base_catalog_info = base_catalog_info_fut.await?;

            let make_error_message = || {
                let available_stabilities = base_catalog_info.available_stabilities().join(", ");
                formatdoc! {"
                    The default stability {} does not exist (or has not yet been populated).
                    Available stabilities are: {available_stabilities}
                ", BaseCatalogInfo::DEFAULT_STABILITY}
            };

            let url = base_catalog_info
                .url_for_latest_page_with_default_stability()
                .with_context(make_error_message)?;

            info!(%url, "using page from default stability");
            url
        },
    };
    Ok(url)
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
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment;
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
        let package = "foo";
        let symlink = dot_flox_parent_path.join(format!("result-{package}"));
        // We just want some random symlink possibly into the /nix/store
        std::os::unix::fs::symlink(known_store_path(), &symlink).unwrap();
        let displayed =
            Build::format_result_links([&symlink], dot_flox_parent_path.canonicalize().unwrap())
                .unwrap();
        assert_eq!(displayed, vec![format!("./result-{package}")]);

        let displayed = Build::format_result_links([&symlink], &flox.temp_dir).unwrap();
        assert_eq!(displayed, vec![symlink.to_string_lossy()]);
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

    #[test]
    fn manifest_builds_not_allowed_with_stabilities_present() {
        let mut packages = vec![PackageTarget::new_unchecked(
            "manifest",
            PackageTargetKind::ManifestBuild,
        )];

        let result = check_stability_compatibility(&packages, true);
        assert!(result.is_err());

        // the presence of expression builds doesnt change the result
        packages.push(PackageTarget::new_unchecked(
            "expression",
            PackageTargetKind::ExpressionBuild,
        ));

        let result = check_stability_compatibility(&packages, true);
        assert!(result.is_err());

        // if all targets are expression builds, the check succeeds
        let packages = packages.split_off(1);
        let result = check_stability_compatibility(&packages, true);
        assert!(result.is_ok());
    }

    #[test]
    fn manifest_builds_allowed_with_stabilities_absent() {
        let mut packages = vec![PackageTarget::new_unchecked(
            "manifest",
            PackageTargetKind::ManifestBuild,
        )];

        let result = check_stability_compatibility(&packages, false);
        assert!(result.is_ok());

        // the presence of expression builds doesnt change the result
        packages.push(PackageTarget::new_unchecked(
            "expression",
            PackageTargetKind::ExpressionBuild,
        ));

        let result = check_stability_compatibility(&packages, false);
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn prefer_explicit_stability_over_toplevel() {
        let mock_base_catalog_info = BaseCatalogInfo::new_mock();

        let actual_without_toplevel = base_catalog_url_for_stability_arg(
            Some("not-default"),
            async { Ok(mock_base_catalog_info.clone()) },
            None,
        )
        .await
        .unwrap();

        let actual_with_toplevel = base_catalog_url_for_stability_arg(
            Some("not-default"),
            async { Ok(mock_base_catalog_info.clone()) },
            Some(&BaseCatalogUrl::from("dont expect this")),
        )
        .await
        .unwrap();

        let expected_url = mock_base_catalog_info
            .url_for_latest_page_with_stability("not-default")
            .unwrap();

        assert_eq!(actual_without_toplevel, expected_url);
        assert_eq!(actual_with_toplevel, expected_url);
    }

    #[tokio::test]
    async fn prefer_toplevel_over_implicit_stability() {
        let expected_url = BaseCatalogUrl::from("expect this");

        let actual_with_toplevel = base_catalog_url_for_stability_arg(
            None,
            async { unreachable!("with a toplevel we don't query for stabilities") },
            Some(&expected_url),
        )
        .await
        .unwrap();

        assert_eq!(actual_with_toplevel, expected_url);
    }

    #[tokio::test]
    async fn prefer_implicit_stability_without_toplevel() {
        let mock_base_catalog_info = BaseCatalogInfo::new_mock();

        let actual_with_toplevel = base_catalog_url_for_stability_arg(
            None,
            async { Ok(mock_base_catalog_info.clone()) },
            None,
        )
        .await
        .unwrap();

        let expected_url = mock_base_catalog_info
            .url_for_latest_page_with_default_stability()
            .unwrap();
        assert_eq!(actual_with_toplevel, expected_url);
    }
}
