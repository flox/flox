use std::io::{stdout, Write};

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::data::CanonicalPath;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::models::lockfile::{
    InstalledPackage,
    LockedManifest,
    LockedPackageFlake,
    PackageInfo,
    PackageToList,
    TypedLockedManifestPkgdb,
};
use flox_rust_sdk::models::manifest::DEFAULT_PRIORITY;
use flox_rust_sdk::providers::flox_cpp_utils::LockedInstallable;
use indoc::formatdoc;
use itertools::Itertools;
use log::debug;
use tracing::instrument;

use super::{environment_select, EnvironmentSelect};
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::message;

// List packages installed in an environment
#[derive(Bpaf, Clone)]
pub struct List {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(external(list_mode), fallback(ListMode::Extended))]
    list_mode: ListMode,
}

#[derive(Bpaf, Clone, PartialEq, Debug)]
pub enum ListMode {
    /// Show the raw contents of the manifest
    #[bpaf(long, short)]
    Config,

    /// Show only the name of each package
    #[bpaf(long("name"), short)]
    NameOnly,

    /// Show the name, pkg-path, and version of each package (default)
    #[bpaf(long, short)]
    Extended,

    /// Show all available package information including priority and license
    #[bpaf(long, short)]
    All,
}

impl List {
    #[instrument(name = "list", fields(mode), skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("list");

        let mut env = self
            .environment
            .detect_concrete_environment(&flox, "List using")?
            .into_dyn_environment();

        let manifest_contents = env.manifest_content(&flox)?;
        if self.list_mode == ListMode::Config {
            tracing::Span::current().record("mode", "config");
            println!("{}", manifest_contents);
            return Ok(());
        }

        let system = &flox.system;
        let lockfile = Self::get_lockfile(&flox, &mut *env)?;
        let packages = match lockfile {
            LockedManifest::Pkgdb(pkgdb_lockfile) => {
                TypedLockedManifestPkgdb::try_from(pkgdb_lockfile)?.list_packages(system)
            },
            LockedManifest::Catalog(catalog_lockfile) => catalog_lockfile.list_packages(system),
        };

        if packages.is_empty() {
            let message = formatdoc! {"
                No packages are installed for your current system ('{system}').

                You can see the whole manifest with 'flox list --config'.
            "};
            message::warning(message);
            return Ok(());
        }

        match self.list_mode {
            ListMode::NameOnly => {
                tracing::Span::current().record("mode", "name");
                Self::print_name_only(stdout().lock(), &packages)?;
            },
            ListMode::Extended => {
                tracing::Span::current().record("mode", "extended");
                Self::print_extended(stdout().lock(), &packages)?;
            },
            ListMode::All => {
                tracing::Span::current().record("mode", "all");
                Self::print_detail(stdout().lock(), &packages)?;
            },
            ListMode::Config => unreachable!(),
        }

        Ok(())
    }

    /// print package ids only
    fn print_name_only(mut out: impl Write, packages: &[PackageToList]) -> Result<()> {
        for p in packages {
            let install_id = match p {
                PackageToList::CatalogOrPkgdb(p) => &p.install_id,
                PackageToList::Flake(p) => &p.install_id,
            };
            writeln!(&mut out, "{install_id}")?;
        }
        Ok(())
    }

    /// print package ids, as well as path and version
    ///
    /// e.g. `pip: python3Packages.pip (20.3.4)`
    ///
    /// This is the default mode
    fn print_extended(mut out: impl Write, packages: &[PackageToList]) -> Result<()> {
        for p in packages {
            match p {
                PackageToList::CatalogOrPkgdb(p) => {
                    writeln!(
                        &mut out,
                        "{id}: {path} ({version})",
                        id = p.install_id,
                        path = p.rel_path,
                        version = p.info.version.as_deref().unwrap_or("N/A")
                    )?;
                },
                PackageToList::Flake(p) => {
                    writeln!(
                        &mut out,
                        "{id}: {locked_url}#{flake_attr_path}",
                        id = p.install_id,
                        locked_url = p.locked_installable.locked_url,
                        flake_attr_path = p.locked_installable.locked_flake_attr_path
                    )?;
                },
            }
        }
        Ok(())
    }

