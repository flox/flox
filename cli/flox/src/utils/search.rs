use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::io::stdout;

use anyhow::Result;
use crossterm::style::Stylize;
use crossterm::tty::IsTty;
use flox_rust_sdk::models::search::{SearchResult, SearchResults};

pub const SEARCH_INPUT_SEPARATOR: &'_ str = ":";
pub const DEFAULT_DESCRIPTION: &'_ str = "<no description provided>";

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
            .map(|d| format_name(&d.to_string()).len())
            .max()
            .unwrap_or_default();

        // Finally print something
        let mut items = self.display_items.iter().peekable();

        while let Some(d) = items.next() {
            let desc = d.description.as_deref().unwrap_or(DEFAULT_DESCRIPTION);
            let name = format_name(&d.to_string());

            // The two spaces here provide visual breathing room.
            write!(f, "{name:<column_width$}  {desc}", name = name)?;
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

#[cfg(test)]
mod tests {

    use std::path::PathBuf;

    use flox_rust_sdk::flox::test_helpers::flox_instance_with_global_lock;
    use flox_rust_sdk::flox::Flox;
    use flox_rust_sdk::models::environment::global_manifest_lockfile_path;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::{
        new_path_environment,
        new_path_environment_from_env_files,
    };
    use flox_rust_sdk::models::lockfile::LockedManifestPkgdb;
    use flox_rust_sdk::providers::catalog::MANUALLY_GENERATED;
    use serial_test::serial;
    use tracing::debug;

    use super::*;
    use crate::commands::{ConcreteEnvironment, UninitializedEnvironment};

    /// Helper function for [manifest_and_lockfile] that can be unit tested.
    fn manifest_and_lockfile_from_detected_environment(
        flox: &Flox,
        detected_environment: Option<UninitializedEnvironment>,
    ) -> Result<(Option<PathBuf>, PathBuf)> {
        let (manifest_path, lockfile_path) = match detected_environment {
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
            None => LockedManifestPkgdb::ensure_global_lockfile(flox)?,
        };
        Ok((manifest_path, lockfile_path))
    }

    /// When no environment has been detected, the global lockfile is used.
    #[test]
    #[serial]
    fn test_manifest_and_lockfile_global_lock() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();
        assert_eq!(
            manifest_and_lockfile_from_detected_environment(&flox, None).unwrap(),
            (None, global_manifest_lockfile_path(&flox))
        );
    }

    /// When an environment has been detected but has no lockfile, the
    /// environment's manifest and the global lockfile are used.
    #[test]
    #[serial]
    fn test_manifest_and_lockfile_environment_manifest() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();
        let environment = new_path_environment(&flox, "");
        let (manifest, lockfile) = manifest_and_lockfile_from_detected_environment(
            &flox,
            Some(
                UninitializedEnvironment::from_concrete_environment(&ConcreteEnvironment::Path(
                    environment,
                ))
                .unwrap(),
            ),
        )
        .unwrap();

        let manifest = manifest.unwrap();

        assert!(manifest.starts_with(flox.temp_dir.canonicalize().unwrap()));
        assert!(manifest.ends_with(".flox/env/manifest.toml"));

        assert_eq!(lockfile, global_manifest_lockfile_path(&flox));
    }

    /// When an environment has been detected and has a lockfile, that lockfile
    /// should be used
    #[test]
    #[serial]
    fn test_manifest_and_lockfile_environment_lock() {
        let (flox, _temp_dir_handle) = flox_instance_with_global_lock();
        let environment =
            new_path_environment_from_env_files(&flox, MANUALLY_GENERATED.join("hello_v0"));
        let (manifest, lockfile) = manifest_and_lockfile_from_detected_environment(
            &flox,
            Some(
                UninitializedEnvironment::from_concrete_environment(&ConcreteEnvironment::Path(
                    environment,
                ))
                .unwrap(),
            ),
        )
        .unwrap();

        let manifest = manifest.unwrap();

        assert!(manifest.starts_with(flox.temp_dir.canonicalize().unwrap()));
        assert!(manifest.ends_with(".flox/env/manifest.toml"));

        assert!(lockfile.starts_with(flox.temp_dir.canonicalize().unwrap()));
        assert!(lockfile.ends_with(".flox/env/manifest.lock"));
    }
}
