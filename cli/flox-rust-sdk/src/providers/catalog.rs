use std::cmp::min;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt::Debug;
use std::fs::{File, OpenOptions};
use std::future::ready;
use std::io::Read;
use std::num::NonZeroU32;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use async_stream::try_stream;
use catalog_api_v1::types::{
    self as api_types,
    error as api_error,
    ErrorResponse,
    MessageLevel,
    MessageType,
    PackageInfoSearch,
    ResolutionMessageGeneral,
};
use catalog_api_v1::{Client as APIClient, Error as APIError, ResponseValue};
use enum_dispatch::enum_dispatch;
use futures::stream::Stream;
use futures::{Future, StreamExt, TryStreamExt};
use once_cell::sync::Lazy;
use reqwest::header::{self, HeaderMap};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::data::System;
use crate::flox::FLOX_VERSION;
use crate::models::search::{ResultCount, SearchLimit, SearchResult, SearchResults};
use crate::utils::traceable_path;

const NIXPKGS_CATALOG: &str = "nixpkgs";
pub const FLOX_CATALOG_MOCK_DATA_VAR: &str = "_FLOX_USE_CATALOG_MOCK";
pub const FLOX_CATALOG_DUMP_DATA_VAR: &str = "_FLOX_CATALOG_DUMP_RESPONSE_FILE";

static GENERATED_DATA: Lazy<PathBuf> =
    Lazy::new(|| PathBuf::from(std::env::var("GENERATED_DATA").unwrap()));

const RESPONSE_PAGE_SIZE: NonZeroU32 = unsafe { NonZeroU32::new_unchecked(1000) };

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
    pub fn new(baseurl: &str, extra_headers: Option<BTreeMap<String, String>>) -> Self {
        // Remove the existing output file if it exists so we don't merge with
        // a previous `flox` invocation
        if let Ok(path_str) = std::env::var(FLOX_CATALOG_DUMP_DATA_VAR) {
            let path = Path::new(&path_str);
            let _ = std::fs::remove_file(path);
        }

        // convert to HeaderMap
        let mut header_map = HeaderMap::new();
        if let Some(headers) = extra_headers {
            for (key, value) in headers {
                header_map.insert(
                    header::HeaderName::from_str(&key).unwrap(),
                    header::HeaderValue::from_str(&value).unwrap(),
                );
            }
        }

        let client = {
            let timeout = std::time::Duration::from_secs(15);
            reqwest::ClientBuilder::new()
                .connect_timeout(timeout)
                .timeout(timeout)
                .user_agent(format!("flox-cli/{}", &*FLOX_VERSION))
                .default_headers(header_map)
        };
        Self {
            client: APIClient::new_with_client(baseurl, client.build().unwrap()),
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
            let (file, mut json) = CatalogClient::read_dump_file(path);
            CatalogClient::append_dumped_response(&mut json, response);
            CatalogClient::write_dump_file(json, file, path);
        }
    }

    fn read_dump_file(path: impl AsRef<Path>) -> (File, Value) {
        tracing::debug!(path = traceable_path(&path), "reading dumped response file");
        let mut options = OpenOptions::new();
        let mut file = options
            .read(true)
            .write(true)
            .create(true)
            .open(path)
            .expect("couldn't open dumped response file");
        let mut contents = String::new();
        let bytes_read = file
            .read_to_string(&mut contents)
            .expect("couldn't read dumped response file contents");
        tracing::debug!(was_empty = bytes_read == 0, "read response file");
        let json: Value = serde_json::from_str(contents.as_ref())
            .or::<serde_json::Error>(Ok(Value::Array(vec![])))
            .expect("couldn't convert file contents to json");
        (file, json)
    }

    fn append_dumped_response<T>(json: &mut Value, response: &T)
    where
        T: ?Sized + Serialize + Debug,
    {
        let new_response =
            serde_json::to_value(response).expect("couldn't convert response to json");
        if let Value::Array(ref mut responses) = json {
            responses.push(new_response);
        } else {
            panic!("expected file to contain a json array, found something else");
        }
    }

    fn write_dump_file(json: Value, file: File, path: impl AsRef<Path>) {
        let contents = serde_json::to_string_pretty(&json)
            .expect("couldn't serialize responses to json")
            + "\n";
        tracing::debug!(
            path = traceable_path(&path),
            "writing response to dumped response file"
        );
        file.write_all_at(contents.as_bytes(), 0)
            .expect("failed writing dumped response file");
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

    /// Clear mock responses and then load responses from a file into the list
    /// of mock responses
    pub fn clear_and_load_responses_from_file(&mut self, relative_path: &str) {
        let responses = read_mock_responses((*GENERATED_DATA).join(relative_path))
            .expect("couldn't read mock responses");
        let mut locked_mock_responses = self
            .mock_responses
            .lock()
            .expect("couldn't acquire mock lock");
        locked_mock_responses.clear();
        locked_mock_responses.extend(responses);
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
        limit: SearchLimit,
    ) -> Result<SearchResults, SearchError>;

    /// Get all versions of an attr_path
    async fn package_versions(
        &self,
        attr_path: impl AsRef<str> + Send + Sync,
    ) -> Result<SearchResults, VersionsError>;
}

