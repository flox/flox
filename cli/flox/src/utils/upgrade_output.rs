use flox_manifest::lockfile::LockedPackage;
use flox_rust_sdk::models::environment::SingleSystemUpgradeDiff;
use itertools::Itertools;

/// Count version changes vs rebuilds in a diff.
pub(crate) fn count_upgrade_categories(diff: &SingleSystemUpgradeDiff) -> (usize, usize) {
    diff.values().fold((0, 0), |(vc, rb), (before, after)| {
        let old_version = before.version().unwrap_or("unknown");
        let new_version = after.version().unwrap_or("unknown");
        if new_version != old_version {
            (vc + 1, rb)
        } else {
            (vc, rb + 1)
        }
    })
}

/// Format a human-readable summary like "2 version changes and 1 rebuild".
pub(crate) fn format_upgrade_summary(version_changes: usize, rebuilds: usize) -> String {
    let version_part = match version_changes {
        0 => None,
        1 => Some("1 version change".to_string()),
        n => Some(format!("{n} version changes")),
    };
    let rebuild_part = match rebuilds {
        0 => None,
        1 => Some("1 rebuild".to_string()),
        n => Some(format!("{n} rebuilds")),
    };
    match (version_part, rebuild_part) {
        (Some(v), Some(b)) => format!("{v} and {b}"),
        (Some(v), None) => v,
        (None, Some(b)) => b,
        (None, None) => "Upgrades".to_string(),
    }
}

/// Render a diff of locked packages before and after an upgrade.
///
/// Version changes show: `- pkg: 1.0 -> 2.0`
/// Rebuilds show: `- pkg: 1.0 (rebuild, rev DATE -> DATE)` with fallback to
/// rev hash or bare `(rebuild)` when rev info is unavailable.
pub(crate) fn render_diff(diff: &SingleSystemUpgradeDiff) -> String {
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
    use super::*;

    #[test]
    fn version_change_singular() {
        assert_eq!(format_upgrade_summary(1, 0), "1 version change");
    }

    #[test]
    fn version_changes_plural() {
        assert_eq!(format_upgrade_summary(3, 0), "3 version changes");
    }

    #[test]
    fn rebuild_singular() {
        assert_eq!(format_upgrade_summary(0, 1), "1 rebuild");
    }

    #[test]
    fn rebuilds_plural() {
        assert_eq!(format_upgrade_summary(0, 4), "4 rebuilds");
    }

    #[test]
    fn mixed() {
        assert_eq!(
            format_upgrade_summary(2, 1),
            "2 version changes and 1 rebuild"
        );
    }

    #[test]
    fn mixed_plural() {
        assert_eq!(
            format_upgrade_summary(3, 5),
            "3 version changes and 5 rebuilds"
        );
    }

    #[test]
    fn fallback_when_zero() {
        assert_eq!(format_upgrade_summary(0, 0), "Upgrades");
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
}
