use std::fmt::{self, Display};
use std::io::Write;

use anyhow::Result;
use crossterm::style::Stylize;
use floxhub_client::{DeprecationInfo, SearchResult, SearchResults};

use crate::utils::deprecation::deprecation_notice;
use crate::utils::table::{table_string, table_writer};

pub const DEFAULT_DESCRIPTION: &'_ str = "<no description provided>";

/// An intermediate representation of a search result used for rendering
#[derive(Debug, PartialEq, Clone)]
pub struct DisplayItem {
    /// The package path of the package, including catalog name
    pkg_path: String,
    /// The package description
    description: Option<String>,
    /// Set when the catalog reports the package as deprecated
    deprecation: Option<DeprecationInfo>,
}

impl Display for DisplayItem {
    /// Render a display item in the format that should be output by
    /// `flox search`.
    ///
    /// It should be possible to copy and paste this as an argument to
    /// `flox install`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.pkg_path)
    }
}

/// Contains [DisplayItem]s that have been disambiguated.
///
/// This should be used for printing search results when the format output by
/// [DisplaySearchResults] is not desired.
pub struct DisplayItems(Vec<DisplayItem>);

impl DisplayItems {
    pub fn iter(&self) -> impl Iterator<Item = &DisplayItem> {
        self.0.iter()
    }
}

impl From<Vec<SearchResult>> for DisplayItems {
    fn from(search_results: Vec<SearchResult>) -> Self {
        // Search results contain a lot of information, but all we need for rendering is the
        // pkg-path, the description and any deprecation.
        let display_items = search_results
            .into_iter()
            .map(|r| DisplayItem {
                pkg_path: r.pkg_path,
                description: r.description.map(|s| s.replace('\n', " ")),
                deprecation: r.deprecation,
            })
            .collect::<Vec<_>>();

        Self(display_items)
    }
}

pub struct DisplaySearchResults {
    /// original search term
    search_term: String,
    /// deduplicated and disambiguated search results
    display_items: DisplayItems,
    /// reported number of results
    count: Option<u64>,
    /// number of actual results (including duplicates)
    n_results: u64,
    /// Whether the stream the table is written to supports colour: bolds the
    /// search term matches and colours the marker of a deprecation notice
    use_color: bool,
}

/// A struct that wraps the functionality needed to print [SearchResults] to a
/// user.
impl DisplaySearchResults {
    /// Display a list of search results for a given search term
    /// This function is responsible for disambiguating search results
    /// and printing them to stdout in a user-friendly table-ish format.
    ///
    /// If no results are found, this function will print nothing
    /// it's the caller's responsibility to print a message,
    /// or error if no results are found.
    pub(crate) fn from_search_results(
        search_term: &str,
        search_results: SearchResults,
        use_color: bool,
    ) -> Result<DisplaySearchResults> {
        let n_results = search_results.results.len();

        let display_items: DisplayItems = search_results.results.into();

        Ok(DisplaySearchResults {
            search_term: search_term.to_string(),
            display_items,
            count: search_results.count,
            n_results: n_results as u64,
            use_color,
        })
    }
}

impl Display for DisplaySearchResults {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let format_name = |name: &str| {
            if self.use_color {
                name.replace(
                    &self.search_term,
                    &format!("{}", self.search_term.clone().bold()),
                )
            } else {
                name.to_string()
            }
        };

        let marker = if self.use_color {
            "!".yellow().to_string()
        } else {
            "!".to_string()
        };

        let mut tw = table_writer(Vec::new());

        for (i, d) in self.display_items.iter().enumerate() {
            let desc = if d.description.as_deref().is_none_or(|s| s.is_empty()) {
                DEFAULT_DESCRIPTION
            } else {
                d.description.as_deref().unwrap()
            };
            let name = format_name(&d.to_string());

            // No newline after the last line, so the newline goes first.
            if i > 0 {
                writeln!(tw).map_err(|_| fmt::Error)?;
            }
            write!(tw, "{name}\t{desc}").map_err(|_| fmt::Error)?;

            // The notice goes under its row, in the description column. Its
            // empty first cell ends in a tab so the line is part of the
            // table: a line without one would be measured as a name and
            // widen the name column of every row. The marker follows the
            // table's colour flag instead of `message::warning`, which
            // always decides on stderr's colour support.
            if let Some(deprecation) = &d.deprecation {
                let notice = deprecation_notice(&d.pkg_path, deprecation);
                write!(tw, "\n\t{marker} {notice}").map_err(|_| fmt::Error)?;
            }
        }

        f.write_str(&table_string(tw))
    }
}

