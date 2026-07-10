//! FloxhubClient: shared catalog + factory SDK client.
//!
//! [`FloxhubClient`] fronts both the catalog and factory surfaces of FloxHub.
//! Both generated inner clients (`catalog_api_v1::Client` and
//! `factory_api_v1::Client`) share a single reqwest connection pool, a single
//! auth pre-request hook, and (when configured) a single record/replay
//! [`MockGuard`]. This means authentication, Sentry trace headers, timeouts,
//! and mock recording are wired once and apply to all outgoing requests
//! regardless of which API surface they target.

use std::cmp::min;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Debug;
use std::future::{Future, ready};
use std::num::NonZeroU32;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use async_stream::try_stream;
use catalog_api_v1::types::{self as api_types};
use catalog_api_v1::{Client as CatalogApiClient, Error as APIError, RequestHooks};
use factory_api_v1::Client as FactoryApiClient;
use futures::stream::Stream;
use futures::{StreamExt, TryStreamExt};
use reqwest::StatusCode;
use reqwest::header::{self, HeaderMap};
use tracing::{debug, instrument};
use url::Url;

use crate::MapApiErrorExt;
use crate::auth::AuthContext;
use crate::config::FloxhubClientConfig;
use crate::error::{FloxhubClientError, ResolveError, SearchError, VersionsError};
use crate::mock::MockGuard;
use crate::types::*;

#[cfg(any(test, feature = "tests"))]
pub const EMPTY_SEARCH_RESPONSE: &api_types::PackageSearchResult =
    &api_types::PackageSearchResult {
        items: vec![],
        total_count: 0,
    };

/// A client for the FloxHub catalog and factory service APIs.
///
/// Wraps both generated API clients (`catalog_api_v1::Client` and
/// `factory_api_v1::Client`) and handles:
/// - HTTP client construction with shared connection pool and timeouts
/// - Bearer token / Kerberos authentication via a shared pre-request hook
/// - Mock server recording/replay for testing (single guard covers both APIs)
///
/// The `base_url` / [`FloxhubClientConfig`] field fronts both the catalog
/// and factory surfaces; both inner clients target the same effective URL.
pub struct FloxhubClient {
    /// Catalog inner client.
    pub(crate) catalog: CatalogApiClient,
    /// Factory inner client, sharing the same reqwest client and auth hook.
    pub(crate) factory: FactoryApiClient,
    config: FloxhubClientConfig,

    _mock_guard: Option<MockGuard>,
}

impl Debug for FloxhubClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FloxhubClient")
            .field("base_url", &self.config.base_url)
            .finish_non_exhaustive()
    }
}

impl FloxhubClient {
    /// Create a new client fronting both the catalog and factory surfaces.
    ///
    /// The reqwest connection pool, auth hook, and (when configured) the
    /// record/replay [`MockGuard`] are built once and shared by both inner
    /// clients. `reqwest::Client` clones share the underlying pool, so there
    /// is no double connection overhead.
    pub fn new(config: FloxhubClientConfig) -> Result<Self, FloxhubClientError> {
        // One MockGuard covers both surfaces.
        let mock_guard = MockGuard::new(&config);
        let effective_url = match mock_guard {
            Some(ref mock) => mock.url(),
            None => config.base_url.clone(),
        };

        // Build the shared auth closure once; wrap it in each crate's
        // RequestHooks. The Arc clone is cheap — both hooks share the closure.
        let pre_request = build_pre_request_hook(config.auth_context.clone());
        let catalog_hooks = RequestHooks {
            pre_request: Arc::clone(&pre_request),
        };
        let factory_hooks = factory_api_v1::RequestHooks {
            pre_request: Arc::clone(&pre_request),
        };

        // One reqwest::Client for both inner clients. Clones share the pool.
        let http_client = build_http_client(
            &config.extra_headers,
            config.user_agent.as_deref(),
            &config.base_url,
        )
        .map_err(FloxhubClientError::Other)?;

        let catalog =
            CatalogApiClient::new_with_client(&effective_url, http_client.clone(), catalog_hooks);
        let factory = FactoryApiClient::new_with_client(&effective_url, http_client, factory_hooks);

        Ok(Self {
            catalog,
            factory,
            config,
            _mock_guard: mock_guard,
        })
    }

