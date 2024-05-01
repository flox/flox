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
                Self::print_name_only(&packages);
            },
            ListMode::Extended => {
                tracing::Span::current().record("mode", "extended");
                Self::print_extended(&packages);
            },
            ListMode::All => {
                tracing::Span::current().record("mode", "all");
                Self::print_detail(&packages);
            },
            ListMode::Config => unreachable!(),
        }

        Ok(())
    }

    /// print package ids only
    fn print_name_only(packages: &[InstalledPackage]) {
        packages.iter().for_each(|p| println!("{}", p.install_id));
    }

    /// print package ids, as well as path and version
    ///
    /// e.g. `pip: python3Packages.pip (20.3.4)`
    ///
    /// This is the default mode
    fn print_extended(packages: &[InstalledPackage]) {
        packages.iter().for_each(|p| {
            println!(
                "{id}: {path} ({version})",
                id = p.install_id,
                path = p.rel_path,
                version = p.info.version.as_deref().unwrap_or("N/A")
            )
        });
    }

    /// print package ids, as well as extended detailed information
    fn print_detail(packages: &[InstalledPackage]) {
        for InstalledPackage {
            install_id: name,
            rel_path,
            info:
                PackageInfo {
                    broken,
                    license,
                    pname,
                    unfree,
                    version,
                    description,
                },
            priority,
        } in packages.iter().sorted_by_key(|p| p.priority)
        {
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
            };

            println!("{message}");
        }
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
