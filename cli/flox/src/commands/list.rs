use std::collections::BTreeSet;
use std::io::{Write, stderr, stdout};
use std::str::FromStr;

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_config::Config;
use flox_events::{CliEnvironmentPayload, EventKind, EventsHub};
use flox_manifest::interfaces::{AsLatestSchema, AsWritableManifest, WriteManifest};
use flox_manifest::lockfile::{LockedInstallable, LockedPackageFlake, Lockfile, PackageToList};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::floxmeta_branch::BranchOrd;
use flox_rust_sdk::models::environment::generations::GenerationsExt;
use flox_rust_sdk::models::environment::{
    ConcreteEnvironment,
    Environment,
    SingleSystemUpgradeDiff,
};
use flox_rust_sdk::providers::buildenv::get_installed_outputs;
use flox_rust_sdk::providers::upgrade_checks::UpgradeInformationGuard;
use indoc::formatdoc;
use itertools::Itertools;
use time::format_description::well_known::Rfc3339;
use tracing::{debug, instrument};

use super::{EnvironmentSelect, environment_select};
use crate::commands::render_composition_manifest;
use crate::environment_subcommand_metric;
use crate::utils::events::env_detail_from_concrete;
use crate::utils::message;
use crate::utils::tracing::sentry_set_tag;
use crate::utils::upgrade_output::{rebuild_detail, render_diff};

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

    /// Show all available package and environment information
    #[bpaf(long, short)]
    All,
}

/// Upgrade status derived entirely from on-disk data, with no network fetch.
#[derive(Debug)]
enum UpgradesStatus {
    /// The upgrade check cache matches the current lockfile.
    Available(SingleSystemUpgradeDiff),
    /// The cache exists but was written against a different lockfile.
    ChangedSinceCheck,
    /// Upgrades are available for other systems but not the current one.
    OtherSystemsOnly,
    /// The cache file does not exist (no background check has run yet).
    NotChecked,
    /// The cache exists and all packages are up to date.
    None,
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

        let system = &flox.system;
        let packages = lockfile.list_packages(system)?;

