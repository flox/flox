mod drv_diff;

use anyhow::Result;
use bpaf::Bpaf;
use crossterm::style::Stylize;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{Environment, SingleSystemUpgradeDiff};
use indoc::formatdoc;
use itertools::Itertools;
use tracing::{info_span, instrument};

use super::services::warn_manifest_changes_for_services;
use super::{EnvironmentSelect, environment_select};
use crate::commands::{ensure_floxhub_token, environment_description};
use crate::utils::message::{self, stderr_supports_color};
use crate::{environment_subcommand_metric, subcommand_metric};

// Upgrade packages in an environment
#[derive(Bpaf, Clone)]
pub struct Upgrade {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Show available upgrades but do not apply them
    #[bpaf(long)]
    dry_run: bool,

    /// Show dependency changes for build-only updates (implies --dry-run)
    #[bpaf(long)]
    detail: bool,

    /// ID of a package or pkg-group name to upgrade
    #[bpaf(positional("package or pkg-group"))]
    groups_or_iids: Vec<String>,
}
impl Upgrade {
    #[instrument(name = "upgrade", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        // Record subcommand metric prior to environment_subcommand_metric below
        // in case we error before then
        subcommand_metric!("upgrade");
        tracing::debug!(
            to_upgrade = self.groups_or_iids.join(","),
            "upgrading groups and install ids"
        );

        // --detail implies --dry-run
        let dry_run = self.dry_run || self.detail;

        // Ensure the user is logged in for the following remote operations
        if let EnvironmentSelect::Remote(_) = self.environment {
            ensure_floxhub_token(&mut flox).await?;
        };

        let mut concrete_environment = self
            .environment
            .detect_concrete_environment(&flox, "Upgrade")?;
        environment_subcommand_metric!("upgrade", concrete_environment);

        let description = environment_description(&concrete_environment)?;

        let progress_message = {
            let num_upgrades = if self.groups_or_iids.is_empty() {
                "all".to_string()
            } else {
                format!("{}", self.groups_or_iids.len())
            };

            let dry_prefix = if dry_run { "Dry run: " } else { "" };

            format!("{dry_prefix}Upgrading {num_upgrades} package(s) or group(s)")
        };

        let span = info_span!(
            "upgrade",
            concrete_environment = %description,
            progress = %progress_message
        );
        let result = span.in_scope(|| {
            let groups_or_iids = &self
                .groups_or_iids
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();

            if dry_run {
                concrete_environment.dry_upgrade(&flox, groups_or_iids)
            } else {
                concrete_environment.upgrade(&flox, groups_or_iids)
            }
        })?;

        let diff = result.diff();

        if diff.is_empty() {
            if self.groups_or_iids.is_empty() {
                message::plain(format!(
                    "No upgrades available for packages in {description}."
                ));
            } else {
                message::plain(format!(
                    "No upgrades available for the specified packages in {description}."
                ));
            }
            return Ok(());
        }

        let diff_for_system = result.diff_for_system(&flox.system);

        let rendered_diff = render_diff(&diff_for_system);
        let num_changes_for_system = diff_for_system.len();

