use std::fmt::Display;
use std::io::stdout;

use anyhow::Result;
use crossterm::style::Stylize;
use crossterm::tty::IsTty;
use flox_rust_sdk::models::search::{SearchResult, SearchResults};

pub const DEFAULT_DESCRIPTION: &'_ str = "<no description provided>";

/// An intermediate representation of a search result used for rendering
#[derive(Debug, PartialEq, Clone)]
pub struct DisplayItem {
    /// The package path of the package, including catalog name
    pkg_path: String,
    /// The package description
    description: Option<String>,
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
        // pkg-path and the description.
        let display_items = search_results
            .into_iter()
            .map(|r| DisplayItem {
                pkg_path: r.pkg_path,
                description: r.description.map(|s| s.replace('\n', " ")),
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
    /// Whether to bold the search term matches in the output
    use_bold: bool,
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
    ) -> Result<DisplaySearchResults> {
        let n_results = search_results.results.len();

        let display_items: DisplayItems = search_results.results.into();

        let use_bold = stdout().is_tty();

        Ok(DisplaySearchResults {
            search_term: search_term.to_string(),
            display_items,
            count: search_results.count,
            n_results: n_results as u64,
            use_bold,
        })
    }
}

impl Display for DisplaySearchResults {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let format_name = |name: &str| {
            if self.use_bold {
                name.replace(
                    &self.search_term,
                    &format!("{}", self.search_term.clone().bold()),
                )
            } else {
                name.to_string()
            }
        };

        let column_width = self
            .display_items
            .iter()
            .map(|d| d.to_string().len())
            .max()
            .unwrap_or_default();

        // Finally print something
        let mut items = self.display_items.iter().peekable();

        while let Some(d) = items.next() {
            let desc = if d.description.as_deref().is_none_or(|s| s.is_empty()) {
                DEFAULT_DESCRIPTION
            } else {
                d.description.as_deref().unwrap()
            };
            let name = format_name(&d.to_string());
            let width = column_width + (name.len() - d.to_string().len());

            // The two spaces here provide visual breathing room.
            write!(f, "{name:<width$}  {desc}")?;
            // Only print a newline if there are more items to print
            if items.peek().is_some() {
                writeln!(f)?;
            }
        }

        Ok(())
    }
}

impl DisplaySearchResults {
    pub fn search_results_truncated_hint(&self) -> Option<String> {
        let count = self.count?;

        if count == self.n_results {
            return None;
        }

        Some(format!(
                "Showing {n_results} of {count} results. Use `flox search {search_term} --all` to see the full list.",
                n_results = self.n_results,
                search_term = self.search_term
            ))
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::models::search::SearchResult;
    use indoc::indoc;

    use super::*;

    #[test]
    fn test_display_search_result() {
        let search_results = vec![
            SearchResult {
                input: "nixpkgs".to_string(),
                pkg_path: "pkg1".to_string(),
                description: Some("description of pkg1".to_string()),
                ..Default::default()
            },
            SearchResult {
                input: "mycatalog".to_string(),
                pkg_path: "mycatalog/pkg1".to_string(),
                description: Some("description of mycatalog/pkg1".to_string()),
                ..Default::default()
            },
        ];

        let display = DisplaySearchResults {
            search_term: "pkg1".to_string(),
            count: Some(search_results.len() as u64),
            display_items: search_results.into(),
            n_results: 2,
            use_bold: false,
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
            SearchResult {
                pkg_path: "pkg1".to_string(),
                description: None,
                ..Default::default()
            },
            SearchResult {
                pkg_path: "pkg2".to_string(),
                description: Some("".to_string()),
                ..Default::default()
            },
        ];

        let display = DisplaySearchResults {
            search_term: "pkg".to_string(),
            count: Some(search_results.len() as u64),
            display_items: search_results.into(),
            n_results: 2,
            use_bold: false,
        };

        let expected = indoc! {"
            pkg1  <no description provided>
            pkg2  <no description provided>
            "};
        assert_eq!(expected, format!("{}\n", display));
    }
}