    /// Access the underlying catalog API client for making requests.
    pub fn api(&self) -> &CatalogApiClient {
        &self.catalog
    }

    /// Get the configured base URL.
    pub fn base_url(&self) -> &str {
        &self.config.base_url
    }

    /// Clear mock recording state if in recording mode.
    ///
    /// Useful in tests where setup operations should not be captured.
    pub fn reset_recording(&mut self) {
        if let Some(ref mut guard) = self._mock_guard {
            guard.reset_recording();
        }
    }

    /// Configure JSON field names to remove from recorded request bodies.
    ///
    /// Any request body that is valid JSON and contains any of the given keys
    /// (at any nesting depth) will have those keys removed in the saved YAML
    /// file, and the matcher switched from exact `body` to
    /// `json_body_includes` (subset matching via
    /// `assert-json-diff::CompareMode::Inclusive`).  Replay therefore accepts
    /// any value for those fields, which makes recordings stable across
    /// machines when the fields vary per build (e.g. `registrationTime`,
    /// `deriver`).
    ///
    /// Only takes effect in `Record` mode; no-op in `Replay`/`None`.
    pub fn set_record_body_redact_keys(&mut self, keys: Vec<String>) {
        if let Some(ref mut guard) = self._mock_guard {
            guard.set_body_redact_keys(keys);
        }
    }

    /// Update the client configuration and recreate the client.
    pub fn update_config(
        &mut self,
        update: impl FnOnce(&mut FloxhubClientConfig),
    ) -> Result<(), FloxhubClientError> {
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

/// Query describing a build to check for prior recording/publication via
/// [`CatalogClientTrait::check_build_already_recorded`].
///
/// Mirrors the server-side `CheckBuildRequest` API type field-by-field
/// (except `locked_inputs`, which is optional on the wire but always sent),
/// plus the catalog/package name path segments. Adding a new server-side
/// field is then a struct-field change here, not a signature change across
/// the trait, the [`FloxhubClient`] impl, and the mock implementation.
#[derive(Debug, Clone, Copy)]
pub struct CheckBuildQuery<'a> {
    pub catalog_name: &'a str,
    pub package_name: &'a str,
    pub source_url: &'a Url,
    pub source_rev: &'a str,
    pub nixpkgs_rev: &'a str,
    pub system: api_types::PackageSystem,
    pub locked_inputs: &'a HashMap<String, api_types::LockedInputEntry>,
}

/// The complete catalog API interface.
///
/// This trait enables alternate implementations:
/// - **HTTP** (current): REST calls to FloxHub catalog API via [`FloxhubClient`]
/// - **Mock** (SDK tests): Canned responses without HTTP
/// - **Direct** (future): FloxHub server calls catalog logic in-process
#[allow(async_fn_in_trait)]
pub trait CatalogClientTrait {
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
    ) -> Result<PublishResponse, FloxhubClientError>;

    /// Get all locked sources for a catalog.
    async fn get_catalog_locked_sources(
        &self,
        catalog_name: impl AsRef<str> + Send + Sync,
    ) -> Result<ResultsPage<LockedSourceItem>, FloxhubClientError>;

    /// Look up and lock build inputs for one or more grouped reference sets in
    /// a single batched request (POST /build-inputs/lookup).
    async fn build_inputs_lookup(
        &self,
        request: BuildInputsLookupRequest,
    ) -> Result<BuildInputsLookupResponse, FloxhubClientError>;

    /// Create a package within a user catalog.
    async fn create_package(
        &self,
        catalog_name: impl AsRef<str> + Send + Sync,
        package_name: impl AsRef<str> + Send + Sync,
        original_url: impl AsRef<str> + Send + Sync,
    ) -> Result<(), FloxhubClientError>;

    /// Publish a build of a user package.
    async fn publish_build(
        &self,
        catalog_name: impl AsRef<str> + Send + Sync,
        package_name: impl AsRef<str> + Send + Sync,
        build_info: &UserBuildPublish,
    ) -> Result<(), FloxhubClientError>;

