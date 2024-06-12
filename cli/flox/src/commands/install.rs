use std::collections::HashSet;
use std::str::FromStr;

use anyhow::{anyhow, bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::data::CanonicalPath;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{CoreEnvironmentError, Environment, EnvironmentError};
use flox_rust_sdk::models::lockfile::{
    LockedManifest,
    LockedManifestError,
    LockedManifestPkgdb,
    LockedPackageCatalog,
};
use flox_rust_sdk::models::manifest::PackageToInstall;
use flox_rust_sdk::models::pkgdb::error_codes;
use indoc::formatdoc;
use itertools::Itertools;
use log::debug;
use tracing::instrument;

use super::{environment_select, EnvironmentSelect};
use crate::commands::{
    ensure_floxhub_token,
    environment_description,
    ConcreteEnvironment,
    EnvironmentSelectError,
};
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::didyoumean::{DidYouMean, InstallSuggestion};
use crate::utils::errors::apply_doc_link_for_unsupported_packages;
use crate::utils::message;

// Install a package into an environment
#[derive(Bpaf, Clone)]
pub struct Install {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Option to specify a package ID
    #[bpaf(external(pkg_with_id_option), many)]
    id: Vec<PkgWithIdOption>,

    #[bpaf(positional("packages"))]
    packages: Vec<String>,
}

#[derive(Debug, Bpaf, Clone)]
#[bpaf(adjacent)]
#[allow(clippy::manual_non_exhaustive)]
pub struct PkgWithIdOption {
    /// Install a package and assign an explicit ID
    #[bpaf(long("id"), short('i'))]
    _option: (),

    /// ID of the package to install
    #[bpaf(positional("id"))]
    pub id: String,

    /// The pkg-path of the package to install as shown by 'flox search'
    #[bpaf(positional("package"))]
    pub path: String,
}

impl Install {
    #[instrument(name = "install", fields(packages), skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        subcommand_metric!("install");

        debug!(
            "installing packages [{}] to {:?}",
            self.packages.as_slice().join(", "),
            self.environment
        );
        let concrete_environment = match self
            .environment
            .detect_concrete_environment(&flox, "Install to")
        {
            Ok(concrete_environment) => concrete_environment,
            Err(EnvironmentSelectError::Environment(
                ref e @ EnvironmentError::DotFloxNotFound(ref dir),
            )) => {
                let parent = dir.parent().unwrap_or(dir).display();
                bail!(formatdoc! {"
                {e}

                Create an environment with 'flox init --dir {parent}'"
                })
            },
            Err(e @ EnvironmentSelectError::EnvNotFoundInCurrentDirectory) => {
                bail!(formatdoc! {"
                {e}

                Create an environment with 'flox init' or install to an environment found elsewhere with 'flox install {} --dir <PATH>'",
                self.packages.join(" ")})
            },
            Err(e) => Err(e)?,
        };
        let description = environment_description(&concrete_environment)?;

        // Ensure the user is logged in for the following remote operations
        if let ConcreteEnvironment::Remote(_) = concrete_environment {
            ensure_floxhub_token(&mut flox).await?;
        };

        let mut environment = concrete_environment.into_dyn_environment();
        let mut packages_to_install = self
            .packages
            .iter()
            .map(|p| PackageToInstall::from_str(p))
            .collect::<Result<Vec<_>, _>>()?;
        packages_to_install.extend(self.id.iter().map(|p| PackageToInstall {
            id: p.id.clone(),
            pkg_path: p.path.clone(),
            version: None,
            input: None,
        }));
        if packages_to_install.is_empty() {
            bail!("Must specify at least one package");
        }

        // We don't know the contents of the packages field when the span is created
        tracing::Span::current().record(
            "packages",
            Install::format_packages_for_tracing(&packages_to_install),
        );

        let installation = Dialog {
            message: &format!("Installing packages to environment {description}..."),
            help_message: None,
            typed: Spinner::new(|| environment.install(&packages_to_install, &flox)),
        }
        .spin()
        .map_err(|err| Self::handle_error(err, &flox, &*environment, &packages_to_install))?;

        let lockfile_path = environment.lockfile_path(&flox)?;
        let lockfile_path = CanonicalPath::new(lockfile_path)?;
        let lockfile_content = std::fs::read_to_string(&lockfile_path)?;

        // Check for warnings in the lockfile
        let lockfile: LockedManifest = serde_json::from_str(&lockfile_content)?;

        match lockfile {
            // TODO: move this behind the `installation.new_manifest.is_some()`
            // check below so we don't warn when we don't even install anything
            LockedManifest::Catalog(locked_manifest) => {
                for warning in
                    Self::generate_warnings(&locked_manifest.packages, &packages_to_install)
                {
                    message::warning(warning);
                }
            },
            LockedManifest::Pkgdb(_) => {
                // run `pkgdb manifest check`
                let warnings = LockedManifestPkgdb::check_lockfile(&lockfile_path)?;
                warnings
                    .iter()
                    .filter(|w| {
                        // Filter out warnings that are not related to the packages we just installed
                        packages_to_install.iter().any(|p| w.package == p.id)
                    })
                    .for_each(|w| message::warning(&w.message));
            },
        };

        if installation.new_manifest.is_some() {
            // Print which new packages were installed
            for pkg in packages_to_install.iter() {
                if let Some(false) = installation.already_installed.get(&pkg.id) {
                    message::package_installed(pkg, &description);
                } else {
                    message::warning(format!(
                        "Package with id '{}' already installed to environment {description}",
                        pkg.id
                    ));
                }
            }
        } else {
            for pkg in packages_to_install.iter() {
                message::warning(format!(
                    "Package with id '{}' already installed to environment {description}",
                    pkg.id
                ));
            }
        }
        Ok(())
    }

    fn format_packages_for_tracing(packages: &[PackageToInstall]) -> String {
        // TODO: settle on a real format for the contents of this string (JSON, etc)
        packages.iter().map(|p| p.pkg_path.clone()).join(",")
    }

    fn handle_error(
        err: EnvironmentError,
        flox: &Flox,
        environment: &dyn Environment,
        packages: &[PackageToInstall],
    ) -> anyhow::Error {
        debug!("install error: {:?}", err);

        subcommand_metric!(
            "install",
            "failed_packages" = packages.iter().map(|p| p.pkg_path.clone()).join(",")
        );

        match err {
            // Try to make suggestions when a package isn't found
            EnvironmentError::Core(CoreEnvironmentError::LockedManifest(
                LockedManifestError::LockManifest(
                    flox_rust_sdk::models::pkgdb::CallPkgDbError::PkgDbError(pkgdberr),
                ),
            )) if pkgdberr.exit_code == error_codes::RESOLUTION_FAILURE => 'error: {
                debug!("attempting to make install suggestion");
                let paths = packages.iter().map(|p| p.pkg_path.clone()).join(", ");

                if packages.len() > 1 {
                    break 'error anyhow!(formatdoc! {"
                        Could not install {paths}.
                        One or more of the packages you are trying to install does not exist.
                    "});
                }
                let path = packages[0].pkg_path.clone();

                let head = format!("Could not find package {path}.");

                let suggestion = DidYouMean::<InstallSuggestion>::new(flox, environment, &path);
                if !suggestion.has_suggestions() {
                    break 'error anyhow!("{head} Try 'flox search' with a broader search term.");
                }

                anyhow!(formatdoc! {"
                    {head}
                    {suggestion}
                "})
            },
            err => apply_doc_link_for_unsupported_packages(err).into(),
        }
    }

    /// Generate warnings to print to the user about unfree and broken packages.
    fn generate_warnings(
        locked_packages: &[LockedPackageCatalog],
        packages_to_install: &[PackageToInstall],
    ) -> Vec<String> {
        let mut warnings = vec![];

        // There could be multiple packages with the same install_id but different systems.
        // A package could be broken on one system but not another.
        // So just keep track of which install_ids we've warned for.
        // TODO: does the warning need to take system into account?
        let mut warned_unfree = HashSet::new();
        let mut warned_broken = HashSet::new();
        for locked_package in locked_packages.iter() {
            // If unfree && just installed && we haven't already warned for this install_id,
            // warn that this package is unfree
            if locked_package.unfree == Some(true)
                && packages_to_install
                    .iter()
                    .any(|p| locked_package.install_id == p.id)
                && !warned_unfree.contains(&locked_package.install_id)
            {
                warnings.push(format!("The package '{}' has an unfree license, please verify the licensing terms of use", locked_package.install_id));
                warned_unfree.insert(&locked_package.install_id);
            }

            // If broken && just installed && we haven't already warned for this install_id,
            // warn that this package is broken
            if locked_package.broken == Some(true)
                && packages_to_install
                    .iter()
                    .any(|p| locked_package.install_id == p.id)
                && !warned_broken.contains(&locked_package.install_id)
            {
                warnings.push(format!("The package '{}' is marked as broken, it may not behave as expected during runtime.", locked_package.install_id));
                warned_broken.insert(&locked_package.install_id);
            }
        }
        warnings
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::models::lockfile::test_helpers::fake_package;
    use flox_rust_sdk::models::lockfile::LockedPackageCatalog;
    use flox_rust_sdk::models::manifest::PackageToInstall;
    use flox_rust_sdk::providers::catalog::SystemEnum;

    use crate::commands::install::Install;

    /// [Install::generate_warnings] shouldn't warn for packages not in packages_to_install
    #[test]
    fn generate_warnings_empty() {
        let (_, _, mut foo_locked) = fake_package("foo", None);
        foo_locked.unfree = Some(true);
        let locked_packages = vec![];
        let packages_to_install = vec![];
        assert_eq!(
            Install::generate_warnings(&locked_packages, &packages_to_install),
            Vec::<String>::new()
        );
    }

    /// [Install::generate_warnings] should warn for an unfree package
    #[test]
    fn generate_warnings_unfree() {
        let (foo_iid, _, mut foo_locked) = fake_package("foo", None);
        foo_locked.unfree = Some(true);
        let locked_packages = vec![foo_locked];
        let packages_to_install = vec![PackageToInstall {
            id: foo_iid.clone(),
            pkg_path: "foo".to_string(),
            version: None,
            input: None,
        }];
        assert_eq!(
            Install::generate_warnings(&locked_packages, &packages_to_install),
            vec![format!(
                "The package '{}' has an unfree license, please verify the licensing terms of use",
                foo_iid
            )]
        );
    }

    /// [Install::generate_warnings] should only warn for an unfree package once
    /// even if it's installed on multiple systems
    #[test]
    fn generate_warnings_unfree_multi_system() {
        let (foo_iid, _, mut foo_locked) = fake_package("foo", None);
        foo_locked.unfree = Some(true);

        // TODO: fake_package shouldn't hardcode system?
        let foo_locked_second_system = LockedPackageCatalog {
            system: SystemEnum::Aarch64Linux.to_string(),
            ..foo_locked.clone()
        };

        let locked_packages = vec![foo_locked, foo_locked_second_system];
        let packages_to_install = vec![PackageToInstall {
            id: foo_iid.clone(),
            pkg_path: "foo".to_string(),
            version: None,
            input: None,
        }];
        assert_eq!(
            Install::generate_warnings(&locked_packages, &packages_to_install),
            vec![format!(
                "The package '{}' has an unfree license, please verify the licensing terms of use",
                foo_iid
            )]
        );
    }

    /// [Install::generate_warnings] should warn for a broken package
    #[test]
    fn generate_warnings_broken() {
        let (foo_iid, _, mut foo_locked) = fake_package("foo", None);
        foo_locked.broken = Some(true);
        let locked_packages = vec![foo_locked];
        let packages_to_install = vec![PackageToInstall {
            id: foo_iid.clone(),
            pkg_path: "foo".to_string(),
            version: None,
            input: None,
        }];
        assert_eq!(
            Install::generate_warnings(&locked_packages, &packages_to_install),
            vec![format!(
                "The package '{}' is marked as broken, it may not behave as expected during runtime.",
                foo_iid
            )]
        );
    }

    /// [Install::generate_warnings] should only warn for a broken package once
    /// even if it's installed on multiple systems
    #[test]
    fn generate_warnings_broken_multi_system() {
        let (foo_iid, _, mut foo_locked) = fake_package("foo", None);
        foo_locked.broken = Some(true);

        // TODO: fake_package shouldn't hardcode system?
        let foo_locked_second_system = LockedPackageCatalog {
            system: SystemEnum::Aarch64Linux.to_string(),
            ..foo_locked.clone()
        };

        let locked_packages = vec![foo_locked, foo_locked_second_system];
        let packages_to_install = vec![PackageToInstall {
            id: foo_iid.clone(),
            pkg_path: "foo".to_string(),
            version: None,
            input: None,
        }];
        assert_eq!(
            Install::generate_warnings(&locked_packages, &packages_to_install),
            vec![format!(
                "The package '{}' is marked as broken, it may not behave as expected during runtime.",
                foo_iid
            )]
        );
    }
}
