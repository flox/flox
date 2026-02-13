//! Catalog client wrapper around the auto-generated API client.

use std::cmp::min;
use std::collections::HashMap;
use std::fmt::Debug;
use std::future::{ready, Future};
use std::num::NonZeroU32;
use std::str::FromStr;
use std::time::Duration;

use async_stream::try_stream;
use catalog_api_v1::types::{self as api_types};
use catalog_api_v1::{Client as APIClient, Error as APIError};
use futures::stream::Stream;
use futures::{StreamExt, TryStreamExt};
use reqwest::header::{self, HeaderMap};
use reqwest::StatusCode;
use tracing::{debug, instrument};

use crate::config::CatalogClientConfig;
use crate::error::{CatalogClientError, ResolveError, SearchError, VersionsError};
use crate::mock::MockGuard;
use crate::types::*;
use crate::MapApiErrorExt;

#[cfg(any(test, feature = "tests"))]
pub const EMPTY_SEARCH_RESPONSE: &api_types::PackageSearchResult =
    &api_types::PackageSearchResult {
        items: vec![],
        total_count: 0,
    };

/// A client for the catalog service.
///
/// This is a wrapper around the auto-generated APIClient that handles:
/// - HTTP client configuration with timeouts
/// - Bearer token authentication for FloxHub
/// - Mock server recording/replay for testing (feature-gated)
pub struct CatalogClient {
    client: APIClient,
    config: CatalogClientConfig,

    _mock_guard: Option<MockGuard>,
}

impl Debug for CatalogClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CatalogClient")
            .field("catalog_url", &self.config.catalog_url)
            .finish_non_exhaustive()
    }
}

impl CatalogClient {
    /// Create a new catalog client from configuration.
    pub fn new(config: CatalogClientConfig) -> Result<Self, CatalogClientError> {
        // create a mock server if configured
        let mock_guard = MockGuard::new(&config);
        let effective_url = match mock_guard {
            Some(ref mock) => mock.url(),
            None => config.catalog_url.clone(),
        };

        let http_client = build_http_client(&config)?;
        let client = APIClient::new_with_client(&effective_url, http_client);

        Ok(Self {
            client,
            config,
            _mock_guard: mock_guard,
        })
    }

    /// Access the underlying API client for making requests.
    pub fn api(&self) -> &APIClient {
        &self.client
    }

    /// Get the configured catalog URL.
    pub fn catalog_url(&self) -> &str {
        &self.config.catalog_url
    }

    /// Clear mock recording state if in recording mode.
    ///
    /// Useful in tests where setup operations should not be captured.
    pub fn reset_recording(&mut self) {
        if let Some(ref mut guard) = self._mock_guard {
            guard.reset_recording();
        }
    }

