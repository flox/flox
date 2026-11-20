use flox_manifest::lockfile::LockedPackage;
use flox_rust_sdk::models::environment::SingleSystemUpgradeDiff;

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
    render_diff_with(diff, false)
}

/// Like [render_diff], but a version change to a package locked from an older
/// revision shows: `- pkg: 2.0 -> 1.0 (older rev, DATE -> DATE)`
pub(crate) fn render_diff_flagging_older_revisions(diff: &SingleSystemUpgradeDiff) -> String {
    render_diff_with(diff, true)
}

fn render_diff_with(diff: &SingleSystemUpgradeDiff, flag_older_revisions: bool) -> String {
    use itertools::Itertools as _;
    diff.iter()
        .map(|(_, (before, after))| {
            let install_id = before.install_id();
            let old_version = before.version().unwrap_or("unknown");
            let new_version = after.version().unwrap_or("unknown");

            if new_version != old_version {
                let marker = flag_older_revisions
                    .then(|| older_revision_detail(before, after))
                    .flatten()
                    .map(|detail| format!(" (older rev, {detail})"))
                    .unwrap_or_default();
                return format!("- {install_id}: {old_version} -> {new_version}{marker}");
            }

            match rebuild_detail(before, after) {
                Some(detail) => format!("- {install_id}: {old_version} (rebuild, {detail})"),
                None => format!("- {install_id}: {old_version} (rebuild)"),
            }
        })
        .join("\n")
}

/// The `OLD -> NEW` revision dates when `after` is locked from a catalog
/// revision on an earlier day than `before`.
///
/// This only says which revision the package was resolved from moved back,
/// not that its version went down. For a package from a custom catalog the
/// revision is not nixpkgs, so a newer version can sit on an older revision.
/// Returns `None` for packages with no catalog revision (flakes, store paths)
/// and when the dates fall on the same day, where printing them would not
/// show the move.
fn older_revision_detail(before: &LockedPackage, after: &LockedPackage) -> Option<String> {
    let (old, new) = (
        before.as_catalog_package_ref()?,
        after.as_catalog_package_ref()?,
    );
    let (old_date, new_date) = (old.rev_date.date_naive(), new.rev_date.date_naive());
    (new_date < old_date).then(|| format!("{old_date} -> {new_date}"))
}

/// Extract a human-readable detail string for build-only changes.
///
/// Tries rev_date first (formatted as YYYY-MM-DD), then rev hash (7 chars).
/// Returns `None` if no rev info is available (e.g. flake packages).
pub(crate) fn rebuild_detail(before: &LockedPackage, after: &LockedPackage) -> Option<String> {
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

    mod older_revision {
        use chrono::TimeZone as _;
        use flox_manifest::lockfile::test_helpers::{
            fake_catalog_package_lock,
            fake_flake_installable_lock,
        };
        use flox_manifest::lockfile::{LockedPackage, LockedPackageFlake};

        use super::*;

        fn catalog(version: &str, derivation: &str, (y, m, d): (i32, u32, u32)) -> LockedPackage {
            let (_, _, mut pkg) = fake_catalog_package_lock("pkg", None);
            pkg.install_id = "pkg".to_string();
            pkg.version = version.to_string();
            pkg.derivation = derivation.to_string();
            pkg.rev_date = chrono::Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap();
            pkg.into()
        }

        fn flake(version: &str, derivation: &str) -> LockedPackage {
            let (_, _, mut pkg): (_, _, LockedPackageFlake) = fake_flake_installable_lock("pkg");
            pkg.install_id = "pkg".to_string();
            pkg.locked_installable.version = Some(version.to_string());
            pkg.locked_installable.derivation = derivation.to_string();
            pkg.into()
        }

        fn rendered(before: LockedPackage, after: LockedPackage) -> String {
            let mut diff = SingleSystemUpgradeDiff::new();
            diff.insert("pkg".to_string(), (before, after));
            render_diff_flagging_older_revisions(&diff)
        }

        #[test]
        fn older_revision_is_marked_with_both_dates() {
            assert_eq!(
                rendered(
                    catalog("6.0.422", "a", (2025, 3, 1)),
                    catalog("6.0.202", "b", (2024, 11, 2)),
                ),
                "- pkg: 6.0.422 -> 6.0.202 (older rev, 2025-03-01 -> 2024-11-02)"
            );
        }

        #[test]
        fn newer_revision_is_not_marked() {
            assert_eq!(
                rendered(
                    catalog("6.0.202", "a", (2024, 11, 2)),
                    catalog("6.0.422", "b", (2025, 3, 1)),
                ),
                "- pkg: 6.0.202 -> 6.0.422"
            );
        }

        #[test]
        fn same_revision_date_is_not_marked() {
            assert_eq!(
                rendered(
                    catalog("2.0", "a", (2025, 3, 1)),
                    catalog("1.0", "b", (2025, 3, 1)),
                ),
                "- pkg: 2.0 -> 1.0"
            );
        }

        /// A newer version can come from an older revision, so the mark follows
        /// the revision and says nothing about the version.
        #[test]
        fn non_numeric_version_on_older_revision_is_marked() {
            assert_eq!(
                rendered(
                    catalog("unstable-2020-04-06", "a", (2025, 3, 1)),
                    catalog("unstable-2019-01-01", "b", (2024, 11, 2)),
                ),
                "- pkg: unstable-2020-04-06 -> unstable-2019-01-01 (older rev, 2025-03-01 -> 2024-11-02)"
            );
        }

        #[test]
        fn package_without_revision_data_is_not_marked() {
            assert_eq!(
                rendered(flake("6.0.422", "a"), flake("6.0.202", "b")),
                "- pkg: 6.0.422 -> 6.0.202"
            );
        }
    }
}
