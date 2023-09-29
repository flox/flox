use std::collections::HashMap;
use std::io::BufRead;
use std::str::FromStr;

use flox_types::catalog::System;
use flox_types::stability::Stability;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::nix::flake_ref::FlakeRef;

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("failed to deserialize search result from JSON: {0}")]
    Deserialize(#[from] serde_json::Error),
    #[error("couldn't parse stdout to separate JSON lines: {0}")]
    ParseStdout(#[from] std::io::Error),
    #[error("invalid search term '{0}', try quoting the search term if this isn't what you searched for")]
    SearchTerm(String),
    #[error("search produced an error: {0}")]
    PkgDb(Value),
}

/// The input parameters for the `pkgdb search` command
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SearchParams {
    /// The collection of package sources to search
    pub registry: Registry,
    /// Which systems to search under
    pub systems: Option<Vec<System>>,
    /// Options for which packages should be allowed in search results
    pub allow: AllowOpts,
    /// Parameters for which semver versions should be allowed
    pub semver: SemverOpts,
    /// Parameters for the actual search query
    pub query: Query,
}

/// A collection of package sources
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Registry {
    /// The names and flakerefs of the package sources
    pub inputs: HashMap<String, RegistryInput>,
    /// A list of package source names indicating the preference
    /// in which to list results
    pub priority: Vec<String>,
    /// Default parameters for all package sources if none
    /// are provided by the specific package source
    pub defaults: RegistryDefaults,
}

/// Default search parameters for a package source
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegistryDefaults {
    /// An optional attr path to restrict the search to
    pub subtrees: Option<Vec<String>>,
    /// Which stabilities should be included in results
    pub stabilities: Option<Vec<Stability>>,
}

/// A package source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryInput {
    /// The flakeref containing packages
    pub from: FlakeRef,
    /// An optional attr path to restrict the search to
    pub subtrees: Option<Vec<String>>,
    /// Which stabilities should be included in the search results
    pub stabilities: Option<Vec<Stability>>,
}

/// Which packages should be allowed in search results
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AllowOpts {
    /// Whether packages with unfree licenses should be included
    pub unfree: bool,
    /// Whether packages that are marked "broken" should be included
    pub broken: bool,
    /// A whitelist of package licenses
    pub licenses: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SemverOpts {
    pub prefer_pre_releases: bool,
}

/// A non-mutually-exclusive set of options for defining a search query
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
        let q = match search_term.find('@') {
            Some(idx) => {
                // If we get a search term ending in '@' it most likely means the
                // user didn't quote a search term that included a '>' character.
                if idx >= search_term.len() - 1 {
                    return Err(SearchError::SearchTerm(search_term.into()));
                }
                // Splitting at `idx` would include the `@`
                let package = String::from(&search_term[..idx]);
                let semver = String::from(&search_term[idx + 1..]);
                Query {
                    semver: Some(semver),
                    r#match: Some(package),
                    ..Query::default()
                }
            },
            None => Query {
                r#match: Some(search_term.into()),
                ..Query::default()
            },
        };
        Ok(q)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
}

impl TryFrom<&[u8]> for SearchResults {
    type Error = SearchError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        let mut results = Vec::new();
        for maybe_line in bytes.lines() {
            let text = maybe_line?;
            match serde_json::from_str(&text) {
                Ok(search_result) => results.push(search_result),
                Err(_) => {
                    // TODO: Errors are currently emitted to stdout as JSON, but there's no spec for the errors.
                    //       For now if we can't turn the text into a SearchResult, we assume that it's an
                    //       error message. If parsing that into a serde_json::Value fails, something else went
                    //       pretty wrong.
                    //
                    //       Once there's a spec for the error messages we can parse this into a typed container.
                    let err = Value::from_str(&text)?;
                    return Err(SearchError::PkgDb(err));
                },
            };
        }
        Ok(SearchResults { results })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub input: String,
    #[serde(rename = "path")]
    pub attr_path: Vec<String>,
    pub pname: Option<String>,
    pub version: Option<String>,
    pub description: Option<String>,
    pub broken: Option<bool>,
    pub unfree: Option<bool>,
    pub license: Option<String>,
}

#[cfg(test)]
mod test {
    use std::process::{Command, Output};
    use std::str::FromStr;

    use anyhow::Error;

    use super::*;
    const PKGDB: &'_ str = env!("PKGDB_BIN");

    fn call_pkgdb(params: &SearchParams) -> Result<Output, Error> {
        let params_json = serde_json::to_string(params).unwrap();
        // Useful for debugging
        // eprintln!("json input:\n{}", params_json);
        let output = Command::new(PKGDB)
            .arg("search")
            .arg("--quiet")
            .arg(params_json)
            .output();
        output.map_err(Error::from)
    }

    fn assert_no_err_msg(stderr: Vec<u8>) {
        if !stderr.is_empty() {
            let err_msg = String::from_utf8(stderr).unwrap();
            // We know this will fail, but this way we'll get to see the error message from pkgdb
            assert_eq!(String::from(""), err_msg);
        }
    }

    #[test]
    fn serializes_search_params() {
        let params = SearchParams {
            query: Query::from_str("hello@2.12.1").unwrap(),
            ..SearchParams::default()
        };
        let results = call_pkgdb(&params).unwrap();
        assert_no_err_msg(results.stderr);
    }

    #[test]
    fn deserializes_search_results() {
        let mut params = SearchParams::default();
        params.query.r#match = Some("hello".into());
        let nixpkgs_flakeref = FlakeRef::from_str("github:NixOS/nixpkgs/nixpkgs-unstable").unwrap();
        let nixpkgs_registry = RegistryInput {
            from: nixpkgs_flakeref,
            subtrees: Some(vec!["legacyPackages".into()]),
            stabilities: None,
        };
        params
            .registry
            .inputs
            .insert("nixpkgs".into(), nixpkgs_registry);
        params.systems = Some(vec!["aarch64-darwin".into()]);
        let results = call_pkgdb(&params).unwrap();
        assert_no_err_msg(results.stderr);
        // Useful for debugging
        // let string_results = String::from_utf8(results.stdout.clone()).unwrap();
        // eprintln!("json results:\n{}", string_results);
        let search_results = SearchResults::try_from(results.stdout.as_slice()).unwrap();
        assert!(search_results.results.len() > 1);
    }
}
