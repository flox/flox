use crate::beta::extensions::Extension;

/// Render a single row of the `flox extension list` table: the extension
/// name and the source path it was installed from.
pub(super) struct TableRow {
    pub name: String,
    pub path: String,
}

pub(super) fn render_header() -> String {
    format!("{:<20}  {}", "NAME", "PATH")
}

pub(super) fn render_row(row: &TableRow) -> String {
    format!("{:<20}  {}", row.name, row.path)
}

/// Convert an `Extension` to a `TableRow`.
pub(super) fn row_from_extension(ext: &Extension) -> TableRow {
    TableRow {
        name: ext.name.clone(),
        path: ext.state.source.clone(),
    }
}

#[cfg(test)]
#[cfg(feature = "beta-tests")]
mod tests {
    use super::*;

    #[test]
    fn render_row_shows_name_and_source_path() {
        let row = TableRow {
            name: "deploy".to_string(),
            path: "/home/u/src/flox-deploy".to_string(),
        };
        let out = render_row(&row);
        assert!(out.contains("deploy"));
        assert!(out.contains("/home/u/src/flox-deploy"));
    }

    #[test]
    fn render_header_matches_column_order() {
        let h = render_header();
        let name_idx = h.find("NAME").unwrap();
        let path_idx = h.find("PATH").unwrap();
        assert!(name_idx < path_idx);
    }
}