impl ClientTrait for CatalogClient {
    /// Wrapper around the autogenerated
    /// [catalog_api_v1::Client::resolve_api_v1_catalog_resolve_post]
    async fn resolve(
        &self,
        package_groups: Vec<PackageGroup>,
    ) -> Result<Vec<ResolvedPackageGroup>, ResolveError> {
        tracing::debug!(n_groups = package_groups.len(), "resolving package groups");
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
            .map(ResolvedPackageGroup::from)
            .collect::<Vec<_>>();

        tracing::debug!(
            n_groups = resolved_package_groups.len(),
            "received resolved package groups"
        );

        Self::maybe_dump_shim_response(&resolved_package_groups);

        Ok(resolved_package_groups)
    }

    /// Wrapper around the autogenerated
    /// [catalog_api_v1::Client::search_api_v1_catalog_search_get]
    async fn search(
        &self,
        search_term: impl AsRef<str> + Send + Sync,
        system: System,
        limit: SearchLimit,
    ) -> Result<SearchResults, SearchError> {
        tracing::debug!(
            search_term = search_term.as_ref().to_string(),
            system,
            limit,
            "sending search request"
        );
        let search_term = search_term.as_ref();
        let system = system
            .try_into()
            .map_err(CatalogClientError::UnsupportedSystem)?;

        // If the limit is less than a full page, only retrieve that many results
        let page_size = min(
            limit
                .map(Into::<NonZeroU32>::into)
                .unwrap_or(RESPONSE_PAGE_SIZE),
            RESPONSE_PAGE_SIZE,
        );
        let stream = make_depaging_stream(
            |page_number, page_size| async move {
                let response = self
                    .client
                    .search_api_v1_catalog_search_get(
                        Some(NIXPKGS_CATALOG),
                        Some(page_number),
                        Some(page_size),
                        &api_types::SearchTerm::from_str(search_term)
                            .map_err(SearchError::InvalidSearchTerm)?,
                        system,
                    )
                    .await
                    .map_err(|e| match e {
                        APIError::ErrorResponse(e) => SearchError::Search(e),
                        _ => CatalogClientError::UnexpectedError(e).into(),
                    })?;

                let packages = response.into_inner();

                Ok::<_, SearchError>((
                    packages.total_count,
                    packages
                        .items
                        .into_iter()
                        .map(TryInto::<SearchResult>::try_into)
                        .collect::<Result<Vec<_>, _>>()?,
                ))
            },
            page_size,
        );

        let (count, results) = collect_search_results(stream, limit).await?;
        let search_results = SearchResults { results, count };

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
            RESPONSE_PAGE_SIZE,
        );

        let (count, results) = collect_search_results(stream, None).await?;
        let search_results = SearchResults { results, count };

        Self::maybe_dump_shim_response(&search_results);

        Ok(search_results)
    }
}

/// Collects a stream of search results into a container, returning the total count as well.
///
/// Note: it is assumed that the first element of the stream contains the total count.
async fn collect_search_results<T, E>(
    stream: impl Stream<Item = Result<StreamItem<T>, E>>,
    limit: SearchLimit,
) -> Result<(ResultCount, Vec<T>), E> {
    let mut count = None;
    let actual_limit = if let Some(checked_limit) = limit {
        checked_limit.get() as usize
    } else {
        // If we survive long enough that this becomes a problem, I'll fix it
        usize::MAX
    };
    let results = stream
        .try_filter_map(|item| {
            let new_item = match item {
                StreamItem::TotalCount(total) => {
                    count = Some(total);
                    None
                },
                StreamItem::Result(res) => Some(res),
            };
            ready(Ok(new_item))
        })
        .take(actual_limit)
        .try_collect::<Vec<_>>()
        .await?;
    Ok((count, results))
}

