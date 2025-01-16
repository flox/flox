use std::num::NonZeroU8;

use serde::{Deserialize, Serialize};

use catalog_api_v1::types::{PackageInfoSearch, PackageResolutionInfo};

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

pub type SearchResult = PackageInfoSearch;
pub type SearchResults = ResultsPage<SearchResult>;

pub type PackageBuild = PackageResolutionInfo;
pub type PackageDetails = ResultsPage<PackageBuild>;
