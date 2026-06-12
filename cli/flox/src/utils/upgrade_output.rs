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
}
