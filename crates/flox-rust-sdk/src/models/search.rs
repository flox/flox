use std::collections::HashMap;
use std::env;
use std::io::{BufRead, BufReader};
use std::process::{Command, ExitStatus, Stdio};
use std::str::FromStr;

use flox_types::catalog::System;
use flox_types::stability::Stability;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::nix::flake_ref::FlakeRef;

// This is the `PKGDB` path that we actually use.
// This is set once and prefers the `PKGDB` env variable, but will use
// the fallback to the binary available at build time if it is unset.
pub static PKGDB_BIN: Lazy<String> =
    Lazy::new(|| env::var("PKGDB").unwrap_or(env!("PKGDB_BIN").to_string()));

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("failed to deserialize from JSON: {0}")]
    Deserialize(serde_json::Error),
    #[error("failed to serialize search params to JSON: {0}")]
    Serialize(serde_json::Error),
    #[error("couldn't split stdout into individual lines: {0}")]
    ParseStdout(std::io::Error),
    #[error("invalid search term '{0}', try quoting the search term if this isn't what you searched for")]
    SearchTerm(String),
    #[error("search encountered an error: {0}")]
    PkgDb(Value),
    #[error("search encountered an error: {0}")]
    PkgDbCall(std::io::Error),
}

/// The input parameters for the `pkgdb search` command
///
/// C++ docs: https://flox.github.io/pkgdb/structflox_1_1pkgdb_1_1PkgQueryArgs.html
///
/// Note that `pkgdb` uses inheritance/mixins to construct the search parameters, so some fields
/// are on `PkgQueryArgs` and some are on `PkgDescriptorBase`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchParams {
    /// The collection of package sources to search
    pub registry: Registry,
    /// Which systems to search under. `None` falls back to `pkgdb` defaults
    pub systems: Option<Vec<System>>,
    /// Options for which packages should be allowed in search results
    pub allow: AllowOpts,
    /// Parameters for which semver versions should be allowed
    pub semver: SemverOpts,
    /// Parameters for the actual search query
    pub query: Query,
}

/// A collection of package sources
///
/// C++ docs: https://flox.github.io/pkgdb/classflox_1_1Registry.html
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Registry {
    /// The names and flakerefs of the package sources
    pub inputs: HashMap<String, RegistryInput>,
    /// A list of package source names indicating the preference
    /// in which to list results
    pub priority: Vec<String>,
    /// Registry-wide defaults for inputs that don't provide them
    pub defaults: RegistryDefaults,
}

/// Default search parameters for a package source
///
/// C++ docs: https://flox.github.io/pkgdb/structflox_1_1InputPreferences.html
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegistryDefaults {
    /// An optional attr path to restrict the search to
    pub subtrees: Option<Vec<String>>,
    /// Which stabilities should be included in results
    pub stabilities: Option<Vec<Stability>>,
}

/// A package source
///
/// TODO: flatten a RegistryDefaults into this struct
/// C++ docs: https://flox.github.io/pkgdb/structflox_1_1RegistryInput.html
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryInput {
    /// The flake containing packages
    pub from: FlakeRef,
    /// An optional attr path to restrict the search to
    pub subtrees: Option<Vec<String>>,
    /// Which stabilities should be included in the search results
    pub stabilities: Option<Vec<Stability>>,
}

/// Which packages should be allowed in search results.
///
/// C++ docs: https://flox.github.io/pkgdb/structflox_1_1pkgdb_1_1QueryPreferences_1_1Allows.html
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AllowOpts {
    /// Whether packages with unfree licenses should be included
    pub unfree: bool,
    /// Whether packages that are marked "broken" should be included
    pub broken: bool,
    /// A whitelist of package licenses
    pub licenses: Option<Vec<String>>,
}

/// Options regarding the ability to perform a semver search.
///
/// C++ docs: https://flox.github.io/pkgdb/structflox_1_1pkgdb_1_1QueryPreferences_1_1Semver.html
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SemverOpts {
    pub prefer_pre_releases: bool,
}

