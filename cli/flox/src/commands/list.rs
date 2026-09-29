use std::fmt::{self, Display};
use std::io::{Write, stdout};
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Result, bail};
use bpaf::Bpaf;
use chrono::{DateTime, Utc};
use flox_config::Config;
use flox_core::data::System;
use flox_events::{CliEnvironmentPayload, EventKind, EventsHub};
use flox_manifest::interfaces::{AsLatestSchema, AsWritableManifest, WriteManifest};
use flox_manifest::lockfile::{LockedInstallable, LockedPackageFlake, Lockfile, PackageToList};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::floxmeta_branch::BranchOrd;
use flox_rust_sdk::models::environment::generations::{
    GenerationId,
    GenerationsExt,
    History,
    HistoryKind,
};
use flox_rust_sdk::models::environment::{
    ConcreteEnvironment,
    Environment,
    SingleSystemUpgradeDiff,
};
use flox_rust_sdk::providers::buildenv::get_installed_outputs;
use flox_rust_sdk::providers::upgrade_checks::{UpgradeInformation, UpgradeInformationGuard};
use indoc::formatdoc;
use itertools::Itertools;
use tracing::{debug, instrument};
use url::Url;

use super::{EnvironmentSelect, environment_select};
use crate::commands::render_composition_manifest;
use crate::environment_subcommand_metric;
use crate::utils::events::env_detail_from_concrete;
use crate::utils::message;
use crate::utils::tracing::sentry_set_tag;
use crate::utils::upgrade_output::{
    count_upgrade_categories,
    format_upgrade_summary,
    render_diff,
    render_upgrade_target,
};

// List packages installed in an environment
#[derive(Bpaf, Clone)]
pub struct List {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(long, short)]
    upstream: bool,

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

    /// Show all available information, including details about the
    /// environment and the upgrades available for each package
    #[bpaf(long, short)]
    All,
}

