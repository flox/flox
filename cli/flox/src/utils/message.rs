use std::collections::BTreeMap;
use std::fmt::Display;
use std::io::Write;

use crossterm::style::Stylize;
use flox_core::data::System;
use flox_core::util::message::{format_error, format_updated};
pub use flox_core::util::message::{stderr_supports_color, stdout_supports_color};
use flox_manifest::compose::{COMPOSER_MANIFEST_ID, Warning};
use flox_manifest::interfaces::{AsLatestSchema, PackageLookup};
use flox_manifest::lockfile::{LockedPackage, Lockfile, PackageOutputs, default_systems_change};
use flox_manifest::parsed::Inner;
use flox_manifest::parsed::common::DEFAULT_GROUP_NAME;
use flox_manifest::parsed::latest::{ManifestLatest, SelectedOutputs};
use flox_manifest::raw::PackageToInstall;
use indoc::formatdoc;
use minus::{ExitStrategy, Pager, page_all};
use tracing::{debug, info};

/// The terminal's current width in columns, or 80 if it can't be
/// determined (not connected to a terminal). `textwrap`'s `terminal_size`
/// feature already provides this fallback; wrapped here so callers go
/// through `message::` rather than reaching for `textwrap` directly.
pub(crate) fn terminal_width() -> usize {
    // Measured on stderr, not stdout: everything sized by this renders
    // there -- `message::` goes through `tracing`, whose subscriber writes
    // to stderr, and glow is handed a dup of that same fd. `termwidth()`
    // reads stdout, so `flox activate >out` on a tty would wrap the
    // description at the 80-column fallback instead of the real width.
    terminal_size::terminal_size_of(std::io::stderr())
        .map(|(terminal_size::Width(columns), _)| columns as usize)
        .unwrap_or(80)
}

/// Write a message to stderr.
///
/// This is printed via the message_layer tracing subscriber
fn print_message(v: impl Display) {
    info!("{v}");
}

fn print_message_to_buffer(out: &mut impl Write, v: impl Display) {
    writeln!(out, "{v}").unwrap();
}

/// alias for [print_message]
pub(crate) fn plain(v: impl Display) {
    print_message(v);
}
pub(crate) fn error(v: impl Display) {
    print_message(format_error(v));
}
pub(crate) fn created(v: impl Display) {
    let icon = if stderr_supports_color() {
        "⚡︎".yellow().to_string()
    } else {
        "⚡︎".to_string()
    };
    print_message(std::format_args!("{icon} {v}"));
}
/// double width character, add an additional space for alignment
pub(crate) fn deleted(v: impl Display) {
    let icon = if stderr_supports_color() {
        "━".red().to_string()
    } else {
        "━".to_string()
    };
    print_message(std::format_args!("{icon} {v}"));
}
pub(crate) fn updated(v: impl Display) {
    print_message(format_updated(v));
}
/// Shown only at `-v` verbosity (`flox::utils::message=debug` filter).
pub(crate) fn verbose(v: impl Display) {
    debug!("{v}");
}
/// double width character, add an additional space for alignment
pub(crate) fn info(v: impl Display) {
    let icon = if stderr_supports_color() {
        "ℹ".blue().to_string()
    } else {
        "ℹ".to_string()
    };
    print_message(std::format_args!("{icon} {v}"));
}
/// double width character, add an additional space for alignment
pub(crate) fn warning(v: impl Display) {
    let icon = if stderr_supports_color() {
        "!".yellow().to_string()
    } else {
        "!".to_string()
    };
    print_message(std::format_args!("{icon} {v}"));
}

/// double width character, add an additional space for alignment
pub(crate) fn warning_to_buffer(out: &mut impl Write, v: impl Display) {
    let icon = if stderr_supports_color() {
        "!".yellow().to_string()
    } else {
        "!".to_string()
    };
    print_message_to_buffer(out, std::format_args!("{icon} {v}"));
}

pub(crate) fn package_installed(pkg: &PackageToInstall, environment_description: &str) {
    updated(format!(
        "'{}' installed to environment {environment_description}",
        pkg.id()
    ));
}