impl DisplaySearchResults {
    pub fn search_results_truncated_hint(&self) -> Option<String> {
        let count = self.count?;

        if count == self.n_results {
            return None;
        }

        Some(format!(
            "Showing {n_results} of {count} results. Use 'flox search {search_term} --all' to see the full list.",
            n_results = self.n_results,
            search_term = self.search_term
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use flox_rust_sdk::providers::catalog::SystemEnum;
    use floxhub_client::DeprecationKind;
    use indoc::{formatdoc, indoc};

    use super::*;

    fn stub_search_result(name: &str, description: Option<&str>) -> SearchResult {
        SearchResult {
            attr_path: name.to_string(),
            name: name.to_string(),
            pname: name.to_string(),
            stabilities: vec![],
            system: SystemEnum::from_str("aarch64-darwin").unwrap(),
            catalog: None,
            pkg_path: name.to_string(),
            description: description.map(|s| s.to_string()),
            version: None,
            deprecation: None,
        }
    }

    #[test]
    fn test_display_search_result() {
        let search_results = vec![
            stub_search_result("pkg1", Some("description of pkg1")),
            stub_search_result("mycatalog/pkg1", Some("description of mycatalog/pkg1")),
        ];

        let display = DisplaySearchResults {
            search_term: "pkg1".to_string(),
            count: Some(search_results.len() as u64),
            display_items: search_results.into(),
            n_results: 2,
            use_color: false,
        };

        let expected = indoc! {"
            pkg1            description of pkg1
            mycatalog/pkg1  description of mycatalog/pkg1
            "};
        assert_eq!(expected, format!("{}\n", display));
    }

    #[test]
    fn test_display_empty_description() {
        let search_results = vec![
            stub_search_result("pkg1", None),
            stub_search_result("pkg2", Some("")),
        ];

        let display = DisplaySearchResults {
            search_term: "pkg".to_string(),
            count: Some(search_results.len() as u64),
            display_items: search_results.into(),
            n_results: 2,
            use_color: false,
        };

        let expected = indoc! {"
            pkg1  <no description provided>
            pkg2  <no description provided>
            "};
        assert_eq!(expected, format!("{}\n", display));
    }

    #[test]
    fn display_search_result_aligns_bolded_search_term() {
        // Only one name contains the search term, so only that one carries
        // escape codes that must not count towards its width.
        let search_results = vec![
            stub_search_result("pkg1", Some("description of pkg1")),
            stub_search_result("ripgrep", Some("mentions pkg1")),
        ];

        let display = DisplaySearchResults {
            search_term: "pkg1".to_string(),
            count: Some(search_results.len() as u64),
            display_items: search_results.into(),
            n_results: 2,
            use_color: true,
        };

        let bold = "pkg1".bold();
        let expected = formatdoc! {"
            {bold}     description of pkg1
            ripgrep  mentions pkg1
            "};
        assert_eq!(expected, format!("{}\n", display));
    }

    fn deprecated_search_result(
        name: &str,
        message: Option<&str>,
        replacement: Option<&str>,
    ) -> SearchResult {
        SearchResult {
            deprecation: Some(DeprecationInfo {
                kind: DeprecationKind::Removed,
                message: message.map(str::to_string),
                replacement: replacement.map(str::to_string),
            }),
            ..stub_search_result(name, Some("a description"))
        }
    }

    fn display_of(search_results: Vec<SearchResult>) -> DisplaySearchResults {
        DisplaySearchResults {
            search_term: "pkg".to_string(),
            count: Some(search_results.len() as u64),
            n_results: search_results.len() as u64,
            display_items: search_results.into(),
            use_color: false,
        }
    }

    /// The notice is on its own line under its row, in the description
    /// column, and doesn't widen the name column.
    #[test]
    fn deprecated_row_is_followed_by_its_notice() {
        let display = display_of(vec![
            stub_search_result("pkg1", Some("a description")),
            deprecated_search_result("pkg2", Some("pkg2 is EOL"), None),
            stub_search_result("longer-pkg3", Some("a description")),
        ]);
        assert_eq!(display.to_string(), indoc! {"
                pkg1         a description
                pkg2         a description
                             ! pkg2 is EOL
                longer-pkg3  a description"});
    }

    /// No newline follows the last line, deprecated or not.
    #[test]
    fn deprecated_last_row_has_no_trailing_newline() {
        let display = display_of(vec![
            stub_search_result("pkg1", Some("a description")),
            deprecated_search_result("pkg2", Some("pkg2 is EOL"), None),
        ]);
        assert_eq!(
            display.to_string(),
            "pkg1  a description\npkg2  a description\n      ! pkg2 is EOL"
        );
    }

    #[test]
    fn deprecated_row_without_description_keeps_the_default() {
        let result = SearchResult {
            description: None,
            ..deprecated_search_result("pkg1", Some("pkg1 is EOL"), None)
        };
        assert_eq!(display_of(vec![result]).to_string(), indoc! {"
                pkg1  <no description provided>
                      ! pkg1 is EOL"});
    }

    /// The notice names its package and the replacement, as `flox show` does.
    #[test]
    fn notice_in_table_is_the_shared_notice() {
        let display = display_of(vec![
            deprecated_search_result("old", Some("no longer maintained"), Some("new")),
            stub_search_result("new", Some("a description")),
        ]);
        assert_eq!(display.to_string(), indoc! {"
                old  a description
                     ! 'old' is marked as removed in nixpkgs: no longer maintained. Use 'new' instead.
                new  a description"});
    }

    #[test]
    fn table_without_deprecations_is_unchanged() {
        let display = display_of(vec![
            stub_search_result("pkg1", Some("a description")),
            stub_search_result("longer-pkg2", Some("a description")),
        ]);
        assert_eq!(display.to_string(), indoc! {"
                pkg1         a description
                longer-pkg2  a description"});
    }

    #[test]
    fn colour_off_has_no_escape_codes() {
        let mut display = display_of(vec![
            stub_search_result("pkg1", Some("a description")),
            deprecated_search_result("pkg2", Some("pkg2 is EOL"), None),
        ]);
        // Not in any name, so bolding the search term doesn't show up here.
        display.search_term = "zzz".to_string();
        assert_eq!(display.to_string(), indoc! {"
                pkg1  a description
                pkg2  a description
                      ! pkg2 is EOL"});
    }

    /// Only the marker is yellow, and the notice stays in the description
    /// column.
    #[test]
    fn colour_on_makes_only_the_marker_yellow() {
        let mut display = display_of(vec![
            stub_search_result("pkg1", Some("a description")),
            deprecated_search_result("pkg2", Some("pkg2 is EOL"), None),
        ]);
        display.search_term = "zzz".to_string();
        display.use_color = true;
        assert_eq!(
            display.to_string(),
            "pkg1  a description\npkg2  a description\n      \x1b[38;5;11m!\x1b[39m pkg2 is EOL"
        );
    }
}