impl List {
    #[instrument(name = "list", skip_all)]
    pub async fn handle(self, config: Config, mut flox: Flox) -> Result<()> {
        sentry_set_tag("list_mode", format!("{:?}", &self.list_mode));

        let mut env = self
            .environment
            .detect_concrete_environment(&mut flox, "List using")
            .await?;
        environment_subcommand_metric!("list", env);
        if let Err(err) = EventsHub::global().record_event(EventKind::CliEnvironmentList(
            CliEnvironmentPayload::new(env_detail_from_concrete(&flox, &env)),
        )) {
            debug!(error = %err, "Failed to record v2 event");
        }

        let (manifest_contents, lockfile) = match (&mut env, self.upstream) {
            (ConcreteEnvironment::Path(_), true) => {
                bail!("'--upstream' cannot be used with path environments");
            },
            (ConcreteEnvironment::Managed(managed_environment), true) => {
                managed_environment.fetch_remote_state(&flox)?;

                let remote_manifest_contents =
                    managed_environment.remote_manifest_contents_for_current_generation()?;

                let remote_lockfile_contents =
                    managed_environment.remote_lockfile_contents_for_current_generation()?;
                let lockfile = Lockfile::from_str(&remote_lockfile_contents)?;

                (remote_manifest_contents, lockfile)
            },
            (ConcreteEnvironment::Remote(remote_environment), true) => {
                remote_environment.fetch_remote_state(&flox)?;

                let remote_manifest_contents =
                    remote_environment.remote_manifest_contents_for_current_generation()?;

                let remote_lockfile_contents =
                    remote_environment.remote_lockfile_contents_for_current_generation()?;
                let lockfile = Lockfile::from_str(&remote_lockfile_contents)?;

                (remote_manifest_contents, lockfile)
            },
            (env, false) => (
                env.manifest_without_migrating(&flox)?
                    .as_writable()
                    .to_string(),
                env.lockfile(&flox)?.into(),
            ),
        };

        if self.list_mode == ListMode::Config {
            Self::print_config(&lockfile, &manifest_contents)?;
            return Ok(());
        }

        // `--all` shows details about the environment along with its packages.
        let details = (self.list_mode == ListMode::All)
            .then(|| EnvironmentDetails::gather(&config, &flox, &env, &lockfile, self.upstream));
        let flags = self
            .environment
            .to_flags()
            .map(|flags| format!(" {}", flags.join(" ")))
            .unwrap_or_default();

        let system = &flox.system;
        let packages = lockfile.list_packages(system)?;

        if let Some(details) = &details {
            print_environment_details(stdout().lock(), details)?;
        }

        if packages.is_empty() {
            let message = formatdoc! {"
                No packages are installed for your current system ('{system}').

                You can see the whole manifest with 'flox list --config'.
            "};
            message::warning(message);
            // The warning already ends with an empty line.
            if let Some(next_step) = details.and_then(|details| next_step(&details, false, &flags))
            {
                message::plain(next_step);
            }
            return Ok(());
        }

        if details.is_some() {
            let heading = if self.upstream {
                "Packages on FloxHub:"
            } else {
                "Packages:"
            };
            writeln!(stdout(), "\n{heading}\n")?;
        }

        match self.list_mode {
            ListMode::NameOnly => {
                Self::print_name_only(stdout().lock(), &packages)?;
            },
            ListMode::Extended => {
                Self::print_extended(
                    stdout().lock(),
                    &packages,
                    if self.upstream {
                        None
                    } else {
                        List::get_cached_upgrades_for_current_system(&flox, &mut env)?
                    },
                )?;
            },
            ListMode::All => {
                Self::print_detail(
                    stdout().lock(),
                    &packages,
                    if self.upstream {
                        // Upgrades apply to FloxHub's packages only if the
                        // last check covered the same lockfile.
                        details
                            .as_ref()
                            .and_then(|details| details.upgrades.available_upgrades())
                    } else {
                        List::get_cached_upgrades_for_current_system(&flox, &mut env)?
                    },
                )?;
            },
            ListMode::Config => unreachable!(),
        }

        // With many packages, the upgrades are hard to find among them,
        // so repeat them together at the end.
        if let Some(upgrades) = details
            .as_ref()
            .and_then(|details| details.upgrades.available_upgrades())
        {
            writeln!(
                stdout(),
                "\nAvailable upgrades:\n\n{}",
                render_diff(&upgrades)
            )?;
        }

        if let Some(next_step) = details.and_then(|details| next_step(&details, true, &flags)) {
            // Set apart from the listing above it.
            message::plain(format!("\n{next_step}"));
        }

        Ok(())
    }

    /// Serialize the manifest to a string.
    /// If the manifest includes other environments,
    /// configure the serializer to produce output closer to the reference
    /// style.
    fn manifest_contents_to_print(
        lockfile: &Lockfile,
        manifest_contents: impl Into<String>,
    ) -> Result<String> {
        let is_composed = lockfile.compose.is_some();
        let manifest_contents = if is_composed {
            render_composition_manifest(&lockfile.manifest)?
        } else {
            manifest_contents.into()
        };

        Ok(manifest_contents)
    }

    /// print the manifest contents
    fn print_config(lockfile: &Lockfile, manifest_contents: impl Into<String>) -> Result<()> {
        println!(
            "{}",
            Self::manifest_contents_to_print(lockfile, manifest_contents)?
        );
        let is_composed = lockfile.compose.is_some();
        if is_composed {
            message::info("Displaying merged manifest.");
            message::print_overridden_manifest_fields(lockfile);
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
            let install_id = match p {
                PackageToList::Catalog(_, p) => &p.install_id,
                PackageToList::Flake(_, p) => &p.install_id,
                PackageToList::StorePath(p) => &p.install_id,
            };
            let upgrade_available = if upgrades
                .as_ref()
                .is_some_and(|diff| diff.contains_key(install_id))
            {
                " - upgrade available"
            } else {
                ""
            };

            match p {
                PackageToList::Catalog(descriptor, p) => {
                    writeln!(
                        &mut out,
                        "{id}: {path} ({version}{upgrade_available})",
                        id = p.install_id,
                        path = descriptor.pkg_path,
                        version = p.version,
                    )?;
                },
                PackageToList::Flake(descriptor, locked_package) => {
                    writeln!(
                        &mut out,
                        "{id}: {flake}{upgrade_available}",
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
    fn print_detail(
        mut out: impl Write,
        packages: &[PackageToList],
        upgrades: Option<SingleSystemUpgradeDiff>,
    ) -> Result<()> {
        // Format the outputs lines for a package
        for (idx, package) in packages
            .iter()
            .sorted_by_key(|p| match p {
                PackageToList::Catalog(_, locked) => locked.priority,
                PackageToList::Flake(_, locked) => locked.locked_installable.priority,
                PackageToList::StorePath(locked) => locked.priority,
            })
            .enumerate()
        {
            let install_id = match package {
                PackageToList::Catalog(_, p) => &p.install_id,
                PackageToList::Flake(_, p) => &p.install_id,
                PackageToList::StorePath(p) => &p.install_id,
            };
            let upgrade = upgrades.as_ref().and_then(|diff| diff.get(install_id));
            let upgrade_available = if upgrade.is_some() {
                " (upgrade available)"
            } else {
                ""
            };
            // Follows the version line, so it starts with the line break.
            let upgrade_line = upgrade
                .map(|(before, after)| {
                    format!(
                        "\n  Upgrade available:    {}",
                        render_upgrade_target(before, after)
                    )
                })
                .unwrap_or_default();

            let message = match package {
                PackageToList::Catalog(descriptor, locked) => {
                    let outputs_lines = format_outputs_lines(package);

                    formatdoc! {"
                        {name}:{upgrade_available}
                          Description:          {description}
                          Package Path:         {attr_path}
                          Package Name:         {pname}
                          Priority:             {priority}
                          Version:              {version}{upgrade_line}
                          Stability:            {stabilities}
                          License:              {license}
                          Unfree:               {unfree}
                          Broken:               {broken}
                        {outputs_lines}",
                        name = &locked.install_id,
                        pname = &locked.pname,
                        attr_path = &descriptor.pkg_path,
                        priority = locked.priority,
                        version = &locked.version,
                        // The stabilities of the catalog page the package is locked to
                        stabilities = locked.stabilities.as_ref().map(|s| s.join(", ")).as_deref().unwrap_or("N/A"),
                        description = locked.description.as_deref().unwrap_or("N/A"),
                        license = locked.license.as_deref().unwrap_or("N/A"),
                        unfree = locked.unfree.map(|u| u.to_string()).as_deref().unwrap_or("N/A"),
                        broken = locked.broken.map(|b| b.to_string()).as_deref().unwrap_or("N/A"),
                    }
                },
                PackageToList::Flake(_, locked) => {
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
                    } = locked;

                    let formatted_licenses = licenses.as_ref().map(|licenses| {
                        if licenses.len() == 1 {
                            format!("License:              {}", licenses[0])
                        } else {
                            format!("Licenses:             {}", licenses.join(", "))
                        }
                    });

                    let outputs_lines = format_outputs_lines(package);

                    formatdoc! {"
                    {install_id}:{upgrade_available}
                      Description:          {description}
                      Locked URL:           {locked_url}
                      Flake attribute:      {locked_flake_attr_path}
                      Package Name:         {formatted_pname}
                      Priority:             {priority}
                      Version:              {version}{upgrade_line}
                      {formatted_licenses}
                      Unfree:               {unfree}
                      Broken:               {broken}
                    {outputs_lines}",
                        formatted_pname = pname.as_deref().unwrap_or("N/A"),
                        description = description.as_deref().unwrap_or("N/A"),
                        version = version.as_deref().unwrap_or("N/A"),
                        formatted_licenses = formatted_licenses.as_deref().unwrap_or("License: N/A"),
                        unfree = unfree.map(|u|u.to_string()).as_deref().unwrap_or("N/A"),
                        broken = broken.map(|b|b.to_string()).as_deref().unwrap_or("N/A"),
                    }
                },
                PackageToList::StorePath(locked_package_store_path) => formatdoc! {"
                    {install_id}:
                    Store Path:           {store_path}
                    Priority:             {priority}
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

    fn get_cached_upgrades_for_current_system(
        flox: &Flox,
        environment: &mut ConcreteEnvironment,
    ) -> Result<Option<SingleSystemUpgradeDiff>> {
        let upgrade_guard = UpgradeInformationGuard::read_in(environment.cache_path()?)?;
        let Some(info) = upgrade_guard.info() else {
            debug!("Not displaying upgrade information; no upgrade information available");
            return Ok(None);
        };

        let current_lockfile = environment.lockfile(flox)?.into();

        if Some(current_lockfile) != info.upgrade_result.old_lockfile {
            // todo: delete the info file?
            debug!("Not using upgrade information; lockfile has changed since last check");
            return Ok(None);
        }

        Ok(Some(info.upgrade_result.diff_for_system(&flox.system)))
    }
}

fn format_outputs_lines(package: &PackageToList) -> String {
    let available_outputs = match package {
        PackageToList::Catalog(_, locked) => {
            format_as_sorted_list(&locked.outputs.keys().collect::<Vec<_>>())
        },
        PackageToList::Flake(_, locked) => {
            format_as_sorted_list(&locked.locked_installable.output_names)
        },
        PackageToList::StorePath(_) => return String::new(),
    };

    let installed_outputs_or_error_message = get_installed_outputs(package).map_or_else(
        |e| {
            message::warning(format!("Error {e} when trying to fetch installed outputs"));
            "[]".to_string()
        },
        |a| format_as_sorted_list(&a),
    );

    format!(
        "  Available Outputs:    {}\n  Installed Outputs:    {}\n",
        available_outputs, installed_outputs_or_error_message
    )
}

fn format_as_sorted_list<S>(arr: &[S]) -> String
where
    S: ToString,
{
    let mut sorted_items = arr
        .iter()
        .map(|s| format!("\"{}\"", s.to_string()))
        .collect::<Vec<_>>();
    sorted_items.sort();
    if sorted_items.is_empty() {
        return "[ ]".to_string();
    }
    format!("[ {} ]", sorted_items.join(", "))
}

/// Environment-level details shown by `flox list --all`.
///
/// Each detail is best-effort:
/// a detail that can't be read is logged and omitted,
/// so they never make `--all` fail.
#[derive(Clone, Debug, PartialEq)]
struct EnvironmentDetails {
    /// `owner/name` for FloxHub environments, the name for path environments
    name: String,
    floxhub_url: Option<Url>,
    /// The directory containing `.flox`.
    /// `None` for environments only cached locally (`--reference`).
    path: Option<PathBuf>,
    /// The system packages are listed for
    system: System,
    /// `None` if the manifest doesn't set systems
    /// and the locked packages don't imply them
    systems: Option<Vec<System>>,
    /// `None` for path environments, which have no generations
    generations: Option<GenerationDetails>,
    upgrade_notices: UpgradeNotices,
    upgrades: UpgradeStatus,
}

impl EnvironmentDetails {
    fn gather(
        config: &Config,
        flox: &Flox,
        env: &ConcreteEnvironment,
        lockfile: &Lockfile,
        upstream: bool,
    ) -> Self {
        let (name, floxhub_url, path, generations) = match env {
            ConcreteEnvironment::Path(env) => (
                env.name().to_string(),
                None,
                ok_or_debug(env.parent_path(), "path"),
                None,
            ),
            ConcreteEnvironment::Managed(env) => (
                env.env_ref().to_string(),
                ok_or_debug(env.pointer().floxhub_url(), "FloxHub URL"),
                ok_or_debug(env.parent_path(), "path"),
                Some(GenerationDetails::gather(env, || {
                    ok_or_debug(env.has_local_changes(flox), "local changes")
                })),
            ),
            ConcreteEnvironment::Remote(env) => (
                env.env_ref().to_string(),
                ok_or_debug(env.pointer().floxhub_url(), "FloxHub URL"),
                None,
                // The local copy of a remote environment isn't edited directly.
                Some(GenerationDetails::gather(env, || Some(false))),
            ),
        };

        // Read from the listed lockfile, the same merged manifest
        // that activation reads its options from.
        let manifest = ok_or_debug(lockfile.migrated_manifest(), "manifest");
        let options = manifest
            .as_ref()
            .map(|manifest| manifest.as_latest_schema().options.clone())
            .unwrap_or_default();
        // The background check only covers the live generation of the local
        // copy, not a pinned generation or FloxHub's (`--upstream`).
        let checks_listed = !upstream && pinned_generation(env).is_none();
        let upgrades = read_upgrade_status(env, lockfile, &flox.system, checks_listed);

        let systems = options.systems.clone().or_else(|| {
            ok_or_debug(lockfile.implicit_resolved_systems(), "systems")
                .flatten()
                .map(|systems| systems.into_iter().collect())
        });

        Self {
            name,
            floxhub_url,
            path,
            system: flox.system.clone(),
            systems,
            generations,
            upgrade_notices: UpgradeNotices::new(
                config.flox.upgrade_notifications,
                options.activate.upgrade_notifications,
            ),
            upgrades,
        }
    }
}

/// Generation details of a FloxHub environment
#[derive(Clone, Debug, PartialEq)]
struct GenerationDetails {
    local: Option<LocalGeneration>,
    floxhub: Option<FloxHubState>,
}

impl GenerationDetails {
    /// Read generation details from the local generation metadata and the
    /// FloxHub metadata of the last fetch.
    /// `local_changes` is only called for the live generation, whose packages
    /// are read from the local checkout.
    fn gather(env: &impl GenerationsExt, local_changes: impl FnOnce() -> Option<bool>) -> Self {
        let local = ok_or_debug(env.generations_metadata(), "generations").and_then(|metadata| {
            let live = metadata.current_gen()?;
            if let Some(pinned) = env.pinned_generation() {
                return Some(LocalGeneration::Pinned { pinned, live });
            }
            // Generation numbers only grow, so the newest generation is the
            // highest one, even after switching back to an older generation.
            let latest = metadata
                .history()
                .iter()
                .map(|change| change.current_generation)
                .max()
                .unwrap_or(live);
            Some(LocalGeneration::Live {
                live,
                latest,
                local_changes: local_changes().unwrap_or(false),
            })
        });

        let floxhub = ok_or_debug(env.remote_generations_metadata(), "FloxHub generations")
            .and_then(|metadata| {
                let generation = metadata.current_gen()?;
                let sync = ok_or_debug(env.compare_remote(), "FloxHub sync state")?;
                Some(FloxHubState {
                    generation,
                    sync,
                    // Read from FloxHub's history,
                    // so automatic upgrades show before they are pulled.
                    last_auto_upgrade: last_auto_upgrade(metadata.history()),
                })
            });

        Self { local, floxhub }
    }
}

/// The generation of the local copy.
/// Its packages are listed, unless `--upstream` lists FloxHub's instead.
#[derive(Clone, Debug, PartialEq)]
enum LocalGeneration {
    /// The live generation, read from the local checkout
    Live {
        live: GenerationId,
        latest: GenerationId,
        local_changes: bool,
    },
    /// A generation pinned by `flox activate --generation`
    Pinned {
        pinned: GenerationId,
        live: GenerationId,
    },
}

impl Display for LocalGeneration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LocalGeneration::Live {
                live,
                latest,
                local_changes,
            } => {
                write!(f, "{live} (live")?;
                if latest > live {
                    write!(f, ", latest is {latest}")?;
                }
                if *local_changes {
                    write!(f, ", with local changes")?;
                }
                write!(f, ")")
            },
            LocalGeneration::Pinned { pinned, live } => write!(
                f,
                "{pinned} (pinned by 'flox activate --generation', live is {live})"
            ),
        }
    }
}

/// The environment on FloxHub, as of the last fetch
#[derive(Clone, Debug, PartialEq)]
struct FloxHubState {
    /// FloxHub's live generation
    generation: GenerationId,
    /// How the local copy compares to FloxHub
    sync: BranchOrd,
    last_auto_upgrade: Option<AutoUpgrade>,
}

impl FloxHubState {
    /// Comparing it with the local generation shows whether the local copy
    /// is ahead or behind, but not whether the two have diverged.
    fn generation(&self) -> String {
        match self.sync {
            BranchOrd::Diverged => {
                format!(
                    "generation {} (diverged from the local copy)",
                    self.generation
                )
            },
            BranchOrd::Equal | BranchOrd::Ahead | BranchOrd::Behind => {
                format!("generation {}", self.generation)
            },
        }
    }

    /// The CLI can't see whether automatic upgrades are enabled,
    /// only the ones FloxHub made.
    fn auto_upgrade(&self) -> String {
        match &self.last_auto_upgrade {
            Some(upgrade) => format!(
                "last upgraded {} (generation {})",
                upgrade.timestamp, upgrade.generation
            ),
            None => "none in FloxHub's history".to_string(),
        }
    }
}

/// Authors FloxHub records for changes it makes on its own,
/// such as automatic upgrades. `floxEM` is the name used by older versions.
const FLOXHUB_AUTHORS: [&str; 2] = ["FloxHub", "floxEM"];

/// The newest generation created by an automatic upgrade on FloxHub
#[derive(Clone, Debug, PartialEq)]
struct AutoUpgrade {
    generation: GenerationId,
    timestamp: DateTime<Utc>,
}

/// Find the newest upgrade that FloxHub made, rather than a user.
fn last_auto_upgrade(history: &History) -> Option<AutoUpgrade> {
    history
        .iter()
        .rev()
        .find(|change| {
            matches!(change.kind, HistoryKind::Upgrade { .. })
                && FLOXHUB_AUTHORS.contains(&change.author.as_str())
        })
        .map(|change| AutoUpgrade {
            generation: change.current_generation,
            timestamp: change.timestamp,
        })
}

/// Whether `flox activate` notifies about available upgrades
#[derive(Clone, Copy, Debug, PartialEq)]
enum UpgradeNotices {
    On,
    DisabledByConfig,
    DisabledByManifest,
}

impl UpgradeNotices {
    /// The manifest can't re-enable notices that the config disables,
    /// so the config takes precedence.
    fn new(config: Option<bool>, manifest: Option<bool>) -> Self {
        if !config.unwrap_or(true) {
            return UpgradeNotices::DisabledByConfig;
        }
        if !manifest.unwrap_or(true) {
            return UpgradeNotices::DisabledByManifest;
        }
        UpgradeNotices::On
    }
}

impl Display for UpgradeNotices {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpgradeNotices::On => write!(f, "on"),
            UpgradeNotices::DisabledByConfig => {
                write!(f, "off (upgrade_notifications = false in 'flox config')")
            },
            UpgradeNotices::DisabledByManifest => {
                write!(f, "off (options.activate.upgrade-notifications = false)")
            },
        }
    }
}

/// Upgrades available for the listed packages,
/// according to the last upgrade check.
///
/// Only the check that `flox activate` starts in the background writes
/// this information.
#[derive(Clone, Debug, PartialEq)]
enum UpgradeStatus {
    /// No upgrade check ran for this environment
    NotChecked,
    /// The environment changed after the last check
    Outdated {
        checked: DateTime<Utc>,
    },
    /// The last check doesn't cover the listed packages
    /// because they aren't the live generation of the local copy
    NotCovered {
        checked: DateTime<Utc>,
    },
    UpToDate {
        checked: DateTime<Utc>,
    },
    OtherSystemsOnly {
        checked: DateTime<Utc>,
    },
    Available {
        checked: DateTime<Utc>,
        upgrades: SingleSystemUpgradeDiff,
    },
}

impl UpgradeStatus {
    /// `checks_listed` is whether the background check covers the listed
    /// lockfile, so that a mismatch means the environment changed.
    fn from_cache(
        info: Option<&UpgradeInformation>,
        listed: &Lockfile,
        system: &str,
        checks_listed: bool,
    ) -> Self {
        let Some(info) = info else {
            return UpgradeStatus::NotChecked;
        };
        let checked =
            DateTime::from_timestamp(info.last_checked.unix_timestamp(), 0).unwrap_or_default();
        if info.upgrade_result.old_lockfile.as_ref() != Some(listed) {
            if !checks_listed {
                return UpgradeStatus::NotCovered { checked };
            }
            return UpgradeStatus::Outdated { checked };
        }

        let upgrades = info.upgrade_result.diff_for_system(system);
        if !upgrades.is_empty() {
            return UpgradeStatus::Available { checked, upgrades };
        }
        if !info.upgrade_result.diff().is_empty() {
            return UpgradeStatus::OtherSystemsOnly { checked };
        }
        UpgradeStatus::UpToDate { checked }
    }

    fn available_upgrades(&self) -> Option<SingleSystemUpgradeDiff> {
        match self {
            UpgradeStatus::Available { upgrades, .. } => Some(upgrades.clone()),
            _ => None,
        }
    }

    fn checked(&self) -> Option<DateTime<Utc>> {
        match self {
            UpgradeStatus::NotChecked => None,
            UpgradeStatus::Outdated { checked }
            | UpgradeStatus::NotCovered { checked }
            | UpgradeStatus::UpToDate { checked }
            | UpgradeStatus::OtherSystemsOnly { checked }
            | UpgradeStatus::Available { checked, .. } => Some(*checked),
        }
    }
}

/// A summary; the packages show the upgrade available for each of them.
impl Display for UpgradeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UpgradeStatus::NotChecked => write!(f, "unknown, 'flox activate' hasn't checked yet"),
            UpgradeStatus::Outdated { .. } => {
                write!(f, "unknown, the environment changed after the last check")
            },
            UpgradeStatus::NotCovered { .. } => {
                write!(f, "unknown, only the live generation is checked")
            },
            UpgradeStatus::UpToDate { .. } => write!(f, "none available"),
            UpgradeStatus::OtherSystemsOnly { .. } => write!(f, "only for other systems"),
            UpgradeStatus::Available { upgrades, .. } => {
                let (version_changes, rebuilds) = count_upgrade_categories(upgrades);
                write!(f, "{}", format_upgrade_summary(version_changes, rebuilds))
            },
        }
    }
}

fn read_upgrade_status(
    env: &ConcreteEnvironment,
    listed: &Lockfile,
    system: &str,
    checks_listed: bool,
) -> UpgradeStatus {
    let guard = ok_or_debug(
        env.cache_path()
            .map_err(anyhow::Error::from)
            .and_then(|cache_path| Ok(UpgradeInformationGuard::read_in(cache_path)?)),
        "upgrade information",
    );
    UpgradeStatus::from_cache(
        guard.as_ref().and_then(|guard| guard.info().as_ref()),
        listed,
        system,
        checks_listed,
    )
}

fn pinned_generation(env: &ConcreteEnvironment) -> Option<GenerationId> {
    match env {
        ConcreteEnvironment::Path(_) => None,
        ConcreteEnvironment::Managed(env) => env.pinned_generation(),
        ConcreteEnvironment::Remote(env) => env.pinned_generation(),
    }
}

/// Print the environment details that precede the packages.
///
/// Details that don't apply to the environment are left out
/// rather than printed as placeholders.
fn print_environment_details(mut out: impl Write, details: &EnvironmentDetails) -> Result<()> {
    let mut rows = vec![("Environment", details.name.clone())];
    if let Some(floxhub_url) = &details.floxhub_url {
        rows.push(("FloxHub URL", floxhub_url.to_string()));
    }
    if let Some(path) = &details.path {
        rows.push(("Path", path.display().to_string()));
    }
    let unsupported = details
        .systems
        .as_ref()
        .is_some_and(|systems| !systems.contains(&details.system));
    let system_note = if unsupported {
        " (not in the environment's systems)"
    } else {
        ""
    };
    rows.push(("System", format!("{}{system_note}", details.system)));
    if let Some(systems) = &details.systems {
        rows.push(("Systems", systems.join(", ")));
    }
    if let Some(generations) = &details.generations {
        if let Some(local) = &generations.local {
            rows.push(("Generation", local.to_string()));
        }
        if let Some(floxhub) = &generations.floxhub {
            rows.push(("FloxHub", floxhub.generation()));
            rows.push(("Auto-upgrade", floxhub.auto_upgrade()));
        }
    }
    rows.push(("Upgrade notices", details.upgrade_notices.to_string()));
    rows.push(("Upgrades", details.upgrades.to_string()));
    if let Some(checked) = details.upgrades.checked() {
        rows.push(("Upgrades checked", checked.to_string()));
    }

    // Align values at the same column for every environment.
    let width = "Upgrades checked: ".len();
    for (label, value) in rows {
        writeln!(&mut out, "{:<width$}{value}", format!("{label}:"))?;
    }
    Ok(())
}

/// The most useful next step for the state `flox list --all` shows,
/// or `None` if there is nothing to do.
/// Upgrades are only suggested if packages are installed for this system.
///
/// Local changes come first,
/// since 'flox pull' and 'flox push' refuse to run until they are committed
/// and 'flox pull --force' discards them.
/// Changes on FloxHub come next,
/// since FloxHub may already have applied the upgrades listed locally.
/// Upgrades come last and apply to the live generation, not a pinned one.
fn next_step(details: &EnvironmentDetails, has_packages: bool, flags: &str) -> Option<String> {
    let generations = details.generations.as_ref();
    let local = generations.and_then(|generations| generations.local.as_ref());
    if let Some(LocalGeneration::Live {
        local_changes: true,
        ..
    }) = local
    {
        return Some(format!(
            "Use 'flox edit --sync{flags}' to commit your local changes to a new generation."
        ));
    }

    match generations
        .and_then(|generations| generations.floxhub.as_ref())
        .map(|floxhub| floxhub.sync)
    {
        Some(BranchOrd::Behind) => {
            return Some(format!(
                "Use 'flox pull{flags}' to fetch updates from FloxHub."
            ));
        },
        Some(BranchOrd::Ahead) => {
            return Some(format!(
                "Use 'flox push{flags}' to update the environment on FloxHub."
            ));
        },
        Some(BranchOrd::Diverged) => {
            return Some(formatdoc! {"
                Use 'flox pull --force{flags}' to replace the local copy with FloxHub's version.
                Use 'flox push --force{flags}' to replace FloxHub's version with the local copy."
            });
        },
        Some(BranchOrd::Equal) | None => {},
    }

    if !has_packages || matches!(local, Some(LocalGeneration::Pinned { .. })) {
        return None;
    }

    match details.upgrades {
        UpgradeStatus::NotChecked | UpgradeStatus::Outdated { .. } => Some(format!(
            "Use 'flox upgrade --dry-run{flags}' to see the upgrades available now."
        )),
        UpgradeStatus::Available { .. } => {
            Some(format!("Use 'flox upgrade{flags}' to apply the upgrades."))
        },
        UpgradeStatus::NotCovered { .. }
        | UpgradeStatus::UpToDate { .. }
        | UpgradeStatus::OtherSystemsOnly { .. } => None,
    }
}

/// Log why a detail couldn't be read and omit it.
fn ok_or_debug<T, E: Display>(result: Result<T, E>, detail: &str) -> Option<T> {
    result
        .inspect_err(|err| debug!(%err, detail, "Omitting environment detail"))
        .ok()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use chrono::TimeZone;
    use flox_manifest::lockfile::LockedPackage;
    use flox_manifest::lockfile::test_helpers::{
        LOCKED_NIX_EVAL_JOBS,
        fake_catalog_package_lock,
        nix_eval_jobs_descriptor,
    };
    use flox_manifest::parsed::common::DEFAULT_PRIORITY;
    use flox_manifest::test_helpers::with_latest_schema;
    use flox_rust_sdk::flox::test_helpers::{flox_instance, flox_instance_with_optional_floxhub};
    use flox_rust_sdk::models::environment::UpgradeResult;
    use flox_rust_sdk::models::environment::generations::{
        AddGenerationOptions,
        AllGenerationsMetadata,
    };
    use flox_rust_sdk::models::environment::managed_environment::ManagedEnvironment;
    use flox_rust_sdk::models::environment::managed_environment::test_helpers::mock_managed_environment_unlocked;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment_in;
    use flox_test_utils::GENERATED_DATA;
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use time::OffsetDateTime;

    use super::*;

    fn test_packages() -> [PackageToList; 2] {
        let (_pip_iid, pip_descriptor, mut pip_lock) = fake_catalog_package_lock("pip", None);
        let (_python_iid, python_descriptor, mut python_lock) =
            fake_catalog_package_lock("python", None);

        // Update descriptors to have the full pkg_path
        let mut pip_descriptor = pip_descriptor.unwrap_catalog_descriptor().unwrap();
        pip_descriptor.pkg_path = "python3Packages.pip".to_string();

        let mut python_descriptor = python_descriptor.unwrap_catalog_descriptor().unwrap();
        python_descriptor.pkg_path = "python3Packages.python".to_string();

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
        pip_lock.stabilities = Some(vec![
            "stable".to_string(),
            "staging".to_string(),
            "unstable".to_string(),
        ]);

        // - python
        python_lock.priority = 200;
        python_lock.attr_path = "python3Packages.python".to_string();
        python_lock.version = "3.9.5".to_string();
        python_lock.description = Some("Python interpreter".to_string());
        python_lock.license = Some("PSF".to_string());
        python_lock.unfree = Some(false);
        python_lock.broken = Some(false);

        [
            PackageToList::Catalog(pip_descriptor, pip_lock),
            PackageToList::Catalog(python_descriptor, python_lock),
        ]
    }

    fn uninformative_package() -> PackageToList {
        let (_pip_iid, pip_descriptor, mut pip_lock) = fake_catalog_package_lock("pip", None);

        let mut pip_descriptor = pip_descriptor.unwrap_catalog_descriptor().unwrap();
        pip_descriptor.pkg_path = "python3Packages.pip".to_string();

        // populate the lock
        pip_lock.attr_path = "python3Packages.pip".to_string();
        pip_lock.pname = "pip".to_string();
        pip_lock.version = "N/A".to_string();

        PackageToList::Catalog(pip_descriptor, pip_lock)
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

        let upgrades = SingleSystemUpgradeDiff::from_iter(vec![(
            "pip_install_id".to_string(),
            (
                LockedPackage::Catalog(pip_lock.clone()),
                LockedPackage::Catalog(pip_lock_upgraded),
            ),
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
        List::print_detail(&mut out, &test_packages(), None).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            pip_install_id:
              Description:          Python package installer
              Package Path:         python3Packages.pip
              Package Name:         pip
              Priority:             100
              Version:              20.3.4
              Stability:            stable, staging, unstable
              License:              MIT
              Unfree:               true
              Broken:               false
              Available Outputs:    [ ]
              Installed Outputs:    [ ]

            python_install_id:
              Description:          Python interpreter
              Package Path:         python3Packages.python
              Package Name:         python
              Priority:             200
              Version:              3.9.5
              Stability:            N/A
              License:              PSF
              Unfree:               false
              Broken:               false
              Available Outputs:    [ ]
              Installed Outputs:    [ ]
        "})
    }

    /// Test detailed output for flake installables
    #[test]
    fn test_print_detail_flake_output() {
        let mut out = Vec::new();
        List::print_detail(&mut out, &[test_flake_package()], None).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            nix-eval-jobs:
              Description:          Hydra's builtin hydra-eval-jobs as a standalone
              Locked URL:           github:nix-community/nix-eval-jobs/c132534bc68eb48479a59a3116ee7ce0f16ce12b
              Flake attribute:      packages.aarch64-darwin.default
              Package Name:         nix-eval-jobs
              Priority:             5
              Version:              2.23.0
              License:              GPL-3.0
              Unfree:               false
              Broken:               false
              Available Outputs:    [ \"out\" ]
              Installed Outputs:    [ \"out\" ]
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

        List::print_detail(&mut out, &[package], None).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            nix-eval-jobs:
              Description:          Hydra's builtin hydra-eval-jobs as a standalone
              Locked URL:           github:nix-community/nix-eval-jobs/c132534bc68eb48479a59a3116ee7ce0f16ce12b
              Flake attribute:      packages.aarch64-darwin.default
              Package Name:         N/A
              Priority:             5
              Version:              2.23.0
              License:              GPL-3.0
              Unfree:               false
              Broken:               false
              Available Outputs:    [ \"out\" ]
              Installed Outputs:    [ \"out\" ]
        "});
    }

    /// Test detailed output for flake installables with multiple licenses
    #[test]
    fn test_print_detail_flake_output_multiple_licenses() {
        let mut out = Vec::new();
        let mut package = test_flake_package();
        if let PackageToList::Flake(_, ref mut locked_package) = package
            && let Some(licenses) = locked_package.locked_installable.licenses.as_mut()
        {
            licenses.push("license 2".to_string());
        }
        List::print_detail(&mut out, &[package], None).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            nix-eval-jobs:
              Description:          Hydra's builtin hydra-eval-jobs as a standalone
              Locked URL:           github:nix-community/nix-eval-jobs/c132534bc68eb48479a59a3116ee7ce0f16ce12b
              Flake attribute:      packages.aarch64-darwin.default
              Package Name:         nix-eval-jobs
              Priority:             5
              Version:              2.23.0
              Licenses:             GPL-3.0, license 2
              Unfree:               false
              Broken:               false
              Available Outputs:    [ \"out\" ]
              Installed Outputs:    [ \"out\" ]
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
        List::print_detail(&mut out, &packages, None).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            python_install_id:
              Description:          Python interpreter
              Package Path:         python3Packages.python
              Package Name:         python
              Priority:             5
              Version:              3.9.5
              Stability:            N/A
              License:              PSF
              Unfree:               false
              Broken:               false
              Available Outputs:    [ ]
              Installed Outputs:    [ ]

            pip_install_id:
              Description:          Python package installer
              Package Path:         python3Packages.pip
              Package Name:         pip
              Priority:             100
              Version:              20.3.4
              Stability:            stable, staging, unstable
              License:              MIT
              Unfree:               true
              Broken:               false
              Available Outputs:    [ ]
              Installed Outputs:    [ ]
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
        List::print_detail(&mut out, &packages, None).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            python_install_id:
              Description:          Python interpreter
              Package Path:         python3Packages.python
              Package Name:         python
              Priority:             10
              Version:              3.9.5
              Stability:            N/A
              License:              PSF
              Unfree:               false
              Broken:               false
              Available Outputs:    [ ]
              Installed Outputs:    [ ]

            pip_install_id:
              Description:          Python package installer
              Package Path:         python3Packages.pip
              Package Name:         pip
              Priority:             100
              Version:              20.3.4
              Stability:            stable, staging, unstable
              License:              MIT
              Unfree:               true
              Broken:               false
              Available Outputs:    [ ]
              Installed Outputs:    [ ]
        "})
    }

    /// If a package is missing some values, they should be replaced with "N/A"
    #[test]
    fn test_print_detail_output_handles_missing_values() {
        let mut out = Vec::new();
        List::print_detail(&mut out, &[uninformative_package()], None).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, formatdoc! {"
            pip_install_id:
              Description:          N/A
              Package Path:         python3Packages.pip
              Package Name:         pip
              Priority:             {DEFAULT_PRIORITY}
              Version:              N/A
              Stability:            N/A
              License:              N/A
              Unfree:               N/A
              Broken:               N/A
              Available Outputs:    [ ]
              Installed Outputs:    [ ]
        "})
    }

    /// If packages have upgrades available, the output should indicate that
    #[test]
    fn test_print_detail_includes_upgrade_indicator() {
        let mut out = Vec::new();

        let mut packages = test_packages();
        let PackageToList::Catalog(_, ref mut pip_lock) = packages[0] else {
            unreachable!()
        };
        let mut pip_lock_upgraded = pip_lock.clone();
        pip_lock_upgraded.version = format!("{}-upgraded", pip_lock.version);
        let pip_upgrade = (
            LockedPackage::Catalog(pip_lock.clone()),
            LockedPackage::Catalog(pip_lock_upgraded),
        );

        let PackageToList::Catalog(_, ref python_lock) = packages[1] else {
            unreachable!()
        };
        let mut python_lock_rebuilt = python_lock.clone();
        python_lock_rebuilt.rev_date = chrono::DateTime::parse_from_rfc3339("2026-09-26T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let python_rebuild = (
            LockedPackage::Catalog(python_lock.clone()),
            LockedPackage::Catalog(python_lock_rebuilt),
        );

        let upgrades = SingleSystemUpgradeDiff::from_iter(vec![
            ("pip_install_id".to_string(), pip_upgrade),
            ("python_install_id".to_string(), python_rebuild),
        ]);

        List::print_detail(&mut out, &packages, Some(upgrades)).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out, indoc! {"
            pip_install_id: (upgrade available)
              Description:          Python package installer
              Package Path:         python3Packages.pip
              Package Name:         pip
              Priority:             100
              Version:              20.3.4
              Upgrade available:    20.3.4-upgraded
              Stability:            stable, staging, unstable
              License:              MIT
              Unfree:               true
              Broken:               false
              Available Outputs:    [ ]
              Installed Outputs:    [ ]

            python_install_id: (upgrade available)
              Description:          Python interpreter
              Package Path:         python3Packages.python
              Package Name:         python
              Priority:             200
              Version:              3.9.5
              Upgrade available:    rebuild (rev 2021-08-31 -> 2026-09-26)
              Stability:            N/A
              License:              PSF
              Unfree:               false
              Broken:               false
              Available Outputs:    [ ]
              Installed Outputs:    [ ]
        "});
    }

    /// Test catalog prefix displays in extended output (flox-cuda/python3Packages.torch case)
    #[test]
    fn test_catalog_prefix_in_extended_output() {
        let (_iid, descriptor, mut lock) = fake_catalog_package_lock("torch", None);
        let mut descriptor = descriptor.unwrap_catalog_descriptor().unwrap();
        descriptor.pkg_path = "flox-cuda/python3Packages.torch".to_string();
        lock.version = "2.7.1".to_string();

        let mut out = Vec::new();
        List::print_extended(&mut out, &[PackageToList::Catalog(descriptor, lock)], None).unwrap();

        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("flox-cuda/python3Packages.torch")
        );
    }

    /// Test catalog prefix displays in detailed output (flox/catalog-util case)
    #[test]
    fn test_catalog_prefix_in_detail_output() {
        let (_iid, descriptor, mut lock) = fake_catalog_package_lock("catalog-util", None);
        let mut descriptor = descriptor.unwrap_catalog_descriptor().unwrap();
        descriptor.pkg_path = "flox/catalog-util".to_string();
        lock.version = "0.1.0".to_string();

        let mut out = Vec::new();
        List::print_detail(&mut out, &[PackageToList::Catalog(descriptor, lock)], None).unwrap();

        assert!(
            String::from_utf8(out)
                .unwrap()
                .contains("Package Path:         flox/catalog-util")
        );
    }

    /// manifest_contents_to_print puts items in the same table with dotted
    /// subtables for composed environments
    #[test]
    fn print_config_puts_packages_in_same_table() {
        let (flox, tempdir) = flox_instance();

        // Create dep environment
        let dep_path = tempdir.path().join("dep");
        let dep_manifest_contents = with_latest_schema(indoc! {r#"
            [services]
            sleep2.command = "sleep infinity"
            sleep2.is-daemon = true
            sleep2.shutdown.command = "cmd"
        "#});

        fs::create_dir(&dep_path).unwrap();
        let mut dep = new_path_environment_in(&flox, &dep_manifest_contents, &dep_path);
        dep.lockfile(&flox).unwrap();

        // Create composer environment
        let composer_path = tempdir.path().join("composer");
        let composer_manifest_contents = with_latest_schema(indoc! {r#"
            [include]
            environments = [
                { dir = "../dep" }
            ]

            [services]
            sleep1.command = "sleep infinity"
            sleep1.is-daemon = true
            sleep1.shutdown.command = "cmd"
        "#});
        fs::create_dir(&composer_path).unwrap();
        let mut composer =
            new_path_environment_in(&flox, &composer_manifest_contents, &composer_path);
        let lockfile: Lockfile = composer.lockfile(&flox).unwrap().into();

        assert_eq!(
            List::manifest_contents_to_print(
                &lockfile,
                composer
                    .manifest_without_migrating(&flox)
                    .unwrap()
                    .as_writable()
                    .to_string()
            )
            .unwrap(),
            with_latest_schema(indoc! {r#"
                [services]
                sleep1.command = "sleep infinity"
                sleep1.is-daemon = true
                sleep1.shutdown.command = "cmd"
                sleep2.command = "sleep infinity"
                sleep2.is-daemon = true
                sleep2.shutdown.command = "cmd""#})
        );
    }

    /// Test that --upstream errors with path environment
    #[tokio::test]
    async fn list_upstream_errors_with_path_environment() {
        let (flox, tempdir) = flox_instance();

        let path_manifest = indoc! {r#"
            version = 1
            [install]
            hello.pkg-path = "hello"
        "#};
        let path_env = new_path_environment_in(&flox, path_manifest, tempdir.path());
        let result = List {
            environment: EnvironmentSelect::Dir(path_env.project_path().unwrap()),
            upstream: true,
            list_mode: ListMode::All,
        }
        .handle(Config::default(), flox)
        .await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("cannot be used with path environments")
        )
    }

    fn timestamp(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, 12, 0, 0).unwrap()
    }

    fn managed_details() -> EnvironmentDetails {
        EnvironmentDetails {
            name: "owner/backend".to_string(),
            floxhub_url: Some(Url::parse("https://hub.flox.dev/owner/backend").unwrap()),
            path: Some(PathBuf::from("/home/user/backend")),
            system: "aarch64-darwin".to_string(),
            systems: Some(vec![
                "aarch64-darwin".to_string(),
                "x86_64-linux".to_string(),
            ]),
            generations: Some(GenerationDetails {
                local: Some(LocalGeneration::Live {
                    live: 10.into(),
                    latest: 12.into(),
                    local_changes: true,
                }),
                floxhub: Some(FloxHubState {
                    generation: 13.into(),
                    sync: BranchOrd::Behind,
                    last_auto_upgrade: Some(AutoUpgrade {
                        generation: 13.into(),
                        timestamp: timestamp(27),
                    }),
                }),
            }),
            upgrade_notices: UpgradeNotices::On,
            upgrades: UpgradeStatus::Available {
                checked: timestamp(28),
                upgrades: upgrade_diff(),
            },
        }
    }

    /// A version change of `curl` and a rebuild of `hello`
    fn upgrade_diff() -> SingleSystemUpgradeDiff {
        let (_, _, mut curl) = fake_catalog_package_lock("curl", None);
        curl.version = "8.9.0".to_string();
        let mut curl_upgraded = curl.clone();
        curl_upgraded.version = "8.10.1".to_string();

        let (_, _, mut hello) = fake_catalog_package_lock("hello", None);
        hello.version = "2.12.1".to_string();
        let mut hello_rebuilt = hello.clone();
        hello_rebuilt.rev_date = timestamp(26);

        SingleSystemUpgradeDiff::from_iter([
            (
                "curl_install_id".to_string(),
                (
                    LockedPackage::Catalog(curl),
                    LockedPackage::Catalog(curl_upgraded),
                ),
            ),
            (
                "hello_install_id".to_string(),
                (
                    LockedPackage::Catalog(hello),
                    LockedPackage::Catalog(hello_rebuilt),
                ),
            ),
        ])
    }

    fn path_details() -> EnvironmentDetails {
        EnvironmentDetails {
            name: "project".to_string(),
            floxhub_url: None,
            path: Some(PathBuf::from("/home/user/project")),
            system: "aarch64-darwin".to_string(),
            systems: None,
            generations: None,
            upgrade_notices: UpgradeNotices::On,
            upgrades: UpgradeStatus::NotChecked,
        }
    }

    #[test]
    fn print_environment_details_managed_environment() {
        let mut out = Vec::new();
        print_environment_details(&mut out, &managed_details()).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), indoc! {"
            Environment:      owner/backend
            FloxHub URL:      https://hub.flox.dev/owner/backend
            Path:             /home/user/backend
            System:           aarch64-darwin
            Systems:          aarch64-darwin, x86_64-linux
            Generation:       10 (live, latest is 12, with local changes)
            FloxHub:          generation 13
            Auto-upgrade:     last upgraded 2026-09-27 12:00:00 UTC (generation 13)
            Upgrade notices:  on
            Upgrades:         1 version change and 1 rebuild
            Upgrades checked: 2026-09-28 12:00:00 UTC
        "});
    }

    /// A remote environment has no local path, and the local copy may have
    /// diverged from FloxHub.
    #[test]
    fn print_environment_details_remote_environment_pinned_after_fetch() {
        let details = EnvironmentDetails {
            name: "owner/tools".to_string(),
            floxhub_url: Some(Url::parse("https://hub.flox.dev/owner/tools").unwrap()),
            path: None,
            system: "x86_64-linux".to_string(),
            systems: None,
            generations: Some(GenerationDetails {
                local: Some(LocalGeneration::Pinned {
                    pinned: 2.into(),
                    live: 3.into(),
                }),
                floxhub: Some(FloxHubState {
                    generation: 3.into(),
                    sync: BranchOrd::Diverged,
                    last_auto_upgrade: None,
                }),
            }),
            upgrade_notices: UpgradeNotices::DisabledByConfig,
            upgrades: UpgradeStatus::NotCovered {
                checked: timestamp(28),
            },
        };

        let mut out = Vec::new();
        print_environment_details(&mut out, &details).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), indoc! {"
            Environment:      owner/tools
            FloxHub URL:      https://hub.flox.dev/owner/tools
            System:           x86_64-linux
            Generation:       2 (pinned by 'flox activate --generation', live is 3)
            FloxHub:          generation 3 (diverged from the local copy)
            Auto-upgrade:     none in FloxHub's history
            Upgrade notices:  off (upgrade_notifications = false in 'flox config')
            Upgrades:         unknown, only the live generation is checked
            Upgrades checked: 2026-09-28 12:00:00 UTC
        "});
    }

    /// Path environments have no generations, and an empty package list is
    /// explained when the current system isn't one of the environment's.
    #[test]
    fn print_environment_details_path_environment_on_other_system() {
        let details = EnvironmentDetails {
            systems: Some(vec!["x86_64-linux".to_string()]),
            upgrade_notices: UpgradeNotices::DisabledByManifest,
            ..path_details()
        };

        let mut out = Vec::new();
        print_environment_details(&mut out, &details).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), indoc! {"
            Environment:      project
            Path:             /home/user/project
            System:           aarch64-darwin (not in the environment's systems)
            Systems:          x86_64-linux
            Upgrade notices:  off (options.activate.upgrade-notifications = false)
            Upgrades:         unknown, 'flox activate' hasn't checked yet
        "});
    }

    /// Upgrade information for the lockfile of the hello environment, where
    /// the upgrade changes the `hello` derivation for the given systems.
    fn upgrade_information(upgraded_systems: &[&str]) -> (Lockfile, UpgradeInformation) {
        let lockfile = Lockfile::from_str(
            &fs::read_to_string(GENERATED_DATA.join("envs/hello/manifest.lock")).unwrap(),
        )
        .unwrap();
        let mut new_lockfile = lockfile.clone();
        for package in new_lockfile.packages.iter_mut() {
            if let LockedPackage::Catalog(package) = package
                && upgraded_systems.contains(&package.system.as_str())
            {
                package.derivation = "upgraded".to_string();
            }
        }
        let info = UpgradeInformation {
            last_checked: OffsetDateTime::from_unix_timestamp(timestamp(28).timestamp()).unwrap(),
            upgrade_result: UpgradeResult {
                old_lockfile: Some(lockfile.clone()),
                new_lockfile,
                store_path: None,
            },
        };
        (lockfile, info)
    }

    #[test]
    fn upgrade_status_from_cache() {
        let system = "aarch64-darwin";
        let checked = timestamp(28);

        let (lockfile, _) = upgrade_information(&[]);
        assert_eq!(
            UpgradeStatus::from_cache(None, &lockfile, system, true),
            UpgradeStatus::NotChecked
        );

        let (lockfile, info) = upgrade_information(&[system]);
        let mut changed_lockfile = lockfile.clone();
        changed_lockfile.packages.clear();
        assert_eq!(
            UpgradeStatus::from_cache(Some(&info), &changed_lockfile, system, true),
            UpgradeStatus::Outdated { checked }
        );
        // e.g. a pinned generation, which the check doesn't cover
        assert_eq!(
            UpgradeStatus::from_cache(Some(&info), &changed_lockfile, system, false),
            UpgradeStatus::NotCovered { checked }
        );

        let (lockfile, info) = upgrade_information(&[]);
        assert_eq!(
            UpgradeStatus::from_cache(Some(&info), &lockfile, system, true),
            UpgradeStatus::UpToDate { checked }
        );

        let (lockfile, info) = upgrade_information(&["x86_64-linux"]);
        assert_eq!(
            UpgradeStatus::from_cache(Some(&info), &lockfile, system, true),
            UpgradeStatus::OtherSystemsOnly { checked }
        );

        // e.g. FloxHub's lockfile with `--upstream`, if it matches the local one
        let (lockfile, info) = upgrade_information(&[system]);
        assert_eq!(
            UpgradeStatus::from_cache(Some(&info), &lockfile, system, false),
            UpgradeStatus::Available {
                checked,
                upgrades: info.upgrade_result.diff_for_system(system),
            }
        );
    }

    #[test]
    fn last_auto_upgrade_is_newest_upgrade_by_floxhub() {
        let changes = [
            ("alice", HistoryKind::Install {
                targets: vec!["hello".to_string()],
            }),
            ("FloxHub", HistoryKind::Upgrade { targets: vec![] }),
            // written by older versions of FloxHub
            ("floxEM", HistoryKind::Upgrade { targets: vec![] }),
            ("alice", HistoryKind::Upgrade { targets: vec![] }),
            ("FloxHub", HistoryKind::Edit),
        ];

        let mut metadata = AllGenerationsMetadata::default();
        assert_eq!(last_auto_upgrade(metadata.history()), None);

        for (day, (author, kind)) in (1..).zip(changes) {
            metadata.add_generation(AddGenerationOptions {
                author: author.to_string(),
                hostname: "host".to_string(),
                argv: vec![],
                timestamp: timestamp(day),
                kind,
            });
        }

        assert_eq!(
            last_auto_upgrade(metadata.history()),
            Some(AutoUpgrade {
                generation: 3.into(),
                timestamp: timestamp(3),
            })
        );
    }

    #[test]
    fn next_step_prefers_local_changes_then_floxhub_then_upgrades() {
        let checked = timestamp(28);
        let flags = " -d /home/user/backend";
        let available = UpgradeStatus::Available {
            checked,
            upgrades: upgrade_diff(),
        };
        let with_state = |sync: BranchOrd, local: LocalGeneration, upgrades: &UpgradeStatus| {
            let mut details = managed_details();
            let generations = details.generations.as_mut().unwrap();
            generations.floxhub.as_mut().unwrap().sync = sync;
            generations.local = Some(local);
            details.upgrades = upgrades.clone();
            details
        };
        let live = |local_changes| LocalGeneration::Live {
            live: 1.into(),
            latest: 1.into(),
            local_changes,
        };
        let pinned = LocalGeneration::Pinned {
            pinned: 1.into(),
            live: 2.into(),
        };
        let path_outdated = EnvironmentDetails {
            upgrades: UpgradeStatus::Outdated { checked },
            ..path_details()
        };

        let cases = [
            (
                with_state(BranchOrd::Diverged, live(true), &available),
                true,
                Some("Use 'flox edit --sync -d /home/user/backend' to commit your local changes to a new generation.".to_string()),
            ),
            (
                with_state(BranchOrd::Behind, live(false), &available),
                true,
                Some("Use 'flox pull -d /home/user/backend' to fetch updates from FloxHub.".to_string()),
            ),
            (
                with_state(BranchOrd::Ahead, pinned.clone(), &available),
                true,
                Some("Use 'flox push -d /home/user/backend' to update the environment on FloxHub.".to_string()),
            ),
            (
                with_state(BranchOrd::Diverged, live(false), &available),
                false,
                Some(indoc! {"
                    Use 'flox pull --force -d /home/user/backend' to replace the local copy with FloxHub's version.
                    Use 'flox push --force -d /home/user/backend' to replace FloxHub's version with the local copy."}
                .to_string()),
            ),
            (
                with_state(BranchOrd::Equal, pinned, &UpgradeStatus::NotChecked),
                true,
                None,
            ),
            (
                with_state(BranchOrd::Equal, live(false), &available),
                true,
                Some("Use 'flox upgrade -d /home/user/backend' to apply the upgrades.".to_string()),
            ),
            (
                with_state(BranchOrd::Equal, live(false), &UpgradeStatus::UpToDate { checked }),
                true,
                None,
            ),
            (
                path_outdated.clone(),
                true,
                Some("Use 'flox upgrade --dry-run -d /home/user/backend' to see the upgrades available now.".to_string()),
            ),
            // no packages to upgrade
            (path_outdated, false, None),
        ];

        for (details, has_packages, expected) in cases {
            assert_eq!(
                next_step(&details, has_packages, flags),
                expected,
                "{details:?} {has_packages}"
            );
        }
    }

    /// Local changes are only checked for the live generation,
    /// because a pinned generation is read from the local generation
    /// metadata rather than the local checkout.
    #[test]
    fn generation_details_for_managed_environment() {
        let owner = "owner".parse().unwrap();
        let (flox, _tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let environment = mock_managed_environment_unlocked(&flox, "version = 1", owner);
        fs::write(
            environment.manifest_path(&flox).unwrap(),
            "version = 1\n\n[vars]\nfoo = \"bar\"\n",
        )
        .unwrap();

        let live =
            GenerationDetails::gather(&environment, || environment.has_local_changes(&flox).ok());
        assert_eq!(live, GenerationDetails {
            local: Some(LocalGeneration::Live {
                live: 1.into(),
                latest: 1.into(),
                local_changes: true,
            }),
            floxhub: Some(FloxHubState {
                generation: 1.into(),
                sync: BranchOrd::Equal,
                last_auto_upgrade: None,
            }),
        });

        let pinned_environment = ManagedEnvironment::open(
            &flox,
            environment.pointer().clone(),
            environment.dot_flox_path(),
            Some(1.into()),
        )
        .unwrap();
        let pinned = GenerationDetails::gather(&pinned_environment, || {
            panic!("local changes don't apply to a pinned generation")
        });
        assert_eq!(pinned, GenerationDetails {
            local: Some(LocalGeneration::Pinned {
                pinned: 1.into(),
                live: 1.into(),
            }),
            floxhub: Some(FloxHubState {
                generation: 1.into(),
                sync: BranchOrd::Equal,
                last_auto_upgrade: None,
            }),
        });
    }
}
