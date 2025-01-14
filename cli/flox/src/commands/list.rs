use std::io::{stdout, Write};

use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{
    ConcreteEnvironment,
    Environment,
    SingleSystemUpgradeDiff,
};
use flox_rust_sdk::models::lockfile::{LockedPackageFlake, Lockfile, PackageToList};
use flox_rust_sdk::providers::flake_installable_locker::LockedInstallable;
use flox_rust_sdk::providers::upgrade_checks::UpgradeInformationGuard;
use indoc::formatdoc;
use itertools::Itertools;
use tracing::{debug, instrument};

use super::{environment_select, EnvironmentSelect};
use crate::subcommand_metric;
use crate::utils::message;
use crate::utils::tracing::sentry_set_tag;

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
    #[instrument(name = "list", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        sentry_set_tag("list_mode", format!("{:?}", &self.list_mode));
        subcommand_metric!("list");

        let mut env = self
            .environment
            .detect_concrete_environment(&flox, "List using")?;

        let manifest_contents = env.manifest_contents(&flox)?;
        if self.list_mode == ListMode::Config {
            println!("{manifest_contents}");
            return Ok(());
        }

        let system = &flox.system;
        let lockfile = Self::get_lockfile(&flox, &mut env)?;
        let packages = lockfile.list_packages(system)?;

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
                Self::print_name_only(stdout().lock(), &packages)?;
            },
            ListMode::Extended => {
                Self::print_extended(
                    stdout().lock(),
                    &packages,
                    List::get_upgrades_for_system(&flox, &mut env, system)?,
                )?;
            },
            ListMode::All => {
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
                PackageToList::Catalog(_, p) => &p.install_id,
                PackageToList::Flake(_, p) => &p.install_id,
                PackageToList::StorePath(p) => &p.install_id,
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
    fn print_extended(
        mut out: impl Write,
        packages: &[PackageToList],
        upgrades: Option<SingleSystemUpgradeDiff>,
    ) -> Result<()> {
        for p in packages {
            match p {
                PackageToList::Catalog(_, p) => {
                    let upgrade_available = if upgrades
                        .as_ref()
                        .is_some_and(|diff| diff.contains_key(&p.install_id))
                    {
                        " - upgrade available"
                    } else {
                        ""
                    };
                    writeln!(
                        &mut out,
                        "{id}: {path} ({version}{upgrade_available})",
                        id = p.install_id,
                        path = p.attr_path,
                        version = p.version,
                    )?;
                },
                PackageToList::Flake(descriptor, locked_package) => {
                    writeln!(
                        &mut out,
                        "{id}: {flake}",
                        id = locked_package.install_id,
                        flake = descriptor.flake
                    )?;
                },
                PackageToList::StorePath(locked_package_store_path) => {
                    writeln!(
                        &mut out,
                        "{id}: {store_path}",
                        id = locked_package_store_path.install_id,
                        store_path = locked_package_store_path.store_path
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
                PackageToList::Catalog(_, locked) => locked.priority,
                PackageToList::Flake(_, locked) => locked.locked_installable.priority,
                PackageToList::StorePath(locked) => locked.priority,
            })
            .enumerate()
        {
            let message = match package {
                PackageToList::Catalog(_, locked) => {
                    formatdoc! {"
                        {name}: ({pname})
                          Description: {description}
                          Path:     {attr_path}
                          Priority: {priority}
                          Version:  {version}
                          License:  {license}
                          Unfree:   {unfree}
                          Broken:   {broken}
                        ",
                        name = &locked.install_id,
                        pname = &locked.pname,
                        attr_path = &locked.attr_path,
                        priority = locked.priority,
                        version = &locked.version,
                        description = locked.description.as_deref().unwrap_or("N/A"),
                        license = locked.license.as_deref().unwrap_or("N/A"),
                        unfree = locked.unfree.map(|u| u.to_string()).as_deref().unwrap_or("N/A"),
                        broken = locked.broken.map(|b| b.to_string()).as_deref().unwrap_or("N/A"),
                    }
                },
                PackageToList::Flake(_, package) => {
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
                                priority,
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
                    let formatted_pname = pname.as_ref().map(|pname| format!(" ({pname})"));

                    formatdoc! {"
                    {install_id}:{formatted_pname}
                      Description:     {description}
                      Locked URL:      {locked_url}
                      Flake attribute: {locked_flake_attr_path}
                      Priority:        {priority}
                      Version:         {version}
                      {formatted_licenses}
                      Unfree:          {unfree}
                      Broken:          {broken}
                    ",
                        formatted_pname = formatted_pname.as_deref().unwrap_or(""),
                        description = description.as_deref().unwrap_or("N/A"),
                        version = version.as_deref().unwrap_or("N/A"),
                        formatted_licenses = formatted_licenses.as_deref().unwrap_or("License: N/A"),
                        unfree = unfree.map(|u|u.to_string()).as_deref().unwrap_or("N/A"),
                        broken = broken.map(|b|b.to_string()).as_deref().unwrap_or("N/A"),
                    }
                },
                PackageToList::StorePath(locked_package_store_path) => formatdoc! {"
                    {install_id}:
                    Store Path: {store_path}
                    Priority:   {priority}
                    ",
                    install_id = locked_package_store_path.install_id,
                    store_path = locked_package_store_path.store_path,
                    priority = locked_package_store_path.priority,
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

    /// Read existing lockfile or lock to create a new [LockedManifest].
    ///
    /// This may write the lockfile depending on the type of environment;
    /// path and managed environments with local checkouts will lock if there
    /// isn't a lockfile or it has different manifest contents than the
    /// manifest.
    ///
    /// Check the implementation docs of [Environment::lockfile] for more
    /// information.
    fn get_lockfile(flox: &Flox, env: &mut ConcreteEnvironment) -> Result<Lockfile> {
        // TODO: it would be better if we knew when a lock was actually happening
        let lockfile = env.lockfile(flox)?;

        Ok(lockfile)
    }

    fn get_upgrades_for_system(
        flox: &Flox,
        environment: &mut ConcreteEnvironment,
        system: &str,
    ) -> Result<Option<SingleSystemUpgradeDiff>> {
        let upgrade_guard = UpgradeInformationGuard::read_in(environment.cache_path()?)?;
        let Some(info) = upgrade_guard.info() else {
            debug!("Not displaying upgrade information; no upgrade information available");
            return Ok(None);
        };

        let current_lockfile = environment.lockfile(flox)?;

        if Some(current_lockfile) != info.result.old_lockfile {
            // todo: delete the info file?
            debug!("Not using upgrade information; lockfile has changed since last check");
            return Ok(None);
        }

        Ok(Some(info.result.diff_for_system(system)))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use flox_rust_sdk::models::lockfile::test_helpers::{
        fake_catalog_package_lock,
        nix_eval_jobs_descriptor,
        LOCKED_NIX_EVAL_JOBS,
    };
    use flox_rust_sdk::models::lockfile::LockedPackage;
    use flox_rust_sdk::models::manifest::DEFAULT_PRIORITY;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;

    fn test_packages() -> [PackageToList; 2] {
        let (_pip_iid, pip_descriptor, mut pip_lock) = fake_catalog_package_lock("pip", None);
        let (_python_iid, python_descriptor, mut python_lock) =
            fake_catalog_package_lock("python", None);

        // populate the locks
        // - pip
        pip_lock.attr_path = "python3Packages.pip".to_string();
        pip_lock.pname = "pip".to_string();
        pip_lock.priority = 100;
        pip_lock.version = "20.3.4".to_string();
        pip_lock.description = Some("Python package installer".to_string());
        pip_lock.license = Some("MIT".to_string());
        pip_lock.unfree = Some(true);
        pip_lock.broken = Some(false);

        // - python
        python_lock.priority = 200;
        python_lock.attr_path = "python3Packages.python".to_string();
        python_lock.version = "3.9.5".to_string();
        python_lock.description = Some("Python interpreter".to_string());
        python_lock.license = Some("PSF".to_string());
        python_lock.unfree = Some(false);
        python_lock.broken = Some(false);

        [
            PackageToList::Catalog(
                pip_descriptor.unwrap_catalog_descriptor().unwrap(),
                pip_lock,
            ),
            PackageToList::Catalog(
                python_descriptor.unwrap_catalog_descriptor().unwrap(),
                python_lock,
            ),
        ]
    }

    fn uninformative_package() -> PackageToList {
        let (_pip_iid, pip_descriptor, mut pip_lock) = fake_catalog_package_lock("pip", None);

        // populate the lock
        pip_lock.attr_path = "python3Packages.pip".to_string();
        pip_lock.pname = "pip".to_string();
        pip_lock.version = "N/A".to_string();

        PackageToList::Catalog(
            pip_descriptor.unwrap_catalog_descriptor().unwrap(),
            pip_lock,
        )
    }

    fn test_flake_package() -> PackageToList {
        PackageToList::Flake(nix_eval_jobs_descriptor(), LOCKED_NIX_EVAL_JOBS.clone())
    }

    #[test]
    fn test_name_only_output() {
        let mut out = Vec::new();
        List::print_name_only(&mut out, &test_packages()).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            pip_install_id
            python_install_id
        "});
    }

    /// Test name only output for flake installables
    #[test]
    fn test_name_only_flake_output() {
        let mut out = Vec::new();
        List::print_name_only(&mut out, &[test_flake_package()]).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            nix-eval-jobs
        "});
    }

    #[test]
    fn test_print_extended_output() {
        let mut out = Vec::new();
        List::print_extended(&mut out, &test_packages(), None).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            pip_install_id: python3Packages.pip (20.3.4)
            python_install_id: python3Packages.python (3.9.5)
        "});
    }

    /// Test extended output for flake installables
    #[test]
    fn test_print_extended_flake_output() {
        let mut out = Vec::new();
        List::print_extended(&mut out, &[test_flake_package()], None).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            nix-eval-jobs: github:nix-community/nix-eval-jobs
        "});
    }

    /// If a package is missing some values, they should be replaced with "N/A"
    #[test]
    fn test_print_extended_output_handles_missing_values() {
        let mut out = Vec::new();
        List::print_extended(&mut out, &[uninformative_package()], None).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            pip_install_id: python3Packages.pip (N/A)
        "});
    }

    /// If packages have upgrades available, the output should indicate that
    #[test]
    fn test_print_extended_includes_upgrade_indicator() {
        let mut out = Vec::new();

        let mut packages = test_packages();
        let PackageToList::Catalog(_, ref mut pip_lock) = packages[0] else {
            unreachable!()
        };
        let mut pip_lock_upgraded = pip_lock.clone();
        pip_lock_upgraded.version = format!("{}-upgraded", pip_lock.version);

        let upgrades = UpgradeDiff::from_iter(vec![(
            "pip_install_id".to_string(),
            BTreeMap::from_iter(vec![(
                "some-system".to_string(),
                (
                    LockedPackage::Catalog(pip_lock.clone()),
                    LockedPackage::Catalog(pip_lock_upgraded),
                ),
            )]),
        )]);

        List::print_extended(&mut out, &packages, Some(upgrades)).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            pip_install_id: python3Packages.pip (20.3.4 - upgrade available)
            python_install_id: python3Packages.python (3.9.5)
        "});
    }

    #[test]
    fn test_print_detail_output() {
        let mut out = Vec::new();
        List::print_detail(&mut out, &test_packages()).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            pip_install_id: (pip)
              Description: Python package installer
              Path:     python3Packages.pip
              Priority: 100
              Version:  20.3.4
              License:  MIT
              Unfree:   true
              Broken:   false

            python_install_id: (python)
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
        List::print_detail(&mut out, &[test_flake_package()]).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            nix-eval-jobs: (nix-eval-jobs)
              Description:     Hydra's builtin hydra-eval-jobs as a standalone
              Locked URL:      github:nix-community/nix-eval-jobs/c132534bc68eb48479a59a3116ee7ce0f16ce12b
              Flake attribute: packages.aarch64-darwin.default
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
        let mut package = test_flake_package();
        if let PackageToList::Flake(_, ref mut locked_package) = package {
            locked_package.locked_installable.pname = None;
        }

        List::print_detail(&mut out, &[package]).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            nix-eval-jobs:
              Description:     Hydra's builtin hydra-eval-jobs as a standalone
              Locked URL:      github:nix-community/nix-eval-jobs/c132534bc68eb48479a59a3116ee7ce0f16ce12b
              Flake attribute: packages.aarch64-darwin.default
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
        let mut package = test_flake_package();
        if let PackageToList::Flake(_, ref mut locked_package) = package {
            if let Some(licenses) = locked_package.locked_installable.licenses.as_mut() {
                licenses.push("license 2".to_string());
            }
        }
        List::print_detail(&mut out, &[package]).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            nix-eval-jobs: (nix-eval-jobs)
              Description:     Hydra's builtin hydra-eval-jobs as a standalone
              Locked URL:      github:nix-community/nix-eval-jobs/c132534bc68eb48479a59a3116ee7ce0f16ce12b
              Flake attribute: packages.aarch64-darwin.default
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
        let PackageToList::Catalog(_, ref mut package_2) = packages[1] else {
            panic!();
        };
        package_2.priority = 5;

        let mut out = Vec::new();
        List::print_detail(&mut out, &packages).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            python_install_id: (python)
              Description: Python interpreter
              Path:     python3Packages.python
              Priority: 5
              Version:  3.9.5
              License:  PSF
              Unfree:   false
              Broken:   false

            pip_install_id: (pip)
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
        let PackageToList::Catalog(_, ref mut package_2) = packages[1] else {
            panic!();
        };
        package_2.priority = 10;

        let mut out = Vec::new();
        List::print_detail(&mut out, &packages).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            python_install_id: (python)
              Description: Python interpreter
              Path:     python3Packages.python
              Priority: 10
              Version:  3.9.5
              License:  PSF
              Unfree:   false
              Broken:   false

            pip_install_id: (pip)
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
        assert_eq!(out, formatdoc! {"
            pip_install_id: (pip)
              Description: N/A
              Path:     python3Packages.pip
              Priority: {DEFAULT_PRIORITY}
              Version:  N/A
              License:  N/A
              Unfree:   N/A
              Broken:   N/A
        "})
    }
}
