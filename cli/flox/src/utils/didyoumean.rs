use std::fmt::Display;

use anyhow::Result;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{global_manifest_path, Environment};
use flox_rust_sdk::models::lockfile::LockedManifest;
use flox_rust_sdk::models::search::{do_search, PathOrJson, SearchResults};
use log::debug;

use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::search::construct_search_params;

const SUGGESTION_SEARCH_LIMIT: u8 = 3;

fn suggest_curated_package(input: &str) -> Option<&'static str> {
    let suggestion = match input {
        "node" => "nodejs",
        "npm" => "nodejs",
        "rust" => "cargo",
        _ => return None,
    };
    Some(suggestion)
}

fn suggest_searched_packages(
    flox: &Flox,
    environment: &dyn Environment,
    term: &str,
) -> Result<SearchResults> {
    let lockfile_path = environment.lockfile_path(flox)?;

    // Use the global lock if we don't have a lock yet
    let lockfile = if lockfile_path.exists() {
        PathOrJson::Path(lockfile_path)
    } else {
        PathOrJson::Path(LockedManifest::ensure_global_lockfile(flox)?)
    };

    let search_params = construct_search_params(
        term,
        Some(SUGGESTION_SEARCH_LIMIT),
        Some(environment.manifest_path(flox)?.try_into()?),
        global_manifest_path(flox).try_into()?,
        lockfile,
    )?;

    let (results, _) = Dialog {
        message: "Looking for suggestions...",
        help_message: None,
        typed: Spinner::new(|| do_search(&search_params)),
    }
    .spin()?;

    Ok(results)
}

pub struct DidYouMean<'a> {
    searched_term: &'a str,
    curated: Option<&'static str>,
    search_results: SearchResults,
}

impl<'a> DidYouMean<'a> {
    pub fn new(flox: &Flox, environment: &dyn Environment, term: &'a str) -> Self {
        let curated = suggest_curated_package(term);
        let searched_term = curated.unwrap_or(term);
        let search_results = match suggest_searched_packages(flox, environment, searched_term) {
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
            term,
            curated,
            searched_term,
        }
    }

    pub fn has_suggestions(&self) -> bool {
        self.curated.is_some() || !self.search_results.results.is_empty()
    }
}

impl Display for DidYouMean<'_> {
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

        for result in self.search_results.results.iter() {
            writeln!(
                f,
                "  $ flox install {path}",
                path = result.rel_path.join(".")
            )?;
        }

        write!(
            f,
            "...or see {count_message} options with 'flox search {term}'",
            term = self.searched_term
        )?;

        Ok(())
    }
}