    /// Update the client configuration and recreate the client.
    pub fn update_config(
        &mut self,
        update: impl FnOnce(&mut CatalogClientConfig),
    ) -> Result<(), CatalogClientError> {
        let mut modified_config = self.config.clone();
        update(&mut modified_config);
        *self = Self::new(modified_config)?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Catalog trait
// ---------------------------------------------------------------------------

const RESPONSE_PAGE_SIZE: NonZeroU32 = NonZeroU32::new(1000).unwrap();

/// The complete catalog API interface.
///
/// This trait enables alternate implementations:
/// - **HTTP** (current): REST calls to FloxHub catalog API via [`CatalogClient`]
/// - **Mock** (SDK tests): Canned responses without HTTP
/// - **Direct** (future): FloxHub server calls catalog logic in-process
#[allow(async_fn_in_trait)]
pub trait ClientTrait {
    /// Resolve a list of [`PackageGroup`]s into [`ResolvedPackageGroup`]s.
    async fn resolve(
        &self,
        package_groups: Vec<PackageGroup>,
    ) -> Result<Vec<ResolvedPackageGroup>, ResolveError>;

    /// Search for packages matching a search term, showing a spinner.
    async fn search_with_spinner(
        &self,
        search_term: impl AsRef<str> + Send + Sync,
        system: api_types::PackageSystem,
        limit: SearchLimit,
    ) -> Result<SearchResults, SearchError> {
        self.search(search_term, system, limit).await
    }

    /// Search for packages matching a search term.
    async fn search(
        &self,
        search_term: impl AsRef<str> + Send + Sync,
        system: api_types::PackageSystem,
        limit: SearchLimit,
    ) -> Result<SearchResults, SearchError>;

    /// Get all versions of an attr_path.
    async fn package_versions(
        &self,
        attr_path: impl AsRef<str> + Send + Sync,
    ) -> Result<PackageDetails, VersionsError>;

    /// Get publish info for a package.
    async fn publish_info(
        &self,
        catalog_name: impl AsRef<str> + Send + Sync,
        package_name: impl AsRef<str> + Send + Sync,
    ) -> Result<PublishResponse, CatalogClientError>;

    /// Create a package within a user catalog.
    async fn create_package(
        &self,
        catalog_name: impl AsRef<str> + Send + Sync,
        package_name: impl AsRef<str> + Send + Sync,
        original_url: impl AsRef<str> + Send + Sync,
    ) -> Result<(), CatalogClientError>;

    /// Publish a build of a user package.
    async fn publish_build(
        &self,
        catalog_name: impl AsRef<str> + Send + Sync,
        package_name: impl AsRef<str> + Send + Sync,
        build_info: &UserBuildPublish,
    ) -> Result<(), CatalogClientError>;

    /// Get store info for a list of derivations.
    async fn get_store_info(
        &self,
        derivations: Vec<String>,
    ) -> Result<HashMap<String, Vec<StoreInfo>>, CatalogClientError>;

    /// Checks whether the provided store paths have been successfully
    /// published.
    async fn is_publish_complete(&self, store_paths: &[String])
        -> Result<bool, CatalogClientError>;

    /// Get information about the base catalog and available stabilities.
    async fn get_base_catalog_info(&self) -> Result<BaseCatalogInfo, CatalogClientError>;
}

// ---------------------------------------------------------------------------
// ClientTrait implementation for CatalogClient
// ---------------------------------------------------------------------------

impl ClientTrait for CatalogClient {
    #[instrument(skip_all, fields(progress = "Resolving packages from catalog"))]
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
            .resolve_api_v1_catalog_resolve_post(None, &package_groups)
            .await
            .map_api_error()
            .await?;

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

        Ok(resolved_package_groups)
    }

    #[instrument(skip_all, fields(
        search_term = %search_term.as_ref(),
        progress = format!("Searching for packages matching '{}' in catalog", search_term.as_ref())))]
    async fn search_with_spinner(
        &self,
        search_term: impl AsRef<str> + Send + Sync,
        system: api_types::PackageSystem,
        limit: SearchLimit,
    ) -> Result<SearchResults, SearchError> {
        self.search(search_term, system, limit).await
    }

    async fn search(
        &self,
        search_term: impl AsRef<str> + Send + Sync,
        system: api_types::PackageSystem,
        limit: SearchLimit,
    ) -> Result<SearchResults, SearchError> {
        tracing::debug!(
            search_term = search_term.as_ref().to_string(),
            ?system,
            ?limit,
            "sending search request"
        );
        let search_term = search_term.as_ref();

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
                        None,
                        Some(page_number),
                        Some(page_size),
                        Some(
                            &api_types::SearchTerm::from_str(search_term)
                                .map_err(SearchError::InvalidSearchTerm)?,
                        ),
                        system,
                    )
                    .await
                    .map_api_error()
                    .await?;

                let packages = response.into_inner();

                Ok::<_, SearchError>((packages.total_count, packages.items))
            },
            page_size,
        );

        let (count, results) = collect_search_results(stream, limit).await?;
        let search_results = SearchResults { results, count };

        Ok(search_results)
    }

    async fn package_versions(
        &self,
        attr_path: impl AsRef<str> + Send + Sync,
    ) -> Result<PackageDetails, VersionsError> {
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
                    .map_api_error()
                    .await
                    .map_err(|e| match e {
                        CatalogClientError::APIError(APIError::ErrorResponse(response))
                            if response.status() == StatusCode::NOT_FOUND =>
                        {
                            VersionsError::NotFound
                        },
                        other => other.into(),
                    })?;

                let packages = response.into_inner();

                Ok::<_, VersionsError>((packages.total_count, packages.items))
            },
            RESPONSE_PAGE_SIZE,
        );

        let (count, results) = collect_search_results(stream, None).await?;
        let search_results = PackageDetails { results, count };

        Ok(search_results)
    }

    async fn publish_info(
        &self,
        catalog_name: impl AsRef<str> + Send + Sync,
        package_name: impl AsRef<str> + Send + Sync,
    ) -> Result<PublishResponse, CatalogClientError> {
        let catalog = str_to_catalog_name(catalog_name)?;
        let package = str_to_package_name(package_name)?;
        let body = api_types::PublishInfoRequest(serde_json::Map::new());
        self.client
            .publish_request_api_v1_catalog_catalogs_catalog_name_packages_package_name_publish_info_post(
                &catalog, &package, &body,
            )
            .await
            .map_api_error()
            .await
            .map(|resp| resp.into_inner())
    }

    async fn create_package(
        &self,
        catalog_name: impl AsRef<str> + Send + Sync,
        package_name: impl AsRef<str> + Send + Sync,
        original_url: impl AsRef<str> + Send + Sync,
    ) -> Result<(), CatalogClientError> {
        let body = api_types::UserPackageCreate {
            original_url: Some(original_url.as_ref().to_string()),
        };
        let catalog = str_to_catalog_name(&catalog_name)?;
        let package = str_to_package_name(&package_name)?;
        self.client
            .create_catalog_package_api_v1_catalog_catalogs_catalog_name_packages_post(
                &catalog, &package, &body,
            )
            .await
            .map_api_error()
            .await?;

        debug!("successfully created package");
        Ok(())
    }

    async fn publish_build(
        &self,
        catalog_name: impl AsRef<str> + Send + Sync,
        package_name: impl AsRef<str> + Send + Sync,
        build_info: &UserBuildPublish,
    ) -> Result<(), CatalogClientError> {
        let catalog = str_to_catalog_name(catalog_name)?;
        let package = str_to_package_name(package_name)?;
        self.client
            .create_package_build_api_v1_catalog_catalogs_catalog_name_packages_package_name_builds_post(
                &catalog, &package, build_info,
            )
            .await
            .map_api_error()
            .await?;
        Ok(())
    }

    async fn get_store_info(
        &self,
        derivations: Vec<String>,
    ) -> Result<HashMap<String, Vec<StoreInfo>>, CatalogClientError> {
        let body = StoreInfoRequest {
            outpaths: derivations.iter().map(|s| s.to_string()).collect(),
        };
        let response = self
            .client
            .get_store_info_api_v1_catalog_store_post(&body)
            .await
            .map_api_error()
            .await?;
        let store_info = response.into_inner();
        Ok(store_info.items)
    }

    async fn is_publish_complete(
        &self,
        store_paths: &[String],
    ) -> Result<bool, CatalogClientError> {
        let req = StoreInfoRequest {
            outpaths: store_paths.to_vec(),
        };
        let statuses = self
            .client
            .get_storepath_status_api_v1_catalog_store_status_post(&req)
            .await
            .map_api_error()
            .await?;
        let all_narinfo_available = statuses.items.values().all(|storepath_statuses_for_drv| {
            storepath_statuses_for_drv
                .iter()
                .all(|status| status.narinfo_known)
        });
        Ok(all_narinfo_available)
    }

    #[instrument(skip_all)]
    async fn get_base_catalog_info(&self) -> Result<BaseCatalogInfo, CatalogClientError> {
        self.client
            .get_base_catalog_api_v1_catalog_info_base_catalog_get()
            .await
            .map_api_error()
            .await
            .map(|res| res.into_inner().into())
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Converts a catalog name to a semantic type with API format validation.
pub fn str_to_catalog_name(
    name: impl AsRef<str>,
) -> Result<api_types::CatalogName, CatalogClientError> {
    api_types::CatalogName::from_str(name.as_ref()).map_err(|_e| {
        CatalogClientError::APIError(APIError::InvalidRequest(format!(
            "catalog name {} does not meet API requirements.",
            name.as_ref()
        )))
    })
}

/// Converts a package name to a semantic type with API format validation.
pub fn str_to_package_name(
    name: impl AsRef<str>,
) -> Result<api_types::PackageName, CatalogClientError> {
    api_types::PackageName::from_str(name.as_ref()).map_err(|_e| {
        CatalogClientError::APIError(APIError::InvalidRequest(format!(
            "package name {} does not meet API requirements.",
            name.as_ref()
        )))
    })
}

/// Collects a stream of results into a container, returning the total count.
async fn collect_search_results<T, E>(
    stream: impl Stream<Item = Result<StreamItem<T>, E>>,
    limit: SearchLimit,
) -> Result<(ResultCount, Vec<T>), E> {
    let mut count = None;
    let actual_limit = if let Some(checked_limit) = limit {
        checked_limit.get() as usize
    } else {
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

/// Create a depaging stream from a page-fetching function.
///
/// Takes a function that returns `(total_count, items)` for a given page, and
/// yields `TotalCount` once followed by all `Result` items across pages.
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

            if items_on_page < page_size.get() as usize {
                break;
            }
            if total_count == (page_number+1) * page_size.get() as i64 {
                break;
            }
            page_number += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP client builder
// ---------------------------------------------------------------------------

/// Build HTTP client with bearer token auth for FloxHub catalog API.
fn build_http_client(config: &CatalogClientConfig) -> Result<reqwest::Client, CatalogClientError> {
    let mut headers = HeaderMap::new();

    // Bearer token for catalog API authentication
    if let Some(token) = &config.floxhub_token {
        headers.insert(
            header::HeaderName::from_static("authorization"),
            header::HeaderValue::from_str(&format!("bearer {token}"))
                .map_err(|e| CatalogClientError::Other(e.to_string()))?,
        );
    }

    // Extra headers (SDK can add invocation-source, QoS, etc.)
    for (key, value) in &config.extra_headers {
        headers.insert(
            header::HeaderName::from_str(key).map_err(
                |e: reqwest::header::InvalidHeaderName| CatalogClientError::Other(e.to_string()),
            )?,
            header::HeaderValue::from_str(value).map_err(
                |e: reqwest::header::InvalidHeaderValue| CatalogClientError::Other(e.to_string()),
            )?,
        );
    }

    debug!(
        catalog_url = %config.catalog_url,
        has_token = config.floxhub_token.is_some(),
        extra_headers = config.extra_headers.len(),
        "building catalog HTTP client"
    );

    let client_builder = reqwest::Client::builder();

    let client_builder = client_builder
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60));

    let client_builder = if let Some(ref user_agent) = config.user_agent {
        client_builder.user_agent(user_agent)
    } else {
        client_builder
    };

    client_builder
        .build()
        .map_err(|e| CatalogClientError::Other(e.to_string()))
}

#[cfg(test)]
pub mod tests {
    use std::collections::BTreeMap;
    use std::num::NonZeroU8;

    use httpmock::MockServer;
    use itertools::Itertools;
    use proptest::prelude::*;
    use proptest::proptest;
    use serde_json::json;
    use tracing::Instrument;
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;

    const SENTRY_TRACE_HEADER: &str = "sentry-trace";

    fn client_config(url: &str) -> CatalogClientConfig {
        CatalogClientConfig {
            catalog_url: url.to_string(),
            floxhub_token: None,
            extra_headers: Default::default(),
            mock_mode: Default::default(),
            user_agent: None,
        }
    }

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

        let client = CatalogClient::new(client_config(server.base_url().as_str())).unwrap();
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
    async fn extra_headers_set_on_all_requests() {
        let mut extra_headers: BTreeMap<String, String> = BTreeMap::new();
        extra_headers.insert("flox-test".to_string(), "test-value".to_string());
        extra_headers.insert("flox-test2".to_string(), "test-value2".to_string());

        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.header("flox-test", "test-value")
                .and(|when| when.header("flox-test2", "test-value2"));
            then.status(200).json_body_obj(EMPTY_SEARCH_RESPONSE);
        });

        let config = CatalogClientConfig {
            extra_headers,
            ..client_config(&server.base_url())
        };

        let client = CatalogClient::new(config).unwrap();
        let _ = client.package_versions("some-package").await;
        mock.assert();
    }

    #[tokio::test]
    async fn user_agent_set_on_all_requests() {
        let expected_agent = "my-custom-user-agent";

        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.header("user-agent", expected_agent);
            then.status(200).json_body_obj(EMPTY_SEARCH_RESPONSE);
        });

        let config = CatalogClientConfig {
            user_agent: Some(expected_agent.to_owned()),
            ..client_config(&server.base_url())
        };

        let client = CatalogClient::new(config).unwrap();
        let _ = client.package_versions("some-package").await;
        mock.assert();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tracing_headers_present_when_sentry_enabled() {
        let server = MockServer::start_async().await;
        let client = CatalogClient::new(client_config(server.base_url().as_str())).unwrap();

        // The following are needed, in this order, for headers to be added:
        //
        // 1. Tracing subscriber with Sentry layer. This is normally initialized
        //    globally by the CLI regardless of whether metrics and Sentry are
        //    enabled. For this test it is scoped.
        let subscriber =
            tracing_subscriber::Registry::default().with(sentry::integrations::tracing::layer());
        let _subscriber_guard = tracing::subscriber::set_default(subscriber);

        let mock = server.mock(|when, then| {
            when.header_exists(SENTRY_TRACE_HEADER); // Ensure present.
            then.status(200).json_body_obj(EMPTY_SEARCH_RESPONSE);
        });

        // 2. Sentry client and hub. This is normally initialized globally by the
        //    CLI only if metrics and Sentry are enabled. For this test it is
        //    scoped.

        sentry::test::with_captured_envelopes(|| {
            // 3. An active span. This is normally already created by the CLI, typically
            //    from `flox::commands`.

            tokio::task::block_in_place(move || {
                tokio::runtime::Handle::current().block_on(async move {
                    let res = client
                        .package_versions("some-package")
                        .instrument(tracing::info_span!("test span"))
                        .await;
                    mock.assert();
                    assert!(res.is_ok(), "Expected successful response, got: {:?}", res);
                })
                // do something async
            });
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tracing_headers_absent_when_sentry_disabled() {
        let server = MockServer::start_async().await;
        let client = CatalogClient::new(client_config(server.base_url().as_str())).unwrap();

        let subscriber =
            tracing_subscriber::Registry::default().with(sentry::integrations::tracing::layer());
        let _subscriber_guard = tracing::subscriber::set_default(subscriber);

        let mock = server.mock(|when, then| {
            when.header_missing(SENTRY_TRACE_HEADER); // Ensure absent.
            then.status(200).json_body_obj(EMPTY_SEARCH_RESPONSE);
        });

        // This does the same as the previous test except for initializing the
        // Sentry client and hub. It would give false positives if the
        // subscriber and span weren't also present.

        let res = client
            .package_versions("some-package")
            .instrument(tracing::info_span!("test span"))
            .await;
        mock.assert();
        assert!(res.is_ok(), "Expected successful response, got: {:?}", res);
    }

    // region: Error response handling
    //
    // Client errors and response error handling of the progenitor generated client
    // follows the client spec.
    // For example the package version API is expected
    // to return 404 and 422 error responses with a json body
    // of the form `{ "detail": <String> }`.
    // Errorneous responses (!= 200) _not_ mathcing these two cases,
    // are represented as `APIError::UnexpectedResponse`s.
    // Responses with expected status but not matching the expected body schema,
    // will turn into `APIError::InvalidResponsePayload`.

    /// 404 errors are mapped to [VersionsError::NotFound],
    /// so consumers dont need to inspect raw error response
    #[tokio::test]
    async fn versions_error_response_not_found() {
        let server = MockServer::start_async().await;

        let mock = server.mock(|_, then| {
            then.status(404)
                .header("content-type", "application/json")
                .json_body(json! ({"detail" : "(╯°□°)╯︵ ┻━┻ "}));
        });

        let client = CatalogClient::new(client_config(server.base_url().as_str())).unwrap();
        let result = client.package_versions("some-package").await;
        assert!(
            matches!(result, Err(VersionsError::NotFound)),
            "expected VersionsError::NotFound, found: {result:?}"
        );
        mock.assert()
    }

    /// Other known error responses are detected
    #[tokio::test]
    async fn version_error_response() {
        let server = MockServer::start_async().await;

        let mock = server.mock(|_, then| {
            then.status(422)
                .header("content-type", "application/json")
                .json_body(json! ({"detail" : "(╯°□°)╯︵ ┻━┻ "}));
        });

        let client = CatalogClient::new(client_config(server.base_url().as_str())).unwrap();
        let result = client.package_versions("some-package").await;
        assert!(
            matches!(
                result,
                Err(VersionsError::CatalogClientError(
                    CatalogClientError::APIError(APIError::ErrorResponse(_))
                ))
            ),
            "expected ErrorResponse, found: {result:?}"
        );
        mock.assert()
    }

    /// Other unknown error responses are [APIError::UnexpectedResponse]s
    #[tokio::test]
    async fn version_unknown_response() {
        let server = MockServer::start_async().await;

        let mock = server.mock(|_, then| {
            then.status(418)
                .header("content-type", "application/json")
                .json_body(json! ({"unknown" : "ceramic"}));
        });

        let client = CatalogClient::new(client_config(server.base_url().as_str())).unwrap();
        let result = client.package_versions("some-package").await;
        assert!(
            matches!(
                result,
                Err(VersionsError::CatalogClientError(
                    CatalogClientError::APIError(APIError::UnexpectedResponse(_))
                ))
            ),
            "expected APIError::UnexpectedResponse, found: {result:?}"
        );
        mock.assert()
    }

    // endregion

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
        fn collects_correct_number_of_results(results in proptest::collection::vec(any::<i32>(), 0..10), raw_limit in 0..10_u8) {
            let total = results.len();
            let results_ref = &results;
            let stream = async_stream::stream! {
                yield Ok::<StreamItem<i32>, String>(StreamItem::TotalCount(total as u64));
                for item in results_ref.iter() {
                    yield Ok(StreamItem::Result(*item));
                }
            };
            let limit = NonZeroU8::new(raw_limit); // None if raw_limit == 0
            let (found_count, collected_results) = tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(collect_search_results(stream, limit))
                .unwrap();
            prop_assert_eq!(found_count, Some(total as u64));

            let expected_results = if limit.is_some() {
                results.into_iter().take(raw_limit as usize).collect::<Vec<_>>()
            } else {
                results
            };
            prop_assert_eq!(expected_results, collected_results);
        }
    }
}