    /// print package ids, as well as extended detailed information
    fn print_detail(mut out: impl Write, packages: &[PackageToList]) -> Result<()> {
        for (idx, package) in packages
            .iter()
            .sorted_by_key(|p| match p {
                PackageToList::CatalogOrPkgdb(p) => p.priority.unwrap_or(DEFAULT_PRIORITY),
                PackageToList::Flake(_) => DEFAULT_PRIORITY,
            })
            .enumerate()
        {
            let message = match package {
                PackageToList::CatalogOrPkgdb(package) => {
                    let InstalledPackage {
                        install_id: name,
                        rel_path,
                        info:
                            PackageInfo {
                                pname,
                                version,
                                description,
                                license,
                                unfree,
                                broken,
                            },
                        priority,
                    } = package;

                    formatdoc! {"
                    {name}: ({pname})
                      Description: {description}
                      Path:     {rel_path}
                      Priority: {priority}
                      Version:  {version}
                      License:  {license}
                      Unfree:   {unfree}
                      Broken:   {broken}
                    ",
                        description = description.as_deref().unwrap_or("N/A"),
                        priority = priority.map(|p| p.to_string()).as_deref().unwrap_or("N/A"),
                        version = version.as_deref().unwrap_or("N/A"),
                        license = license.as_deref().unwrap_or("N/A"),
                        unfree = unfree.map(|u|u.to_string()).as_deref().unwrap_or("N/A"),
                        broken = broken.map(|b|b.to_string()).as_deref().unwrap_or("N/A"),
                    }
                },
                PackageToList::Flake(package) => {
                    let LockedPackageFlake {
                        install_id,
                        locked_installable:
                            LockedInstallable {
                                locked_url,
                                locked_flake_attr_path,
                                pname,
                                version,
                                description,
                                licenses,
                                broken,
                                unfree,
                                ..
                            },
                    } = package;

                    let formatted_licenses = licenses.as_ref().map(|licenses| {
                        if licenses.len() == 1 {
                            format!("License:         {}", licenses[0])
                        } else {
                            format!("Licenses:        {}", licenses.join(", "))
                        }
                    });

                    // Add parenthesis and a space to pname if it's not None
                    let formatted_pname = pname.as_ref().map(|pname| format!(" ({})", pname));

                    formatdoc! {"
                    {install_id}:{formatted_pname}
                      Locked URL:      {locked_url}
                      Flake attribute: {locked_flake_attr_path}
                      Description:     {description}
                      Priority:        {priority}
                      Version:         {version}
                      {formatted_licenses}
                      Unfree:          {unfree}
                      Broken:          {broken}
                    ",
                        formatted_pname = formatted_pname.as_deref().unwrap_or(""),
                        description = description.as_deref().unwrap_or("N/A"),
                        priority = DEFAULT_PRIORITY,
                        version = version.as_deref().unwrap_or("N/A"),
                        formatted_licenses = formatted_licenses.as_deref().unwrap_or("License: N/A"),
                        unfree = unfree.map(|u|u.to_string()).as_deref().unwrap_or("N/A"),
                        broken = broken.map(|b|b.to_string()).as_deref().unwrap_or("N/A"),
                    }
                },
            };
            // add an empty line between packages
            if idx < packages.len() - 1 {
                writeln!(&mut out, "{message}")?;
            } else {
                write!(&mut out, "{message}")?;
            }
        }
        Ok(())
    }