        match self.list_mode {
            ListMode::NameOnly | ListMode::Extended if packages.is_empty() => {
                let message = formatdoc! {"
                    No packages are installed for your current system ('{system}').

                    You can see the whole manifest with 'flox list --config'.
                "};
                message::warning(message);
            },
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
                let (upgrades_status, upgrades_checked_at) = if self.upstream {
                    (None, None)
                } else {
                    let (status, checked_at) =
                        Self::get_upgrades_status(&flox, &mut env, &lockfile)?;
                    (Some(status), checked_at)
                };

                let env_detail = EnvDetail::gather(
                    &flox,
                    &env,
                    &lockfile,
                    upgrades_checked_at,
                    config.flox.upgrade_notifications,
                );

                Self::print_all(
                    stdout().lock(),
                    stderr().lock(),
                    &packages,
                    &lockfile,
                    env_detail,
                    upgrades_status,
                    system,
                )?;
            },
            ListMode::Config => unreachable!(),
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

    /// Print the full `--all` view: environment-detail block, packages with
    /// upgrade/stability lines, available-upgrades summary, and a next-step
    /// hint on stderr.
    ///
    /// Every line in the environment-detail block is best-effort: a line that
    /// can't be read is silently omitted rather than failing the command.
    fn print_all(
        mut out: impl Write,
        mut err: impl Write,
        packages: &[PackageToList],
        lockfile: &Lockfile,
        env_detail: EnvDetail,
        upgrades_status: Option<UpgradesStatus>,
        system: &str,
    ) -> Result<()> {
        // --- Environment detail block ---
        if let Some(ref name) = env_detail.display_name {
            writeln!(&mut out, "Environment:          {name}")?;
        }
        if let Some(ref url) = env_detail.floxhub_url {
            writeln!(&mut out, "FloxHub URL:          {url}")?;
        }
        if let Some(ref path) = env_detail.path {
            writeln!(&mut out, "Path:                 {path}")?;
        }

        // System line — note when the current system isn't in the env's systems
        let systems = locked_systems(lockfile);
        // Prefer manifest options.systems (pre-computed in gather); fall back
        // to the set of locked systems when the manifest couldn't be read.
        let effective_systems: Vec<String> = if !env_detail.effective_systems.is_empty() {
            env_detail.effective_systems.clone()
        } else {
            systems.iter().sorted().cloned().collect()
        };

        // Note when the current system is absent from the declared set.
        // `locked_systems` always contains the current system (handle bails
        // when list_packages returns empty), so checking it would make the
        // else branch unreachable.  Use the declared set instead.
        if effective_systems.is_empty() || effective_systems.iter().any(|s| s == system) {
            writeln!(&mut out, "System:               {system}")?;
        } else {
            writeln!(
                &mut out,
                "System:               {system} (not in the environment's systems)"
            )?;
        }
        if !effective_systems.is_empty() {
            writeln!(
                &mut out,
                "Systems:              {}",
                effective_systems.join(", ")
            )?;
        }

        // Generation line
        if let Some(ref gen_line) = env_detail.generation_line {
            writeln!(&mut out, "Generation:           {gen_line}")?;
        }

        // FloxHub sync status
        if let Some(ref floxhub_gen_line) = env_detail.floxhub_gen_line {
            writeln!(&mut out, "FloxHub:              {floxhub_gen_line}")?;
        }

        // Upgrade notices setting
        if let Some(ref notices_line) = env_detail.upgrade_notices_line {
            writeln!(&mut out, "Upgrade notices:      {notices_line}")?;
        }

        // Upgrades status from the background-check cache
        if let Some(ref status) = upgrades_status {
            let upgrades_line = match status {
                UpgradesStatus::Available(_) => "available".to_string(),
                UpgradesStatus::None => "none".to_string(),
                UpgradesStatus::NotChecked => "not checked".to_string(),
                UpgradesStatus::ChangedSinceCheck => "changed since check".to_string(),
                UpgradesStatus::OtherSystemsOnly => "other systems only".to_string(),
            };
            writeln!(&mut out, "Upgrades:             {upgrades_line}")?;
        }

        // When the cache was last written
        if let Some(ref checked_at) = env_detail.upgrades_checked_at {
            writeln!(&mut out, "Upgrades checked:     {checked_at}")?;
        }

        writeln!(&mut out)?;

        // --- Packages ---
        writeln!(&mut out, "Packages:")?;
        writeln!(&mut out)?;

        if packages.is_empty() {
            writeln!(&mut out, "  No packages installed for {system}.")?;
            return Ok(());
        }

        let maybe_diff = upgrades_status.as_ref().and_then(|s| match s {
            UpgradesStatus::Available(diff) => Some(diff),
            _ => None,
        });

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
                PackageToList::Catalog(_, p) => p.install_id.as_str(),
                PackageToList::Flake(_, p) => p.install_id.as_str(),
                PackageToList::StorePath(p) => p.install_id.as_str(),
            };

            let upgrade_line = maybe_diff.and_then(|diff| {
                diff.get(install_id).map(|(before, after)| {
                    let old_v = before.version().unwrap_or("unknown");
                    let new_v = after.version().unwrap_or("unknown");
                    if new_v != old_v {
                        format!("  Upgrade available:    {old_v} -> {new_v}")
                    } else {
                        match rebuild_detail(before, after) {
                            Some(d) => format!("  Upgrade available:    rebuild ({d})"),
                            None => "  Upgrade available:    rebuild".to_string(),
                        }
                    }
                })
            });

            let stability_line = match package {
                PackageToList::Catalog(_, locked) => {
                    locked.stabilities.as_deref().and_then(|stabs| {
                        if stabs.is_empty() {
                            None
                        } else {
                            Some(format!("  Stability:            {}", stabs.join(", ")))
                        }
                    })
                },
                _ => None,
            };

            let pkg_block = format_package_block_all(package, upgrade_line, stability_line);

            if idx < packages.len() - 1 {
                writeln!(&mut out, "{pkg_block}")?;
            } else {
                write!(&mut out, "{pkg_block}")?;
            }
        }

        // --- Available upgrades summary ---
        if let Some(UpgradesStatus::Available(diff)) = upgrades_status.as_ref()
            && !diff.is_empty()
        {
            writeln!(&mut out)?;
            writeln!(&mut out, "Available upgrades:")?;
            let rendered = render_diff(diff);
            writeln!(&mut out, "{rendered}")?;
        }

        // --- Next-step hints on stderr, one "{reason}: {resolution}" per line ---
        let mut hints: Vec<String> = Vec::new();

        if env_detail.has_local_changes {
            hints.push("Local changes not synced: 'flox edit --sync'".to_string());
        } else if matches!(
            env_detail.branch_ord,
            Some(BranchOrd::Behind) | Some(BranchOrd::Diverged)
        ) {
            hints.push("Local copy is behind FloxHub: 'flox pull'".to_string());
        } else if matches!(env_detail.branch_ord, Some(BranchOrd::Ahead)) {
            hints.push("Unpushed changes: 'flox push'".to_string());
        }

        match upgrades_status {
            Some(UpgradesStatus::Available(_)) => {
                hints.push("Upgrades available: 'flox upgrade'".to_string())
            },
            Some(UpgradesStatus::NotChecked)
            | Some(UpgradesStatus::OtherSystemsOnly)
            | Some(UpgradesStatus::ChangedSinceCheck) => {
                hints.push("Upgrade status unknown: 'flox upgrade --dry-run'".to_string())
            },
            _ => {},
        }

