use std::cell::RefCell;
use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use async_stream::try_stream;
use async_trait::async_trait;
use catalog_api_v1::types::{
    self as api_types,
    error as api_error,
    ErrorResponse,
    PackageInfoApiInput,
    PackageInfoCommonInput,
};
use catalog_api_v1::{Client as APIClient, Error as APIError, ResponseValue};
use enum_dispatch::enum_dispatch;
use futures::stream::Stream;
use futures::{Future, TryStreamExt};
use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::data::System;
use crate::models::search::{SearchResult, SearchResults};

pub const DEFAULT_CATALOG_URL: &str = "https://flox-catalog.flox.dev";
const NIXPKGS_CATALOG: &str = "nixpkgs";
pub const FLOX_CATALOG_MOCK_DATA_VAR: &str = "_FLOX_USE_CATALOG_MOCK";

type ResolvedGroups = Vec<ResolvedPackageGroup>;

// Arc allows you to push things into the client from outside the client if necessary
// Mutex allows you to share across threads (necessary because of tokio)
// RefCell lets us mutate the field without needing to make the Client trait methods mutable
type MockField<T> = Arc<Mutex<RefCell<T>>>;

/// A generic response that can be turned into a [ResponseValue]. This is only necessary for
/// representing error responses.
// TODO: we can handle headers later if we need to
#[derive(Debug, Serialize, Deserialize)]
pub struct GenericResponse<T> {
    pub(crate) inner: T,
    pub(crate) status: u16,
}

impl<T> TryFrom<GenericResponse<T>> for ResponseValue<T> {
    type Error = MockDataError;