/// A set of options for defining a search query.
///
/// The search options aren't mutually exclusive. For instance, the query
/// `hello@>=2` will populate the `match` field with `hello` and the `semver`
/// field with `>=2`. The `match` field specifically searches the `name`, `pname`,
/// and `description` fields.
///
/// The result of the query will be the logical AND of all provided parameters.
///
/// C++ docs: https://flox.github.io/pkgdb/structflox_1_1search_1_1SearchQuery.html
/// Note that the `match` field here becomes the `partialMatch` field on the
/// C++ struct.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Query {
    /// Match against the full name of the package e.g. `<pname>-<version>`
    pub name: Option<String>,
    /// Match against the `pname` of the package
    pub pname: Option<String>,
    /// Match against the explicit version number of the package
    pub version: Option<String>,
    /// Match against a semver specifier
    pub semver: Option<String>,
    /// Match against a regular expression
    pub r#match: Option<String>,
}

impl FromStr for Query {
    type Err = SearchError;

    // This can't actually error, but the trait requires an error type
    fn from_str(search_term: &str) -> Result<Self, Self::Err> {
        // If there's an '@' in the query, it means the user is trying to use the semver
        // search capability. This means we need to split the query into package name and
        // semver specifier parts. Note that the 'semver' field is distinct from the 'version'
        // field in that the 'version' field refers to the '<pname>-<version>' form of the
        // package name. The user doesn't search this field directly.
        let q = match search_term.split_once('@') {
            Some((package, semver)) => {
                // If we get a search term ending in '@' it most likely means the
                // user didn't quote a search term that included a '>' character.
                if semver.is_empty() {
                    return Err(SearchError::SearchTerm(search_term.into()));
                }
                Query {
                    semver: Some(semver.to_string()),
                    r#match: Some(package.to_string()),
                    ..Query::default()
                }
            },
            None => Query {
                r#match: Some(search_term.to_string()),
                ..Query::default()
            },
        };
        Ok(q)
    }
}

/// The deserialized search results.
///
/// Note that the JSON results are returned by `pkgdb` one result per line
/// without an enclosing `[]`, so the results returned by `pkgdb` can't be
/// directly deserialized to a JSON object. To parse the results you should
/// use the provided `TryFrom` impl.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
}

impl TryFrom<&[u8]> for SearchResults {
    type Error = SearchError;

    // Note, this impl isn't actually used in the CLI, it's leftover from a previous iteration on the design.
    // It still works, so we should keep it around. It may prove useful for testing or something.
    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        let mut results = Vec::new();
        for maybe_line in bytes.lines() {
            let text = maybe_line.map_err(SearchError::ParseStdout)?;
            match serde_json::from_str(&text) {
                Ok(search_result) => results.push(search_result),
                Err(_) => {
                    // TODO: Errors are currently emitted to stdout as JSON, but there's no spec for the errors.
                    //       For now if we can't turn the text into a SearchResult, we assume that it's an
                    //       error message. If parsing that into a serde_json::Value fails, something else went
                    //       pretty wrong.
                    //
                    //       Once there's a spec for the error messages we can parse this into a typed container.
                    let err = Value::from_str(&text).map_err(SearchError::Deserialize)?;
                    return Err(SearchError::PkgDb(err));
                },
            };
        }
        Ok(SearchResults { results })
    }
}

impl SearchResults {
    /// Read search results from a buffered input source.
    ///
    /// Fails fast on reading the first error.
    pub fn collect_results(
        result_reader: impl BufRead,
        _err_reader: impl BufRead,
    ) -> Result<Self, SearchError> {
        let mut results = Vec::new();
        for maybe_line in result_reader.lines() {
            let text = maybe_line.map_err(SearchError::ParseStdout)?;
            match serde_json::from_str(&text) {
                Ok(search_result) => results.push(search_result),
                Err(_) => {
                    // TODO: Errors are currently emitted to stdout as JSON, but there's no spec for the errors.
                    //       For now if we can't turn the text into a SearchResult, we assume that it's an
                    //       error message. If parsing that into a serde_json::Value fails, something else went
                    //       pretty wrong.
                    //
                    //       Once there's a spec for the error messages we can parse this into a typed container.
                    let err = Value::from_str(&text).map_err(SearchError::Deserialize)?;
                    return Err(SearchError::PkgDb(err));
                },
            };
        }
        Ok(SearchResults { results })
    }
}

