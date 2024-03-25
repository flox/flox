use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::path::PathBuf;

use anyhow::Result;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::lockfile::LockedManifest;
use flox_rust_sdk::models::search::{PathOrJson, Query, SearchParams, SearchResult, SearchResults};
use log::debug;

use crate::commands::detect_environment;
use crate::config::features::Features;

pub const SEARCH_INPUT_SEPARATOR: &'_ str = ":";
pub const DEFAULT_DESCRIPTION: &'_ str = "<no description provided>";

/// Return an optional manifest and a lockfile to use for search and show.
///
/// This searches for an environment to use,
/// and if one is found, it returns the path to its manifest and optionally the
/// path to its lockfile.
///
/// If no environment is found, or if environment does not have a lockfile, the
/// global lockfile is used.
/// The global lockfile is created if it does not exist.
///
/// Note that this may perform network operations to pull a
/// [ManagedEnvironment],
/// since a freshly cloned user repo with a [ManagedEnvironment] may not have a
/// manifest or lockfile in floxmeta unless the environment is initialized.
pub fn manifest_and_lockfile(flox: &Flox, message: &str) -> Result<(Option<PathBuf>, PathBuf)> {
    let (manifest_path, lockfile_path) = match detect_environment(message)? {
        None => {
            debug!("no environment found");
            (None, None)
        },
        Some(uninitialized) => {
            debug!("using environment {}", uninitialized.bare_description()?);

            let environment = uninitialized
                .into_concrete_environment(flox)?
                .into_dyn_environment();

            let lockfile_path = environment.lockfile_path(flox)?;
            debug!("checking lockfile: path={}", lockfile_path.display());
            let lockfile = if lockfile_path.exists() {
                debug!("lockfile exists");
                Some(lockfile_path)
            } else {
                debug!("lockfile doesn't exist");
                None
            };
            (Some(environment.manifest_path(flox)?), lockfile)
        },
    };

    // Use the global lock if we don't have a lock yet
    let lockfile_path = match lockfile_path {
        Some(lockfile_path) => lockfile_path,
        None => LockedManifest::ensure_global_lockfile(flox)?,
    };
    Ok((manifest_path, lockfile_path))
}

/// Create [SearchParams] from the given search term
/// using available manifests and lockfiles for resolution.
pub(crate) fn construct_search_params(
    search_term: &str,
    results_limit: Option<u8>,
    manifest: Option<PathOrJson>,
    global_manifest: PathOrJson,
    lockfile: PathOrJson,
) -> Result<SearchParams> {
    let query = Query::new(
        search_term,
        Features::parse()?.search_strategy,
        results_limit,
        true,
    )?;
    let params = SearchParams {
        manifest,
        global_manifest,
        lockfile,
        query,
    };
    debug!("search params raw: {:?}", params);
    Ok(params)
}

/// An intermediate representation of a search result used for rendering
#[derive(Debug, PartialEq, Clone)]
pub struct DisplayItem {
    /// The input that the package came from
    input: String,
    /// The attribute path of the package, excluding subtree and system
    rel_path: Vec<String>,
    /// The package description
    description: Option<String>,
    /// Whether to join the `input` and `package` fields with a separator when rendering
    render_with_input: bool,
}

impl Display for DisplayItem {
    /// Render a display item in the format that should be output by
    /// `flox search`.
    ///
    /// It should be possible to copy and paste this as an argument to
    /// `flox install`.
    ///
    /// If we change this function, we will likely need to update what the
    /// deduplicate field controls in pkgdb.
    /// Technically, pkgdb shouldn't have knowledge of this format,
    /// but it's nicer to perform deduplication in SQL.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.render_with_input {
            write!(f, "{}{SEARCH_INPUT_SEPARATOR}", self.input)?;
        }
        write!(f, "{}", self.rel_path.join("."))
    }
}

/// Contains [DisplayItem]s that have been disambiguated.
///
/// This should be used for printing search results when the format output by
/// [DisplaySearchResults] is not desired.
pub struct DisplayItems(Vec<DisplayItem>);

impl DisplayItems {
    /// Disambiguate display items.
    ///
    /// This gets complicated because we have to satisfy a few constraints:
    /// - The order of results from `pkgdb` is important (best matches come first),
    ///   so that order must be preserved.
    /// - Packages that appear in more than one input need to be disambiguated by prepending
    ///   the name of the input and a separator.
    fn disambiguate_display_items(display_items: &mut [DisplayItem]) {
        let mut package_to_inputs: HashMap<Vec<String>, HashSet<String>> = HashMap::new();
        for d in display_items.iter() {
            // Build a collection of packages and which inputs they are seen in so we can tell
            // which packages need to be disambiguated when rendering search results.
            package_to_inputs
                .entry(d.rel_path.clone())
                .and_modify(|inputs| {
                    inputs.insert(d.input.clone());
                })
                .or_insert_with(|| HashSet::from_iter([d.input.clone()]));
        }

        // For any package that comes from more than one input, mark it as needing to be joined
        for d in display_items.iter_mut() {
            if let Some(inputs) = package_to_inputs.get(&d.rel_path) {
                d.render_with_input = inputs.len() > 1;
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &DisplayItem> {
        self.0.iter()
    }
}

impl From<Vec<SearchResult>> for DisplayItems {
    fn from(search_results: Vec<SearchResult>) -> Self {
        // Search results contain a lot of information, but all we need for rendering are
        // the input, the package subpath (e.g. "python310Packages.flask"), and the description.
        let mut display_items = search_results
            .into_iter()
            .map(|r| DisplayItem {
                input: r.input,
                rel_path: r.rel_path,
                description: r.description.map(|s| s.replace('\n', " ")),
                render_with_input: false,
            })
            .collect::<Vec<_>>();

        // TODO: we could disambiguate as we're collecting above
        Self::disambiguate_display_items(&mut display_items);

        Self(display_items)
    }
}

///
pub struct DisplaySearchResults {
    /// original search term
    search_term: String,
    /// deduplicated and disambiguated search results
    display_items: DisplayItems,
    /// reported number of results
    count: Option<u64>,
    /// number of actual results (including duplicates)
    n_results: u64,
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

        Ok(DisplaySearchResults {
            search_term: search_term.to_string(),
            display_items,
            count: search_results.count,
            n_results: n_results as u64,
        })
    }
}

impl Display for DisplaySearchResults {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let column_width = self
            .display_items
            .iter()
            .map(|d| d.to_string().len())
            .max()
            .unwrap_or_default();

        // Finally print something
        let mut items = self.display_items.iter().peekable();

        while let Some(d) = items.next() {
            let desc = d.description.as_deref().unwrap_or(DEFAULT_DESCRIPTION);
            write!(f, "{d:<column_width$}  {desc}", d = d.to_string())?;
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
        let Some(count) = self.count else {
            return None;
        };

        // Don't show the message if we have exactly the number of results as the limit,
        // otherwise we would get messages like `Showing 10 of 10...`
        // In addition after deduplication we may have fewer results than the limit,
        // but we dont want to show a message like `Showing 5 of 9...`,
        // when the requested number of results is 10.
        // There is still an issue wit duplicate results where even when called with `--all`
        // the number of elements in `deduped_display_items` may be less than `count`.
        // That bug will be fixed separately.
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
