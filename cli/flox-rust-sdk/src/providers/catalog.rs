use std::collections::VecDeque;
use std::fmt::Debug;
use std::fs::OpenOptions;
use std::io::Read;
use std::num::NonZeroU32;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use async_stream::try_stream;
use catalog_api_v1::types::{
    self as api_types,
    error as api_error,
    ErrorResponse,
    PackageInfoApi,
    PackageInfoCommon,
};
use catalog_api_v1::{Client as APIClient, Error as APIError, ResponseValue};
use enum_dispatch::enum_dispatch;
use futures::stream::Stream;
use futures::{Future, TryStreamExt};
use reqwest::header::HeaderMap;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::data::System;
use crate::models::search::{SearchResult, SearchResults};
use crate::utils::traceable_path;

pub const DEFAULT_CATALOG_URL: &str = "https://flox-catalog.flox.dev";
const NIXPKGS_CATALOG: &str = "nixpkgs";
pub const FLOX_CATALOG_MOCK_DATA_VAR: &str = "_FLOX_USE_CATALOG_MOCK";
pub const FLOX_CATALOG_DUMP_DATA_VAR: &str = "_FLOX_CATALOG_DUMP_RESPONSE_FILE";

type ResolvedGroups = Vec<ResolvedPackageGroup>;

// Arc allows you to push things into the client from outside the client if necessary
// Mutex allows you to share across threads (necessary because of tokio)
type MockField<T> = Arc<Mutex<T>>;

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
#[serde(untagged)]
pub enum Response {
    Resolve(ResolvedGroups),
    // Note that this variant _also_ works for `flox show`/`package_versions` since they return
    // the same type
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

/// Reads a list of mock responses from disk.
fn read_mock_responses(path: impl AsRef<Path>) -> Result<VecDeque<Response>, MockDataError> {
    let mut responses = VecDeque::new();
    let contents = std::fs::read_to_string(path).map_err(MockDataError::ReadMockFile)?;
    let deserialized: Vec<Response> =
        serde_json::from_str(&contents).map_err(MockDataError::ParseJson)?;
    responses.extend(deserialized);
    Ok(responses)
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
    pub fn new(baseurl: &str) -> Self {
        Self {
            client: APIClient::new(baseurl),
        }
    }

    /// Serialize data to the file pointed to by FLOX_CATALOG_DUMP_DATA_VAR if
    /// it is set
    fn maybe_dump_shim_response<T>(response: &T)
    where
        T: ?Sized + Serialize + Debug,
    {
        if let Ok(path_str) = std::env::var(FLOX_CATALOG_DUMP_DATA_VAR) {
            let path = Path::new(&path_str);
            tracing::debug!(path = traceable_path(&path), "reading dumped response file");
            let mut options = OpenOptions::new();
            let mut file = options
                .read(true)
                .write(true)
                .create(true)
                .open(path)
                .expect("couldn't open dumped response file");
            let mut contents = String::new();
            file.read_to_string(&mut contents)
                .expect("couldn't read dumped response file contents");
            let mut json: Value = serde_json::from_str(&contents)
                .or::<serde_json::Error>(Ok(
                    serde_json::from_str("[]").expect("failed to make empty json array")
                ))
                .expect("couldn't convert file contents to json");
            let new_response =
                serde_json::to_value(response).expect("couldn't serialize response to json");
            if let Value::Array(ref mut responses) = json {
                responses.push(new_response);
            } else {
                panic!("expected file to contain a json array, found something else");
            }
            let contents =
                serde_json::to_string_pretty(&json).expect("couldn't serialize responses to json");
            tracing::debug!(
                path = traceable_path(&path),
                "writing response to dumped response file"
            );
            file.write_all_at(contents.as_bytes(), 0)
                .expect("failed writing dumped response file");
        }
    }
}

impl Default for CatalogClient {
    fn default() -> Self {
        Self::new(DEFAULT_CATALOG_URL)
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
    pub fn new(mock_data_path: Option<impl AsRef<Path>>) -> Result<Self, CatalogClientError> {
        let mock_responses = if let Some(path) = mock_data_path {
            read_mock_responses(&path).expect("couldn't read mock responses from disk")
        } else {
            VecDeque::new()
        };
        Ok(Self {
            mock_responses: Arc::new(Mutex::new(mock_responses)),
        })
    }

    /// Push a new response into the list of mock responses
    pub fn push_resolve_response(&mut self, resp: ResolvedGroups) {
        self.mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .push_back(Response::Resolve(resp));
    }

    /// Push a new response into the list of mock responses
    pub fn push_search_response(&mut self, resp: SearchResults) {
        self.mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
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
            .push_back(Response::Error(generic_resp));
    }
}

#[enum_dispatch]
#[allow(async_fn_in_trait)]
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

const PAGE_SIZE: NonZeroU32 = unsafe { NonZeroU32::new_unchecked(10) };

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
            .map_err(|e| match e {
                APIError::ErrorResponse(e) => ResolveError::Resolve(e),
                _ => CatalogClientError::UnexpectedError(e).into(),
            })?;