        if !hints.is_empty() {
            writeln!(&mut err)?;
            for hint in &hints {
                writeln!(&mut err, "{hint}")?;
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

    /// Determine the upgrade status from the on-disk cache, without any
    /// network fetch.  The cache may reference a lockfile that differs from
    /// the current one (i.e. after an install or edit).
    ///
    /// Returns `(status, checked_at)` so the caller can pass the timestamp
    /// into `EnvDetail::gather` without a second `read_in` call.
    fn get_upgrades_status(
        flox: &Flox,
        environment: &mut ConcreteEnvironment,
        current_lockfile: &Lockfile,
    ) -> Result<(UpgradesStatus, Option<String>)> {
        let upgrade_guard = UpgradeInformationGuard::read_in(environment.cache_path()?)?;
        let Some(info) = upgrade_guard.info() else {
            return Ok((UpgradesStatus::NotChecked, None));
        };

        let checked_at = info
            .last_checked
            .format(&Rfc3339)
            .ok()
            .map(|s| s.to_string());

        if info.upgrade_result.old_lockfile.as_ref() != Some(current_lockfile) {
            debug!("Upgrade cache references a different lockfile");
            return Ok((UpgradesStatus::ChangedSinceCheck, checked_at));
        }

        // Call diff() once and inspect the full map, then extract the
        // current-system slice without a second walk.
        let diff_all = info.upgrade_result.diff();
        let has_any = !diff_all.is_empty();
        let diff_current: SingleSystemUpgradeDiff = diff_all
            .into_iter()
            .filter_map(|(install_id, mut by_system)| {
                by_system
                    .remove(&flox.system)
                    .map(|pair| (install_id, pair))
            })
            .collect();

        if diff_current.is_empty() && has_any {
            return Ok((UpgradesStatus::OtherSystemsOnly, checked_at));
        }

        if diff_current.is_empty() {
            return Ok((UpgradesStatus::None, checked_at));
        }

        Ok((UpgradesStatus::Available(diff_current), checked_at))
    }
}

/// Environment-level metadata gathered for the `--all` display block.
/// All fields are `Option` — a field that can't be read is simply omitted.
#[derive(Debug, Default)]
struct EnvDetail {
    /// Human-readable name: `<name>` for path envs, `<owner>/<name>` for
    /// managed/remote envs.
    display_name: Option<String>,
    /// FloxHub URL for managed/remote environments.
    floxhub_url: Option<String>,
    /// Filesystem path to the `.flox` parent directory for local environments.
    path: Option<String>,
    /// "<N> (live)", "<N> (pinned by --generation)", "<N> with local changes",
    /// or "<N> (newer generation exists after rollback)".
    generation_line: Option<String>,
    /// FloxHub generation description: the remote current gen + diverged note.
    floxhub_gen_line: Option<String>,
    /// `on` / `off (<setting>)` from `options.activate.upgrade-notifications`.
    upgrade_notices_line: Option<String>,
    /// RFC-3339 timestamp of the last upgrade check.
    upgrades_checked_at: Option<String>,
    /// Whether the local `.flox/env` differs from the current generation.
    has_local_changes: bool,
    /// Local vs. remote branch comparison, for the next-step hint.
    branch_ord: Option<BranchOrd>,
    /// Systems declared in the manifest or present in the lockfile.
    effective_systems: Vec<String>,
}

impl EnvDetail {
    /// Gather all display metadata for an environment, silently skipping any
    /// field that produces an error.
    ///
    /// `upgrades_checked_at` is the timestamp from the upgrade cache, already
    /// read by `get_upgrades_status` — pass it here to avoid a second
    /// `read_in` call.
    ///
    /// `config_upgrade_notifications` is the per-user `upgrade_notifications`
    /// setting from `flox config`.  It is read locally and requires no network.
    fn gather(
        flox: &Flox,
        env: &ConcreteEnvironment,
        lockfile: &Lockfile,
        upgrades_checked_at: Option<String>,
        config_upgrade_notifications: Option<bool>,
    ) -> Self {
        // Display name
        let display_name = Some(match env {
            ConcreteEnvironment::Managed(m) => format!("{}/{}", m.owner(), m.pointer().name),
            ConcreteEnvironment::Remote(r) => {
                format!("{}/{}", r.pointer().owner, r.pointer().name)
            },
            ConcreteEnvironment::Path(p) => p.name().to_string(),
        });

        // FloxHub URL (managed and remote environments only)
        let floxhub_url = match env {
            ConcreteEnvironment::Managed(m) => {
                m.pointer().floxhub_url().ok().map(|u| u.to_string())
            },
            ConcreteEnvironment::Remote(r) => r.pointer().floxhub_url().ok().map(|u| u.to_string()),
            ConcreteEnvironment::Path(_) => None,
        };

        // Filesystem path (local and managed environments)
        let path = match env {
            ConcreteEnvironment::Path(p) => p.parent_path().ok().map(|p| p.display().to_string()),
            ConcreteEnvironment::Managed(m) => {
                m.parent_path().ok().map(|p| p.display().to_string())
            },
            ConcreteEnvironment::Remote(_) => None,
        };

        // Generation, sync state, and local-change flag (managed and remote).
        // Compute `branch_ord` and `has_local_changes` once here and pass into
        // helpers to avoid redundant calls.
        let (generation_line, has_local_changes, branch_ord, floxhub_gen_line) = match env {
            ConcreteEnvironment::Managed(m) => {
                let local_changes = m.has_local_changes(flox).unwrap_or(false);
                let ord = m.compare_remote().ok();
                let gen_line = generation_line_for_managed(m, local_changes);
                let hub_line = floxhub_gen_line_for_managed(m, ord.as_ref());
                (gen_line, local_changes, ord, hub_line)
            },
            ConcreteEnvironment::Remote(r) => {
                let gen_line = r
                    .generations_metadata()
                    .ok()
                    .and_then(|meta| meta.current_gen().map(|g| g.to_string()));
                let ord = r.compare_remote().ok();
                let hub_line = floxhub_gen_line_for_remote(r, ord.as_ref());
                (gen_line, false, ord, hub_line)
            },
            ConcreteEnvironment::Path(_) => (None, false, None, None),
        };

        // Call migrated_manifest() once and read both options fields from it.
        let (upgrade_notices_line, effective_systems) =
            lockfile.migrated_manifest().ok().map_or_else(
                || (None, Vec::new()),
                |migrated| {
                    let latest = migrated.as_latest_schema();
                    // Compute the effective upgrade-notifications state from
                    // both the per-user config key and the manifest option.
                    // The config key takes precedence: if either source disables
                    // it, we report which one did.  Both are local reads.
                    let notices = if config_upgrade_notifications == Some(false) {
                        Some("off (upgrade_notifications = false in 'flox config')".to_string())
                    } else if latest.options.activate.upgrade_notifications == Some(false) {
                        Some("off (options.activate.upgrade-notifications = false)".to_string())
                    } else {
                        Some("on".to_string())
                    };
                    let systems: Vec<String> = latest
                        .options
                        .systems
                        .clone()
                        .map(|s| {
                            let mut v: Vec<String> =
                                s.into_iter().map(|sys| sys.to_string()).collect();
                            v.sort();
                            v
                        })
                        .unwrap_or_default();
                    (notices, systems)
                },
            );

        EnvDetail {
            display_name,
            floxhub_url,
            path,
            generation_line,
            floxhub_gen_line,
            upgrade_notices_line,
            upgrades_checked_at,
            has_local_changes,
            branch_ord,
            effective_systems,
        }
    }
}

/// Build the "Generation" line for a managed environment.
///
/// Shows "<N> (live)", "<N> (pinned by --generation)", "<N> with local changes",
/// or "<N> (newer generation exists after rollback)".
///
/// `has_local_changes` must be pre-computed by the caller so this function
/// does not repeat the `has_local_changes` call made by `EnvDetail::gather`.
fn generation_line_for_managed(
    managed: &flox_rust_sdk::models::environment::managed_environment::ManagedEnvironment,
    has_local_changes: bool,
) -> Option<String> {
    let generation = managed.generation().ok()??;
    let gen_num = *generation;

    let local_meta = managed.generations_metadata().ok()?;
    let current_live = local_meta.current_gen()?;

    // The environment was opened at a specific generation with `--generation`
    if generation != current_live {
        return Some(format!("{gen_num} (pinned by --generation)"));
    }

    if has_local_changes {
        return Some(format!("{gen_num} with local changes"));
    }

    // A newer generation exists — the user has rolled back to this one
    let all_gens = local_meta.generations();
    let highest = all_gens.keys().next_back().copied()?;
    if highest > current_live {
        return Some(format!(
            "{gen_num} (newer generation exists after rollback)"
        ));
    }

    Some(format!("{gen_num} (live)"))
}

/// Return the parenthetical note for a branch-ord comparison result.
///
/// Empty string for `Equal`; the returned slice is `'static` so it can be
/// used with `format!` without an allocation.
fn branch_ord_note(ord: &BranchOrd) -> &'static str {
    match ord {
        BranchOrd::Equal => "",
        BranchOrd::Ahead => " (local has unpushed history)",
        BranchOrd::Behind => " (local is behind)",
        BranchOrd::Diverged => " (diverged)",
    }
}

/// Build the "FloxHub" line for a managed environment.
///
/// `ord` must be pre-computed by the caller so this function does not repeat
/// the `compare_remote` call made by `EnvDetail::gather`.
fn floxhub_gen_line_for_managed(
    managed: &flox_rust_sdk::models::environment::managed_environment::ManagedEnvironment,
    ord: Option<&BranchOrd>,
) -> Option<String> {
    let remote_meta = managed.remote_generations_metadata().ok()?;
    let remote_gen = remote_meta.current_gen()?;
    let remote_gen_num = *remote_gen;
    let note = branch_ord_note(ord?);
    Some(format!("generation {remote_gen_num}{note}"))
}

/// Build the "FloxHub" line for a remote environment.
///
/// `ord` must be pre-computed by the caller so this function does not repeat
/// the `compare_remote` call made by `EnvDetail::gather`.
fn floxhub_gen_line_for_remote(
    remote: &flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment,
    ord: Option<&BranchOrd>,
) -> Option<String> {
    let remote_meta = remote.remote_generations_metadata().ok()?;
    let remote_gen = remote_meta.current_gen()?;
    let remote_gen_num = *remote_gen;
    let note = branch_ord_note(ord?);
    Some(format!("generation {remote_gen_num}{note}"))
}

/// Collect the unique set of systems that appear in the lockfile packages.
fn locked_systems(lockfile: &Lockfile) -> BTreeSet<String> {
    lockfile
        .packages
        .iter()
        .map(|p| p.system().to_string())
        .collect()
}

/// Format one package entry for the `--all` package block, appending
/// upgrade and stability lines when present.
fn format_package_block_all(
    package: &PackageToList,
    upgrade_line: Option<String>,
    stability_line: Option<String>,
) -> String {
    // Extra lines are appended after the standard fields, before the outputs
    let extra = {
        let mut lines = String::new();
        if let Some(u) = upgrade_line {
            lines.push_str(&u);
            lines.push('\n');
        }
        if let Some(s) = stability_line {
            lines.push_str(&s);
            lines.push('\n');
        }
        lines
    };

    match package {
        PackageToList::Catalog(descriptor, locked) => {
            let outputs_lines = format_outputs_lines(package);
            formatdoc! {"
                {name}:
                  Description:          {description}
                  Package Path:         {attr_path}
                  Package Name:         {pname}
                  Priority:             {priority}
                  Version:              {version}
                  License:              {license}
                  Unfree:               {unfree}
                  Broken:               {broken}
                {extra}{outputs_lines}",
                name = &locked.install_id,
                pname = &locked.pname,
                attr_path = &descriptor.pkg_path,
                priority = locked.priority,
                version = &locked.version,
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
            {install_id}:
              Description:          {description}
              Locked URL:           {locked_url}
              Flake attribute:      {locked_flake_attr_path}
              Package Name:         {formatted_pname}
              Priority:             {priority}
              Version:              {version}
              {formatted_licenses}
              Unfree:               {unfree}
              Broken:               {broken}
            {extra}{outputs_lines}",
                formatted_pname = pname.as_deref().unwrap_or("N/A"),
                description = description.as_deref().unwrap_or("N/A"),
                version = version.as_deref().unwrap_or("N/A"),
                formatted_licenses = formatted_licenses.as_deref().unwrap_or("License: N/A"),
                unfree = unfree.map(|u|u.to_string()).as_deref().unwrap_or("N/A"),
                broken = broken.map(|b|b.to_string()).as_deref().unwrap_or("N/A"),
            }
        },
        PackageToList::StorePath(locked_package_store_path) => {
            formatdoc! {"
                {install_id}:
                Store Path:           {store_path}
                Priority:             {priority}
                {extra}",
                install_id = locked_package_store_path.install_id,
                store_path = locked_package_store_path.store_path,
                priority = locked_package_store_path.priority,
            }
        },
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

#[cfg(test)]
mod tests {
    use std::fs;

    use flox_manifest::lockfile::LockedPackage;
    use flox_manifest::lockfile::test_helpers::{
        LOCKED_NIX_EVAL_JOBS,
        fake_catalog_package_lock,
        nix_eval_jobs_descriptor,
    };
    use flox_manifest::test_helpers::with_latest_schema;
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment_in;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

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

    /// `print_all` renders all catalog package fields with the correct format.
    #[test]
    fn test_print_all_catalog_package_fields() {
        let packages = test_packages();
        let lockfile = Lockfile::default();
        let env_detail = EnvDetail::default();

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            &packages,
            &lockfile,
            env_detail,
            None,
            "aarch64-darwin",
        )
        .unwrap();
        let out = String::from_utf8(out).unwrap();

        // Both packages present with all expected fields
        assert!(out.contains("pip_install_id:"), "pip package block missing");
        assert!(
            out.contains("Description:          Python package installer"),
            "pip description missing"
        );
        assert!(
            out.contains("Package Path:         python3Packages.pip"),
            "pip path missing"
        );
        assert!(
            out.contains("Package Name:         pip"),
            "pip pname missing"
        );
        assert!(
            out.contains("Priority:             100"),
            "pip priority missing"
        );
        assert!(
            out.contains("Version:              20.3.4"),
            "pip version missing"
        );
        assert!(
            out.contains("License:              MIT"),
            "pip license missing"
        );
        assert!(
            out.contains("Unfree:               true"),
            "pip unfree missing"
        );
        assert!(
            out.contains("Broken:               false"),
            "pip broken missing"
        );
        assert!(
            out.contains("python_install_id:"),
            "python package block missing"
        );
        assert!(
            out.contains("Description:          Python interpreter"),
            "python description missing"
        );
    }

    /// `print_all` renders flake package fields correctly.
    #[test]
    fn test_print_all_flake_package_fields() {
        let packages = [test_flake_package()];
        let lockfile = Lockfile::default();
        let env_detail = EnvDetail::default();

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            &packages,
            &lockfile,
            env_detail,
            None,
            "aarch64-darwin",
        )
        .unwrap();
        let out = String::from_utf8(out).unwrap();

        assert!(out.contains("nix-eval-jobs:"), "package block missing");
        assert!(
            out.contains(
                "Locked URL:           github:nix-community/nix-eval-jobs/c132534bc68eb48479a59a3116ee7ce0f16ce12b"
            ),
            "locked url missing"
        );
        assert!(
            out.contains("Flake attribute:      packages.aarch64-darwin.default"),
            "flake attr missing"
        );
        assert!(
            out.contains("Package Name:         nix-eval-jobs"),
            "pname missing"
        );
        assert!(out.contains("Priority:             5"), "priority missing");
        assert!(
            out.contains("Version:              2.23.0"),
            "version missing"
        );
        assert!(
            out.contains("License:              GPL-3.0"),
            "license missing"
        );
        assert!(
            out.contains("Available Outputs:    [ \"out\" ]"),
            "outputs missing"
        );
    }

    /// `print_all` shows N/A for missing pname on flake packages.
    #[test]
    fn test_print_all_flake_pname_missing() {
        let mut package = test_flake_package();
        if let PackageToList::Flake(_, ref mut locked_package) = package {
            locked_package.locked_installable.pname = None;
        }
        let packages = [package];
        let lockfile = Lockfile::default();
        let env_detail = EnvDetail::default();

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            &packages,
            &lockfile,
            env_detail,
            None,
            "aarch64-darwin",
        )
        .unwrap();
        let out = String::from_utf8(out).unwrap();

        assert!(
            out.contains("Package Name:         N/A"),
            "expected N/A for missing pname"
        );
    }