    fn try_from(value: GenericResponse<T>) -> Result<Self, Self::Error> {
        let status_code = StatusCode::from_u16(value.status)
            .map_err(|_| MockDataError::InvalidData("invalid status code".into()))?;
        let headers = HeaderMap::new();
        Ok(ResponseValue::new(value.inner, status_code, headers))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Response {
    Resolve(ResolvedGroups),
    Search(SearchResults),
    Error(GenericResponse<ErrorResponse>),
}

#[derive(Debug, Error)]
pub enum MockDataError {
    /// Failed to read the JSON file pointed at by the _FLOX_USE_CATALOG_MOCK var
    #[error("failed to read mock response file")]
    ReadMockFile(#[source] std::io::Error),
    /// Failed to parse the contents of the mock data file as JSON
    #[error("failed to parse mock data as JSON")]
    ParseJson(#[source] serde_json::Error),
    /// The data was parsed as JSON but it wasn't semantically valid
    #[error("invalid mocked data: {0}")]
    InvalidData(String),
}

/// Reads mock responses from disk when the appropriate environment variable is set.
fn read_mock_responses() -> Result<MockField<VecDeque<Response>>, MockDataError> {
    let mut responses = VecDeque::new();
    if let Ok(path) = std::env::var(FLOX_CATALOG_MOCK_DATA_VAR) {
        let contents = std::fs::read_to_string(path).map_err(MockDataError::ReadMockFile)?;
        let json: Vec<Response> =
            serde_json::from_str(&contents).map_err(MockDataError::ParseJson)?;
        responses.extend(json);
    }
    Ok(Arc::new(Mutex::new(RefCell::new(responses))))
}

/// Either a client for the actual catalog service,
/// or a mock client for testing.
#[derive(Debug)]
#[enum_dispatch(ClientTrait)]
pub enum Client {
    Catalog(CatalogClient),
    Mock(MockClient),
}

/// A client for the catalog service.
///
/// This is a wrapper around the auto-generated APIClient.
#[derive(Debug)]
pub struct CatalogClient {
    client: APIClient,
}

impl CatalogClient {
    pub fn new() -> Self {
        Self {
            client: APIClient::new(DEFAULT_CATALOG_URL),
        }
    }
}

/// A catalog client that can be seeded with mock responses
#[derive(Debug, Default)]
pub struct MockClient {
    // We use a RefCell here so that we don't have to modify the trait to allow mutable access
    // to `self` just to get mock responses out.
    pub mock_responses: MockField<VecDeque<Response>>,
}

impl MockClient {
    /// Create a new mock client, potentially reading mock responses from disk
    pub fn new() -> Result<Self, CatalogClientError> {
        Ok(Self {
            mock_responses: read_mock_responses()?,
        })
    }

    /// Push a new response into the list of mock responses
    pub fn push_resolve_response(&mut self, resp: ResolvedGroups) {
        self.mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .get_mut()
            .push_back(Response::Resolve(resp));
    }

    /// Push a new response into the list of mock responses
    pub fn push_search_response(&mut self, resp: SearchResults) {
        self.mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .get_mut()
            .push_back(Response::Search(resp));
    }

    /// Push an API error into the list of mock responses
    pub fn push_error_response(&mut self, err: ErrorResponse, status_code: u16) {
        let generic_resp = GenericResponse {
            inner: err,
            status: status_code,
        };
        self.mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .get_mut()
            .push_back(Response::Error(generic_resp));
    }
}

impl Default for CatalogClient {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
#[enum_dispatch]
pub trait ClientTrait {
    /// Resolve a list of [PackageGroup]s into a list of
    /// [ResolvedPackageGroup]s.
    async fn resolve(
        &self,
        package_groups: Vec<PackageGroup>,
    ) -> Result<Vec<ResolvedPackageGroup>, ResolveError>;

    /// Search for packages in the catalog that match a given search_term.
    async fn search(
        &self,
        search_term: impl AsRef<str> + Send + Sync,
        system: System,
        limit: u8,
    ) -> Result<SearchResults, SearchError>;

    /// Get all versions of an attr_path
    async fn package_versions(
        &self,
        attr_path: impl AsRef<str> + Send + Sync,
    ) -> Result<SearchResults, VersionsError>;
}

#[async_trait]
impl ClientTrait for CatalogClient {
    /// Wrapper around the autogenerated
    /// [catalog_api_v1::Client::resolve_api_v1_catalog_resolve_post]
    async fn resolve(
        &self,
        package_groups: Vec<PackageGroup>,
    ) -> Result<Vec<ResolvedPackageGroup>, ResolveError> {
        let package_groups = api_types::PackageGroups {
            items: package_groups
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
        };

        let response = self
            .client
            .resolve_api_v1_catalog_resolve_post(&package_groups)
            .await
            .map_err(|e| {
                if CatalogClientError::is_unexpected_error(&e) {
                    CatalogClientError::UnexpectedError(e).into()
                } else {
                    ResolveError::Resolve(e)
                }
            })?;

        let resolved_package_groups = response.into_inner();

        Ok(resolved_package_groups
            .items
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?)
    }

    /// Wrapper around the autogenerated
    /// [catalog_api_v1::Client::search_api_v1_catalog_search_get]
    async fn search(
        &self,
        search_term: impl AsRef<str> + Send + Sync,
        system: System,
        limit: u8,
    ) -> Result<SearchResults, SearchError> {
        let response = self
            .client
            .search_api_v1_catalog_search_get(
                Some(NIXPKGS_CATALOG),
                None,
                Some(limit.into()),
                &api_types::SearchTerm::from_str(search_term.as_ref())
                    .map_err(SearchError::InvalidSearchTerm)?,
                system
                    .try_into()
                    .map_err(CatalogClientError::UnsupportedSystem)?,
            )
            .await
            .map_err(|e| {
                if CatalogClientError::is_unexpected_error(&e) {
                    CatalogClientError::UnexpectedError(e).into()
                } else {
                    SearchError::Search(e)
                }
            })?;

        let api_search_result = response.into_inner();
        let search_results = SearchResults {
            results: api_search_result
                .items
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
            count: Some(
                api_search_result
                    .total_count
                    .try_into()
                    .map_err(|_| CatalogClientError::NegativeNumberOfResults)?,
            ),
        };
        Ok(search_results)
    }

    /// Wrapper around the autogenerated
    /// [catalog_api_v1::Client::packages_api_v1_catalog_packages_pkgpath_get]
    async fn package_versions(
        &self,
        attr_path: impl AsRef<str> + Send + Sync,
    ) -> Result<SearchResults, VersionsError> {
        let attr_path = attr_path.as_ref();
        let stream = make_depaging_stream(
            |page_number, page_size| async move {
                let response = self
                    .client
                    .packages_api_v1_catalog_packages_pkgpath_get(
                        attr_path,
                        Some(page_number),
                        Some(page_size),
                    )
                    .await
                    .map_err(|e| {
                        if CatalogClientError::is_unexpected_error(&e) {
                            CatalogClientError::UnexpectedError(e).into()
                        } else {
                            VersionsError::Versions(e)
                        }
                    })?;

                let packages = response.into_inner();

                Ok::<_, VersionsError>((
                    packages.total_count,
                    packages
                        .items
                        .into_iter()
                        .map(TryInto::<SearchResult>::try_into)
                        .collect::<Result<Vec<_>, _>>()?,
                ))
            },
            // I'm quite confident 10 can be stored as a NonZeroU32
            unsafe { NonZeroU32::new_unchecked(10) },
        );

        let results: Vec<SearchResult> = stream.try_collect().await?;
        let count = Some(results.len() as u64);
        Ok(SearchResults { results, count })
    }
}

/// Take a function that takes a page_number and page_size and returns a
/// total_count of results and a Vec of results on a page.
///
/// Create a stream that yields all results from all pages.
fn make_depaging_stream<T, E, Fut>(
    generator: impl Fn(i64, i64) -> Fut,
    page_size: NonZeroU32,
) -> impl Stream<Item = Result<T, E>>
where
    Fut: Future<Output = Result<(i64, Vec<T>), E>>,
{
    try_stream! {
        let mut page_number = 0;
        // TODO: this will loop forever if page_size = 0
        loop {
            let (total_count, results) = generator(page_number, page_size.get().into()).await?;

            let items_on_page = results.len();

            for result in results {
                yield result;
            }

            // If there are fewer items on this page than page_size, it should
            // be the last page.
            // If there are more pages, we assume that's a bug in the server.
            if items_on_page < page_size.get() as usize {
                break;
            }
            // This prevents us from making one extra request
            if total_count == (page_number+1) * page_size.get() as i64 {
                break;
            }
            page_number += 1;
        }
    }
}

#[async_trait]
impl ClientTrait for MockClient {
    async fn resolve(
        &self,
        _package_groups: Vec<PackageGroup>,
    ) -> Result<ResolvedGroups, ResolveError> {
        let mock_resp = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .borrow_mut()
            .pop_front();
        if let Some(Response::Search(_)) = mock_resp {
            panic!("found search response, expected resolve response");
        } else if mock_resp.is_none() {
            panic!("expected mock response, found nothing");
        } else if let Some(Response::Error(err)) = mock_resp {
            return Err(ResolveError::Resolve(APIError::ErrorResponse(
                err.try_into()?,
            )));
        } else if let Some(Response::Resolve(resp)) = mock_resp {
            return Ok(resp);
        }
        return Err(MockDataError::InvalidData("unrecognized response".into()).into());
    }

    async fn search(
        &self,
        _search_term: impl AsRef<str> + Send + Sync,
        _system: System,
        _limit: u8,
    ) -> Result<SearchResults, SearchError> {
        unimplemented!()
    }

    async fn package_versions(
        &self,
        _attr_path: impl AsRef<str> + Send + Sync,
    ) -> Result<SearchResults, VersionsError> {
        unimplemented!()
    }
}

/// Just an alias until the auto-generated PackageDescriptor diverges from what
/// we need.
pub type PackageDescriptor = api_types::PackageDescriptor;

#[derive(Debug)]
pub struct PackageGroup {
    pub descriptors: Vec<PackageDescriptor>,
    pub name: String,
    pub system: System,
}

#[derive(Debug, Error)]
pub enum CatalogClientError {
    #[error("system not supported by catalog")]
    UnsupportedSystem(#[source] api_error::ConversionError),
    // TODO: would be nicer if this contained a ResponseValue<api_types::ErrorResponse>,
    // but that doesn't implement the necessary traits.
    /// UnexpectedError corresponds to any variant of APIError other than
    /// ErrorResponse, which is the only error that is in the API schema.
    #[error("unexpected catalog connection error")]
    UnexpectedError(#[source] APIError<api_types::ErrorResponse>),
    #[error("negative number of results")]
    NegativeNumberOfResults,
    /// An error related to loading mock response data
    #[error("failed handling mock response data")]
    MockData(#[from] MockDataError),
}

#[derive(Debug, Error)]
pub enum SearchError {
    // TODO: would be nicer if this contained a ResponseValue<api_types::ErrorResponse>,
    // but that doesn't implement the necessary traits.
    #[error("search failed")]
    Search(#[source] APIError<api_types::ErrorResponse>),
    #[error("invalid search term")]
    InvalidSearchTerm(#[source] api_error::ConversionError),
    #[error("encountered attribute path with less than 3 elements: {0}")]
    ShortAttributePath(String),
    #[error(transparent)]
    CatalogClientError(#[from] CatalogClientError),
    #[error("failed handling mock response data")]
    MockData(#[from] MockDataError),
}

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("resolution failed")]
    Resolve(#[source] APIError<api_types::ErrorResponse>),
    #[error(transparent)]
    CatalogClientError(#[from] CatalogClientError),
    #[error("failed handling mock response data")]
    MockData(#[from] MockDataError),
}

#[derive(Debug, Error)]
pub enum VersionsError {
    // TODO: would be nicer if this contained a ResponseValue<api_types::ErrorResponse>,
    // but that doesn't implement the necessary traits.
    #[error("getting package versions failed")]
    Versions(#[source] APIError<api_types::ErrorResponse>),
    #[error(transparent)]
    CatalogClientError(#[from] CatalogClientError),
}

impl CatalogClientError {
    /// UnexpectedError corresponds to any variant of APIError other than
    /// ErrorResponse, which is the only error that is in the API schema.
    fn is_unexpected_error(error: &APIError<api_types::ErrorResponse>) -> bool {
        !matches!(error, APIError::ErrorResponse(_))
    }
}

impl TryFrom<PackageGroup> for api_types::PackageGroup {
    type Error = CatalogClientError;

    fn try_from(package_group: PackageGroup) -> Result<Self, CatalogClientError> {
        Ok(Self {
            descriptors: package_group.descriptors,
            name: package_group.name,
            system: package_group
                .system
                .try_into()
                .map_err(CatalogClientError::UnsupportedSystem)?,
            stability: None,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResolvedPackageGroup {
    pub name: String,
    pub pages: Vec<CatalogPage>,
    pub system: System,
}

impl TryFrom<api_types::ResolvedPackageGroupInput> for ResolvedPackageGroup {
    type Error = CatalogClientError;

    fn try_from(
        resolved_package_group: api_types::ResolvedPackageGroupInput,
    ) -> Result<Self, CatalogClientError> {
        Ok(Self {
            name: resolved_package_group.name,
            pages: resolved_package_group
                .pages
                .into_iter()
                .map(Into::into)
                .collect::<Vec<_>>(),
            system: resolved_package_group.system.to_string(),
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CatalogPage {
    pub packages: Vec<PackageResolutionInfo>,
    pub page: i64,
    pub url: String,
}

impl From<api_types::CatalogPage> for CatalogPage {
    fn from(catalog_page: api_types::CatalogPage) -> Self {
        Self {
            packages: catalog_page.packages,
            page: catalog_page.page,
            url: catalog_page.url,
        }
    }
}

/// TODO: fix types for outputs and outputs_to_install,
/// at which point this will probably no longer be an alias.
type PackageResolutionInfo = api_types::PackageResolutionInfo;

impl TryFrom<PackageInfoApiInput> for SearchResult {
    type Error = SearchError;

    fn try_from(package_info: PackageInfoApiInput) -> Result<Self, SearchError> {
        Ok(Self {
            input: NIXPKGS_CATALOG.to_string(),
            system: package_info.system.to_string(),
            // The server does not include legacyPackages.<system> in attr_path
            rel_path: package_info
                .attr_path
                .split('.')
                .map(String::from)
                .collect(),
            pname: Some(package_info.pname),
            version: Some(package_info.version),
            description: Some(package_info.description),
            license: Some(package_info.license),
        })
    }
}

impl TryFrom<PackageInfoCommonInput> for SearchResult {
    type Error = VersionsError;

    fn try_from(package_info: PackageInfoCommonInput) -> Result<Self, VersionsError> {
        Ok(Self {
            input: NIXPKGS_CATALOG.to_string(),
            system: package_info.system.to_string(),
            // The server does not include legacyPackages.<system> in attr_path
            rel_path: package_info
                .attr_path
                .split('.')
                .map(String::from)
                .collect(),
            pname: Some(package_info.pname),
            version: Some(package_info.version),
            description: Some(package_info.description),
            license: Some(package_info.license),
        })
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    /// make_depaging_stream collects items from multiple pages
    #[tokio::test]
    async fn test_depage_multiple_pages() {
        let results = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let results = &results;
        let stream = make_depaging_stream(
            |page_number, _page_size| async move {
                if page_number >= results.len() as i64 {
                    return Ok((9, vec![]));
                }
                Ok::<_, VersionsError>((9, results[page_number as usize].clone()))
            },
            NonZeroU32::new(3).unwrap(),
        );

        let collected: Vec<i32> = stream.try_collect().await.unwrap();

        assert_eq!(collected, (1..=9).collect::<Vec<_>>());
    }

    /// make_depaging_stream stops if a page has fewer than page_size items
    #[tokio::test]
    async fn test_depage_stops_on_small_page() {
        let results = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let results = &results;
        let stream = make_depaging_stream(
            |page_number, _page_size| async move {
                if page_number >= results.len() as i64 {
                    return Ok((9, vec![]));
                }
                // This is a bad response from the server since 9 should actually be 3
                Ok::<_, VersionsError>((9, results[page_number as usize].clone()))
            },
            NonZeroU32::new(4).unwrap(),
        );

        let collected: Vec<i32> = stream.try_collect().await.unwrap();

        assert_eq!(collected, (1..=3).collect::<Vec<_>>());
    }

    /// make_depaging_stream stops when total_count is reached
    #[tokio::test]
    async fn test_depage_stops_at_total_count() {
        let results = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let results = &results;
        let stream = make_depaging_stream(
            |page_number, _page_size| async move {
                if page_number >= results.len() as i64 {
                    return Ok((3, vec![]));
                }
                Ok::<_, VersionsError>((3, results[page_number as usize].clone()))
            },
            NonZeroU32::new(3).unwrap(),
        );

        let collected: Vec<i32> = stream.try_collect().await.unwrap();

        assert_eq!(collected, (1..=3).collect::<Vec<_>>());
    }
}

#[cfg(test)]
mod test {
    use pollster::FutureExt;

    use super::*;

    #[test]
    fn mock_client_uses_seeded_responses() {
        let mut client = MockClient::new().unwrap();
        client.push_resolve_response(vec![]);
        let resp = client.resolve(vec![]).block_on().unwrap();
        assert!(resp.is_empty());
    }
}
