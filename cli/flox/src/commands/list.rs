use std::io::{stdout, Write};

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::data::CanonicalPath;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::models::lockfile::{
    InstalledPackage,
    LockedManifest,
    PackageInfo,
    TypedLockedManifestPkgdb,
};
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
    fn print_name_only(mut out: impl Write, packages: &[InstalledPackage]) -> Result<()> {
        for p in packages {
            writeln!(&mut out, "{}", p.install_id)?;
        }
        Ok(())
    }

    /// print package ids, as well as path and version
    ///
    /// e.g. `pip: python3Packages.pip (20.3.4)`
    ///
    /// This is the default mode
    fn print_extended(mut out: impl Write, packages: &[InstalledPackage]) -> Result<()> {
        for p in packages {
            writeln!(
                &mut out,
                "{id}: {path} ({version})",
                id = p.install_id,
                path = p.rel_path,
                version = p.info.version.as_deref().unwrap_or("N/A")
            )?;
        }
        Ok(())
    }

    /// print package ids, as well as extended detailed information
    fn print_detail(mut out: impl Write, packages: &[InstalledPackage]) -> Result<()> {
        for (idx, package) in packages.iter().sorted_by_key(|p| p.priority).enumerate() {
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

            let message = formatdoc! {"
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
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;

    fn test_packages() -> [InstalledPackage; 2] {
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
            },
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
            },
        ]
    }

    fn uninformative_package() -> InstalledPackage {
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

    #[test]
    fn test_print_detail_output_orders_by_priority_unknown_first() {
        let mut packages = test_packages();
        packages[1].priority = None;

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
        packages[1].priority = Some(10);

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