    /// Read existing lockfile or resolve to create a new [LockedManifest].
    ///
    /// Does not write the lockfile,
    /// as that would require writing to the environment in case of remote environments)
    fn get_lockfile(flox: &Flox, env: &mut dyn Environment) -> Result<LockedManifest> {
        let lockfile_path = env
            .lockfile_path(flox)
            .context("Could not get lockfile path")?;

        let lockfile = if !lockfile_path.exists() {
            debug!("No lockfile found, locking environment...");
            Dialog {
                message: "No lockfile found for environment, building...",
                help_message: None,
                typed: Spinner::new(|| env.lock(flox)),
            }
            .spin()?
        } else {
            debug!("Using existing lockfile");
            // we have already checked that the lockfile exists
            let path = CanonicalPath::new(lockfile_path).unwrap();
            LockedManifest::read_from_file(&path)?
        };

        Ok(lockfile)
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::models::lockfile::test_helpers::LOCKED_NIX_EVAL_JOBS;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;

    fn test_packages() -> [PackageToList; 2] {
        [
            InstalledPackage {
                install_id: "pip-iid".to_string(),
                rel_path: "python3Packages.pip".to_string(),
                info: PackageInfo {
                    pname: "pip".to_string(),
                    version: Some("20.3.4".to_string()),
                    description: Some("Python package installer".to_string()),
                    license: Some("MIT".to_string()),
                    unfree: Some(true),
                    broken: Some(false),
                },
                priority: Some(100),
            }
            .into(),
            InstalledPackage {
                install_id: "python".to_string(),
                rel_path: "python3Packages.python".to_string(),
                info: PackageInfo {
                    pname: "python".to_string(),
                    version: Some("3.9.5".to_string()),
                    description: Some("Python interpreter".to_string()),
                    license: Some("PSF".to_string()),
                    unfree: Some(false),
                    broken: Some(false),
                },
                priority: Some(200),
            }
            .into(),
        ]
    }

    fn uninformative_package() -> PackageToList {
        InstalledPackage {
            install_id: "pip-iid".to_string(),
            rel_path: "python3Packages.pip".to_string(),
            info: PackageInfo {
                pname: "pip".to_string(),
                version: None,
                description: None,
                license: None,
                unfree: None,
                broken: None,
            },
            priority: None,
        }
        .into()
    }

    #[test]
    fn test_name_only_output() {
        let mut out = Vec::new();
        List::print_name_only(&mut out, &test_packages()).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            pip-iid
            python
        "});
    }

    /// Test name only output for flake installables
    #[test]
    fn test_name_only_flake_output() {
        let mut out = Vec::new();
        List::print_name_only(&mut out, &[PackageToList::Flake(
            LOCKED_NIX_EVAL_JOBS.clone(),
        )])
        .unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            nix-eval-jobs
        "});
    }

    #[test]
    fn test_print_extended_output() {
        let mut out = Vec::new();
        List::print_extended(&mut out, &test_packages()).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            pip-iid: python3Packages.pip (20.3.4)
            python: python3Packages.python (3.9.5)
        "});
    }

    /// Test extended output for flake installables
    #[test]
    fn test_print_extended_flake_output() {
        let mut out = Vec::new();
        List::print_extended(&mut out, &[PackageToList::Flake(
            LOCKED_NIX_EVAL_JOBS.clone(),
        )])
        .unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            nix-eval-jobs: github:nix-community/nix-eval-jobs/c132534bc68eb48479a59a3116ee7ce0f16ce12b#packages.aarch64-darwin.default
        "});
    }

    /// If a package is missing some values, they should be replaced with "N/A"
    #[test]
    fn test_print_extended_output_handles_missing_values() {
        let mut out = Vec::new();
        List::print_extended(&mut out, &[uninformative_package()]).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            pip-iid: python3Packages.pip (N/A)
        "});
    }

    #[test]
    fn test_print_detail_output() {
        let mut out = Vec::new();
        List::print_detail(&mut out, &test_packages()).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            pip-iid: (pip)
              Description: Python package installer
              Path:     python3Packages.pip
              Priority: 100
              Version:  20.3.4
              License:  MIT
              Unfree:   true
              Broken:   false

            python: (python)
              Description: Python interpreter
              Path:     python3Packages.python
              Priority: 200
              Version:  3.9.5
              License:  PSF
              Unfree:   false
              Broken:   false
        "})
    }

    /// Test detailed output for flake installables
    #[test]
    fn test_print_detail_flake_output() {
        let mut out = Vec::new();
        List::print_detail(&mut out, &[PackageToList::Flake(
            LOCKED_NIX_EVAL_JOBS.clone(),
        )])
        .unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            nix-eval-jobs: (nix-eval-jobs)
              Locked URL:      github:nix-community/nix-eval-jobs/c132534bc68eb48479a59a3116ee7ce0f16ce12b
              Flake attribute: packages.aarch64-darwin.default
              Description:     Hydra's builtin hydra-eval-jobs as a standalone
              Priority:        5
              Version:         2.23.0
              License:         GPL-3.0
              Unfree:          false
              Broken:          false
        "});
    }

    /// Test detailed output for flake installables when pname is missing
    #[test]
    fn test_print_detail_flake_output_pname_missing() {
        let mut out = Vec::new();
        let mut package = LOCKED_NIX_EVAL_JOBS.clone();
        package.locked_installable.pname = None;
        List::print_detail(&mut out, &[PackageToList::Flake(package)]).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            nix-eval-jobs:
              Locked URL:      github:nix-community/nix-eval-jobs/c132534bc68eb48479a59a3116ee7ce0f16ce12b
              Flake attribute: packages.aarch64-darwin.default
              Description:     Hydra's builtin hydra-eval-jobs as a standalone
              Priority:        5
              Version:         2.23.0
              License:         GPL-3.0
              Unfree:          false
              Broken:          false
        "});
    }

    /// Test detailed output for flake installables with multiple licenses
    #[test]
    fn test_print_detail_flake_output_multiple_licenses() {
        let mut out = Vec::new();
        let mut package = LOCKED_NIX_EVAL_JOBS.clone();
        if let Some(licenses) = package.locked_installable.licenses.as_mut() {
            licenses.push("license 2".to_string());
        }
        List::print_detail(&mut out, &[PackageToList::Flake(package)]).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            nix-eval-jobs: (nix-eval-jobs)
              Locked URL:      github:nix-community/nix-eval-jobs/c132534bc68eb48479a59a3116ee7ce0f16ce12b
              Flake attribute: packages.aarch64-darwin.default
              Description:     Hydra's builtin hydra-eval-jobs as a standalone
              Priority:        5
              Version:         2.23.0
              Licenses:        GPL-3.0, license 2
              Unfree:          false
              Broken:          false
        "});
    }

    #[test]
    fn test_print_detail_output_orders_by_priority_unknown_first() {
        let mut packages = test_packages();
        let PackageToList::CatalogOrPkgdb(ref mut package_2) = packages[1] else {
            panic!();
        };
        package_2.priority = None;

        let mut out = Vec::new();
        List::print_detail(&mut out, &packages).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            python: (python)
              Description: Python interpreter
              Path:     python3Packages.python
              Priority: N/A
              Version:  3.9.5
              License:  PSF
              Unfree:   false
              Broken:   false

            pip-iid: (pip)
              Description: Python package installer
              Path:     python3Packages.pip
              Priority: 100
              Version:  20.3.4
              License:  MIT
              Unfree:   true
              Broken:   false
        "})
    }

    #[test]
    fn test_print_detail_output_orders_by_priority() {
        let mut packages = test_packages();
        let PackageToList::CatalogOrPkgdb(ref mut package_2) = packages[1] else {
            panic!();
        };
        package_2.priority = Some(10);

        let mut out = Vec::new();
        List::print_detail(&mut out, &packages).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            python: (python)
              Description: Python interpreter
              Path:     python3Packages.python
              Priority: 10
              Version:  3.9.5
              License:  PSF
              Unfree:   false
              Broken:   false

            pip-iid: (pip)
              Description: Python package installer
              Path:     python3Packages.pip
              Priority: 100
              Version:  20.3.4
              License:  MIT
              Unfree:   true
              Broken:   false
        "})
    }

    /// If a package is missing some values, they should be replaced with "N/A"
    #[test]
    fn test_print_detail_output_handles_missing_values() {
        let mut out = Vec::new();
        List::print_detail(&mut out, &[uninformative_package()]).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            pip-iid: (pip)
              Description: N/A
              Path:     python3Packages.pip
              Priority: N/A
              Version:  N/A
              License:  N/A
              Unfree:   N/A
              Broken:   N/A
        "})
    }
}
