use anyhow::Result;
use bpaf::Bpaf;
use crossterm::style::Stylize;
use flox_events::{CliEnvironmentPayload, CliPackageUpgradePayload, EventKind, EventsHub, Outcome};
use flox_manifest::interfaces::{AsLatestSchema, PackageLookup};
use flox_manifest::lockfile::LockedPackage;
use flox_manifest::parsed::latest::ManifestPackageDescriptor;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{Environment, SingleSystemUpgradeDiff};
use indoc::formatdoc;
use itertools::Itertools;
use tracing::{debug, info_span, instrument};

use super::services::warn_manifest_changes_for_services;
use super::{EnvironmentSelect, environment_select};
use crate::commands::{ensure_auth, environment_description};
use crate::utils::events::env_detail_from_concrete;
use crate::utils::message::{self, stderr_supports_color};
use crate::utils::upgrade_output::{count_upgrade_categories, format_upgrade_summary};
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
            ensure_auth(&mut flox).await?;
        };

        let mut concrete_environment = self
            .environment
            .detect_concrete_environment(&mut flox, "Upgrade")
            .await?;
        environment_subcommand_metric!("upgrade", concrete_environment);
        if let Err(err) = EventsHub::global().record_event(EventKind::CliEnvironmentUpgrade(
            CliEnvironmentPayload::new(env_detail_from_concrete(&flox, &concrete_environment)),
        )) {
            debug!(error = %err, "Failed to record v2 event");
        }

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
            let (version_changes, rebuilds) = count_upgrade_categories(&diff_for_system);
            let summary = format_upgrade_summary(version_changes, rebuilds);
            message::plain(formatdoc! {"
                Dry run: {summary} in {description}:
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
            let (version_changes, rebuilds) = count_upgrade_categories(&diff_for_system);
            let summary = format_upgrade_summary(version_changes, rebuilds);
            message::plain(formatdoc! {"
            {icon} Upgraded {summary} in {description}:
            {rendered_diff}
            "});
        }

        // `store_path` is only set when the upgrade wrote a new lockfile.
        if result.store_path.is_some() {
            message::print_default_systems_changed(
                result.old_lockfile.as_ref(),
                &result.new_lockfile,
            );
        }

        warn_manifest_changes_for_services(&flox, &concrete_environment);

        let hub = EventsHub::global();
        hub.when_client_set(|| {
            let manifest = result.new_lockfile.migrated_manifest().ok();
            let manifest = manifest.as_ref().map(AsLatestSchema::as_latest_schema);
            for (install_id, (before, after)) in diff_for_system.iter() {
                let descriptor =
                    manifest.and_then(|manifest| manifest.pkg_descriptor_with_id(install_id));
                let payload = upgrade_payload(descriptor.as_ref(), install_id, before, after);
                if let Err(err) = hub.record_event(EventKind::CliPackageUpgrade(payload)) {
                    debug!(error = %err, "Failed to record v2 event");
                }
            }
        });

        Ok(())
    }
}

/// Build the per-package payload for a `cli.package.upgrade` event.
///
/// The package key is the manifest descriptor's coordinate — stable across
/// upgrades and shared with the install events — falling back to the
/// install id when the descriptor cannot be found. Empty version strings
/// collapse to absent.
fn upgrade_payload(
    descriptor: Option<&ManifestPackageDescriptor>,
    install_id: &str,
    before: &LockedPackage,
    after: &LockedPackage,
) -> CliPackageUpgradePayload {
    let package = descriptor
        .map(|descriptor| descriptor.package_identifier().to_string())
        .unwrap_or_else(|| install_id.to_string());
    let mut payload =
        CliPackageUpgradePayload::new(package, Outcome::Success).with_install_id(install_id);
    if let Some(version) = before.version().filter(|version| !version.is_empty()) {
        payload = payload.with_previous_version(version);
    }
    if let Some(version) = after.version().filter(|version| !version.is_empty()) {
        payload = payload.with_version(version);
    }
    payload
}

/// Render a diff of locked packages before and after an upgrade.
///
/// Version changes show: `- pkg: 1.0 -> 2.0`
/// Rebuilds show: `- pkg: 1.0 (rebuild, rev DATE -> DATE)` with fallback to
/// rev hash or bare `(rebuild)` when rev info is unavailable.
fn render_diff(diff: &SingleSystemUpgradeDiff) -> String {
    diff.iter()
        .map(|(_, (before, after))| {
            let install_id = before.install_id();
            let old_version = before.version().unwrap_or("unknown");
            let new_version = after.version().unwrap_or("unknown");

            if new_version != old_version {
                return format!("- {install_id}: {old_version} -> {new_version}");
            }

            match rebuild_detail(before, after) {
                Some(detail) => format!("- {install_id}: {old_version} (rebuild, {detail})"),
                None => format!("- {install_id}: {old_version} (rebuild)"),
            }
        })
        .join("\n")
}

/// Extract a human-readable detail string for build-only changes.
///
/// Tries rev_date first (formatted as YYYY-MM-DD), then rev hash (7 chars).
/// Returns `None` if no rev info is available (e.g. flake packages).
fn rebuild_detail(before: &LockedPackage, after: &LockedPackage) -> Option<String> {
    let (old, new) = (
        before.as_catalog_package_ref()?,
        after.as_catalog_package_ref()?,
    );

    let old_date = old.rev_date.format("%Y-%m-%d");
    let new_date = new.rev_date.format("%Y-%m-%d");
    if old_date.to_string() != new_date.to_string() {
        return Some(format!("rev {old_date} -> {new_date}"));
    }

    let old_rev = &old.rev[..7.min(old.rev.len())];
    let new_rev = &new.rev[..7.min(new.rev.len())];
    if old_rev != new_rev {
        return Some(format!("rev {old_rev} -> {new_rev}"));
    }

    None
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use flox_events::test_helpers::MockEventsConnection;
    use flox_events::{CredentialType, Event, EventsClient, SharedMetadataTemplate};
    use flox_manifest::lockfile::test_helpers::fake_catalog_package_lock;
    use flox_manifest::parsed::latest::PackageDescriptorCatalog;
    use flox_manifest::raw::PackageToInstall;
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::Environment;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_named_path_environment;
    use flox_rust_sdk::providers::catalog::test_helpers::catalog_replay_client;
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use flox_test_utils::GENERATED_DATA;
    use flox_test_utils::manifests::HELLO;
    use indoc::indoc;
    use pretty_assertions::{assert_eq, assert_str_eq};
    use serial_test::serial;
    use tempfile::TempDir;
    use tracing::instrument::WithSubscriber;
    use uuid::Uuid;

    use super::*;
    use crate::commands::EnvironmentSelect;

    /// A mock-backed events client on the global hub, restored on drop so a
    /// panicking assertion cannot leak it into the next test. Holders must
    /// be `#[serial(global_events_client)]`.
    struct MockHub {
        previous: Option<EventsClient>,
        sent_batches: Arc<Mutex<Vec<Vec<Event>>>>,
        _buffer_dir: TempDir,
    }

    impl MockHub {
        fn install() -> Self {
            let buffer_dir = tempfile::tempdir().expect("tempdir");
            let connection = MockEventsConnection::default();
            let sent_batches = connection.sent_batches();
            let client = EventsClient::new_with_connection(
                Uuid::new_v4(),
                buffer_dir.path(),
                Uuid::new_v4(),
                None,
                SharedMetadataTemplate {
                    credential_type: CredentialType::None,
                    flox_version: "0.0.0-test".to_string(),
                    os_family: Some("Linux".to_string()),
                    os_family_release: None,
                    os: None,
                    os_version: None,
                    os_platform_version: None,
                    shell: None,
                    architecture: None,
                    empty_flags: vec![],
                    invocation_sources: vec!["shell".to_string()],
                },
                connection,
            );
            Self {
                previous: EventsHub::global().set_client(client),
                sent_batches,
                _buffer_dir: buffer_dir,
            }
        }

        /// Flushes, then returns the package-upgrade payloads that were sent.
        fn package_upgrade_payloads(&self) -> Vec<CliPackageUpgradePayload> {
            EventsHub::global().flush(true).expect("flush");
            self.sent_batches
                .lock()
                .unwrap()
                .iter()
                .flatten()
                .filter_map(|event| match &event.kind {
                    EventKind::CliPackageUpgrade(payload) => Some(payload.clone()),
                    _ => None,
                })
                .collect()
        }
    }

    impl Drop for MockHub {
        fn drop(&mut self) {
            EventsHub::global().clear_client();
            if let Some(previous) = self.previous.take() {
                EventsHub::global().set_client(previous);
            }
        }
    }

    #[cfg_attr(
        all(target_os = "macos", target_arch = "x86_64"),
        ignore = "catalog recordings don't cover x86_64-darwin"
    )]
    #[tokio::test(flavor = "multi_thread")]
    #[serial(global_events_client)]
    async fn upgrade_records_previous_version_on_package_events() {
        let (mut flox, _tempdir) = flox_instance();
        let mut environment = new_named_path_environment(&flox, "version = 1", "name");

        let response_path = if cfg!(target_os = "macos") {
            "resolve/old_darwin_hello.yaml"
        } else {
            "resolve/old_linux_hello.yaml"
        };
        flox.floxhub_client = catalog_replay_client(GENERATED_DATA.join(response_path)).await;
        // The replay fixtures were recorded with install id == pkg path, so
        // the two coincide here; the coordinate-vs-id distinction is pinned
        // by the upgrade_payload unit tests.
        environment
            .install(
                &[PackageToInstall::parse(&flox.system, "hello").unwrap()],
                &flox,
            )
            .unwrap();

        flox.floxhub_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;

        let hub = MockHub::install();
        Upgrade {
            environment: EnvironmentSelect::Dir(environment.parent_path().unwrap()),
            dry_run: false,
            groups_or_iids: Vec::new(),
        }
        .handle(flox)
        .await
        .unwrap();

        assert_eq!(hub.package_upgrade_payloads(), vec![
            CliPackageUpgradePayload::new("hello".to_string(), Outcome::Success)
                .with_install_id("hello")
                .with_previous_version("2.10.1")
                .with_version("2.12.3")
        ]);
    }

    #[test]
    fn upgrade_payload_collapses_unknown_versions_and_falls_back_to_install_id() {
        // The catalog fake locks version "" — the empty string must read as
        // unknown on the wire, and a missing descriptor falls back to the
        // install id as the package key.
        let (install_id, _, locked) = fake_catalog_package_lock("hello", None);
        let before = LockedPackage::Catalog(locked.clone());
        let after = LockedPackage::Catalog(locked);
        assert_eq!(
            upgrade_payload(None, &install_id, &before, &after),
            CliPackageUpgradePayload::new(install_id.clone(), Outcome::Success)
                .with_install_id(&install_id)
        );
    }

    #[test]
    fn upgrade_payload_carries_descriptor_coordinate_and_both_versions() {
        let (_, _, mut locked_before) = fake_catalog_package_lock("hello", None);
        let mut locked_after = locked_before.clone();
        locked_before.version = "2.10.1".to_string();
        locked_after.version = "2.12.3".to_string();
        let descriptor = ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog {
            pkg_path: "hello".to_string(),
            pkg_group: None,
            priority: None,
            version: None,
            systems: None,
            outputs: None,
        });
        assert_eq!(
            upgrade_payload(
                Some(&descriptor),
                "greeting",
                &LockedPackage::Catalog(locked_before),
                &LockedPackage::Catalog(locked_after),
            ),
            CliPackageUpgradePayload::new("hello".to_string(), Outcome::Success)
                .with_install_id("greeting")
                .with_previous_version("2.10.1")
                .with_version("2.12.3")
        );
    }

    /// Check message printed when there are no upgrades available
    #[cfg_attr(
        all(target_os = "macos", target_arch = "x86_64"),
        ignore = "catalog recordings don't cover x86_64-darwin"
    )]
    #[tokio::test(flavor = "multi_thread")]
    #[serial(global_events_client)]
    async fn confirmation_when_up_to_date() {
        let (mut flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber_message_only();

        let mut environment = new_named_path_environment(&flox, HELLO, "name");

        flox.floxhub_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/hello.yaml")).await;
        environment.lockfile(&flox).unwrap();

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
        flox.floxhub_client = catalog_replay_client(GENERATED_DATA.join(response_path)).await;

        environment
            .install(
                &[PackageToInstall::parse(&flox.system, "hello").unwrap()],
                &flox,
            )
            .unwrap();

        flox.floxhub_client =
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

    #[cfg_attr(
        all(target_os = "macos", target_arch = "x86_64"),
        ignore = "catalog recordings don't cover x86_64-darwin"
    )]
    #[tokio::test(flavor = "multi_thread")]
    #[serial(global_events_client)]
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

    #[cfg_attr(
        all(target_os = "macos", target_arch = "x86_64"),
        ignore = "catalog recordings don't cover x86_64-darwin"
    )]
    #[tokio::test(flavor = "multi_thread")]
    #[serial(global_events_client)]
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
        use flox_manifest::lockfile::{LockedPackage, LockedPackageCatalog};

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
        fn upgrade_with_different_versions() {
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
        fn rebuild_with_different_rev_dates() {
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
                "- terraform-docs: 0.21.0 (rebuild, rev 2025-01-15 -> 2025-02-10)"
            );
        }

        #[test]
        fn rebuild_same_date_different_rev() {
            let date = chrono::Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
            let before =
                make_catalog_package("jq", "1.7.1", "/nix/store/old", "abc1234def567", date);
            let after =
                make_catalog_package("jq", "1.7.1", "/nix/store/new", "fff9999aaa000", date);
            let mut diff = SingleSystemUpgradeDiff::new();
            diff.insert("jq".to_string(), (before, after));
            assert_eq!(
                render_diff(&diff),
                "- jq: 1.7.1 (rebuild, rev abc1234 -> fff9999)"
            );
        }

        #[test]
        fn rebuild_same_date_same_rev_shows_bare() {
            let date = chrono::Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
            let before = make_catalog_package("hello", "2.12.1", "/nix/store/old", "abc1234", date);
            let after = make_catalog_package("hello", "2.12.1", "/nix/store/new", "abc1234", date);
            let mut diff = SingleSystemUpgradeDiff::new();
            diff.insert("hello".to_string(), (before, after));
            assert_eq!(render_diff(&diff), "- hello: 2.12.1 (rebuild)");
        }

        #[test]
        fn dry_run_summary_with_rebuild() {
            let date = chrono::Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
            let before = make_catalog_package("hello", "2.12.1", "/nix/store/old", "abc1234", date);
            let after = make_catalog_package("hello", "2.12.1", "/nix/store/new", "abc1234", date);
            let mut diff = SingleSystemUpgradeDiff::new();
            diff.insert("hello".to_string(), (before, after));
            let (vc, rb) = count_upgrade_categories(&diff);
            assert_eq!(format_upgrade_summary(vc, rb), "1 rebuild");
            assert_eq!(render_diff(&diff), "- hello: 2.12.1 (rebuild)");
        }

        #[test]
        fn dry_run_summary_with_version_change_and_rebuild() {
            let date = chrono::Utc.with_ymd_and_hms(2025, 1, 15, 0, 0, 0).unwrap();
            let before_curl = make_catalog_package("curl", "8.9.0", "/nix/store/old", "aaa", date);
            let after_curl = make_catalog_package("curl", "8.10.1", "/nix/store/new", "bbb", date);
            let before_hello =
                make_catalog_package("hello", "2.12.1", "/nix/store/old", "abc1234", date);
            let after_hello =
                make_catalog_package("hello", "2.12.1", "/nix/store/new", "abc1234", date);
            let mut diff = SingleSystemUpgradeDiff::new();
            diff.insert("curl".to_string(), (before_curl, after_curl));
            diff.insert("hello".to_string(), (before_hello, after_hello));
            let (vc, rb) = count_upgrade_categories(&diff);
            assert_eq!(
                format_upgrade_summary(vc, rb),
                "1 version change and 1 rebuild"
            );
            assert_eq!(
                render_diff(&diff),
                "- curl: 8.9.0 -> 8.10.1\n- hello: 2.12.1 (rebuild)"
            );
        }

        #[test]
        fn count_categories_mixed() {
            let before_curl =
                make_catalog_package("curl", "8.9.0", "/nix/store/old", "aaa", chrono::Utc::now());
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
            assert_eq!(count_upgrade_categories(&diff), (1, 1));
        }
    }

    /// Run a dry-run upgrade of an environment that has a version change on this system
    async fn run_dry_run_with_version_change() -> String {
        let (mut flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber_message_only();

        let mut environment = new_named_path_environment(&flox, "version = 1", "name");

        // Use the fixture that has an older version for THIS system
        let response_path = if cfg!(target_os = "macos") {
            "resolve/old_darwin_hello.yaml"
        } else {
            "resolve/old_linux_hello.yaml"
        };
        flox.floxhub_client = catalog_replay_client(GENERATED_DATA.join(response_path)).await;

        environment
            .install(
                &[PackageToInstall::parse(&flox.system, "hello").unwrap()],
                &flox,
            )
            .unwrap();

        flox.floxhub_client =
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

        writer.to_string()
    }

    #[cfg_attr(
        all(target_os = "macos", target_arch = "x86_64"),
        ignore = "catalog recordings don't cover x86_64-darwin"
    )]
    #[tokio::test(flavor = "multi_thread")]
    #[serial(global_events_client)]
    async fn dry_run_shows_version_change_summary() {
        assert_str_eq!(run_dry_run_with_version_change().await, indoc! {"
            Dry run: 1 version change in 'name':
            - hello: 2.10.1 -> 2.12.3

            To apply these changes, run upgrade without the '--dry-run' flag.

            "});
    }
}
