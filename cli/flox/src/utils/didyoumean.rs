use std::fmt::Display;
use std::num::NonZeroU8;
use std::time::Duration;

use anyhow::Result;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{global_manifest_path, Environment};
use flox_rust_sdk::models::lockfile::LockedManifestPkgdb;
use flox_rust_sdk::models::search::{do_search, PathOrJson, SearchResults};
use flox_rust_sdk::providers::catalog::{Client, ClientTrait};
use log::debug;

use super::search::{DisplayItems, DisplaySearchResults};
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::search::construct_search_params;

pub const SUGGESTION_SEARCH_LIMIT: u8 = 3;

/// Dynamically generate a "did you mean" message for a given search term.
/// Will look up curated suggested search terms and query related search results.
///
/// [DidYouMean] is parameterized by a type `S`,
/// which is used to distinguish input types for the suggestion
/// and specific suggestion output.
#[derive(Debug)]
pub struct DidYouMean<'a, S> {
    searched_term: &'a str,
    curated: Option<&'static str>,
    search_results: SearchResults,
    _suggestion: S,
}

#[derive(Debug)]
pub struct InstallSuggestion;

impl<S> DidYouMean<'_, S> {
    pub fn has_suggestions(&self) -> bool {
        self.curated.is_some() || !self.search_results.results.is_empty()
    }
}

/// Suggestions for `install` subcommand
impl<'a> DidYouMean<'a, InstallSuggestion> {
    /// `install` specific curated terms
    fn suggest_curated_package(input: &str) -> Option<&'static str> {
        let suggestion = match input {
            "java" => "jdk",
            "node" => "nodejs",
            "npm" => "nodejs",
            "rust" => "cargo",
            "sed" => "gnused",
            "make" => "gnumake",
            "awk" => "gawk",
            "diff" => "diffutils",
            "grep" => "gnugrep",
            _ => return None,
        };
        Some(suggestion)
    }

    fn suggest_searched_packages(
        flox: &Flox,
        environment: &dyn Environment,
        term: &str,
    ) -> Result<SearchResults> {
        match flox.catalog_client {
            Some(ref client) => {
                tracing::debug!("using client for install suggestions");
                Self::suggest_searched_packages_catalog(client, term, flox.system.clone())
            },
            None => {
                tracing::debug!("using pkgdb for install suggestions");
                Self::suggest_searched_packages_pkgdb(flox, environment, term)
            },
        }
    }

    /// Collects installation suggestions for a given query using pkgdb
    fn suggest_searched_packages_pkgdb(
        flox: &Flox,
        environment: &dyn Environment,
        term: &str,
    ) -> Result<SearchResults> {
        let lockfile_path = environment.lockfile_path(flox)?;

        // Use the global lock if we don't have a lock yet
        let lockfile = if lockfile_path.exists() {
            PathOrJson::Path(lockfile_path)
        } else {
            PathOrJson::Path(LockedManifestPkgdb::ensure_global_lockfile(flox)?)
        };

        let search_params = construct_search_params(
            term,
            NonZeroU8::new(SUGGESTION_SEARCH_LIMIT),
            Some(environment.manifest_path(flox)?.try_into()?),
            global_manifest_path(flox).try_into()?,
            lockfile,
        )?;

        let (results, _) = Dialog {
            message: &format!("Could not find package for {term}. Looking for suggestions..."),
            help_message: None,
            typed: Spinner::new(|| do_search(&search_params)),
        }
        .spin()?;

        Ok(results)
    }

    /// Collects installation suggestions for a given query using the catalog
    fn suggest_searched_packages_catalog(
        client: &Client,
        term: &str,
        system: String,
    ) -> Result<SearchResults> {
        let results = Dialog {
            message: "Looking for alternative suggestions...",
            help_message: None,
            typed: Spinner::new(|| {
                tokio::runtime::Handle::current().block_on(client.search(
                    term,
                    system.to_string(),
                    NonZeroU8::new(SUGGESTION_SEARCH_LIMIT),
                ))
            }),
        }
        .spin_with_delay(Duration::from_secs(1))?;
        Ok(results)
    }

    /// Create a new [DidYouMean] instance for the given search term
    /// in the context of an [Environment].
    ///
    /// This will attempt to find curated suggestions for the given term,
    /// based on the lockfile of the given environment.
    pub fn new(flox: &Flox, environment: &dyn Environment, term: &'a str) -> Self {
        let curated = Self::suggest_curated_package(term);
        let searched_term = curated.unwrap_or(term);
        let search_results = match Self::suggest_searched_packages(flox, environment, searched_term)
        {
            Ok(results) => results,
            Err(err) => {
                debug!("failed to search for suggestions: {}", err);
                SearchResults {
                    results: Default::default(),
                    count: Some(0),
                }
            },
        };
        Self {
            searched_term,
            curated,
            search_results,
            _suggestion: InstallSuggestion,
        }
    }
}