    /// `print_all` formats multiple licenses with "Licenses:" label.
    #[test]
    fn test_print_all_flake_multiple_licenses() {
        let mut package = test_flake_package();
        if let PackageToList::Flake(_, ref mut locked_package) = package
            && let Some(licenses) = locked_package.locked_installable.licenses.as_mut()
        {
            licenses.push("license 2".to_string());
        }
        let packages = [package];
        let lockfile = Lockfile::default();
        let env_detail = EnvDetail::default();

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            &packages,
            &lockfile,
            env_detail,
            None,
            "aarch64-darwin",
        )
        .unwrap();
        let out = String::from_utf8(out).unwrap();

        assert!(
            out.contains("Licenses:             GPL-3.0, license 2"),
            "multiple licenses label missing"
        );
    }

    /// `print_all` sorts packages by priority (lower number first).
    #[test]
    fn test_print_all_orders_by_priority() {
        let mut packages = test_packages();
        // Give python priority 10 so it sorts before pip (100)
        let PackageToList::Catalog(_, ref mut package_2) = packages[1] else {
            panic!();
        };
        package_2.priority = 10;

        let lockfile = Lockfile::default();
        let env_detail = EnvDetail::default();

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            &packages,
            &lockfile,
            env_detail,
            None,
            "aarch64-darwin",
        )
        .unwrap();
        let out = String::from_utf8(out).unwrap();

        let python_pos = out.find("python_install_id:").unwrap();
        let pip_pos = out.find("pip_install_id:").unwrap();
        assert!(
            python_pos < pip_pos,
            "python (priority 10) should appear before pip (priority 100)"
        );
    }

    /// `print_all` sorts packages by priority — priority 5 before priority 100.
    #[test]
    fn test_print_all_orders_by_priority_low_first() {
        let mut packages = test_packages();
        let PackageToList::Catalog(_, ref mut package_2) = packages[1] else {
            panic!();
        };
        package_2.priority = 5;

        let lockfile = Lockfile::default();
        let env_detail = EnvDetail::default();

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            &packages,
            &lockfile,
            env_detail,
            None,
            "aarch64-darwin",
        )
        .unwrap();
        let out = String::from_utf8(out).unwrap();

        let python_pos = out.find("python_install_id:").unwrap();
        let pip_pos = out.find("pip_install_id:").unwrap();
        assert!(
            python_pos < pip_pos,
            "python (priority 5) should appear before pip (priority 100)"
        );
    }

    /// `print_all` uses N/A for missing optional fields.
    #[test]
    fn test_print_all_handles_missing_values() {
        let packages = [uninformative_package()];
        let lockfile = Lockfile::default();
        let env_detail = EnvDetail::default();

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            &packages,
            &lockfile,
            env_detail,
            None,
            "aarch64-darwin",
        )
        .unwrap();
        let out = String::from_utf8(out).unwrap();

        assert!(
            out.contains("Description:          N/A"),
            "N/A description missing"
        );
        assert!(
            out.contains("License:              N/A"),
            "N/A license missing"
        );
        assert!(
            out.contains("Unfree:               N/A"),
            "N/A unfree missing"
        );
        assert!(
            out.contains("Broken:               N/A"),
            "N/A broken missing"
        );
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

    /// Test catalog prefix displays in --all output (flox/catalog-util case)
    #[test]
    fn test_catalog_prefix_in_all_output() {
        let (_iid, descriptor, mut lock) = fake_catalog_package_lock("catalog-util", None);
        let mut descriptor = descriptor.unwrap_catalog_descriptor().unwrap();
        descriptor.pkg_path = "flox/catalog-util".to_string();
        lock.version = "0.1.0".to_string();

        let packages = [PackageToList::Catalog(descriptor, lock)];
        let lockfile = Lockfile::default();
        let env_detail = EnvDetail::default();

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            &packages,
            &lockfile,
            env_detail,
            None,
            "aarch64-darwin",
        )
        .unwrap();

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

    /// `print_all` emits the environment-detail block, package list header,
    /// and upgrade-available lines.
    #[test]
    fn print_all_includes_env_detail_and_packages() {
        let mut packages = test_packages();
        let PackageToList::Catalog(_, ref mut pip_lock) = packages[0] else {
            unreachable!()
        };
        pip_lock.stabilities = Some(vec!["stable".to_string()]);
        let mut pip_lock_upgraded = pip_lock.clone();
        pip_lock_upgraded.version = "20.4.0".to_string();

        let upgrades = UpgradesStatus::Available(SingleSystemUpgradeDiff::from_iter(vec![(
            "pip_install_id".to_string(),
            (
                LockedPackage::Catalog(pip_lock.clone()),
                LockedPackage::Catalog(pip_lock_upgraded),
            ),
        )]));

        let lockfile = Lockfile::default();
        let env_detail = EnvDetail {
            display_name: Some("my-env".to_string()),
            path: Some("/home/user/project".to_string()),
            ..Default::default()
        };

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            &packages,
            &lockfile,
            env_detail,
            Some(upgrades),
            "aarch64-darwin",
        )
        .unwrap();

        let out = String::from_utf8(out).unwrap();
        let err_str = String::from_utf8(err).unwrap();

        assert!(
            out.contains("Environment:          my-env"),
            "missing env name"
        );
        assert!(
            out.contains("Path:                 /home/user/project"),
            "missing path"
        );
        assert!(out.contains("Packages:"), "missing packages heading");
        assert!(
            out.contains("Upgrade available:    20.3.4 -> 20.4.0"),
            "missing upgrade line"
        );
        assert!(
            out.contains("Stability:            stable"),
            "missing stability line"
        );
        assert!(
            out.contains("Available upgrades:"),
            "missing upgrades section"
        );
        assert!(
            out.contains("pip_install_id: 20.3.4 -> 20.4.0"),
            "missing upgrade entry"
        );
        assert!(err_str.contains("'flox upgrade'"), "missing next-step hint");
        // Phase A omits Auto-upgrade entirely; Phase C will reintroduce it
        // once the setting can be fetched from FloxHub.
        assert!(
            !out.contains("Auto-upgrade:"),
            "Auto-upgrade line should not appear in Phase A output"
        );
    }

    /// Next-step hint for local changes should suggest 'flox edit --sync'.
    #[test]
    fn print_all_next_step_for_local_changes() {
        let packages = test_packages();
        let lockfile = Lockfile::default();
        let env_detail = EnvDetail {
            display_name: Some("env".to_string()),
            has_local_changes: true,
            ..Default::default()
        };

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            &packages,
            &lockfile,
            env_detail,
            Some(UpgradesStatus::None),
            "aarch64-darwin",
        )
        .unwrap();

        let err_str = String::from_utf8(err).unwrap();
        assert!(
            err_str.contains("'flox edit --sync'"),
            "missing edit --sync hint"
        );
    }

    /// When not checked, the next-step hint should suggest 'flox upgrade --dry-run'.
    #[test]
    fn print_all_next_step_for_not_checked() {
        let packages = test_packages();
        let lockfile = Lockfile::default();
        let env_detail = EnvDetail {
            display_name: Some("env".to_string()),
            ..Default::default()
        };

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            &packages,
            &lockfile,
            env_detail,
            Some(UpgradesStatus::NotChecked),
            "aarch64-darwin",
        )
        .unwrap();

        let err_str = String::from_utf8(err).unwrap();
        assert!(
            err_str.contains("'flox upgrade --dry-run'"),
            "missing dry-run hint"
        );
    }

    /// Full-block assertion for a catalog package with upgrade and stability lines.
    ///
    /// Pins the exact column alignment so a spacing regression is caught at
    /// once rather than needing several `contains` checks to triangulate.
    #[test]
    fn print_all_catalog_package_block_exact() {
        let (_iid, descriptor, mut lock) = fake_catalog_package_lock("pip", None);
        let mut descriptor = descriptor.unwrap_catalog_descriptor().unwrap();
        descriptor.pkg_path = "python3Packages.pip".to_string();
        lock.attr_path = "python3Packages.pip".to_string();
        lock.pname = "pip".to_string();
        lock.priority = 100;
        lock.version = "20.3.4".to_string();
        lock.description = Some("Python package installer".to_string());
        lock.license = Some("MIT".to_string());
        lock.unfree = Some(false);
        lock.broken = Some(false);
        lock.stabilities = Some(vec!["stable".to_string()]);

        let mut pip_lock_upgraded = lock.clone();
        pip_lock_upgraded.version = "20.4.0".to_string();

        let upgrades = UpgradesStatus::Available(SingleSystemUpgradeDiff::from_iter(vec![(
            "pip_install_id".to_string(),
            (
                LockedPackage::Catalog(lock.clone()),
                LockedPackage::Catalog(pip_lock_upgraded),
            ),
        )]));

        let packages = [PackageToList::Catalog(descriptor, lock)];
        let lockfile = Lockfile::default();
        let env_detail = EnvDetail::default();

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            &packages,
            &lockfile,
            env_detail,
            Some(upgrades),
            "aarch64-darwin",
        )
        .unwrap();
        let out = String::from_utf8(out).unwrap();

        // Locate the package block and assert its lines exactly.
        // The "Packages:" header comes before the block; check the key lines.
        assert!(
            out.contains(indoc! {"
                pip_install_id:
                  Description:          Python package installer
                  Package Path:         python3Packages.pip
                  Package Name:         pip
                  Priority:             100
                  Version:              20.3.4
                  License:              MIT
                  Unfree:               false
                  Broken:               false
                  Upgrade available:    20.3.4 -> 20.4.0
                  Stability:            stable
            "}),
            "catalog package block does not match expected format:\n{out}"
        );
    }

    /// Full-block assertion for a flake package — no upgrade or stability lines.
    #[test]
    fn print_all_flake_package_block_exact() {
        let packages = [test_flake_package()];
        let lockfile = Lockfile::default();
        let env_detail = EnvDetail::default();

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            &packages,
            &lockfile,
            env_detail,
            None,
            "aarch64-darwin",
        )
        .unwrap();
        let out = String::from_utf8(out).unwrap();

        assert!(
            out.contains(indoc! {"
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
            "}),
            "flake package block does not match expected format:\n{out}"
        );
    }

    /// System line notes when the current system is absent from the declared systems.
    ///
    /// The check uses `effective_systems` (the manifest-declared set), not the
    /// locked set.  The locked set always contains the current system — handle
    /// bails when `list_packages` returns empty — so only the declared set
    /// can produce a "not in systems" annotation.
    #[test]
    fn print_all_system_not_in_env_systems() {
        let packages = test_packages();
        let lockfile = Lockfile::default();

        // Declare only x86_64-linux as a supported system in the env detail,
        // so aarch64-darwin triggers the "not in systems" branch.
        let env_detail = EnvDetail {
            display_name: Some("env".to_string()),
            effective_systems: vec!["x86_64-linux".to_string()],
            ..Default::default()
        };

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            &packages,
            &lockfile,
            env_detail,
            None,
            "aarch64-darwin",
        )
        .unwrap();

        let out = String::from_utf8(out).unwrap();
        assert!(
            out.contains("aarch64-darwin (not in the environment's systems)"),
            "missing system warning; got:\n{out}"
        );
    }

    /// `print_all` with zero packages still prints the env-detail block and
    /// a no-packages notice, rather than bailing without output.
    #[test]
    fn print_all_no_packages_shows_env_detail() {
        let packages: &[PackageToList] = &[];
        let lockfile = Lockfile::default();
        let env_detail = EnvDetail {
            display_name: Some("my-env".to_string()),
            path: Some("/home/user/project".to_string()),
            ..Default::default()
        };

        let mut out = Vec::new();
        let mut err = Vec::new();
        List::print_all(
            &mut out,
            &mut err,
            packages,
            &lockfile,
            env_detail,
            None,
            "aarch64-darwin",
        )
        .unwrap();

        let out = String::from_utf8(out).unwrap();
        assert!(
            out.contains("Environment:          my-env"),
            "env-detail block missing with no packages; got:\n{out}"
        );
        assert!(
            out.contains("Packages:"),
            "packages section header missing; got:\n{out}"
        );
        assert!(
            out.contains("No packages installed for aarch64-darwin"),
            "no-packages notice missing; got:\n{out}"
        );
    }
}
