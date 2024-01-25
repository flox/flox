use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::lockfile::LockedManifest;
use flox_rust_sdk::models::search::{PathOrJson, Query, SearchParams, SearchResults};
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
            debug!("using environment {uninitialized}");

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
    let query = Query::from_term_and_limit(
        search_term,
        Features::parse()?.search_strategy,
        results_limit,
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

/// Deduplicate and disambiguate display items.
///
/// This gets complicated because we have to satisfy a few constraints:
/// - The order of results from `pkgdb` is important (best matches come first),
///   so that order must be preserved.
/// - Versions shouldn't appear in the output, so multiple package versions from a single
///   input should be deduplicated.
/// - Packages that appear in more than one input need to be disambiguated by prepending
///   the name of the input and a separator.
fn dedup_and_disambiguate_display_items(mut display_items: Vec<DisplayItem>) -> Vec<DisplayItem> {
    let mut package_to_inputs: HashMap<String, HashSet<String>> = HashMap::new();
    for d in display_items.iter() {
        // Build a collection of packages and which inputs they are seen in so we can tell
        // which packages need to be disambiguated when rendering search results.
        package_to_inputs
            .entry(d.package.clone())
            .and_modify(|inputs| {
                inputs.insert(d.input.clone());
            })
            .or_insert_with(|| HashSet::from_iter([d.input.clone()]));
    }

    // For any package that comes from more than one input, mark it as needing to be joined
    for d in display_items.iter_mut() {
        if let Some(inputs) = package_to_inputs.get(&d.package) {
            d.render_with_input = inputs.len() > 1;
        }
    }

    // For each package in the search results, `package_to_inputs` contains the set of
    // inputs that the package is found in. Logically `package_to_inputs` contains
    // (package, input) pairs. If the `package` and `input` from a `DisplayItem` are
    // found in `package_to_inputs` it means that we have not yet seen this (package, input)
    // pair and we should render it (e.g. add it to `deduped_display_items`). Once we've
    // done that we remove this (package, input) pair from `package_to_inputs` so that
    // we never see that pair again.
    let mut deduped_display_items = Vec::new();
    for d in display_items.into_iter() {
        if let Some(inputs) = package_to_inputs.get_mut(d.package.as_str()) {
            // Remove this input so this (package, input) pair is never seen again
            if inputs.remove(&d.input) {
                deduped_display_items.push(d.clone());
            }
            if inputs.is_empty() {
                package_to_inputs.remove(&d.package);
            }
        }
    }

    deduped_display_items
}

/// An intermediate representation of a search result used for rendering
#[derive(Debug, PartialEq, Clone)]
struct DisplayItem {
    /// The input that the package came from
    input: String,
    /// The displayable part of the package's attribute path
    package: String,
    /// The package description
    description: Option<String>,
    /// Whether to join the `input` and `package` fields with a separator when rendering
    render_with_input: bool,
}

pub struct DisplaySearchResults {
    /// original search term
    search_term: String,
    /// deduplicated and disambiguated search results
    deduped_display_items: Vec<DisplayItem>,
    /// reported number of results
    count: Option<u64>,
    /// number of actual results (including duplicates)
    n_results: u64,
}

impl Display for DisplaySearchResults {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let column_width = self
            .deduped_display_items
            .iter()
            .map(|d| {
                if d.render_with_input {
                    d.input.len() + d.package.len() + SEARCH_INPUT_SEPARATOR.len()
                } else {
                    d.package.len()
                }
            })
            .max()
            .unwrap_or_default();

        // Finally print something
        let mut items = self.deduped_display_items.iter().peekable();

        while let Some(d) = items.next() {
            let desc = d.description.as_deref().unwrap_or(DEFAULT_DESCRIPTION);
            let package = if d.render_with_input {
                [&*d.input, &*d.package].join(SEARCH_INPUT_SEPARATOR)
            } else {
                d.package.to_string()
            };
            write!(f, "{package:<column_width$}  {desc}")?;
            // Only print a newline if there are more items to print
            if items.peek().is_some() {
                writeln!(f)?;
            }
        }
        Ok(())
    }
}

impl DisplaySearchResults {
    pub fn hint(&self) -> Option<String> {
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
                "Showing {n_deduplicated} of {count} results. Use `flox search {search_term} --all` to see the full list.",
                n_deduplicated = self.deduped_display_items.len(),
                search_term = self.search_term
            ))
    }
}

/// Display a list of search results for a given search term
/// This function is responsible for deduplicating and disambiguating search results
/// and printing them to stdout in a user-friendly table-ish format.
///
/// If no results are found, this function will print nothing
/// it's the caller's responsibility to print a message,
/// or error if no results are found.
pub(crate) fn render_search_results_user_facing(
    search_term: &str,
    search_results: SearchResults,
) -> Result<DisplaySearchResults> {
    let n_results = search_results.results.len();

    // Search results contain a lot of information, but all we need for rendering are
    // the input, the package subpath (e.g. "python310Packages.flask"), and the description.
    let display_items = search_results
        .results
        .into_iter()
        .map(|r| {
            Ok(DisplayItem {
                input: r.input,
                package: r.rel_path.join("."),
                description: r.description.map(|s| s.replace('\n', " ")),
                render_with_input: false,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let deduped_display_items = dedup_and_disambiguate_display_items(display_items);
    if deduped_display_items.is_empty() {
        bail!("deduplicating search results failed");
    }

    Ok(DisplaySearchResults {
        search_term: search_term.to_string(),
        deduped_display_items,
        count: search_results.count,
        n_results: n_results as u64,
    })
}
