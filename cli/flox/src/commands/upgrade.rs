use anyhow::Result;
use bpaf::Bpaf;
use chrono::Datelike;
use crossterm::style::Stylize;
use flox_manifest::lockfile::LockedPackage;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{Environment, SingleSystemUpgradeDiff};
use indoc::formatdoc;
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

            let dry_prefix = if self.dry_run { "Dry run: " } else { "" };

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

            if self.dry_run {
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

        if self.dry_run {
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
            let (ver_ups, src_ups) = count_upgrade_categories(&diff_for_system);
            let summary = format_upgrade_summary(ver_ups, src_ups);
            message::plain(formatdoc! {"
                Dry run: {summary} available for {description}:
                {rendered_diff}

                To apply these changes, run upgrade without the '--dry-run' flag.
            "});

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
            let (ver_ups, src_ups) = count_upgrade_categories(&diff_for_system);
            let summary = format_upgrade_summary(ver_ups, src_ups);
            message::plain(formatdoc! {"
            {icon} {summary} applied in {description}:
            {rendered_diff}
            "});
        }

        warn_manifest_changes_for_services(&flox, &concrete_environment);

        Ok(())
    }
}

/// Format a rev_date as "Mon DD" for current year, "Mon DD, YYYY" otherwise.
fn format_rev_date(date: &chrono::DateTime<chrono::offset::Utc>) -> String {
    let now = chrono::Utc::now();
    if date.year() == now.year() {
        date.format("%b %d").to_string()
    } else {
        date.format("%b %d, %Y").to_string()
    }
}

/// Format source update detail showing rev_count and date changes.
/// Returns None for non-catalog packages.
fn source_update_detail(before: &LockedPackage, after: &LockedPackage) -> Option<String> {
    let old = before.as_catalog_package_ref()?;
    let new = after.as_catalog_package_ref()?;

    // Try rev_count + date first
    if old.rev_count != 0 || new.rev_count != 0 {
        let old_date = format_rev_date(&old.rev_date);
        let new_date = format_rev_date(&new.rev_date);
        return Some(format!(
            "source: {} {} -> {} {}",
            old.rev_count, old_date, new.rev_count, new_date
        ));
    }

    // Fallback to 7-char rev SHA
    let old_rev = &old.rev[..7.min(old.rev.len())];
    let new_rev = &new.rev[..7.min(new.rev.len())];
    if old_rev != new_rev {
        return Some(format!("source: {} -> {}", old_rev, new_rev));
    }

    None
}

/// Count version upgrades vs source updates in a diff.
pub(crate) fn count_upgrade_categories(diff: &SingleSystemUpgradeDiff) -> (usize, usize) {
    let mut version_upgrades = 0;
    let mut source_updates = 0;
    for (_, (before, after)) in diff.iter() {
        let old_version = before.version().unwrap_or("unknown");
        let new_version = after.version().unwrap_or("unknown");
        if new_version != old_version {
            version_upgrades += 1;
        } else {
            source_updates += 1;
        }
    }
    (version_upgrades, source_updates)
}

/// Determine whether a package upgrade involves a version change.
fn is_version_upgrade(before: &LockedPackage, after: &LockedPackage) -> bool {
    before.version().unwrap_or("unknown") != after.version().unwrap_or("unknown")
}

/// Format a human-readable summary of upgrade counts.
pub(crate) fn format_upgrade_summary(version_upgrades: usize, source_updates: usize) -> String {
    let version_part = match version_upgrades {
        0 => None,
        1 => Some("1 version upgrade".to_string()),
        n => Some(format!("{n} version upgrades")),
    };
    let source_part = match source_updates {
        0 => None,
        1 => Some("1 source update".to_string()),
        n => Some(format!("{n} source updates")),
    };
    match (version_part, source_part) {
        (Some(v), Some(s)) => format!("{v} and {s}"),
        (Some(v), None) => v,
        (None, Some(s)) => s,
        (None, None) => "Upgrades".to_string(),
    }
}

/// Render a diff of locked packages before and after an upgrade, grouped by category.
fn render_diff(diff: &SingleSystemUpgradeDiff) -> String {
    let (version_upgrades, source_updates) = count_upgrade_categories(diff);
    let mut lines = Vec::new();

    // Version upgrades first
    if version_upgrades > 0 {
        let label = if version_upgrades == 1 {
            "1 version upgrade:".to_string()
        } else {
            format!("{version_upgrades} version upgrades:")
        };
        lines.push(format!("  {label}"));
        for (_, (before, after)) in diff.iter() {
            if is_version_upgrade(before, after) {
                let id = before.install_id();
                let old_ver = before.version().unwrap_or("unknown");
                let new_ver = after.version().unwrap_or("unknown");
                lines.push(format!("  - {id}: {old_ver} -> {new_ver}"));
            }
        }
    }

    // Source updates
    if source_updates > 0 {
        let label = if source_updates == 1 {
            "1 source update:".to_string()
        } else {
            format!("{source_updates} source updates:")
        };
        lines.push(format!("  {label}"));
        for (_, (before, after)) in diff.iter() {
            if !is_version_upgrade(before, after) {
                let id = before.install_id();
                let ver = after.version().unwrap_or("unknown");
                match source_update_detail(before, after) {
                    Some(detail) => lines.push(format!("  - {id}: {ver} ({detail})")),
                    None => lines.push(format!("  - {id}: {ver} (source updated)")),
                }
            }
        }
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use flox_manifest::lockfile::LockedPackage;
    use flox_manifest::lockfile::test_helpers::fake_catalog_package_lock;
    use flox_manifest::raw::PackageToInstall;
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::Environment;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::{
        new_named_path_environment,
        new_named_path_environment_from_env_files,
    };
    use flox_rust_sdk::providers::catalog::test_helpers::catalog_replay_client;
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use flox_test_utils::GENERATED_DATA;
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

    /// Test format_upgrade_summary with various combinations
    #[test]
    fn test_format_upgrade_summary_version_only() {
        assert_eq!(format_upgrade_summary(1, 0), "1 version upgrade");
        assert_eq!(format_upgrade_summary(3, 0), "3 version upgrades");
    }

    #[test]
    fn test_format_upgrade_summary_source_only() {
        assert_eq!(format_upgrade_summary(0, 1), "1 source update");
        assert_eq!(format_upgrade_summary(0, 2), "2 source updates");
    }

    #[test]
    fn test_format_upgrade_summary_mixed() {
        assert_eq!(
            format_upgrade_summary(1, 1),
            "1 version upgrade and 1 source update"
        );
        assert_eq!(
            format_upgrade_summary(2, 3),
            "2 version upgrades and 3 source updates"
        );
    }

    #[test]
    fn test_format_upgrade_summary_none() {
        assert_eq!(format_upgrade_summary(0, 0), "Upgrades");
    }

    /// Test render_diff with version upgrades
    #[test]
    fn test_render_diff_version_upgrades() {
        let (_iid, _desc, mut before) = fake_catalog_package_lock("curl", None);
        before.version = "7.0.0".to_string();
        let mut after = before.clone();
        after.version = "8.0.0".to_string();

        let diff = SingleSystemUpgradeDiff::from_iter(vec![(
            "curl_install_id".to_string(),
            (
                LockedPackage::Catalog(before),
                LockedPackage::Catalog(after),
            ),
        )]);

        let rendered = render_diff(&diff);
        assert!(rendered.contains("1 version upgrade:"), "Missing header");
        assert!(
            rendered.contains("- curl_install_id: 7.0.0 -> 8.0.0"),
            "Missing entry"
        );
        assert!(
            !rendered.contains("source update"),
            "Should not show source update section"
        );
    }

    /// Test render_diff with source updates (same version, different rev_count)
    #[test]
    fn test_render_diff_source_updates() {
        let (_iid, _desc, mut before) = fake_catalog_package_lock("awscli2", None);
        before.version = "2.33.2".to_string();
        before.rev_count = 100;
        before.rev_date = chrono::DateTime::parse_from_rfc3339("2020-01-15T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::offset::Utc);
        let mut after = before.clone();
        after.rev_count = 200;
        after.rev_date = chrono::DateTime::parse_from_rfc3339("2020-02-20T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::offset::Utc);

        let diff = SingleSystemUpgradeDiff::from_iter(vec![(
            "awscli2_install_id".to_string(),
            (
                LockedPackage::Catalog(before),
                LockedPackage::Catalog(after),
            ),
        )]);

        let rendered = render_diff(&diff);
        assert!(rendered.contains("1 source update:"), "Missing header");
        assert!(rendered.contains("awscli2_install_id"), "Missing package");
        assert!(rendered.contains("2.33.2"), "Missing version");
        assert!(rendered.contains("source:"), "Missing source detail");
        assert!(
            !rendered.contains("version upgrade"),
            "Should not show version upgrade section"
        );
    }

    /// Test render_diff with mixed version upgrades and source updates
    #[test]
    fn test_render_diff_mixed() {
        let (_c_iid, _c_desc, mut curl_before) = fake_catalog_package_lock("curl", None);
        curl_before.version = "7.0.0".to_string();
        let mut curl_after = curl_before.clone();
        curl_after.version = "8.0.0".to_string();

        let (_a_iid, _a_desc, mut aws_before) = fake_catalog_package_lock("awscli2", None);
        aws_before.version = "2.33.2".to_string();
        aws_before.rev_count = 100;
        let mut aws_after = aws_before.clone();
        aws_after.rev_count = 200;

        let diff = SingleSystemUpgradeDiff::from_iter(vec![
            (
                "awscli2_install_id".to_string(),
                (
                    LockedPackage::Catalog(aws_before),
                    LockedPackage::Catalog(aws_after),
                ),
            ),
            (
                "curl_install_id".to_string(),
                (
                    LockedPackage::Catalog(curl_before),
                    LockedPackage::Catalog(curl_after),
                ),
            ),
        ]);

        let rendered = render_diff(&diff);
        assert!(
            rendered.contains("1 version upgrade:"),
            "Missing version header"
        );
        assert!(
            rendered.contains("1 source update:"),
            "Missing source header"
        );
        // Version upgrades section should come before source updates
        let ver_pos = rendered.find("version upgrade").unwrap();
        let src_pos = rendered.find("source update").unwrap();
        assert!(
            ver_pos < src_pos,
            "Version upgrades should come before source updates"
        );
    }
}
