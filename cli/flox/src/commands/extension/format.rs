use beta::extensions::Extension;

/// Render a single row of the `flox extension list` / `upgrade` table.
/// Column widths match the existing `list` handler (20 / 32 / 14 / 6 / _)
/// with a trailing STATUS column. STATUS is `None` for rows produced by
/// `list` (column omitted), `Some("...")` for rows produced by `upgrade`
/// and `upgrade --dry-run`.
pub(super) struct TableRow {
    pub name: String,
    pub repo: String,
    pub version: String,
    pub pinned: bool,
    pub status: Option<String>,
}

pub(super) fn render_header() -> String {
    format!(
        "{:<20}  {:<32}  {:<14}  {:<6}  {}",
        "NAME", "REPO", "VERSION", "PINNED", "STATUS"
    )
}

pub(super) fn render_row(row: &TableRow) -> String {
    let version = short_sha(&row.version);
    let pinned = if row.pinned { "yes" } else { "" };
    let status = row.status.as_deref().unwrap_or("");
    format!(
        "{:<20}  {:<32}  {:<14}  {:<6}  {}",
        row.name, row.repo, version, pinned, status
    )
}

/// Convert an `Extension` to a `TableRow`. Reused by list and upgrade
/// handlers. `status` is left `None`; the caller fills it.
pub(super) fn row_from_extension(ext: &Extension) -> TableRow {
    let repo = if ext.state.kind == "local" {
        ".".to_string()
    } else if !ext.state.owner.is_empty() && !ext.state.repo.is_empty() {
        format!("{}/{}", ext.state.owner, ext.state.repo)
    } else {
        ext.state.source.clone()
    };
    let version = if !ext.state.tag.is_empty() {
        ext.state.tag.clone()
    } else {
        ext.state.commit.clone()
    };
    TableRow {
        name: ext.name.clone(),
        repo,
        version,
        pinned: ext.state.pinned,
        status: None,
    }
}

/// Synthesize a `TableRow` for an extension whose name is not present in
/// the `list` snapshot. Used by `upgrade --all` so that an outcome (often
/// an error) reported by the SDK still surfaces on stdout instead of
/// being silently dropped when the on-disk state disagrees with the
/// snapshot we took before the upgrade loop started.
pub(super) fn row_for_unknown(name: &str) -> TableRow {
    TableRow {
        name: name.to_string(),
        repo: "-".to_string(),
        version: "-".to_string(),
        pinned: false,
        status: None,
    }
}

/// Truncate a commit SHA to 8 chars; leave tag-shaped strings untouched.
pub(super) fn short_sha(v: &str) -> String {
    let is_sha = v.len() >= 40 && v.chars().all(|c| c.is_ascii_hexdigit());
    if is_sha {
        v.chars().take(8).collect()
    } else {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_row_for_installed_script_includes_short_sha_as_version() {
        let row = TableRow {
            name: "deploy".to_string(),
            repo: "flox-examples/flox-deploy".to_string(),
            version: "abcdef1234567890abcdef1234567890abcdef12".to_string(),
            pinned: false,
            status: Some("up-to-date".to_string()),
        };
        let out = render_row(&row);
        assert!(out.contains("deploy"));
        assert!(out.contains("flox-examples/flox-deploy"));
        // version column truncates commit SHAs to 8 chars.
        assert!(out.contains("abcdef12"));
        assert!(!out.contains("abcdef1234567890abcdef1234567890abcdef12"));
        assert!(out.contains("up-to-date"));
    }

    #[test]
    fn render_row_pinned_shows_yes() {
        let row = TableRow {
            name: "report".to_string(),
            repo: "acme/flox-report".to_string(),
            version: "v1.2.3".to_string(),
            pinned: true,
            status: None,
        };
        let out = render_row(&row);
        assert!(out.contains("yes"));
        assert!(out.contains("v1.2.3"));
    }

    /// BUG-15 regression: `row_for_unknown` must produce a renderable row
    /// that surfaces a name reported by `upgrade_all` but missing from the
    /// pre-loop snapshot — otherwise error outcomes are silently dropped.
    #[test]
    fn row_for_unknown_renders_name_with_placeholders() {
        let row = row_for_unknown("ghost");
        let out = render_row(&row);
        assert!(out.contains("ghost"));
        // repo and version placeholders are '-' so the row is obviously
        // partial; pinned/status columns are empty-by-default.
        assert!(out.contains('-'));
    }

    #[test]
    fn render_header_matches_column_order() {
        let h = render_header();
        let name_idx = h.find("NAME").unwrap();
        let repo_idx = h.find("REPO").unwrap();
        let version_idx = h.find("VERSION").unwrap();
        let pinned_idx = h.find("PINNED").unwrap();
        let status_idx = h.find("STATUS").unwrap();
        assert!(name_idx < repo_idx);
        assert!(repo_idx < version_idx);
        assert!(version_idx < pinned_idx);
        assert!(pinned_idx < status_idx);
    }
}