/// Page large output to terminal stdout.
/// The output will be printed without a pager if it's not larger than the
/// terminal window or the terminal is not interactive.
pub(crate) fn page_output(s: impl Into<String>) -> anyhow::Result<()> {
    let pager = Pager::new();

    // Allow destructors to run.
    pager.set_exit_strategy(ExitStrategy::PagerQuit)?;
    // Don't use pager if output fits in terminal.
    pager.set_run_no_overflow(false)?;

    pager.set_text(s)?;
    page_all(pager)?;

    Ok(())
}

/// Display a message for packages that were successfully installed for all
/// requested systems.
pub(crate) fn packages_successfully_installed(
    pkgs: &[PackageToInstall],
    environment_description: &str,
) {
    if !pkgs.is_empty() {
        let pkg_list = pkgs
            .iter()
            .map(|p| format!("'{}'", p.id()))
            .collect::<Vec<_>>()
            .join(", ");
        updated(format!(
            "{pkg_list} installed to environment {environment_description}"
        ));
    }
}

/// Display the stability of the pkg-group that each installed catalog
/// package joined.
///
/// Packages installed with `--stability` set it for their pkg-group. Other
/// packages inherit the stability that their pkg-group already has, possibly
/// from an included environment, so it's read from the merged manifest.
pub(crate) fn packages_group_stability(pkgs: &[PackageToInstall], lockfile: &Lockfile) {
    let merged_manifest = match lockfile.migrated_manifest() {
        Ok(merged_manifest) => merged_manifest,
        Err(err) => {
            debug!(%err, "failed to read merged manifest for pkg-group stabilities");
            return;
        },
    };
    let merged_manifest = merged_manifest.as_latest_schema();

    let mut requested_stabilities = BTreeMap::new();
    let mut inheriting_pkgs: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for pkg in pkgs {
        let PackageToInstall::Catalog(pkg) = pkg else {
            continue;
        };
        let group = pkg
            .target_group()
            .unwrap_or_else(|| DEFAULT_GROUP_NAME.to_string());
        match &pkg.stability {
            Some(stability) => {
                requested_stabilities.insert(group, stability);
            },
            None if merged_manifest.group_stability(&group).is_some() => {
                inheriting_pkgs
                    .entry(group)
                    .or_default()
                    .push(format!("'{}'", pkg.id));
            },
            None => {},
        }
    }

    for (group, stability) in requested_stabilities {
        info(format!(
            "pkg-group '{group}' resolves against the '{stability}' stability."
        ));
    }
    for (group, pkgs) in inheriting_pkgs {
        let Some(stability) = merged_manifest.group_stability(&group) else {
            continue;
        };
        info(format!(
            "{} joined pkg-group '{group}', which resolves against the '{stability}' stability.",
            pkgs.join(", ")
        ));
    }
}

/// Display messages for each package that could only be installed for some of
/// the requested systems.
pub(crate) fn packages_installed_with_system_subsets(pkgs: &[PackageToInstall]) {
    for pkg in pkgs.iter() {
        // Sort for deterministic output: `systems()` order follows the catalog
        // response, so without sorting the message order is unstable.
        // Only `None` for flakes, which can't reach this code path anyway.
        let mut systems = pkg.systems().unwrap_or_default();
        systems.sort();
        warning(format!(
            "'{}' installed only for the following systems: {}",
            pkg.id(),
            systems.join(", ")
        ))
    }
}

/// Display a message for packages whose outputs were updated.
pub(crate) fn packages_outputs_updated(
    pkgs: &[(PackageToInstall, SelectedOutputs)],
    environment_description: &str,
) {
    for (pkg, outputs) in pkgs {
        updated(format!(
            "'{}' outputs updated to '{outputs}' in environment {environment_description}",
            pkg.id(),
        ));
    }
}