        let api_resolved_package_groups = response.into_inner();

        let resolved_package_groups = api_resolved_package_groups
            .items
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        Self::maybe_dump_shim_response(&resolved_package_groups);

        Ok(resolved_package_groups)
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
            .map_err(|e| match e {
                APIError::ErrorResponse(e) => SearchError::Search(e),
                _ => CatalogClientError::UnexpectedError(e).into(),
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

        Self::maybe_dump_shim_response(&search_results);

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
                    .packages_api_v1_catalog_packages_attr_path_get(
                        attr_path,
                        Some(page_number),
                        Some(page_size),
                    )
                    .await
                    .map_err(|e| match e {
                        APIError::ErrorResponse(e) => VersionsError::Versions(e),
                        _ => CatalogClientError::UnexpectedError(e).into(),
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
            PAGE_SIZE,
        );

        let results: Vec<SearchResult> = stream.try_collect().await?;
        let count = Some(results.len() as u64);

        let search_results = SearchResults { results, count };

        Self::maybe_dump_shim_response(&search_results);

        Ok(search_results)
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

impl ClientTrait for MockClient {
    async fn resolve(
        &self,
        _package_groups: Vec<PackageGroup>,
    ) -> Result<ResolvedGroups, ResolveError> {
        let mock_resp = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .pop_front();
        match mock_resp {
            Some(Response::Resolve(resp)) => Ok(resp),
            Some(Response::Search(_)) => {
                panic!("found search response, expected resolve response");
            },
            Some(Response::Error(err)) => Err(ResolveError::Resolve(
                err.try_into()
                    .expect("couldn't convert mock error response"),
            )),
            None => {
                panic!("expected mock response, found nothing");
            },
        }
    }

    async fn search(
        &self,
        _search_term: impl AsRef<str> + Send + Sync,
        _system: System,
        _limit: u8,
    ) -> Result<SearchResults, SearchError> {
        let mock_resp = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .pop_front();
        match mock_resp {
            Some(Response::Search(resp)) => Ok(resp),
            Some(Response::Resolve(_)) => {
                panic!("found resolve response, expected search response");
            },
            Some(Response::Error(err)) => Err(SearchError::Search(
                err.try_into()
                    .expect("couldn't convert mock error response"),
            )),
            None => {
                panic!("expected mock response, found nothing");
            },
        }
    }

    async fn package_versions(
        &self,
        _attr_path: impl AsRef<str> + Send + Sync,
    ) -> Result<SearchResults, VersionsError> {
        let mock_resp = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock")
            .pop_front();
        match mock_resp {
            Some(Response::Search(resp)) => Ok(resp),
            Some(Response::Resolve(_)) => {
                panic!("found resolve response, expected search response");
            },
            Some(Response::Error(err)) => Err(VersionsError::Versions(
                err.try_into()
                    .expect("couldn't convert mock error response"),
            )),
            None => {
                panic!("expected mock response, found nothing");
            },
        }
    }
}

/// Just an alias until the auto-generated PackageDescriptor diverges from what
/// we need.
pub type PackageDescriptor = api_types::PackageDescriptor;

/// Alias to type representing expected errors that are in the API spec
pub type ApiErrorResponse = api_types::ErrorResponse;
pub type ApiErrorResponseValue = ResponseValue<ApiErrorResponse>;

#[derive(Debug, PartialEq, Clone)]
pub struct PackageGroup {
    pub descriptors: Vec<PackageDescriptor>,
    pub name: String,
    pub system: System,
}

#[derive(Debug, Error)]
pub enum CatalogClientError {
    #[error("system not supported by catalog")]
    UnsupportedSystem(#[source] api_error::ConversionError),
    /// UnexpectedError corresponds to any variant of APIError other than
    /// ErrorResponse, which is the only error that is in the API schema.
    #[error("unexpected catalog connection error")]
    UnexpectedError(#[source] APIError<api_types::ErrorResponse>),
    #[error("negative number of results")]
    NegativeNumberOfResults,
}

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("search failed: {}", fmt_info(_0))]
    Search(ApiErrorResponseValue),
    #[error("invalid search term")]
    InvalidSearchTerm(#[source] api_error::ConversionError),
    #[error("encountered attribute path with less than 3 elements: {0}")]
    ShortAttributePath(String),
    #[error(transparent)]
    CatalogClientError(#[from] CatalogClientError),
}

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("resolution failed: {}", fmt_info(_0))]
    Resolve(ApiErrorResponseValue),
    #[error(transparent)]
    CatalogClientError(#[from] CatalogClientError),
}
#[derive(Debug, Error)]
pub enum VersionsError {
    #[error("getting package versions failed: {}", fmt_info(_0))]
    Versions(ApiErrorResponseValue),
    #[error(transparent)]
    CatalogClientError(#[from] CatalogClientError),
}

/// TODO: I copied this from the fmt_info function used by the Display impl of
/// APIError.
/// We should find something cleaner.
fn fmt_info(error_response: &ApiErrorResponseValue) -> String {
    format!(
        "status: {}; headers: {:?}; value: {:?}",
        error_response.status(),
        error_response.headers(),
        error_response.as_ref()
    )
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedPackageGroup {
    pub name: String,
    pub pages: Vec<CatalogPage>,
    pub system: System,
}

impl ResolvedPackageGroup {
    pub fn packages(&self) -> impl Iterator<Item = PackageResolutionInfo> {
        self.pages
            .iter()
            .filter_map(|page| page.packages.clone())
            .flat_map(|pkgs| pkgs.into_iter())
            .collect::<Vec<_>>()
            .into_iter()
    }
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

/// Packages from a single revision of the catalog
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogPage {
    pub packages: Option<Vec<PackageResolutionInfo>>,
    pub page: i64,
    pub url: String,
}

impl From<api_types::CatalogPageInput> for CatalogPage {
    fn from(catalog_page: api_types::CatalogPageInput) -> Self {
        Self {
            packages: catalog_page.packages,
            page: catalog_page.page,
            url: catalog_page.url,
        }
    }
}

/// TODO: Implement a shim for [api_types::PackageResolutionInfo]
///
/// Since we plan to list resolved packages in a flat list within the lockfile,
/// [lockfile::LockedPackageCatalog] adds (at least) a `system` field.
/// We should consider whether adding a shim to [api_types::PackageResolutionInfo]
/// is not adding unnecessary complexity.
pub type PackageResolutionInfo = api_types::ResolvedPackageDescriptor;

impl TryFrom<PackageInfoApi> for SearchResult {
    type Error = SearchError;

    fn try_from(package_info: PackageInfoApi) -> Result<Self, SearchError> {
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
            description: package_info.description,
            license: package_info.license,
        })
    }
}

impl TryFrom<PackageInfoCommon> for SearchResult {
    type Error = VersionsError;

    fn try_from(package_info: PackageInfoCommon) -> Result<Self, VersionsError> {
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
            description: package_info.description,
            license: package_info.license,
        })
    }
}

#[cfg(test)]
mod tests {

    use std::io::Write;
    use std::path::PathBuf;

    use pollster::FutureExt;
    use tempfile::NamedTempFile;

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

    #[test]
    fn mock_client_uses_seeded_responses() {
        let path: Option<&PathBuf> = None;
        let mut client = MockClient::new(path).unwrap();
        client.push_resolve_response(vec![]);
        let resp = client.resolve(vec![]).block_on().unwrap();
        assert!(resp.is_empty());
    }

    #[test]
    fn can_push_responses_outside_of_client() {
        let path: Option<&PathBuf> = None;
        let client = MockClient::new(path).unwrap();
        {
            // Need to drop the mutex guard otherwise `resolve` will block trying to read
            // the queue of mock responses
            let resp_handle = client.mock_responses.clone();
            let mut responses = resp_handle.lock().unwrap();
            responses.push_back(Response::Resolve(vec![]));
        }
        let resp = client.resolve(vec![]).block_on().unwrap();
        assert!(resp.is_empty());
    }

    #[test]
    fn error_when_invalid_json() {
        let tmp = NamedTempFile::new().unwrap();
        // There's nothing in the mock data file yet, so it can't be parsed as JSON.
        // This will cause a panic, which is returned as an error from `catch_unwind`.
        let res = std::panic::catch_unwind(|| MockClient::new(Some(&tmp)));
        assert!(res.is_err());
    }

    #[test]
    fn parses_basic_json() {
        let mut tmp = NamedTempFile::new().unwrap();
        tmp.write_all("[[]]".as_bytes()).unwrap();
        let client = MockClient::new(Some(&tmp)).unwrap();
        let resp = client.resolve(vec![]).block_on().unwrap();
        assert!(resp.is_empty());
    }
}