    /// Get store info for a list of derivations.
    async fn get_store_info(
        &self,
        derivations: Vec<String>,
    ) -> Result<HashMap<String, Vec<StoreInfo>>, FloxhubClientError>;

    /// Checks whether the provided store paths have been successfully
    /// published.
    async fn is_publish_complete(&self, store_paths: &[String])
    -> Result<bool, FloxhubClientError>;

    /// Get information about the base catalog and available stabilities.
    async fn get_base_catalog_info(&self) -> Result<BaseCatalogInfo, FloxhubClientError>;

    /// Query the catalog to check whether a build matching the given source
    /// tuple (source URL, source rev, nixpkgs rev, system, package name, and
    /// direct closure inputs) has already been recorded/published.
    ///
    /// Callers must always provide the direct catalog inputs. An empty map
    /// means the build has no catalog dependencies and serialises as `{}` on
    /// the wire. Optionality at the generated request type level exists only
    /// for old-CLI compatibility and does not apply here.
    ///
    /// Returns provenance data (source rev date, rev) in `CheckBuildResponse`
    /// when `already_published` is true. Used for dedup pre-check before
    /// uploading/publishing.
    async fn check_build_already_recorded(
        &self,
        query: CheckBuildQuery<'_>,
    ) -> Result<CheckBuildResponse, FloxhubClientError>;
}

// ---------------------------------------------------------------------------
// CatalogClientTrait implementation for FloxhubClient
// ---------------------------------------------------------------------------