/// Display a message for packages that were requested but were already installed.
///
/// Packages whose pkg-group or stability `--pkg-group` or `--stability`
/// would have changed get a message of their own, which says that they
/// weren't changed.
pub(crate) fn packages_already_installed(
    pkgs: &[PackageToInstall],
    environment_description: &str,
    lockfile: &Lockfile,
) {
    let merged_manifest = lockfile
        .migrated_manifest()
        .inspect_err(|err| debug!(%err, "failed to read merged manifest for pkg-groups"))
        .ok();
    let mut unchanged_group_messages = Vec::new();
    let mut other_pkgs = Vec::new();
    for pkg in pkgs {
        let unchanged_group_message = merged_manifest.as_ref().and_then(|merged_manifest| {
            unchanged_group_message(pkg, merged_manifest.as_latest_schema())
        });
        match unchanged_group_message {
            Some(msg) => unchanged_group_messages.push(msg),
            None => other_pkgs.push(pkg),
        }
    }

    let already_installed_msg = match other_pkgs.as_slice() {
        [] => None,
        [pkg] => Some(format!(
            "Package with id '{}' already installed to environment {environment_description}",
            pkg.id()
        )),
        pkgs => {
            let joined = pkgs
                .iter()
                .map(|p| format!("'{}'", p.id()))
                .collect::<Vec<_>>();
            let joined = joined.join(", ");
            Some(format!(
                "Packages with ids {joined} already installed to environment {environment_description}"
            ))
        },
    };
    if let Some(msg) = already_installed_msg {
        warning(msg)
    }
    for msg in unchanged_group_messages {
        warning(msg);
    }
}

