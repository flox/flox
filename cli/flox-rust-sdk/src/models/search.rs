use std::num::NonZeroU8;

use serde::{Deserialize, Serialize};

pub type SearchLimit = Option<NonZeroU8>;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Default, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SearchStrategy {
    Match,
    MatchName,
    #[default]
    MatchNameOrRelPath,
}

/// Representation of search results.
/// Created via [crate::providers::catalog::ClientTrait::search],
/// which translates raw api responses to this struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
    pub count: ResultCount,
}
pub type ResultCount = Option<u64>;

/// A package search result
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// The system that the package can be built for
    pub system: String,
    /// The package path including catalog name
    pub pkg_path: String,
    /// The package version
    pub version: Option<String>,
    /// The package description
    pub description: Option<String>,
}
