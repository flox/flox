use std::num::NonZeroU8;

use serde::{Deserialize, Serialize};

pub type SearchLimit = Option<NonZeroU8>;

/// Representation of search results.
/// Created via [crate::providers::catalog::ClientTrait::search],
/// which translates raw api responses to this struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultsPage<T> {
    pub results: Vec<T>,
    pub count: ResultCount,
}
pub type ResultCount = Option<u64>;

pub type SearchResults = ResultsPage<SearchResult>;

pub type PackageDetails = ResultsPage<PackageBuild>;

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

/// Details about a single build of a package
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageBuild {
    /// The system that the package can be built for
    pub system: String,
    /// The package path including catalog name
    pub pkg_path: String,
    /// The package version
    pub version: Option<String>,
    /// The package description
    pub description: Option<String>,
}