#[derive(Debug, Clone, PartialEq)]
enum StreamItem<T> {
    TotalCount(u64),
    Result(T),
}

impl<T> From<T> for StreamItem<T> {
    fn from(value: T) -> Self {
        Self::Result(value)
    }
}

/// Take a function that takes a page_number and page_size and returns a
/// total_count of results and a Vec of results on a page.
///
/// Create a stream that yields TotalCount as the first item and then all
/// Results from all pages.
fn make_depaging_stream<T, E, Fut>(
    generator: impl Fn(i64, i64) -> Fut,
    page_size: NonZeroU32,
) -> impl Stream<Item = Result<StreamItem<T>, E>>
where
    Fut: Future<Output = Result<(i64, Vec<T>), E>>,
{
    try_stream! {
        let mut page_number = 0;
        let mut total_count_yielded = false;

        loop {
            let (total_count, results) = generator(page_number, page_size.get().into()).await?;

            let items_on_page = results.len();

            if !total_count_yielded {
                yield StreamItem::TotalCount(total_count as u64);
                total_count_yielded = true;
            }

            for result in results {
                yield StreamItem::Result(result)
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
        _limit: SearchLimit,
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

/// An alias so the flox crate doesn't have to depend on the catalog-api crate
pub type SystemEnum = api_types::SystemEnum;

#[derive(Debug, PartialEq, Clone)]
pub struct PackageGroup {
    pub name: String,
    pub descriptors: Vec<PackageDescriptor>,
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
    #[error("resolution message error: {0}")]
    ResolutionMessage(String),
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
    #[error("did not provide total result count")]
    NoTotalCount,
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
            stability: None,
        })
    }
}

/// The content of a generic message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgGeneral {
    /// The log level of the message
    pub level: MessageLevel,
    /// The actual message
    pub msg: String,
}

/// The content of a "attr path not found" message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgAttrPathNotFound {
    /// The log level of the message
    pub level: MessageLevel,
    /// The actual message
    pub msg: String,
    /// The requested attribute path
    pub attr_path: String,
    /// The install id that requested this attribute path
    pub install_id: String,
    /// The systems on which this attribute path is valid
    pub valid_systems: Vec<System>,
}

/// The content of a "constraints too tight" message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgConstraintsTooTight {
    /// The log level of the message
    pub level: MessageLevel,
    /// The actual message
    pub msg: String,
}

/// The content of a generic message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MsgUnknown {
    /// The original message type string
    pub msg_type: String,
    /// The log level of the message
    pub level: MessageLevel,
    /// The actual message
    pub msg: String,
    /// The delivered `context`
    pub context: HashMap<String, String>,
}

/// The kinds of resolution messages we can receive
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResolutionMessage {
    /// A generic message about resolution
    General(MsgGeneral),
    /// The attribute path requested for an install id either doesn't exist at all,
    /// or isn't available on this system
    AttrPathNotFound(MsgAttrPathNotFound),
    /// Couldn't resolve a package group because the constraints were too tight,
    /// which could mean that all the version constraints can't be satisfied by
    /// a single page.
    ConstraintsTooTight(MsgConstraintsTooTight),
    /// A (yet) unknown message type.
    Unknown(MsgUnknown),
}

impl ResolutionMessage {
    pub fn msg(&self) -> String {
        match self {
            ResolutionMessage::General(msg) => msg.msg.clone(),
            ResolutionMessage::AttrPathNotFound(msg) => msg.msg.clone(),
            ResolutionMessage::ConstraintsTooTight(msg) => msg.msg.clone(),
            ResolutionMessage::Unknown(msg) => msg.msg.clone(),
        }
    }
}