        if dry_run {
            if diff_for_system.is_empty() {
                message::plain(formatdoc! {"
                    Upgrades are not available for {description} on this system, but upgrades are
                    available for other systems supported by this environment."});
                if self.groups_or_iids.is_empty() {
                } else {
                    message::plain(format!(
                        "No upgrades available for the specified packages in {description}."
                    ));
                }
                return Ok(());
            }
            message::plain(formatdoc! {"
                Dry run: Upgrades available for {num_changes_for_system} package(s) in {description}:
                {rendered_diff}
            "});

            if self.detail {
                let detail_span = info_span!(
                    "detail",
                    progress = "Analyzing dependency changes"
                );
                let detail = detail_span.in_scope(|| {
                    drv_diff::render_detail_tree(&diff_for_system)
                })?;
                if !detail.is_empty() {
                    message::plain(detail);
                }
            }

            message::plain(
                "To apply these changes, run upgrade without the '--dry-run' flag.",
            );

            return Ok(());
        }

        let icon = if stderr_supports_color() {
            "✔".green().to_string()
        } else {
            "✔".to_string()
        };
        if diff_for_system.is_empty() {
            message::plain(formatdoc! {"
            {icon} Upgraded {description}.
            Upgrades were not available for this system, but upgrades were applied for other
            systems supported by this environment."});
        } else {
            message::plain(formatdoc! {"
            {icon} Upgraded {num_changes_for_system} package(s) in {description}:
            {rendered_diff}
            "});
        }

        warn_manifest_changes_for_services(&flox, &concrete_environment);

        Ok(())
    }
}

/// Render a diff of locked packages before and after an upgrade.
///
/// Version changes show: `- pkg: 1.0 -> 2.0`
/// Build-only changes show: `- pkg: 1.0 (build update, rev ...)` with
/// fallback to rev hash or bare "(build update)" when rev info is unavailable.
fn render_diff(diff: &SingleSystemUpgradeDiff) -> String {
    diff.iter()
        .map(|(_, (before, after))| {
            let install_id = before.install_id();
            let old_version = before.version().unwrap_or("unknown");
            let new_version = after.version().unwrap_or("unknown");

            if new_version != old_version {
                return format!("- {install_id}: {old_version} -> {new_version}");
            }

            // Same version — build-only change. Try to show rev info.
            let build_detail = build_update_detail(before, after);
            match build_detail {
                Some(detail) => format!("- {install_id}: {old_version} (build update, {detail})"),
                None => format!("- {install_id}: {old_version} (build update)"),
            }
        })
        .join("\n")
}

/// Extract a human-readable detail string for build-only changes.
///
/// Tries rev_date first, then rev hash (truncated to 7 chars).
/// Returns `None` if no rev info is available (e.g. flake packages).
fn build_update_detail(
    before: &flox_rust_sdk::models::lockfile::LockedPackage,
    after: &flox_rust_sdk::models::lockfile::LockedPackage,
) -> Option<String> {
    let before_catalog = before.as_catalog_package_ref();
    let after_catalog = after.as_catalog_package_ref();

    match (before_catalog, after_catalog) {
        (Some(old), Some(new)) => {
            let old_date = old.rev_date.format("%Y-%m-%d");
            let new_date = new.rev_date.format("%Y-%m-%d");
            if old_date.to_string() != new_date.to_string() {
                return Some(format!("rev {old_date} -> {new_date}"));
            }
            // Same date — fall back to rev hash
            let old_rev = &old.rev[..7.min(old.rev.len())];
            let new_rev = &new.rev[..7.min(new.rev.len())];
            if old_rev != new_rev {
                return Some(format!("rev {old_rev} -> {new_rev}"));
            }
            None
        },
        _ => None,
    }
}

/// Count how many entries in a diff are version upgrades vs build-only updates.
pub(crate) fn count_upgrade_categories(
    diff: &SingleSystemUpgradeDiff,
) -> (usize, usize) {
    let mut version_upgrades = 0;
    let mut build_updates = 0;
    for (_, (before, after)) in diff.iter() {
        let old_version = before.version().unwrap_or("unknown");
        let new_version = after.version().unwrap_or("unknown");
        if new_version != old_version {
            version_upgrades += 1;
        } else {
            build_updates += 1;
        }
    }
    (version_upgrades, build_updates)
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::Environment;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::{
        new_named_path_environment,
        new_named_path_environment_from_env_files,
    };
    use flox_rust_sdk::models::manifest::raw::PackageToInstall;
    use flox_rust_sdk::providers::catalog::GENERATED_DATA;
    use flox_rust_sdk::providers::catalog::test_helpers::catalog_replay_client;
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use indoc::indoc;
    use tracing::instrument::WithSubscriber;

    use super::*;
    use crate::commands::EnvironmentSelect;

    /// Check message printed when there are no upgrades available
    #[tokio::test(flavor = "multi_thread")]
    async fn confirmation_when_up_to_date() {
        let (mut flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber_message_only();

        let environment = new_named_path_environment_from_env_files(
            &flox,
            GENERATED_DATA.join("envs/hello"),
            "name",
        );

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;

        Upgrade {
            environment: EnvironmentSelect::Dir(environment.parent_path().unwrap()),
            dry_run: true,
            detail: false,
            groups_or_iids: Vec::new(),
        }
        .handle(flox)
        .with_subscriber(subscriber)
        .await
        .unwrap();

        let printed = writer.to_string();

        assert_eq!(printed, "No upgrades available for packages in 'name'.\n");
    }

    /// Run an upgrade of an environment that only has upgrades on other systems
    async fn run_upgrade_with_upgrades_on_other_system(dry_run: bool) -> String {
        let (mut flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber_message_only();

        let mut environment = new_named_path_environment(&flox, "version = 1", "name");

        let response_path = if cfg!(target_os = "macos") {
            "resolve/old_linux_hello.yaml"
        } else {
            "resolve/old_darwin_hello.yaml"
        };
        flox.catalog_client = catalog_replay_client(GENERATED_DATA.join(response_path)).await;

        environment
            .install(
                &[PackageToInstall::parse(&flox.system, "hello").unwrap()],
                &flox,
            )
            .unwrap();

        flox.catalog_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;
        Upgrade {
            environment: EnvironmentSelect::Dir(environment.parent_path().unwrap()),
            dry_run,
            detail: false,
            groups_or_iids: Vec::new(),
        }
        .handle(flox)
        .with_subscriber(subscriber)
        .await
        .unwrap();

        writer.to_string()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn upgrade_on_other_system() {
        assert_eq!(
            run_upgrade_with_upgrades_on_other_system(false).await,
            indoc! {"
            ✔ Upgraded 'name'.
            Upgrades were not available for this system, but upgrades were applied for other
            systems supported by this environment.
            "}
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn upgrade_dry_run_on_other_system() {
        assert_eq!(
            run_upgrade_with_upgrades_on_other_system(true).await,
            indoc! {"
            Upgrades are not available for 'name' on this system, but upgrades are
            available for other systems supported by this environment.
            "}
        );
    }

    mod render_diff_tests {
        use std::collections::BTreeMap;

        use chrono::TimeZone;
        use flox_rust_sdk::models::lockfile::{LockedPackage, LockedPackageCatalog};

        use super::super::*;

        fn make_catalog_package(
            install_id: &str,
            version: &str,
            derivation: &str,
            rev: &str,
            rev_date: chrono::DateTime<chrono::Utc>,
        ) -> LockedPackage {
            LockedPackage::Catalog(LockedPackageCatalog {
                attr_path: format!("legacyPackages.x86_64-linux.{install_id}"),
                broken: None,
                derivation: derivation.to_string(),
                description: None,
                install_id: install_id.to_string(),
                license: None,
                locked_url: "https://github.com/NixOS/nixpkgs".to_string(),
                name: install_id.to_string(),
                pname: install_id.to_string(),
                rev: rev.to_string(),
                rev_count: 1,
                rev_date,
                scrape_date: chrono::Utc::now(),
                stabilities: None,
                unfree: None,
                version: version.to_string(),
                outputs_to_install: None,
                outputs: BTreeMap::new(),
                system: "x86_64-linux".to_string(),
                group: "toplevel".to_string(),
                priority: 5,
            })
        }

        #[test]
        fn version_change_shows_arrow() {
            let before = make_catalog_package(
                "curl",
                "8.9.0",
                "/nix/store/old",
                "aaa1111",
                chrono::Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap(),
            );
            let after = make_catalog_package(
                "curl",
                "8.10.1",
                "/nix/store/new",
                "bbb2222",
                chrono::Utc.with_ymd_and_hms(2025, 2, 10, 0, 0, 0).unwrap(),
            );
            let mut diff = SingleSystemUpgradeDiff::new();
            diff.insert("curl".to_string(), (before, after));

            assert_eq!(render_diff(&diff), "- curl: 8.9.0 -> 8.10.1");
        }

        #[test]
        fn build_only_with_different_rev_dates() {
            let before = make_catalog_package(
                "terraform-docs",
                "0.21.0",
                "/nix/store/old",
                "aaa1111",
                chrono::Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap(),
            );
            let after = make_catalog_package(
                "terraform-docs",
                "0.21.0",
                "/nix/store/new",
                "bbb2222",
                chrono::Utc.with_ymd_and_hms(2025, 2, 10, 0, 0, 0).unwrap(),
            );
            let mut diff = SingleSystemUpgradeDiff::new();
            diff.insert("terraform-docs".to_string(), (before, after));

            assert_eq!(
                render_diff(&diff),
                "- terraform-docs: 0.21.0 (build update, rev 2025-01-15 -> 2025-02-10)"
            );
        }

        #[test]
        fn build_only_same_date_different_rev() {
            let date = chrono::Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
            let before = make_catalog_package(
                "jq",
                "1.7.1",
                "/nix/store/old",
                "abc1234def5678",
                date,
            );
            let after = make_catalog_package(
                "jq",
                "1.7.1",
                "/nix/store/new",
                "fff9999aaa0000",
                date,
            );
            let mut diff = SingleSystemUpgradeDiff::new();
            diff.insert("jq".to_string(), (before, after));

            assert_eq!(
                render_diff(&diff),
                "- jq: 1.7.1 (build update, rev abc1234 -> fff9999)"
            );
        }

        #[test]
        fn build_only_same_date_same_rev_shows_bare() {
            let date = chrono::Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
            let before = make_catalog_package(
                "hello",
                "2.12.1",
                "/nix/store/old",
                "abc1234",
                date,
            );
            let after = make_catalog_package(
                "hello",
                "2.12.1",
                "/nix/store/new",
                "abc1234",
                date,
            );
            let mut diff = SingleSystemUpgradeDiff::new();
            diff.insert("hello".to_string(), (before, after));

            assert_eq!(
                render_diff(&diff),
                "- hello: 2.12.1 (build update)"
            );
        }

        #[test]
        fn mixed_upgrades_rendered_together() {
            let before_curl = make_catalog_package(
                "curl",
                "8.9.0",
                "/nix/store/old-curl",
                "aaa1111",
                chrono::Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap(),
            );
            let after_curl = make_catalog_package(
                "curl",
                "8.10.1",
                "/nix/store/new-curl",
                "bbb2222",
                chrono::Utc.with_ymd_and_hms(2025, 2, 10, 0, 0, 0).unwrap(),
            );
            let before_tf = make_catalog_package(
                "terraform-docs",
                "0.21.0",
                "/nix/store/old-tf",
                "ccc3333",
                chrono::Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap(),
            );
            let after_tf = make_catalog_package(
                "terraform-docs",
                "0.21.0",
                "/nix/store/new-tf",
                "ddd4444",
                chrono::Utc.with_ymd_and_hms(2025, 2, 10, 0, 0, 0).unwrap(),
            );
            let mut diff = SingleSystemUpgradeDiff::new();
            diff.insert("curl".to_string(), (before_curl, after_curl));
            diff.insert(
                "terraform-docs".to_string(),
                (before_tf, after_tf),
            );

            let rendered = render_diff(&diff);
            assert_eq!(
                rendered,
                "- curl: 8.9.0 -> 8.10.1\n\
                 - terraform-docs: 0.21.0 (build update, rev 2025-01-15 -> 2025-02-10)"
            );
        }

        #[test]
        fn count_categories_mixed() {
            let before_curl = make_catalog_package(
                "curl",
                "8.9.0",
                "/nix/store/old",
                "aaa",
                chrono::Utc::now(),
            );
            let after_curl = make_catalog_package(
                "curl",
                "8.10.1",
                "/nix/store/new",
                "bbb",
                chrono::Utc::now(),
            );
            let before_tf = make_catalog_package(
                "terraform-docs",
                "0.21.0",
                "/nix/store/old",
                "ccc",
                chrono::Utc::now(),
            );
            let after_tf = make_catalog_package(
                "terraform-docs",
                "0.21.0",
                "/nix/store/new",
                "ddd",
                chrono::Utc::now(),
            );
            let mut diff = SingleSystemUpgradeDiff::new();
            diff.insert("curl".to_string(), (before_curl, after_curl));
            diff.insert("terraform-docs".to_string(), (before_tf, after_tf));

            let (version_upgrades, build_updates) = count_upgrade_categories(&diff);
            assert_eq!(version_upgrades, 1);
            assert_eq!(build_updates, 1);
        }
    }
}