/// Calls `pkgdb` to get search results
pub fn do_search(search_params: &SearchParams) -> Result<(SearchResults, ExitStatus), SearchError> {
    let json = serde_json::to_string(search_params).map_err(SearchError::Serialize)?;

    let mut pkgdb_process = Command::new(PKGDB_BIN.as_str())
        .arg("search")
        .arg("--quiet")
        .arg(json)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(SearchError::PkgDbCall)?;

    // SAFETY: Could panic if somehow we aren't capturing `stdout`, but
    //         we _need_ to capture `stdout` to read the search results
    //         anyway. This is an error you would absolutely encounter
    //         during integration tests, so it's safe to unwrap here.
    let output_reader = BufReader::new(pkgdb_process.stdout.take().unwrap());
    let err_reader = BufReader::new(pkgdb_process.stderr.take().unwrap());

    let results = SearchResults::collect_results(output_reader, err_reader)?;

    let exit_status = pkgdb_process.wait().map_err(SearchError::PkgDbCall)?;

    Ok((results, exit_status))
}

/// A package search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Which input the package came from
    pub input: String,
    /// The attribute path of the package inside the input
    #[serde(rename = "path")]
    pub attr_path: Vec<String>,
    /// The package name
    pub pname: Option<String>,
    /// The package version
    pub version: Option<String>,
    /// The package description
    pub description: Option<String>,
    /// Whether the package is marked "broken"
    pub broken: Option<bool>,
    /// Whether the package has an unfree license
    pub unfree: Option<bool>,
    /// Which license the package is licensed under
    pub license: Option<String>,
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use super::*;

    const EXAMPLE_SEARCH_TERM: &'_ str = "hello@2.12.1";

    const EXAMPLE_PARAMS: &'_ str = r#"{
        "registry": {
            "inputs": {},
            "priority": [],
            "defaults": {
                "subtrees": null,
                "stabilities": null
            }
        },
        "systems": null,
        "allow": {
            "unfree": false,
            "broken": false,
            "licenses": null
        },
        "semver": {
            "preferPreReleases": false
        },
        "query": {
            "name": null,
            "pname": null,
            "version": null,
            "semver": "2.12.1",
            "match": "hello"
        }
    }"#;

    // This is illegible when put on a single line, but the deserializer will fail due to
    // the newlines. You'll need to `EXAMPLE_SEARCH_RESULTS.replace('\n', "").as_bytes()`
    // to deserialize it.
    const EXAMPLE_SEARCH_RESULTS: &'_ str = r#"{
        "broken": false,
        "description": "A program that produces a familiar, friendly greeting",
        "input": "nixpkgs",
        "license": "GPL-3.0-or-later",
        "path": [
            "legacyPackages",
            "aarch64-darwin",
            "hello"
        ],
        "pname": "hello",
        "unfree": false,
        "version": "2.12.1"
    }"#;

    #[test]
    fn serializes_search_params() {
        let params = SearchParams {
            query: Query::from_str(EXAMPLE_SEARCH_TERM).unwrap(),
            ..SearchParams::default()
        };
        let json = serde_json::to_string(&params).unwrap();
        // Convert both to `serde_json::Value` to test equality without worrying about whitespace
        let params_value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let example_value: serde_json::Value = serde_json::from_str(EXAMPLE_PARAMS).unwrap();
        assert_eq!(params_value, example_value);
    }

    #[test]
    fn deserializes_search_results() {
        let search_results =
            SearchResults::try_from(EXAMPLE_SEARCH_RESULTS.replace('\n', "").as_bytes()).unwrap();
        assert!(search_results.results.len() == 1);
    }
}
