use std::fmt::Display;
use std::num::NonZeroU8;

use anyhow::Result;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{CoreEnvironmentError, EnvironmentError};
use flox_rust_sdk::providers::lock_manifest::{
    ResolutionFailure,
    ResolutionFailures,
    ResolveError,
};
use floxhub_client::{CatalogClientTrait, MsgAttrPathNotFoundNotInCatalog, SearchResults};
use indoc::formatdoc;
use pollster::FutureExt;
use tracing::{debug, instrument};

use super::search::{DisplayItems, DisplaySearchResults};

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
    suggestion: S,
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

    fn suggest_searched_packages(flox: &Flox, term: &str) -> Result<SearchResults> {
        tracing::debug!("using client for install suggestions");
        let catalog = &flox.floxhub_client;
        Self::suggest_searched_packages_catalog(catalog, term, flox.system.clone())
    }

    /// Collects installation suggestions for a given query using the catalog
    #[instrument(skip(client), fields(progress = "Looking for alternative suggestions"))]
    fn suggest_searched_packages_catalog(
        client: &impl CatalogClientTrait,
        term: &str,
        system: String,
    ) -> Result<SearchResults> {
        let results = client
            .search_with_spinner(
                term,
                system.try_into().map_err(|e| anyhow::anyhow!("{e}"))?,
                NonZeroU8::new(SUGGESTION_SEARCH_LIMIT),
            )
            .block_on()?;
        Ok(results)
    }

    /// Create a new [DidYouMean] instance for the given search term
    /// in the context of an [Environment].
    ///
    /// This will attempt to find curated suggestions for the given term,
    /// based on the lockfile of the given environment.
    pub fn new(flox: &Flox, term: &'a str) -> Self {
        let curated = Self::suggest_curated_package(term);
        let searched_term = curated.unwrap_or(term);
        let search_results = match Self::suggest_searched_packages(flox, searched_term) {
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
            suggestion: InstallSuggestion,
        }
    }
}

impl Display for DidYouMean<'_, InstallSuggestion> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(curated) = self.curated {
            writeln!(f, "Try 'flox install {curated}' instead.")?;
        }

        if self.search_results.results.is_empty() {
            return Ok(());
        }

        writeln!(f)?;
        writeln!(f, "Here are a few other similar options:")?;

        // apparently its permitted that the count is absent,
        // if the catalog service does not send it
        // see: `providers::catalog::collect_search_results`.
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

/// Replace each "package not in catalog" resolution failure in `err`
/// with the message `flox install` shows for it, suggestions included.
/// Any other error is returned unchanged.
///
/// The suggestions come from a catalog search, which may take a while,
/// so they are built here, only after resolution has already failed,
/// and under the spinner that [DidYouMean] shows.
/// A search that fails is logged and leaves the generic hint in place,
/// so the original failure is never lost.
pub fn add_package_suggestions(flox: &Flox, err: EnvironmentError) -> EnvironmentError {
    let EnvironmentError::Core(CoreEnvironmentError::Resolve(ResolveError::ResolutionFailed(
        failures,
    ))) = err
    else {
        return err;
    };

    let (not_found, mut failures): (Vec<_>, Vec<_>) = failures
        .0
        .into_iter()
        .partition(|f| matches!(f, ResolutionFailure::PackageNotFound { .. }));
    for failure in not_found {
        let ResolutionFailure::PackageNotFound(MsgAttrPathNotFoundNotInCatalog {
            attr_path, ..
        }) = failure
        else {
            unreachable!("already checked that these failures are 'package not found'")
        };
        let suggestion = DidYouMean::<InstallSuggestion>::new(flox, &attr_path);
        let head = format!("Could not find package '{attr_path}'.");
        let msg = if suggestion.has_suggestions() {
            debug!(query = attr_path, "found suggestions for package");
            formatdoc! {"
            {head}
            {suggestion}"}
        } else {
            format!("{head}\nTry 'flox search' with a broader search term.")
        };
        failures.push(ResolutionFailure::FallbackMessage { msg });
    }
    EnvironmentError::Core(CoreEnvironmentError::Resolve(
        ResolveError::ResolutionFailed(ResolutionFailures(failures)),
    ))
}

pub struct SearchSuggestion {
    use_bold_fmt: bool,
}

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
            "k8s" => "kubernetes",
            _ => return None,
        };
        Some(suggestion)
    }

    #[instrument(skip(client), fields(progress = "Looking for alternative suggestions"))]
    fn suggest_searched_packages_catalog(
        client: &impl CatalogClientTrait,
        term: &str,
        system: String,
    ) -> Result<SearchResults> {
        let results = client
            .search_with_spinner(
                term,
                system.try_into().map_err(|e| anyhow::anyhow!("{e}"))?,
                NonZeroU8::new(SUGGESTION_SEARCH_LIMIT),
            )
            .block_on()?;
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
        catalog_client: &impl CatalogClientTrait,
        system: String,
        use_bold_fmt: bool,
    ) -> Self {
        let curated = Self::suggest_curated_package(term);

        let default_results = SearchResults {
            results: Default::default(),
            count: Some(0),
        };

        let search_results = if let Some(curated) = curated {
            let res = Self::suggest_searched_packages_catalog(catalog_client, curated, system);

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
            suggestion: SearchSuggestion { use_bold_fmt },
        }
    }
}

