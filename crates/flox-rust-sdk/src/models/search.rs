use std::env;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};

use log::debug;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::skip_serializing_none;

use super::pkgdb_errors::PkgDbError;

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
    #[error("search encountered an error")]
    PkgDb(#[from] PkgDbError),
    #[error("search encountered an error: {0}")]
    PkgDbCall(std::io::Error),
    #[error("failed to canonicalize manifest path: {0}")]
    CanonicalManifestPath(std::io::Error),
    #[error("inline manifest was malformed: {0}")]
    InlineManifestMalformed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ShowError {
    #[error("failed to perform search: {0}")]
    Search(#[from] SearchError),
    #[error("invalid search term: {0}")]
    InvalidSearchTerm(String),
}

/// The input parameters for the `pkgdb search` command
///
/// C++ docs: https://flox.github.io/pkgdb/structflox_1_1pkgdb_1_1PkgQueryArgs.html
///
/// Note that `pkgdb` uses inheritance/mixins to construct the search parameters, so some fields
/// are on `PkgQueryArgs` and some are on `PkgDescriptorBase`.
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SearchParams {
    /// Either an absolute path to a manifest or an inline JSON manifest
    pub manifest: Option<PathOrJson>,
    /// Either an absolute path to a manifest or an inline JSON manifest
    pub global_manifest: PathOrJson,
    /// An optional exisiting lockfile
    pub lockfile: Option<PathOrJson>,
    /// Parameters for the actual search query
    pub query: Query,
}

/// Either an absolute path to a manifest or an inline JSON manifest
#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum PathOrJson {
    /// An absolute path to a manifest
    Path(PathBuf),
    /// An inline JSON manifest
    Json(serde_json::Value),
}

impl TryFrom<PathBuf> for PathOrJson {
    type Error = SearchError;

    fn try_from(value: PathBuf) -> Result<Self, Self::Error> {
        let canonical_path = value
            .canonicalize()
            .map_err(SearchError::CanonicalManifestPath)?;
        Ok(PathOrJson::Path(canonical_path))
    }
}

impl TryFrom<serde_json::Value> for PathOrJson {
    type Error = SearchError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Null => Err(SearchError::InlineManifestMalformed(
                "inline manifest must be a JSON object but found 'null'".into(),
            )),
            Value::Bool(_) => Err(SearchError::InlineManifestMalformed(
                "inline manifest must be a JSON object but found bool".into(),
            )),
            Value::Number(_) => Err(SearchError::InlineManifestMalformed(
                "inline manifest must be a JSON object but found number".into(),
            )),
            Value::String(_) => Err(SearchError::InlineManifestMalformed(
                "inline manifest must be a JSON object but found string".into(),
            )),
            Value::Array(_) => Err(SearchError::InlineManifestMalformed(
                "inline manifest must be a JSON object but found array".into(),
            )),
            Value::Object(value) => Ok(PathOrJson::Json(Value::Object(value))),
        }
    }
}

impl std::fmt::Display for PathOrJson {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathOrJson::Path(path) => write!(f, "{}", path.display()),
            PathOrJson::Json(json) => write!(f, "{}", json),
        }
    }
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
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
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
    /// Match against the package name
    pub match_name: Option<String>,
    /// Limit search results to a specified number
    pub limit: Option<u8>,
}

impl Query {
    /// Construct a query from a search term and an optional search result limit.
    pub fn from_term_and_limit(
        search_term: &str,
        prefer_match_name: bool,
        limit: Option<u8>,
    ) -> Result<Self, SearchError> {
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
                let mut q = Query {
                    semver: Some(semver.to_string()),
                    limit,
                    ..Query::default()
                };
                if prefer_match_name {
                    q.match_name = Some(package.to_string());
                } else {
                    q.r#match = Some(package.to_string());
                }
                q
            },
            None => {
                let mut q = Query {
                    limit,
                    ..Default::default()
                };
                if prefer_match_name {
                    q.match_name = Some(search_term.to_string());
                } else {
                    q.r#match = Some(search_term.to_string());
                }
                q
            },
        };
        Ok(q)
    }
}

/// Which subtree a package is under.
///
/// This identifies which kind of package source a package came from (catalog, flake, or nixpkgs).
#[derive(Debug, PartialEq, Eq, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Subtree {
    /// The package came from a catalog
    Catalog,
    /// The package came from a nixpkgs checkout
    LegacyPackages,
    /// The package came from an arbitrary flake
    Packages,
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
                    let mut deserializer = serde_json::Deserializer::from_str(&text);
                    let err = PkgDbError::deserialize(&mut deserializer)
                        .map_err(SearchError::Deserialize)?;
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
                    let mut deserializer = serde_json::Deserializer::from_str(&text);
                    let err = PkgDbError::deserialize(&mut deserializer)
                        .map_err(SearchError::Deserialize)?;
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

    let mut pkgdb_command = Command::new(PKGDB_BIN.as_str());
    pkgdb_command
        .arg("search")
        .arg("--quiet")
        .arg("--ga-registry")
        .arg(json)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    debug!("running search command {:?}", pkgdb_command);
    let mut pkgdb_process = pkgdb_command.spawn().map_err(SearchError::PkgDbCall)?;

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
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// Which input the package came from
    pub input: String,
    /// The full attribute path of the package inside the input.
    ///
    /// Most attributes in the attribute path are broken out into other subfields
    /// with the exception of the package version for a package from a catalog
    /// (i.e. the last attribute in the path). This attribute can be extracted from
    pub abs_path: Vec<String>,
    /// Which subtree the package is under e.g. "catalog", "legacyPackages", etc
    pub subtree: Subtree,
    /// The system that the package can be built for
    pub system: String,
    /// The part of the attribute path after `<subtree>.<system>`.
    ///
    /// For an arbitrary flake this will simply be the name of the package, but
    /// for nixpkgs this can be something like `python310Packages.flask`
    pub rel_path: Vec<String>,
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
    /// The database ID of this package
    pub id: u64,
}

#[cfg(test)]
mod test {
    use super::*;

    const EXAMPLE_SEARCH_TERM: &'_ str = "hello@2.12.1";

    const EXAMPLE_PARAMS: &'_ str = r#"{
        "manifest": "/path/to/manifest",
        "global-manifest": "/path/to/manifest",
        "query": {
            "semver": "2.12.1",
            "match": "hello",
            "limit": 10
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
        "absPath": [
            "legacyPackages",
            "aarch64-darwin",
            "hello"
        ],
        "relPath": [
            "hello"
        ],
        "subtree": "legacyPackages",
        "system": "aarch64-darwin",
        "stability": null,
        "pname": "hello",
        "unfree": false,
        "version": "2.12.1",
        "id": 420
    }"#;

    #[test]
    fn serializes_search_params() {
        let params = SearchParams {
            manifest: Some(PathOrJson::Path("/path/to/manifest".into())),
            global_manifest: PathOrJson::Path("/path/to/manifest".into()),
            lockfile: None,
            query: Query::from_term_and_limit(EXAMPLE_SEARCH_TERM, false, Some(10)).unwrap(),
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