impl Display for DidYouMean<'_, InstallSuggestion> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(curated) = self.curated {
            writeln!(
                f,
                "Try 'flox install {curated}' instead.",
                curated = curated
            )?;
        }

        if self.search_results.results.is_empty() {
            return Ok(());
        }

        writeln!(f)?;
        writeln!(f, "Here are a few other similar options:")?;

        // apparently its possible for pkgdb to _not_ return a count?
        let count_message = match self.search_results.count {
            Some(n) => format!("up to {n}"),
            None => "more".to_string(),
        };

        let display_items: DisplayItems = self.search_results.results.clone().into();
        for result in display_items.iter() {
            writeln!(f, "  $ flox install {result}",)?;
        }

        write!(
            f,
            "...or see {count_message} options with 'flox search {term}'",
            term = self.searched_term
        )?;

        Ok(())
    }
}

pub struct SearchSuggestion;

/// Suggestions for `search` subcommand
impl<'a> DidYouMean<'a, SearchSuggestion> {
    /// `search` specific curated terms
    fn suggest_curated_package(input: &str) -> Option<&'static str> {
        let suggestion = match input {
            "node" => "nodejs",
            "java" => "jdk",
            "npm" => "nodejs",
            "rust" => "cargo",
            "diff" => "diffutils",
            "make" => "gnumake",
            _ => return None,
        };
        Some(suggestion)
    }

    /// `search` may run without a (local) manifest,
    /// but still needs to be able to suggest search results
    /// based on an existing (global) manifest/lockfile.
    fn suggest_searched_packages_pkgdb(
        term: &str,
        manifest: Option<PathOrJson>,
        global_manifest: PathOrJson,
        lockfile: PathOrJson,
    ) -> Result<SearchResults> {
        let search_params = construct_search_params(
            term,
            NonZeroU8::new(SUGGESTION_SEARCH_LIMIT),
            manifest,
            global_manifest,
            lockfile,
        )?;

        let (results, _) = Dialog {
            message: "Looking for alternative suggestions...",
            help_message: None,
            typed: Spinner::new(|| do_search(&search_params)),
        }
        .spin()?;

        Ok(results)
    }

    fn suggest_searched_packages_catalog(
        client: &Client,
        term: &str,
        system: String,
    ) -> Result<SearchResults> {
        let results = Dialog {
            message: "Looking for alternative suggestions...",
            help_message: None,
            typed: Spinner::new(|| {
                tokio::runtime::Handle::current().block_on(client.search(
                    term,
                    system.to_string(),
                    NonZeroU8::new(SUGGESTION_SEARCH_LIMIT),
                ))
            }),
        }
        .spin_with_delay(Duration::from_secs(1))?;
        Ok(results)
    }

    /// Create a new [DidYouMean] instance for the given search term.
    ///
    /// This will attempt to find curated suggestions for the given term,
    /// and then query for related search results.
    /// Either of these may fail, in which case we will return with empty [SearchResults]
    /// and log the error.
    pub fn new(
        term: &'a str,
        catalog_client: Option<Client>,
        system: String,
        manifest: Option<PathOrJson>,
        global_manifest: PathOrJson,
        lockfile: PathOrJson,
    ) -> Self {
        let curated = Self::suggest_curated_package(term);

        let default_results = SearchResults {
            results: Default::default(),
            count: Some(0),
        };

        let search_results = if let Some(curated) = curated {
            let res = if let Some(ref client) = catalog_client {
                Self::suggest_searched_packages_catalog(client, curated, system)
            } else {
                Self::suggest_searched_packages_pkgdb(curated, manifest, global_manifest, lockfile)
            };
            match res {
                Ok(results) => results,
                Err(err) => {
                    debug!("failed to search for suggestions: {}", err);
                    default_results
                },
            }
        } else {
            default_results
        };

        Self {
            searched_term: term,
            curated,
            search_results,
            _suggestion: SearchSuggestion,
        }
    }
}

impl Display for DidYouMean<'_, SearchSuggestion> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Some(curated) = self.curated else {
            debug!("no curated suggestions");
            return Ok(());
        };

        let search_results_rendered =
            match DisplaySearchResults::from_search_results(curated, self.search_results.clone()) {
                Ok(rendered) => rendered,
                Err(err) => {
                    debug!("failed to render search results: {}", err);
                    return Ok(());
                },
            };

        writeln!(f, "Related search results for '{curated}':")?;
        write!(f, "{search_results_rendered}")?;

        Ok(())
    }
}