impl Display for DidYouMean<'_, SearchSuggestion> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Some(curated) = self.curated else {
            debug!("no curated suggestions");
            return Ok(());
        };

        let search_results_rendered = match DisplaySearchResults::from_search_results(
            curated,
            self.search_results.clone(),
            self.suggestion.use_bold_fmt,
        ) {
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

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use floxhub_client::client::test_helpers::client_config;
    use floxhub_client::{FloxhubClient, MessageLevel};
    use httpmock::{Method, MockServer};
    use serde_json::json;

    use super::*;

    fn not_in_catalog(attr_path: &str) -> ResolutionFailure {
        ResolutionFailure::PackageNotFound(MsgAttrPathNotFoundNotInCatalog {
            level: MessageLevel::Error,
            msg: format!("The package '{attr_path}' is not found in the catalog."),
            attr_path: attr_path.to_string(),
            install_id: attr_path.to_string(),
        })
    }

    fn resolve_error(failures: Vec<ResolutionFailure>) -> EnvironmentError {
        EnvironmentError::Core(CoreEnvironmentError::Resolve(
            ResolveError::ResolutionFailed(ResolutionFailures(failures)),
        ))
    }

    fn failures_of(err: EnvironmentError) -> Vec<ResolutionFailure> {
        let EnvironmentError::Core(CoreEnvironmentError::Resolve(ResolveError::ResolutionFailed(
            failures,
        ))) = err
        else {
            panic!("expected a resolution failure, got: {err:?}");
        };
        failures.0
    }

    fn fallback(msg: &str) -> ResolutionFailure {
        ResolutionFailure::FallbackMessage {
            msg: msg.to_string(),
        }
    }

    /// A [Flox] whose catalog search is answered by `server`.
    fn flox_with_search_server(server: &MockServer) -> (Flox, tempfile::TempDir) {
        let (mut flox, temp_dir) = flox_instance();
        flox.floxhub_client = FloxhubClient::new(client_config(&server.base_url())).unwrap();
        (flox, temp_dir)
    }

    // `DidYouMean` blocks the calling thread on the search request,
    // which needs a runtime thread other than the blocked one.
    #[tokio::test(flavor = "multi_thread")]
    async fn package_not_found_gets_install_suggestions() {
        let server = MockServer::start();
        let search = server.mock(|when, then| {
            when.method(Method::GET).path("/api/v1/catalog/search");
            then.status(200).json_body(json!({
                "items": [{
                    "attr_path": "nodejs",
                    "catalog": null,
                    "pkg_path": "nodejs",
                    "name": "nodejs-24.22.2",
                    "pname": "nodejs",
                    "version": "24.22.2",
                    "description": "Event-driven I/O framework for the V8 JavaScript engine",
                    "stabilities": [],
                    "system": env!("NIX_TARGET_SYSTEM"),
                    "deprecation": null,
                }],
                "total_count": 84,
            }));
        });
        let (flox, _temp_dir) = flox_with_search_server(&server);

        let failures = failures_of(add_package_suggestions(
            &flox,
            resolve_error(vec![not_in_catalog("node")]),
        ));

        search.assert_calls(1);
        assert_eq!(failures.len(), 1);
        let ResolutionFailure::FallbackMessage { msg } = &failures[0] else {
            panic!("expected a fallback message, got: {:?}", failures[0]);
        };
        assert_eq!(msg.lines().collect::<Vec<_>>()[..4], [
            "Could not find package 'node'.",
            "Try 'flox install nodejs' instead.",
            "",
            "Here are a few other similar options:",
        ]);
        assert!(msg.contains("  $ flox install nodejs"), "{msg}");
        assert!(
            msg.ends_with("...or see up to 84 options with 'flox search nodejs'"),
            "{msg}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn package_not_found_without_suggestions_points_to_search() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(Method::GET).path("/api/v1/catalog/search");
            then.status(200)
                .json_body(json!({"items": [], "total_count": 0}));
        });
        let (flox, _temp_dir) = flox_with_search_server(&server);

        let failures = failures_of(add_package_suggestions(
            &flox,
            resolve_error(vec![not_in_catalog("badpkg")]),
        ));

        assert_eq!(failures, vec![fallback(
            "Could not find package 'badpkg'.\nTry 'flox search' with a broader search term."
        )]);
    }

    /// A search that errors must not replace the resolution error.
    #[tokio::test(flavor = "multi_thread")]
    async fn failed_search_keeps_the_not_found_error() {
        let server = MockServer::start();
        let search = server.mock(|when, then| {
            when.method(Method::GET).path("/api/v1/catalog/search");
            then.status(500);
        });
        let (flox, _temp_dir) = flox_with_search_server(&server);

        let failures = failures_of(add_package_suggestions(
            &flox,
            resolve_error(vec![not_in_catalog("badpkg")]),
        ));

        assert!(search.calls() >= 1);
        assert_eq!(failures, vec![fallback(
            "Could not find package 'badpkg'.\nTry 'flox search' with a broader search term."
        )]);
    }

    /// Failures that are not "not found" keep their order and come first,
    /// as `flox install` has always printed them.
    #[tokio::test(flavor = "multi_thread")]
    async fn other_failures_are_kept_ahead_of_suggestions() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(Method::GET).path("/api/v1/catalog/search");
            then.status(200)
                .json_body(json!({"items": [], "total_count": 0}));
        });
        let (flox, _temp_dir) = flox_with_search_server(&server);
        let other = fallback("some other failure");

        let failures = failures_of(add_package_suggestions(
            &flox,
            resolve_error(vec![not_in_catalog("badpkg"), other.clone()]),
        ));

        assert_eq!(failures, vec![
            other,
            fallback(
                "Could not find package 'badpkg'.\nTry 'flox search' with a broader search term."
            ),
        ]);
    }

    /// Errors that are not resolution failures, and resolution failures
    /// with nothing to suggest, must not cost a search.
    #[test]
    fn unrelated_errors_are_unchanged_and_not_searched() {
        let server = MockServer::start();
        let search = server.mock(|when, then| {
            when.method(Method::GET).path("/api/v1/catalog/search");
            then.status(200)
                .json_body(json!({"items": [], "total_count": 0}));
        });
        let (flox, _temp_dir) = flox_with_search_server(&server);

        let other = fallback("some other failure");
        let failures = failures_of(add_package_suggestions(
            &flox,
            resolve_error(vec![other.clone()]),
        ));
        assert_eq!(failures, vec![other]);

        let err = add_package_suggestions(&flox, EnvironmentError::ManifestNotFound);
        assert!(matches!(err, EnvironmentError::ManifestNotFound));

        search.assert_calls(0);
    }
}