impl CatalogClientTrait for FloxhubClient {
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
                .collect::<Result<Vec<api_types::PackageGroup>, _>>()?
                .into_iter()
                .map(|mut group| {
                    // Fall back to the client's stability pin when a group
                    // doesn't carry its own value. A per-group value (once
                    // plumbed in) always wins over the config default.
                    // Test/regen-only — see `FloxhubClientConfig::stability`.
                    group.stability = group.stability.or_else(|| self.config.stability.clone());
                    group
                })
                .collect(),
        };

        let response = self
            .catalog
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
                    .catalog
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
                    .catalog
                    .packages_api_v1_catalog_packages_attr_path_get(
                        attr_path,
                        Some(page_number),
                        Some(page_size),
                    )
                    .await
                    .map_api_error()
                    .await
                    .map_err(|e| match e {
                        FloxhubClientError::APIError(APIError::ErrorResponse(response))
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

    async fn get_catalog_locked_sources(
        &self,
        catalog_name: impl AsRef<str> + Send + Sync,
    ) -> Result<ResultsPage<LockedSourceItem>, FloxhubClientError> {
        let catalog_name = catalog_name.as_ref();
        tracing::debug!(catalog_name, "fetching locked sources");

        let stream = make_depaging_stream(
            |page_number, page_size| async move {
                let catalog_name_api = str_to_catalog_name(catalog_name)?;
                let response = self
                    .catalog
                    .get_catalog_locked_sources_api_v1_catalog_catalogs_catalog_name_locked_sources_get(
                        &catalog_name_api,
                        Some(page_number),
                        Some(page_size),
                    )
                    .await
                    .map_api_error()
                    .await?
                    .into_inner();

                Ok::<_, FloxhubClientError>((response.total_count, response.items))
            },
            RESPONSE_PAGE_SIZE,
        );

        let (count, results) = collect_all_results(stream).await?;

        Ok(ResultsPage { results, count })
    }

    async fn build_inputs_lookup(
        &self,
        request: BuildInputsLookupRequest,
    ) -> Result<BuildInputsLookupResponse, FloxhubClientError> {
        tracing::debug!(n_groups = request.groups.len(), "looking up build inputs");

        // NOTE: unlike sibling catalog endpoints, the generated lookup endpoint
        // is typed with `HttpValidationError` (the default 422 envelope) rather
        // than the shared `ErrorResponse`, so the `map_api_error` machinery does
        // not apply. Map to a string error for now; richer token/4xx handling
        // lands with the lock binary in ECO-94. (Likely wants a backend schema
        // fix to declare `ErrorResponse` like the other endpoints.)
        let response = self
            .catalog
            .lookup_api_v1_catalog_build_inputs_lookup_post(&request)
            .await
            .map_err(|err| FloxhubClientError::Other(err.to_string()))?;

        Ok(response.into_inner())
    }

    async fn publish_info(
        &self,
        catalog_name: impl AsRef<str> + Send + Sync,
        package_name: impl AsRef<str> + Send + Sync,
    ) -> Result<PublishResponse, FloxhubClientError> {
        let catalog = str_to_catalog_name(catalog_name)?;
        let package = str_to_package_name(package_name)?;
        let body = api_types::PublishInfoRequest(serde_json::Map::new());
        self.catalog
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
    ) -> Result<(), FloxhubClientError> {
        let body = api_types::UserPackageCreate {
            original_url: Some(original_url.as_ref().to_string()),
        };
        let catalog = str_to_catalog_name(&catalog_name)?;
        let package = str_to_package_name(&package_name)?;
        self.catalog
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
    ) -> Result<(), FloxhubClientError> {
        let catalog = str_to_catalog_name(catalog_name)?;
        let package = str_to_package_name(package_name)?;
        self.catalog
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
    ) -> Result<HashMap<String, Vec<StoreInfo>>, FloxhubClientError> {
        let body = StoreInfoRequest {
            outpaths: derivations.iter().map(|s| s.to_string()).collect(),
        };
        let response = self
            .catalog
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
    ) -> Result<bool, FloxhubClientError> {
        let req = StoreInfoRequest {
            outpaths: store_paths.to_vec(),
        };
        let statuses = self
            .catalog
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
    async fn get_base_catalog_info(&self) -> Result<BaseCatalogInfo, FloxhubClientError> {
        self.catalog
            .get_base_catalog_api_v1_catalog_info_base_catalog_get()
            .await
            .map_api_error()
            .await
            .map(|res| res.into_inner().into())
    }

    async fn check_build_already_recorded(
        &self,
        query: CheckBuildQuery<'_>,
    ) -> Result<CheckBuildResponse, FloxhubClientError> {
        let catalog = str_to_catalog_name(query.catalog_name)?;
        let package = str_to_package_name(query.package_name)?;
        let body = api_types::CheckBuildRequest {
            source_url: query.source_url.to_string(),
            source_rev: query.source_rev.to_string(),
            nixpkgs_rev: query.nixpkgs_rev.to_string(),
            system: query.system,
            locked_inputs: Some(query.locked_inputs.clone()),
        };
        self.catalog
            .check_build_api_v1_catalog_catalogs_catalog_name_packages_package_name_check_build_post(
                &catalog,
                &package,
                &body,
            )
            .await
            .map_api_error()
            .await
            .map(|resp| resp.into_inner())
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Converts a catalog name to a semantic type with API format validation.
pub fn str_to_catalog_name(
    name: impl AsRef<str>,
) -> Result<api_types::CatalogName, FloxhubClientError> {
    api_types::CatalogName::from_str(name.as_ref()).map_err(|_e| {
        FloxhubClientError::APIError(APIError::InvalidRequest(format!(
            "catalog name {} does not meet API requirements.",
            name.as_ref()
        )))
    })
}

/// Converts a package name to a semantic type with API format validation.
pub fn str_to_package_name(
    name: impl AsRef<str>,
) -> Result<api_types::PackageName, FloxhubClientError> {
    api_types::PackageName::from_str(name.as_ref()).map_err(|_e| {
        FloxhubClientError::APIError(APIError::InvalidRequest(format!(
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

/// Collects all results from a stream, returning the total count and all items.
pub(crate) async fn collect_all_results<T, E>(
    stream: impl Stream<Item = Result<StreamItem<T>, E>>,
) -> Result<(ResultCount, Vec<T>), E> {
    let mut count = None;
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
        .try_collect::<Vec<T>>()
        .await?;

    Ok((count, results))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum StreamItem<T> {
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
pub(crate) fn make_depaging_stream<T, E, Fut>(
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
// Shared HTTP client construction helpers (pub(crate) for factory module)
// ---------------------------------------------------------------------------

/// Build the pre-request closure that injects Sentry trace headers and the
/// bearer/Kerberos `Authorization` header on every outgoing request.
///
/// The returned `Arc` can be wrapped in either `catalog_api_v1::RequestHooks`
/// or `factory_api_v1::RequestHooks`; both accept the same closure type.
/// Capturing `credential` by value means a single SPNEGO token is negotiated
/// per Kerberos session, which is the intended behaviour.
pub(crate) fn build_pre_request_hook(
    credential: AuthContext,
) -> Arc<dyn Fn(&mut reqwest::Request) + Send + Sync> {
    Arc::new(move |request: &mut reqwest::Request| {
        // Propagate the Sentry trace ID to the backend service.
        // This is a no-op when metrics are disabled because Sentry will
        // not have been initialized.
        if let Some(span) = sentry::configure_scope(|scope| scope.get_span()) {
            for (k, v) in span.iter_headers() {
                if let Ok(value) = reqwest::header::HeaderValue::from_str(&v) {
                    request.headers_mut().append(k, value);
                }
            }
        }

        if let Some(result) = credential.authorization_header(request.url()) {
            match result {
                Ok(value) => {
                    if let Ok(header_value) = reqwest::header::HeaderValue::from_str(&value) {
                        request
                            .headers_mut()
                            .insert(reqwest::header::AUTHORIZATION, header_value);
                    }
                },
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to produce authorization header")
                },
            }
        }
    })
}

/// Build a configured `reqwest::Client` with standard timeouts and the
/// provided extra headers and user-agent.
///
/// Authentication is injected per-request via the hook returned by
/// [`build_pre_request_hook`], not baked into the default headers here.
///
/// `base_url` is used only in the debug log line.
pub(crate) fn build_http_client(
    extra_headers: &BTreeMap<String, String>,
    user_agent: Option<&str>,
    base_url: &str,
) -> Result<reqwest::Client, String> {
    let headers = build_header_map(extra_headers)?;

    debug!(
        base_url = %base_url,
        extra_headers = extra_headers.len(),
        "building HTTP client"
    );

    let client_builder = reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60));

    let client_builder = if let Some(ua) = user_agent {
        client_builder.user_agent(ua)
    } else {
        client_builder
    };

    client_builder.build().map_err(|e| e.to_string())
}

/// Build the default header map from extra headers.
///
/// Authentication headers are NOT included here — they are injected
/// per-request via the hook returned by [`build_pre_request_hook`].
pub(crate) fn build_header_map(
    extra_headers: &BTreeMap<String, String>,
) -> Result<HeaderMap, String> {
    let mut header_map = HeaderMap::new();

    for (key, value) in extra_headers {
        let name = header::HeaderName::from_str(key)
            .map_err(|_| format!("invalid extra header name '{key}'"))?;
        let value = header::HeaderValue::from_str(value)
            .map_err(|_| format!("invalid value for extra header '{key}'"))?;
        header_map.insert(name, value);
    }

    Ok(header_map)
}

/// Test helpers for constructing [`FloxhubClient`] instances.
///
/// Intentionally not behind `#[cfg(test)]` so that other crates' (also
/// non-gated) test helpers can build a client without enabling a feature.
/// Nothing here should be used in production code.
pub mod test_helpers {
    use super::FloxhubClient;
    use crate::auth::AuthContext;
    use crate::config::FloxhubClientConfig;

    /// Build an unauthenticated [`FloxhubClientConfig`] pointed at `url`,
    /// with no mock mode, extra headers, or user agent.
    pub fn client_config(url: &str) -> FloxhubClientConfig {
        FloxhubClientConfig {
            base_url: url.to_string(),
            extra_headers: Default::default(),
            mock_mode: Default::default(),
            auth_context: AuthContext::from_mode(&Default::default(), None),
            user_agent: None,
            stability: None,
        }
    }

    /// Build a no-op client for tests that need a structurally valid
    /// [`FloxhubClient`] but never issue a request.
    ///
    /// Pointed at an unroutable dummy URL with no mock mode, so an unexpected
    /// catalog or factory call fails fast and locally rather than reaching a
    /// real server. Tests that exercise the network install a replay client.
    pub fn new_noop() -> FloxhubClient {
        FloxhubClient::new(client_config("http://localhost:0"))
            .expect("failed to build no-op FloxhubClient")
    }
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

    use super::test_helpers::client_config;
    use super::*;
    const SENTRY_TRACE_HEADER: &str = "sentry-trace";

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

        let client = FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();
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

    /// `resolve()` applies `FloxhubClientConfig::stability` to every
    /// outgoing group, regardless of what (if anything) `TryFrom` set.
    #[tokio::test]
    async fn resolve_applies_config_stability_to_each_group() {
        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path("/api/v1/catalog/resolve")
                .json_body(json!({
                    "items": [{
                        "descriptors": [],
                        "name": "group-one",
                        "stability": "lts",
                    }, {
                        "descriptors": [],
                        "name": "group-two",
                        "stability": "lts",
                    }]
                }));
            then.status(200).json_body(json!({"items": []}));
        });

        let config = FloxhubClientConfig {
            stability: Some("lts".to_string()),
            ..client_config(&server.base_url())
        };
        let client = FloxhubClient::new(config).unwrap();
        client
            .resolve(vec![
                PackageGroup {
                    name: "group-one".to_string(),
                    descriptors: vec![],
                },
                PackageGroup {
                    name: "group-two".to_string(),
                    descriptors: vec![],
                },
            ])
            .await
            .unwrap();
        mock.assert();
    }

    /// When `FloxhubClientConfig::stability` is `None`, the outgoing group
    /// carries no `stability` key at all (not a `null`).
    #[tokio::test]
    async fn resolve_omits_stability_when_config_unset() {
        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path("/api/v1/catalog/resolve")
                .json_body(json!({
                    "items": [{
                        "descriptors": [],
                        "name": "group",
                    }]
                }));
            then.status(200).json_body(json!({"items": []}));
        });

        let client = FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();
        client
            .resolve(vec![PackageGroup {
                name: "group".to_string(),
                descriptors: vec![],
            }])
            .await
            .unwrap();
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

        let config = FloxhubClientConfig {
            extra_headers,
            ..client_config(&server.base_url())
        };

        let client = FloxhubClient::new(config).unwrap();
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

        let config = FloxhubClientConfig {
            user_agent: Some(expected_agent.to_owned()),
            ..client_config(&server.base_url())
        };

        let client = FloxhubClient::new(config).unwrap();
        let _ = client.package_versions("some-package").await;
        mock.assert();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tracing_headers_present_when_sentry_enabled() {
        let server = MockServer::start_async().await;
        let client = FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();

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
        let client = FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();

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
    // Erroneous responses (!= 200) _not_ matching these two cases,
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

        let client = FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();
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

        let client = FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();
        let result = client.package_versions("some-package").await;
        assert!(
            matches!(
                result,
                Err(VersionsError::FloxhubClientError(
                    FloxhubClientError::APIError(APIError::ErrorResponse(_))
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

        let client = FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();
        let result = client.package_versions("some-package").await;
        assert!(
            matches!(
                result,
                Err(VersionsError::FloxhubClientError(
                    FloxhubClientError::APIError(APIError::UnexpectedResponse(_))
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

    // ---------------------------------------------------------------------------
    // check_build_already_recorded — closure-aware dedup pre-check
    // ---------------------------------------------------------------------------

    const CHECK_BUILD_PATH: &str = "/api/v1/catalog/catalogs/myorg/packages/mypkg/check-build";

    /// locked_inputs (non-empty map) is serialised into the check-build request
    /// body so the server can perform a closure-aware dedup match.
    #[tokio::test]
    async fn check_build_sends_locked_inputs() {
        use catalog_api_v1::types as api_types;

        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            // Verify key fields are present; inputs=null is omitted by
            // skip_serializing_if so we don't assert on it here.
            when.method("POST")
                .path(CHECK_BUILD_PATH)
                .json_body_includes(
                    json!({
                        "locked_inputs": {
                            "dep-key": {
                                "catalog": "nixpkgs",
                                "attr_path": ["hello"],
                                "build_type": "manifest",
                                "locked_inputs_hash": "sha256:aabbcc"
                            }
                        }
                    })
                    .to_string(),
                );
            then.status(200).json_body(json!({
                "already_published": false
            }));
        });

        let client = FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();

        let entry = api_types::LockedInputEntry {
            catalog: "nixpkgs".to_string(),
            attr_path: vec!["hello".to_string()],
            build_type: api_types::BuildType::Manifest,
            source: api_types::LockedGitSource {
                type_: "git".to_string(),
                url: "https://github.com/NixOS/nixpkgs".to_string(),
                rev: "abc123".to_string(),
                ref_: "main".to_string(),
                dir: ".".to_string(),
            },
            inputs: None,
            locked_inputs_hash: "sha256:aabbcc".to_string(),
        };
        let mut locked_inputs = HashMap::new();
        locked_inputs.insert("dep-key".to_string(), entry);

        let result = client
            .check_build_already_recorded(CheckBuildQuery {
                catalog_name: "myorg",
                package_name: "mypkg",
                source_url: &"https://example.com/repo".parse().unwrap(),
                source_rev: "deadbeef",
                nixpkgs_rev: "cafebabe",
                system: api_types::PackageSystem::X8664Linux,
                locked_inputs: &locked_inputs,
            })
            .await;

        mock.assert();
        assert_eq!(result.expect("expected Ok"), CheckBuildResponse {
            already_published: false,
            published_at: None,
            source_rev: None,
            source_rev_date: None,
        });
    }

    /// An empty locked_inputs map serialises as `{}` (not omitted).
    #[tokio::test]
    async fn check_build_sends_empty_locked_inputs_as_object() {
        use catalog_api_v1::types as api_types;

        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.method("POST")
                .path(CHECK_BUILD_PATH)
                .json_body_includes(json!({ "locked_inputs": {} }).to_string());
            then.status(200).json_body(json!({
                "already_published": false
            }));
        });

        let client = FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();

        let result = client
            .check_build_already_recorded(CheckBuildQuery {
                catalog_name: "myorg",
                package_name: "mypkg",
                source_url: &"https://example.com/repo".parse().unwrap(),
                source_rev: "deadbeef",
                nixpkgs_rev: "cafebabe",
                system: api_types::PackageSystem::X8664Linux,
                locked_inputs: &HashMap::new(),
            })
            .await;

        mock.assert();
        assert_eq!(result.expect("expected Ok"), CheckBuildResponse {
            already_published: false,
            published_at: None,
            source_rev: None,
            source_rev_date: None,
        });
    }

    /// already_published=true → Ok with all provenance fields populated.
    #[tokio::test]
    async fn check_build_returns_already_published_true() {
        use catalog_api_v1::types as api_types;

        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.method("POST").path(CHECK_BUILD_PATH);
            then.status(200).json_body(json!({
                "already_published": true,
                "source_rev": "deadbeef",
                "published_at": "2024-01-01T00:00:00Z"
            }));
        });

        let client = FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();

        let result = client
            .check_build_already_recorded(CheckBuildQuery {
                catalog_name: "myorg",
                package_name: "mypkg",
                source_url: &"https://example.com/repo".parse().unwrap(),
                source_rev: "deadbeef",
                nixpkgs_rev: "cafebabe",
                system: api_types::PackageSystem::X8664Linux,
                locked_inputs: &HashMap::new(),
            })
            .await;

        mock.assert();
        let expected: CheckBuildResponse = serde_json::from_value(json!({
            "already_published": true,
            "source_rev": "deadbeef",
            "published_at": "2024-01-01T00:00:00Z",
        }))
        .unwrap();
        assert_eq!(result.expect("expected Ok"), expected);
    }

    /// A 5xx error from the server is returned as Err, not panicked or swallowed.
    #[tokio::test]
    async fn check_build_server_error_returns_err() {
        use catalog_api_v1::types as api_types;

        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.method("POST").path(CHECK_BUILD_PATH);
            then.status(500);
        });

        let client = FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();

        let result = client
            .check_build_already_recorded(CheckBuildQuery {
                catalog_name: "myorg",
                package_name: "mypkg",
                source_url: &"https://example.com/repo".parse().unwrap(),
                source_rev: "deadbeef",
                nixpkgs_rev: "cafebabe",
                system: api_types::PackageSystem::X8664Linux,
                locked_inputs: &HashMap::new(),
            })
            .await;

        mock.assert();
        assert!(result.is_err(), "expected Err from 5xx, got: {result:?}");
    }
}