impl From<ResolutionMessageGeneral> for ResolutionMessage {
    fn from(r_msg: ResolutionMessageGeneral) -> Self {
        match r_msg.type_ {
            MessageType::General => ResolutionMessage::General(MsgGeneral {
                level: r_msg.level,
                msg: r_msg.message,
            }),
            MessageType::ResolutionTrace => ResolutionMessage::General(MsgGeneral {
                level: MessageLevel::Trace,
                msg: r_msg.message,
            }),
            MessageType::AttrPathNotFound => {
                // Should always be present for this type of message, but that's not enforced
                // by the type system
                let attr_path = r_msg
                    .context
                    .get("attr_path")
                    .cloned()
                    .unwrap_or("default_attr_path".into());

                // TODO: `valid_systems` currently come back as a ',' delimited string rather than
                //       and array of strings, so you need to check whether the string is empty,
                //       and if it's not empty you need to split on ',' hoping that there's not
                //       and escaped ',' in there somewhere.
                let valid_systems = r_msg
                    .context
                    .get("valid_systems")
                    .and_then(|s| if s.is_empty() { None } else { Some(s) })
                    .map(|combined| {
                        combined
                            .split(',')
                            .map(|s| s.to_string())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let install_id: String = r_msg
                    .context
                    .get("install_id")
                    .map(|s| s.to_string())
                    .unwrap_or("default_install_id".to_string());
                ResolutionMessage::AttrPathNotFound(MsgAttrPathNotFound {
                    level: r_msg.level,
                    msg: r_msg.message,
                    attr_path: attr_path.to_string(),
                    install_id,
                    valid_systems,
                })
            },
            MessageType::ConstraintsTooTight => {
                ResolutionMessage::ConstraintsTooTight(MsgConstraintsTooTight {
                    level: r_msg.level,
                    msg: r_msg.message,
                })
            },
            MessageType::Unknown(message_type) => ResolutionMessage::Unknown(MsgUnknown {
                msg_type: message_type,
                level: r_msg.level,
                msg: r_msg.message,
                context: r_msg.context,
            }),
        }
    }
}

/// A resolved package group
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedPackageGroup {
    /// Messages generated by the server regarding how this group was resolved
    pub msgs: Vec<ResolutionMessage>,
    /// The name of the group
    pub name: String,
    /// Which page this group was resolved to if it resolved at all
    pub page: Option<CatalogPage>,
}

impl ResolvedPackageGroup {
    pub fn packages(&self) -> impl Iterator<Item = PackageResolutionInfo> {
        if let Some(page) = &self.page {
            page.packages.clone().unwrap_or_default().into_iter()
        } else {
            vec![].into_iter()
        }
    }
}

impl From<api_types::ResolvedPackageGroupInput> for ResolvedPackageGroup {
    fn from(resolved_package_group: api_types::ResolvedPackageGroupInput) -> Self {
        Self {
            name: resolved_package_group.name,
            page: resolved_package_group.page.map(CatalogPage::from),
            msgs: resolved_package_group
                .messages
                .into_iter()
                .map(|msg| msg.into())
                .collect::<Vec<_>>(),
        }
    }
}

/// Packages from a single revision of the catalog
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogPage {
    /// Indicates whether all packages that were requested to resolve to this page were actually
    /// resolved to this page.
    pub complete: bool,
    pub packages: Option<Vec<PackageResolutionInfo>>,
    pub page: i64,
    pub url: String,
    pub msgs: Vec<ResolutionMessage>,
}

impl From<api_types::CatalogPageInput> for CatalogPage {
    fn from(catalog_page: api_types::CatalogPageInput) -> Self {
        Self {
            complete: catalog_page.complete,
            packages: catalog_page.packages,
            page: catalog_page.page,
            url: catalog_page.url,
            msgs: catalog_page
                .messages
                .into_iter()
                .map(|msg| msg.into())
                .collect::<Vec<_>>(),
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

impl TryFrom<PackageInfoSearch> for SearchResult {
    type Error = SearchError;

    fn try_from(package_info: PackageInfoSearch) -> Result<Self, SearchError> {
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
            version: None,
            description: package_info.description,
            license: None,
        })
    }
}

impl TryFrom<api_types::PackageResolutionInfo> for SearchResult {
    type Error = VersionsError;

    fn try_from(package_info: api_types::PackageResolutionInfo) -> Result<Self, VersionsError> {
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

#[derive(Debug, Clone, PartialEq)]
pub enum SearchTerm {
    Clean(String),
    VersionStripped(String),
}

impl SearchTerm {
    pub fn from_arg(search_term: &str) -> Self {
        match search_term.split_once('@') {
            Some((package, _version)) => SearchTerm::VersionStripped(package.to_string()),
            None => SearchTerm::Clean(search_term.to_string()),
        }
    }
}

pub mod test_helpers {
    use super::*;
    use crate::data::System;

    // This function should really be a #[cfg(test)] method on ResolvedPackageGroup,
    // but you can't import test features across crates
    pub fn resolved_pkg_group_with_dummy_package(
        group_name: &str,
        system: &System,
        install_id: &str,
        pkg_path: &str,
        version: &str,
    ) -> ResolvedPackageGroup {
        let pkg = PackageResolutionInfo {
            attr_path: pkg_path.to_string(),
            broken: Some(false),
            derivation: String::new(),
            description: None,
            install_id: install_id.to_string(),
            license: None,
            locked_url: String::new(),
            name: String::new(),
            outputs: vec![],
            outputs_to_install: None,
            pname: String::new(),
            rev: String::new(),
            rev_count: 0,
            rev_date: chrono::offset::Utc::now(),
            scrape_date: chrono::offset::Utc::now(),
            stabilities: None,
            unfree: None,
            version: version.to_string(),
            system: system.parse().unwrap(),
        };
        let page = CatalogPage {
            packages: Some(vec![pkg]),
            page: 0,
            url: String::new(),
            complete: true,
            msgs: vec![],
        };
        ResolvedPackageGroup {
            name: group_name.to_string(),
            page: Some(page),
            msgs: vec![],
        }
    }
}
#[cfg(test)]
mod tests {

    use std::io::Write;
    use std::num::NonZeroU8;
    use std::path::PathBuf;

    use futures::TryStreamExt;
    use httpmock::prelude::MockServer;
    use itertools::Itertools;
    use pollster::FutureExt;
    use proptest::collection::vec;
    use proptest::prelude::*;
    use serde_json::json;
    use tempfile::NamedTempFile;

    use super::*;

    #[tokio::test]
    async fn resolve_response_with_new_message_type() {
        let user_message = "User consumable Message";
        let user_message_type = "willnevereverexist_ihope";
        let json_response = json!(
        {
        "items": [
            {
            "messages": [
                {
                    "type": user_message_type,
                    "level": "error",
                    "message": user_message,
                    "context": {},
                }
            ],
            "name": "group",
            "page": null,
            } ]
        });
        let resolve_req = vec![PackageGroup {
            name: "group".to_string(),
            descriptors: vec![],
        }];

        let server = MockServer::start_async().await;
        let mock = server.mock(|_when, then| {
            then.status(200).json_body(json_response);
        });

        let client = CatalogClient::new(&server.base_url(), None);
        let res = client.resolve(resolve_req).await.unwrap();
        match &res[0].msgs[0] {
            ResolutionMessage::Unknown(msg_struct) => {
                assert!(msg_struct.msg == user_message);
                assert!(msg_struct.msg_type == user_message_type);
            },
            _ => {
                panic!();
            },
        };
        mock.assert();
    }

    #[tokio::test]
    async fn user_agent_set_on_all_requests() {
        let expected_agent = format!("flox-cli/{}", &*FLOX_VERSION);
        let empty_response = &api_types::PackageSearchResultOutput {
            items: vec![],
            total_count: 0,
        };

        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.header("user-agent", expected_agent);
            then.status(200).json_body_obj(empty_response);
        });

        let client = CatalogClient::new(&server.base_url(), None);
        let _ = client.package_versions("some-package").await;
        mock.assert();
    }

    #[tokio::test]
    async fn extra_headers_set_on_all_requests() {
        let mut extra_headers: BTreeMap<String, String> = BTreeMap::new();
        extra_headers.insert("flox-test".to_string(), "test-value".to_string());
        extra_headers.insert("flox-test2".to_string(), "test-value2".to_string());

        let empty_response = &api_types::PackageSearchResultOutput {
            items: vec![],
            total_count: 0,
        };

        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.header("flox-test", "test-value")
                .and(|when| when.header("flox-test2", "test-value2"));
            then.status(200).json_body_obj(empty_response);
        });

        let client = CatalogClient::new(&server.base_url(), Some(extra_headers));
        let _ = client.package_versions("some-package").await;
        mock.assert();
    }

    /// make_depaging_stream collects items from multiple pages
    #[tokio::test]
    async fn depage_multiple_pages() {
        let results = vec![vec![1, 2, 3], vec![4, 5, 6], vec![7, 8, 9]];
        let n_pages = results.len();
        let page_size = NonZeroU32::new(3).unwrap();
        let expected_results = results
            .iter()
            .flat_map(|chunk| chunk.iter())
            .map(|&item| StreamItem::from(item))
            .collect::<Vec<_>>();
        let total_results = results.iter().flat_map(|chunk| chunk.iter()).count() as i64;
        let results = &results;
        let stream = make_depaging_stream(
            |page_number, _page_size| async move {
                if page_number as usize >= n_pages {
                    return Ok((total_results, vec![]));
                }
                let page_data = results[page_number as usize].clone();
                Ok::<_, VersionsError>((total_results, page_data))
            },
            page_size,
        );

        // First item is the total count, skip it
        let collected_results = stream.skip(1).try_collect::<Vec<_>>().await.unwrap();

        assert_eq!(collected_results, expected_results);
    }

    /// make_depaging_stream stops if a page has fewer than page_size items
    #[tokio::test]
    async fn depage_stops_on_small_page() {
        let results = (1..=9)
            .chunks(3)
            .into_iter()
            .map(|chunk| chunk.collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let total_results = results.iter().flat_map(|chunk| chunk.iter()).count() as i64;
        let page_size = NonZeroU32::new(4).unwrap();
        let results = &results;
        let stream = make_depaging_stream(
            |page_number, _page_size| async move {
                if page_number >= results.len() as i64 {
                    return Ok((total_results, vec![]));
                }
                // This is a bad response from the server since 9 should actually be 3
                let page_data = results[page_number as usize].clone();
                Ok::<_, VersionsError>((total_results, page_data))
            },
            page_size,
        );

        // First item is the total count, skip it
        let collected: Vec<StreamItem<i32>> = stream.skip(1).try_collect().await.unwrap();

        assert_eq!(collected, (1..=3).map(StreamItem::from).collect::<Vec<_>>());
    }

    /// make_depaging_stream stops when total_count is reached
    #[tokio::test]
    async fn depage_stops_at_total_count() {
        let results = (1..=9)
            .chunks(3)
            .into_iter()
            .map(|chunk| chunk.collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let results = &results;
        // note that this isn't the _real_ total_count, we just want to make sure that
        // none of the items _after_ this number are collected
        let total_count = 3;
        let page_size = NonZeroU32::new(3).unwrap();
        let stream = make_depaging_stream(
            |page_number, _page_size| async move {
                if page_number >= results.len() as i64 {
                    return Ok((total_count, vec![]));
                }
                Ok::<_, VersionsError>((total_count, results[page_number as usize].clone()))
            },
            page_size,
        );

        let collected: Vec<StreamItem<i32>> = stream.try_collect().await.unwrap();

        assert_eq!(collected, [
            StreamItem::TotalCount(3),
            StreamItem::Result(1),
            StreamItem::Result(2),
            StreamItem::Result(3)
        ]);
    }

    proptest! {
        #[test]
        fn collects_correct_number_of_results(results in vec(any::<i32>(), 0..10), raw_limit in 0..10_u8) {
            let total = results.len();
            let results_ref = &results;
            let stream = async_stream::stream! {
                yield Ok::<StreamItem<i32>, String>(StreamItem::TotalCount(total as u64));
                for item in results_ref.iter() {
                    yield Ok(StreamItem::Result(*item));
                }
            };
            let limit = NonZeroU8::new(raw_limit); // None if raw_limit == 0
            let (found_count, collected_results) = collect_search_results(stream, limit).block_on().unwrap();
            prop_assert_eq!(found_count, Some(total as u64));

            let expected_results = if limit.is_some() {
                results.into_iter().take(raw_limit as usize).collect::<Vec<_>>()
            } else {
                results
            };
            prop_assert_eq!(expected_results, collected_results);
        }
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

    #[test]
    fn nonexistent_dump_file_makes_empty_array() {
        let tmp = NamedTempFile::new().expect("failed to create tempfile");
        // Empty file will fail to deserialize, so we should get the default (an empty array)
        let (_, json) = CatalogClient::read_dump_file(tmp.path());
        assert!(matches!(json, Value::Array(_)));
    }

    #[test]
    fn search_term_without_version() {
        assert_eq!(
            SearchTerm::from_arg("hello"),
            SearchTerm::Clean("hello".to_string())
        );
    }

    #[test]
    fn search_term_with_version_specifiers() {
        let inputs = vec!["hello@", "hello@1.x", "hello@>=1", "hello@>1 <3"];
        for input in inputs {
            assert_eq!(
                SearchTerm::from_arg(input),
                SearchTerm::VersionStripped("hello".to_string())
            );
        }
    }

    #[test]
    fn search_term_with_at_in_attr_path() {
        let inputs = vec![
            "nodePackages.@angular/cli",
            "nodePackages.@angular/cli@_at_angular_slash_cli-18.0.2",
        ];
        for input in inputs {
            assert_eq!(
                SearchTerm::from_arg(input),
                // Catalog service indexes on the last tuple of `attr_path` so neither
                // of these searches will work. However at least the behaviour with
                // `split_once("@")` is consistently wrong whereas `rsplit_once("@")`
                // would be inconsistently wrong.
                SearchTerm::VersionStripped("nodePackages.".to_string())
            );
        }
    }
}