/// A message for an already installed package whose pkg-group, or the
/// pkg-group's stability, differs from what `--pkg-group` and `--stability`
/// requested, since installing it again changes neither.
fn unchanged_group_message(
    pkg: &PackageToInstall,
    merged_manifest: &ManifestLatest,
) -> Option<String> {
    let PackageToInstall::Catalog(pkg) = pkg else {
        return None;
    };
    let descriptor = merged_manifest.catalog_descriptor_with_id(&pkg.id)?;
    let current_group = descriptor
        .pkg_group
        .unwrap_or_else(|| DEFAULT_GROUP_NAME.to_string());
    // Without `--pkg-group`, only `--stability` applies, to the package's
    // current pkg-group.
    let requested_group = pkg.pkg_group.as_ref().map(|_| {
        pkg.target_group()
            .unwrap_or_else(|| DEFAULT_GROUP_NAME.to_string())
    });
    let current_stability = merged_manifest.group_stability(&current_group);
    let id = &pkg.id;

    let problem = if let Some(requested_group) =
        requested_group.filter(|requested_group| *requested_group != current_group)
    {
        format!(
            "Package '{id}' is already installed in pkg-group '{current_group}', so it was not moved to pkg-group '{requested_group}'."
        )
    } else if let Some(stability) = pkg
        .stability
        .as_ref()
        .filter(|stability| current_stability != Some(stability.as_str()))
    {
        format!(
            "Package '{id}' is already installed in pkg-group '{current_group}', so '--stability {stability}' did not change it."
        )
    } else {
        return None;
    };
    Some(formatdoc! {"
        {problem}
        To apply these options, run 'flox uninstall {id}' and then run 'flox install' again."})
}

pub(crate) fn packages_with_additional_outputs(
    new_pkgs: &[PackageToInstall],
    lockfile: &Lockfile,
    current_system: &System,
) {
    let mut pkgs_with_additional_outputs = vec![];
    let locked_pkgs = lockfile.packages.as_slice();
    // Yes this is n^2, but n is small
    for pkg in new_pkgs.iter() {
        // When the user explicitly selected outputs (^.. or ^out,man) the hint
        // is redundant — they already know what they are and aren't selecting.
        if pkg.outputs().is_some() {
            continue;
        }
        let install_id = pkg.id();
        for locked in locked_pkgs.iter() {
            if (locked.install_id() == install_id) && (locked.system() == current_system) {
                match locked {
                    LockedPackage::Catalog(locked) => {
                        let maybe_matched = locked.outputs_match_outputs_to_install();
                        if maybe_matched.is_some_and(|matched| !matched) {
                            pkgs_with_additional_outputs.push(install_id);
                        }
                    },
                    LockedPackage::Flake(locked) => {
                        let maybe_matched = locked.outputs_match_outputs_to_install();
                        if maybe_matched.is_some_and(|matched| !matched) {
                            pkgs_with_additional_outputs.push(install_id);
                        }
                    },
                    _ => {},
                }
            }
        }
    }
    let maybe_msg = match pkgs_with_additional_outputs.as_slice() {
        [] => None,
        [pkg] => Some(format!(
            "'{pkg}' has additional outputs, use 'flox list -a' to see more"
        )),
        pkgs => {
            let joined = pkgs.iter().map(|p| format!("'{}'", p)).collect::<Vec<_>>();
            let joined = joined.join(", ");
            Some(format!(
                "{joined} have additional outputs, use 'flox list -a' to see more"
            ))
        },
    };
    if let Some(msg) = maybe_msg {
        info(msg)
    }
}

/// Display a message for packages that are newly overridden by the composing manifest
pub(crate) fn packages_newly_overridden_by_composer(pkgs: &[String]) {
    let already_installed_msg = match pkgs {
        [] => None,
        [pkg] => Some(format!(
            "This environment now overrides package with id '{}'",
            pkg
        )),
        pkgs => {
            let joined = pkgs.iter().map(|p| format!("'{}'", p)).collect::<Vec<_>>();
            let joined = joined.join(", ");
            Some(format!(
                "This environment now overrides packages with ids {joined}"
            ))
        },
    };
    if let Some(msg) = already_installed_msg {
        info(msg)
    }
}

/// Format a list of overridden fields for an environment.
fn format_overridden_fields(fields: &[String]) -> String {
    fields
        .iter()
        .map(|key| format!("  - {}", key))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Print notices for any environments that have overridden fields during composition.
pub(crate) fn print_overridden_manifest_fields(lockfile: &Lockfile) {
    let Some(ref compose) = lockfile.compose else {
        return;
    };

    type Field = String;
    type Environment = String;

    // De-duplicate fields by the last "winning" environment.
    let winning_env_by_field: BTreeMap<Field, Environment> = compose
        .warnings
        .iter()
        .filter_map(|warning_context| match &warning_context.warning {
            Warning::Overriding(field) => Some((
                field.to_string(),
                warning_context.higher_priority_name.clone(),
            )),
            _ => None,
        })
        .collect();

    // Invert the de-duplicated map.
    let mut fields_by_env: BTreeMap<Environment, Vec<Field>> = BTreeMap::new();
    for (field, env) in winning_env_by_field {
        fields_by_env.entry(env).or_default().push(field);
    }

    // Sort the notices by the order that the environments were included and
    // then the current composer environment (if present) last.
    let mut messages_by_env: Vec<String> = Vec::new();
    let ordered_envs = compose.include.iter().map(|include| include.name.clone());
    for env in ordered_envs {
        if let Some(fields) = fields_by_env.get(&env) {
            messages_by_env.push(format!(
                "- Environment '{}' set:\n{}",
                env,
                format_overridden_fields(fields),
            ));
        }
    }
    if let Some(fields) = fields_by_env.get(COMPOSER_MANIFEST_ID) {
        messages_by_env.push(format!(
            "- This environment set:\n{}",
            format_overridden_fields(fields),
        ));
    }
    if !messages_by_env.is_empty() {
        let message = formatdoc! {"
                The following manifest fields were overridden during merging:
                {}", messages_by_env.join("\n")
        };
        info(message);
    }
}

/// Warn about `[pkg-groups.<NAME>]` tables in the environment's own manifest
/// that no package uses, which usually means the pkg-group name has a typo.
///
/// `toplevel` is skipped, since packages join it by default. Packages from
/// included environments count, since the settings apply to them too.
pub(crate) fn print_unused_pkg_groups(lockfile: &Lockfile) {
    let (user_manifest, merged_manifest) = match (
        lockfile.migrated_user_manifest(),
        lockfile.migrated_manifest(),
    ) {
        (Ok(user_manifest), Ok(merged_manifest)) => (user_manifest, merged_manifest),
        (Err(err), _) | (_, Err(err)) => {
            debug!(%err, "failed to read manifests for unused pkg-groups");
            return;
        },
    };
    let merged_manifest = merged_manifest.as_latest_schema();
    for group in user_manifest.as_latest_schema().pkg_groups.inner().keys() {
        if group == DEFAULT_GROUP_NAME || merged_manifest.group_has_packages(group) {
            continue;
        }
        warning(formatdoc! {"
            No package is in pkg-group '{group}', so '[pkg-groups.{group}]' has no effect.
            Check the pkg-group name with 'flox edit'."});
    }
}

/// Report when re-locking changed the implicit default systems the environment
/// is locked for, e.g. because a newer Flox with a different default set
/// re-locked an environment without explicit `options.systems`.
pub(crate) fn print_default_systems_changed(
    old_lockfile: Option<&Lockfile>,
    new_lockfile: &Lockfile,
) {
    let Some(old_lockfile) = old_lockfile else {
        return;
    };

    let change = match default_systems_change(old_lockfile, new_lockfile) {
        Ok(Some(change)) => change,
        Ok(None) => return,
        // A warning must never fail the command that triggered the re-lock.
        Err(err) => {
            debug!(%err, "failed to detect default systems change");
            return;
        },
    };

    for system in change.removed {
        warning(
            formatdoc! {"
            packages have been removed from lockfile for '{system}'
            To reinstall, add '{system}' to 'options.systems' with 'flox edit'
        "}
            .trim_end(),
        );
    }

    for system in change.added {
        info(format!("packages have been added for '{system}'"));
    }
}

#[cfg(test)]
mod tests {
    use flox_core::Version;
    use flox_manifest::interfaces::AsTypedOnlyManifest;
    use flox_manifest::lockfile::test_helpers::fake_catalog_package_lock;
    use flox_manifest::parsed::Inner;
    use flox_manifest::parsed::latest::{ManifestLatest, ManifestPackageDescriptor};
    use flox_manifest::raw::CatalogPackage;
    use flox_manifest::raw::test_helpers::mk_test_manifest_from_contents;
    use flox_manifest::test_helpers::with_latest_schema;
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::Environment;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment;
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use tracing::instrument::WithSubscriber;

    use super::*;

    /// Build a lockfile with a single catalog package locked for
    /// `locked_systems`, optionally with explicit `options.systems` in the
    /// embedded manifest.
    fn lockfile_locked_for_systems(
        locked_systems: &[&str],
        options_systems: Option<&[&str]>,
    ) -> Lockfile {
        let (iid, mut descriptor, locked) = fake_catalog_package_lock("hello", None);
        if let ManifestPackageDescriptor::Catalog(ref mut descriptor) = descriptor {
            descriptor.systems = None;
        } else {
            panic!("Expected a catalog descriptor");
        }

        let mut manifest = ManifestLatest::default();
        manifest.options.systems =
            options_systems.map(|systems| systems.iter().map(|s| s.to_string()).collect());
        manifest.install.inner_mut().insert(iid, descriptor);

        let packages = locked_systems
            .iter()
            .map(|system| {
                let mut locked = locked.clone();
                locked.system = system.to_string();
                locked.into()
            })
            .collect();

        Lockfile {
            version: Version::<1>,
            manifest: manifest.as_typed_only(),
            packages,
            compose: None,
        }
    }

    /// A package installed with `--stability` reports the stability it set,
    /// and a package without one reports the stability it inherited from its
    /// pkg-group, if the pkg-group has one.
    #[tokio::test]
    async fn packages_group_stability_reports_set_and_inherited_stabilities() {
        let merged_manifest = mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
            [install]
            curl.pkg-path = "curl"
            curl.pkg-group = "tools"
            gh.pkg-path = "gh"
            gh.pkg-group = "legacy"
            jq.pkg-path = "jq"

            [pkg-groups.legacy]
            stability = "lts"

            [pkg-groups.tools]
            stability = "staging"
        "#}));
        let lockfile = Lockfile {
            manifest: merged_manifest.as_latest_schema().as_typed_only(),
            ..Default::default()
        };
        let catalog_package = |id: &str, pkg_group: &str, stability: Option<&str>| {
            PackageToInstall::Catalog(CatalogPackage {
                id: id.to_string(),
                pkg_path: id.to_string(),
                version: None,
                systems: None,
                outputs: None,
                pkg_group: Some(pkg_group.to_string()),
                stability: stability.map(str::to_string),
            })
        };
        let pkgs = [
            catalog_package("curl", "tools", Some("staging")),
            catalog_package("gh", "legacy", None),
            catalog_package("jq", "toplevel", None),
        ];

        let (subscriber, writer) = test_subscriber_message_only();
        async {
            packages_group_stability(&pkgs, &lockfile);
        }
        .with_subscriber(subscriber)
        .await;

        assert_eq!(writer.to_string(), indoc! {"
            ℹ pkg-group 'tools' resolves against the 'staging' stability.
            ℹ 'gh' joined pkg-group 'legacy', which resolves against the 'lts' stability.
            "});
    }

    /// Installing an already installed package with `--pkg-group` or
    /// `--stability` says that the package wasn't changed, unless it already
    /// matches the options.
    #[tokio::test]
    async fn packages_already_installed_reports_unchanged_pkg_groups() {
        let merged_manifest = mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
            [install]
            hello.pkg-path = "hello"
            jq.pkg-path = "jq"
            curl.pkg-path = "curl"
            curl.pkg-group = "legacy"
            gh.pkg-path = "gh"
            gh.pkg-group = "legacy"

            [pkg-groups.legacy]
            stability = "lts"
        "#}));
        let lockfile = Lockfile {
            manifest: merged_manifest.as_latest_schema().as_typed_only(),
            ..Default::default()
        };
        let catalog_package = |id: &str, pkg_group: Option<&str>, stability: Option<&str>| {
            PackageToInstall::Catalog(CatalogPackage {
                id: id.to_string(),
                pkg_path: id.to_string(),
                version: None,
                systems: None,
                outputs: None,
                pkg_group: pkg_group.map(str::to_string),
                stability: stability.map(str::to_string),
            })
        };
        let pkgs = [
            catalog_package("hello", Some("legacy"), None),
            catalog_package("jq", Some("toplevel"), None),
            catalog_package("curl", None, Some("stable")),
            catalog_package("gh", None, Some("lts")),
        ];

        let (subscriber, writer) = test_subscriber_message_only();
        async {
            packages_already_installed(&pkgs, "'name'", &lockfile);
        }
        .with_subscriber(subscriber)
        .await;

        assert_eq!(writer.to_string(), indoc! {"
            ! Packages with ids 'jq', 'gh' already installed to environment 'name'
            ! Package 'hello' is already installed in pkg-group 'toplevel', so it was not moved to pkg-group 'legacy'.
            To apply these options, run 'flox uninstall hello' and then run 'flox install' again.
            ! Package 'curl' is already installed in pkg-group 'legacy', so '--stability stable' did not change it.
            To apply these options, run 'flox uninstall curl' and then run 'flox install' again.
            "});
    }

    /// Settings for a pkg-group that no package is in, e.g. because of a typo,
    /// are reported, unless they're for `toplevel`.
    #[tokio::test]
    async fn print_unused_pkg_groups_reports_groups_without_packages() {
        let manifest = mk_test_manifest_from_contents(with_latest_schema(indoc! {r#"
            [install]
            gh.pkg-path = "gh"
            gh.pkg-group = "legacy"

            [pkg-groups.legacy]
            stability = "lts"

            [pkg-groups.legcy]
            stability = "lts"

            [pkg-groups.toplevel]
            stability = "stable"
        "#}));
        let lockfile = Lockfile {
            manifest: manifest.as_latest_schema().as_typed_only(),
            ..Default::default()
        };

        let (subscriber, writer) = test_subscriber_message_only();
        async {
            print_unused_pkg_groups(&lockfile);
        }
        .with_subscriber(subscriber)
        .await;

        assert_eq!(writer.to_string(), indoc! {"
            ! No package is in pkg-group 'legcy', so '[pkg-groups.legcy]' has no effect.
            Check the pkg-group name with 'flox edit'.
            "});
    }

    #[tokio::test]
    async fn print_default_systems_changed_reports_added_and_removed() {
        let old = lockfile_locked_for_systems(
            &[
                "aarch64-darwin",
                "aarch64-linux",
                "x86_64-darwin",
                "x86_64-linux",
            ],
            None,
        );
        let new = lockfile_locked_for_systems(
            &[
                "aarch64-darwin",
                "aarch64-linux",
                "riscv64-linux",
                "x86_64-linux",
            ],
            None,
        );

        let (subscriber, writer) = test_subscriber_message_only();
        async {
            print_default_systems_changed(Some(&old), &new);
        }
        .with_subscriber(subscriber)
        .await;

        assert_eq!(writer.to_string(), indoc! {"
            ! packages have been removed from lockfile for 'x86_64-darwin'
            To reinstall, add 'x86_64-darwin' to 'options.systems' with 'flox edit'
            ℹ packages have been added for 'riscv64-linux'
            "});
    }

    #[tokio::test]
    async fn print_default_systems_changed_removal_only() {
        let old = lockfile_locked_for_systems(
            &[
                "aarch64-darwin",
                "aarch64-linux",
                "x86_64-darwin",
                "x86_64-linux",
            ],
            None,
        );
        let new =
            lockfile_locked_for_systems(&["aarch64-darwin", "aarch64-linux", "x86_64-linux"], None);

        let (subscriber, writer) = test_subscriber_message_only();
        async {
            print_default_systems_changed(Some(&old), &new);
        }
        .with_subscriber(subscriber)
        .await;

        assert_eq!(writer.to_string(), indoc! {"
            ! packages have been removed from lockfile for 'x86_64-darwin'
            To reinstall, add 'x86_64-darwin' to 'options.systems' with 'flox edit'
            "});
    }

    #[tokio::test]
    async fn print_default_systems_changed_silent_when_explicit_or_first_lock() {
        let systems = ["aarch64-darwin", "aarch64-linux", "x86_64-linux"];
        let explicit = lockfile_locked_for_systems(&systems, Some(&systems));
        let implicit = lockfile_locked_for_systems(
            &[
                "aarch64-darwin",
                "aarch64-linux",
                "x86_64-darwin",
                "x86_64-linux",
            ],
            None,
        );

        let (subscriber, writer) = test_subscriber_message_only();
        async {
            print_default_systems_changed(Some(&explicit), &implicit);
            print_default_systems_changed(Some(&implicit), &explicit);
            print_default_systems_changed(None, &implicit);
        }
        .with_subscriber(subscriber)
        .await;

        assert_eq!(writer.to_string(), "");
    }

    #[tokio::test]
    async fn test_print_overridden_manifest_fields() {
        let (flox, _tempdir) = flox_instance();

        let mut dep1 = new_path_environment(&flox, indoc! {r#"
            version = 1

            [vars]
            overridden_by_all = "set by dep1"
            overridden_by_dep2 = "set by dep1"
            overridden_by_composer = "set by dep1"
        "#});
        dep1.lockfile(&flox).unwrap();

        let mut dep2 = new_path_environment(&flox, indoc! {r#"
            version = 1

            [vars]
            overridden_by_all = "updated by dep2"
            overridden_by_dep2 = "updated by dep2"
        "#});
        dep2.lockfile(&flox).unwrap();

        let composer_original_manifest = formatdoc! {r#"
            version = 1

            [vars]
            overridden_by_all = "updated by composer"
            overridden_by_composer = "updated by composer"

            [include]
            environments = [
                {{ dir = "{dep1_dir}", name = "dep_one" }},
                {{ dir = "{dep2_dir}", name = "dep_two" }},
            ]"#,
            dep1_dir = dep1.parent_path().unwrap().to_string_lossy(),
            dep2_dir = dep2.parent_path().unwrap().to_string_lossy(),
        };
        let mut composer = new_path_environment(&flox, &composer_original_manifest);
        let lockfile = composer.lockfile(&flox).unwrap().into();

        let (subscriber, writer) = test_subscriber_message_only();
        async {
            print_overridden_manifest_fields(&lockfile);
        }
        .with_subscriber(subscriber)
        .await;

        // - environments are listed by the order they were included
        // - composer environment is listed last
        // - environment `dep_one` doesn't appear because its fields are overridden later
        assert_eq!(writer.to_string(), indoc! {"
            ℹ The following manifest fields were overridden during merging:
            - Environment 'dep_two' set:
              - vars.overridden_by_dep2
            - This environment set:
              - vars.overridden_by_all
              - vars.overridden_by_composer
            "});
    }
}
