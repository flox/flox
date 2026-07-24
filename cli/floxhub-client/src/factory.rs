//! Factory Service client surface, fronted by [`FloxhubClient`].
//!
//! [`FloxhubClientConfig`] is the single configuration type for both the
//! catalog and factory surfaces. [`FloxhubClient`] holds both generated
//! inner clients, constructed from a shared reqwest client and auth hook.
//!
//! The factory builds endpoints are exposed via [`FactoryClientTrait`], which
//! is implemented for [`FloxhubClient`]. Callers construct a [`FloxhubClient`]
//! and use it for both catalog and factory operations.

use std::num::{NonZeroU32, NonZeroU64};

use factory_api_v1::types::{
    AttrPathItem,
    BuildResponse,
    EffectiveBuildStatus,
    ErrorResponse,
    Since,
    SourceCommitShaItem,
    SystemItem,
};
use factory_api_v1::{ByteStream, Error as APIError, ResponseValue};

use crate::client::{collect_all_results, make_depaging_stream};
use crate::types::ResultsPage;

/// The Factory listing endpoint caps `page_size` at 200; depage at that size.
const FACTORY_PAGE_SIZE: NonZeroU32 = NonZeroU32::new(200).unwrap();

/// Alias for the expected error type in the API spec.
pub type ApiErrorResponse = ErrorResponse;
pub type ApiErrorResponseValue = ResponseValue<ApiErrorResponse>;

/// Common error type for factory API operations.
#[derive(Debug, thiserror::Error)]
pub enum FactoryClientError {
    /// The requested build was not found (HTTP 404).
    ///
    /// Deliberately carries no detail: the calling verb knows what it asked
    /// for and renders the user-facing message.
    #[error("not found")]
    NotFound,

    /// The Flox Factory service could not be reached (transport failure).
    ///
    /// progenitor renders this case with the request URL in the message; this
    /// variant lets callers show a product-level message without that detail.
    #[error("could not reach the Flox Factory")]
    Transport(#[source] reqwest::Error),

    /// Authentication or authorization was rejected (HTTP 401/403).
    ///
    /// Distinct from a generic API error because it is never retryable: the
    /// caller must re-authenticate, not back off. Wraps the underlying error so
    /// the server's `detail` still renders, matching the `APIError` variant.
    #[error("{}", fmt_api_error(.0))]
    AuthRejected(APIError<ErrorResponse>),

    /// The Flox Factory reported a server-side error (HTTP 5xx, or 422).
    ///
    /// Distinct from [`FactoryClientError::Transport`] because the host did
    /// respond: the failure is the service's, and the same request may succeed
    /// on retry. Wraps the underlying error so the server's `detail` still
    /// renders, matching the `APIError` variant.
    #[error("{}", fmt_api_error(.0))]
    Server(APIError<ErrorResponse>),

    /// Any other factory API error, rendered from the generated error type.
    ///
    /// This includes an unrecognised response (a non-auth 4xx, or a 200 whose
    /// body did not parse as a `BuildResponse`): it degrades to this generic
    /// case rather than escalating to a bespoke variant.
    #[error("{}", fmt_api_error(.0))]
    APIError(APIError<ErrorResponse>),
}

/// Extension trait for converting `factory-api-v1` errors into
/// [`FactoryClientError`].
#[allow(async_fn_in_trait)]
pub trait MapApiErrorExt<T> {
    async fn map_api_error(self) -> Result<T, FactoryClientError>;
}

impl<T: Send> MapApiErrorExt<T> for Result<T, APIError<ErrorResponse>> {
    async fn map_api_error(self) -> Result<T, FactoryClientError> {
        let err = match self {
            Ok(v) => return Ok(v),
            Err(err) => err,
        };

        match err {
            // progenitor renders a transport failure with the request URL in
            // the message; classify it so the caller can show a product-level
            // message without that detail.
            APIError::CommunicationError(source) => Err(FactoryClientError::Transport(source)),
            APIError::UnexpectedResponse(resp) => parse_api_error(resp).await,
            other => Err(FactoryClientError::APIError(other)),
        }
    }
}

async fn parse_api_error<T>(resp: reqwest::Response) -> Result<T, FactoryClientError> {
    let status = resp.status();
    match ApiErrorResponseValue::from_response::<ErrorResponse>(resp).await {
        Ok(resp_parsed) => Err(FactoryClientError::APIError(APIError::ErrorResponse(
            resp_parsed,
        ))),
        Err(_) => {
            let resp_bare = http::Response::builder()
                .status(status)
                .body("response body omitted by error parsing")
                .expect("failed to rebuild response while parsing error response")
                .into();
            Err(FactoryClientError::APIError(APIError::UnexpectedResponse(
                resp_bare,
            )))
        },
    }
}

fn fmt_api_error(api_error: &APIError<ErrorResponse>) -> String {
    match api_error {
        APIError::ErrorResponse(error_response) => {
            let status = error_response.status();
            let details = &error_response.detail;
            format!("{status}: {details}")
        },
        APIError::UnexpectedResponse(resp) => {
            let status = resp.status();
            format!("{status}")
        },
        _ => format!("{api_error}"),
    }
}

/// Reclassify a 404 from a resource-specific endpoint as
/// [`FactoryClientError::NotFound`].
///
/// Only callers where a 404 unambiguously means the requested resource does
/// not exist (such as [`FactoryClientTrait::get_build`]) should apply this. A
/// 404 from a listing endpoint or from a misconfigured base URL is a route or
/// configuration error, not a missing resource, and is left as the underlying
/// API error so the message describes what is actually wrong.
fn not_found_on_404(err: FactoryClientError) -> FactoryClientError {
    match err {
        FactoryClientError::APIError(api)
            if api.status() == Some(reqwest::StatusCode::NOT_FOUND) =>
        {
            FactoryClientError::NotFound
        },
        other => other,
    }
}

/// Classify a single-build resource error into the typed variants that the
/// cancel verb's exit-code space distinguishes.
///
/// Applied by [`FactoryClientTrait::cancel_build`], whose caller must tell apart
/// a missing build, an auth rejection, a server-side fault, and a 200 whose body
/// did not come from Factory Service. Builds on [`not_found_on_404`] for the 404
/// case. Endpoints without that need (`get_build`, the listing endpoint) keep
/// the underlying API error.
fn classify_build_error(err: FactoryClientError) -> FactoryClientError {
    let api = match not_found_on_404(err) {
        FactoryClientError::APIError(api) => api,
        // NotFound, Transport, and the typed variants are already classified.
        other => return other,
    };

    match api.status().map(|status| status.as_u16()) {
        // Auth failures are never retryable; keep them distinct from 5xx.
        Some(401 | 403) => FactoryClientError::AuthRejected(api),
        // 5xx is a service fault. 422 only arises from a non-integer path
        // (FastAPI request validation), which an `i64` never produces, so it is
        // mapped here defensively rather than left as a generic API error.
        Some(422 | 500..=599) => FactoryClientError::Server(api),
        // Everything else (a non-auth 4xx, or a 200 whose body did not parse as
        // a `BuildResponse`) degrades to the generic API error.
        _ => FactoryClientError::APIError(api),
    }
}

// ---------------------------------------------------------------------------
// BuildFilters
// ---------------------------------------------------------------------------

/// Server-side filters for [`FactoryClientTrait::list_builds`], one field per
/// query parameter.
///
/// The string fields hold the generated newtypes, which the schema pins as
/// non-empty, so no field can carry an empty value. The status vocabulary is
/// parsed at the CLI flag boundary. Empty collections and `None` mean "no
/// filter": a default `BuildFilters` requests every build, and the emitted
/// request omits the filter parameters entirely, so it is byte-identical to an
/// unfiltered call. The server ORs the values within a field (any match) and
/// ANDs across fields (all must match).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuildFilters {
    /// Match builds whose effective status is any of these.
    pub status: Vec<EffectiveBuildStatus>,
    /// Match builds for any of these systems, compared exactly. The server owns
    /// the system vocabulary and rejects a name outside it.
    pub system: Vec<SystemItem>,
    /// Match builds whose attribute path starts with any of these prefixes.
    pub attr_path: Vec<AttrPathItem>,
    /// Match builds whose source commit SHA starts with any of these prefixes.
    pub source_commit_sha: Vec<SourceCommitShaItem>,
    /// Restrict to builds created at or after this instant, given as a relative
    /// offset (`7d`) or ISO 8601. The server is the sole authority on the
    /// grammar.
    pub since: Option<Since>,
}

// ---------------------------------------------------------------------------
// FactoryClientTrait
// ---------------------------------------------------------------------------

/// The factory builds API interface, implemented for [`FloxhubClient`].
///
/// Methods return domain types (`ResultsPage<BuildResponse>`, `BuildResponse`,
/// `ByteStream`) and domain errors (`FactoryClientError`), keeping generated
/// HTTP types contained within the `floxhub-client` crate.
#[allow(async_fn_in_trait)]
pub trait FactoryClientTrait {
    /// Return all builds across pages, narrowed by `filters`.
    async fn list_builds(
        &self,
        filters: &BuildFilters,
    ) -> Result<ResultsPage<BuildResponse>, FactoryClientError>;

    /// Fetch a single build by its numeric ID.
    async fn get_build(&self, build_id: i64) -> Result<BuildResponse, FactoryClientError>;

    /// Cancel a single build by its numeric ID (DELETE).
    ///
    /// Idempotent: a satisfied-intent cancel returns the build with its
    /// effective `status`; the caller reads that field to distinguish a
    /// just-initiated cancellation from an already-terminal build.
    async fn cancel_build(&self, build_id: i64) -> Result<BuildResponse, FactoryClientError>;

    /// Proxy the raw log stream for a build.
    async fn get_build_logs(&self, build_id: i64) -> Result<ByteStream, FactoryClientError>;
}

// ---------------------------------------------------------------------------
// FactoryClientTrait implementation for FloxhubClient
// ---------------------------------------------------------------------------

impl FactoryClientTrait for crate::FloxhubClient {
    async fn list_builds(
        &self,
        filters: &BuildFilters,
    ) -> Result<ResultsPage<BuildResponse>, FactoryClientError> {
        // Map empty collections to None so an unfiltered call omits the query
        // parameter entirely, keeping the wire shape identical to before. These
        // references are Copy, so the per-page closure reuses them each call.
        let status = (!filters.status.is_empty()).then_some(&filters.status);
        let system = (!filters.system.is_empty()).then_some(&filters.system);
        let attr_path = (!filters.attr_path.is_empty()).then_some(&filters.attr_path);
        let source_commit_sha =
            (!filters.source_commit_sha.is_empty()).then_some(&filters.source_commit_sha);
        let since = filters.since.as_ref();

        let stream = make_depaging_stream(
            |page_number, page_size| async move {
                let response = self
                    .factory
                    .list_builds_api_v1_factory_builds_get(
                        attr_path,
                        None,
                        Some(page_number),
                        NonZeroU64::new(page_size as u64),
                        since,
                        None,
                        source_commit_sha,
                        status,
                        system,
                    )
                    .await
                    .map_api_error()
                    .await?
                    .into_inner();

                Ok::<_, FactoryClientError>((response.total, response.builds))
            },
            FACTORY_PAGE_SIZE,
        );

        let (count, results) = collect_all_results(stream).await?;
        Ok(ResultsPage { results, count })
    }

    async fn get_build(&self, build_id: i64) -> Result<BuildResponse, FactoryClientError> {
        // A 404 on this resource-specific endpoint means the build ID does not
        // exist; reclassify it so the `status` verb can report it as such. Other
        // endpoints leave a 404 as the underlying API error.
        Ok(self
            .factory
            .get_build_api_v1_factory_builds_build_id_get(build_id)
            .await
            .map_api_error()
            .await
            .map_err(not_found_on_404)?
            .into_inner())
    }

    async fn cancel_build(&self, build_id: i64) -> Result<BuildResponse, FactoryClientError> {
        // The slice's one destructive call. Every HTTP and transport outcome is
        // classified here so the verb can map each to a distinct exit code
        // without re-inspecting raw responses at the call site.
        Ok(self
            .factory
            .cancel_build_api_v1_factory_builds_build_id_delete(build_id)
            .await
            .map_api_error()
            .await
            .map_err(classify_build_error)?
            .into_inner())
    }

    async fn get_build_logs(&self, build_id: i64) -> Result<ByteStream, FactoryClientError> {
        // A 404 on this resource-specific endpoint means there are no logs to
        // serve — the build does not exist, was never dispatched, or its
        // coordinator counterpart is gone. Reclassify it so the caller can say
        // so. Other endpoints leave a 404 as the underlying API error.
        Ok(self
            .factory
            .get_build_logs_api_v1_factory_builds_build_id_logs_get(build_id)
            .await
            .map_api_error()
            .await
            .map_err(not_found_on_404)?
            .into_inner())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod tests {
    use std::collections::BTreeMap;

    use http::{HeaderMap, StatusCode};
    use httpmock::MockServer;
    use serde_json::json;

    use super::*;
    use crate::client::test_helpers::client_config;
    use crate::{AccessToken, AuthContext, FloxhubClientConfig, FloxhubMockMode};

    /// Build a bearer credential for tests. A personal access token is just a
    /// string, so no JWT construction is needed to test header handling.
    fn make_test_auth(secret: &str) -> AuthContext {
        AuthContext::AccessToken(AccessToken::new(secret.to_string()))
    }

    /// Exercise `list_builds` against a mock server, asserting:
    /// 1. The Authorization header carries the token supplied at construction.
    /// 2. The paginated response collapses into a `ResultsPage` correctly.
    #[tokio::test]
    async fn list_builds_sends_auth_header_and_deserialises_response() {
        let server = MockServer::start_async().await;

        let auth = make_test_auth("flox_pat_test");
        let expected_header = "bearer flox_pat_test";

        let mock = server.mock(|when, then| {
            when.method("GET")
                .path("/api/v1/factory/builds")
                .header("authorization", expected_header);
            then.status(200).json_body(json!({
                "builds": [],
                "page": 1,
                "page_size": 20,
                "total": 0
            }));
        });

        let config = FloxhubClientConfig {
            base_url: server.base_url(),
            auth_context: auth,
            ..client_config(&server.base_url())
        };
        let client = crate::FloxhubClient::new(config).unwrap();
        let result = client.list_builds(&BuildFilters::default()).await.unwrap();

        mock.assert();
        assert_eq!(result, ResultsPage {
            results: vec![],
            count: Some(0),
        });
    }

    /// Verify `list_builds` forwards each status as a repeated `status` query
    /// param; the server ORs them.
    #[tokio::test]
    async fn list_builds_forwards_status_filter() {
        let server = MockServer::start_async().await;

        let mock = server.mock(|when, then| {
            when.method("GET")
                .path("/api/v1/factory/builds")
                .query_param("status", "running")
                .query_param("status", "failed");
            then.status(200).json_body(json!({
                "builds": [],
                "page": 0,
                "page_size": 50,
                "total": 0
            }));
        });

        let config = FloxhubClientConfig {
            base_url: server.base_url(),
            ..client_config(&server.base_url())
        };
        let client = crate::FloxhubClient::new(config).unwrap();
        let filters = BuildFilters {
            status: vec![EffectiveBuildStatus::Running, EffectiveBuildStatus::Failed],
            ..Default::default()
        };
        let result = client.list_builds(&filters).await.unwrap();

        mock.assert();
        assert_eq!(result, ResultsPage {
            results: vec![],
            count: Some(0),
        });
    }

    /// Verify `list_builds` forwards the system, attr_path, source_commit_sha,
    /// and since filters as query params.
    #[tokio::test]
    async fn list_builds_forwards_remaining_filter_params() {
        let server = MockServer::start_async().await;

        let mock = server.mock(|when, then| {
            when.method("GET")
                .path("/api/v1/factory/builds")
                .query_param("system", "aarch64-darwin")
                .query_param("attr_path", "hello")
                .query_param("source_commit_sha", "abc123")
                .query_param("since", "7d");
            then.status(200).json_body(json!({
                "builds": [],
                "page": 0,
                "page_size": 50,
                "total": 0
            }));
        });

        let config = FloxhubClientConfig {
            base_url: server.base_url(),
            ..client_config(&server.base_url())
        };
        let client = crate::FloxhubClient::new(config).unwrap();
        let filters = BuildFilters {
            system: vec!["aarch64-darwin".parse().unwrap()],
            attr_path: vec!["hello".parse().unwrap()],
            source_commit_sha: vec!["abc123".parse().unwrap()],
            since: Some("7d".parse().unwrap()),
            ..Default::default()
        };
        let result = client.list_builds(&filters).await.unwrap();

        mock.assert();
        assert_eq!(result, ResultsPage {
            results: vec![],
            count: Some(0),
        });
    }

    /// The filters are re-sent on every page, not just the first. The depaging
    /// loop holds them across iterations, so a change that dropped them after
    /// page one would return unfiltered builds presented as matches: a wrong
    /// answer with no error to signal it.
    #[tokio::test]
    async fn list_builds_forwards_filters_on_every_page() {
        let server = MockServer::start_async().await;

        // A full page is what makes the loop ask for a second one, and the
        // total must exceed one page so it does not stop on the count check.
        let full_page: Vec<serde_json::Value> = (0..200)
            .map(|_| valid_build_json(EffectiveBuildStatus::Completed))
            .collect();

        let page_zero = server.mock(|when, then| {
            when.method("GET")
                .path("/api/v1/factory/builds")
                .query_param("page", "0")
                .query_param("status", "running")
                .query_param("system", "aarch64-darwin")
                .query_param("attr_path", "hello")
                .query_param("source_commit_sha", "abc123")
                .query_param("since", "7d");
            then.status(200).json_body(json!({
                "builds": full_page,
                "page": 0,
                "page_size": 200,
                "total": 201
            }));
        });

        let page_one = server.mock(|when, then| {
            when.method("GET")
                .path("/api/v1/factory/builds")
                .query_param("page", "1")
                .query_param("status", "running")
                .query_param("system", "aarch64-darwin")
                .query_param("attr_path", "hello")
                .query_param("source_commit_sha", "abc123")
                .query_param("since", "7d");
            then.status(200).json_body(json!({
                "builds": [valid_build_json(EffectiveBuildStatus::Completed)],
                "page": 1,
                "page_size": 200,
                "total": 201
            }));
        });

        let config = FloxhubClientConfig {
            base_url: server.base_url(),
            ..client_config(&server.base_url())
        };
        let client = crate::FloxhubClient::new(config).unwrap();
        let filters = BuildFilters {
            status: vec![EffectiveBuildStatus::Running],
            system: vec!["aarch64-darwin".parse().unwrap()],
            attr_path: vec!["hello".parse().unwrap()],
            source_commit_sha: vec!["abc123".parse().unwrap()],
            since: Some("7d".parse().unwrap()),
        };
        let result = client.list_builds(&filters).await.unwrap();

        // The second mock only matches a request carrying the filters, so both
        // assertions passing is what proves they survived the page boundary.
        page_zero.assert();
        page_one.assert();
        assert_eq!(result.count, Some(201));
        assert_eq!(result.results.len(), 201);
    }

    /// A default (empty) `BuildFilters` sends no filter query params, so the
    /// request is byte-identical to an unfiltered call.
    #[tokio::test]
    async fn list_builds_empty_filters_send_no_filter_params() {
        let server = MockServer::start_async().await;

        let mock = server.mock(|when, then| {
            when.method("GET")
                .path("/api/v1/factory/builds")
                .query_param_missing("status")
                .query_param_missing("system")
                .query_param_missing("attr_path")
                .query_param_missing("source_commit_sha")
                .query_param_missing("since");
            then.status(200).json_body(json!({
                "builds": [],
                "page": 0,
                "page_size": 50,
                "total": 0
            }));
        });

        let config = FloxhubClientConfig {
            base_url: server.base_url(),
            ..client_config(&server.base_url())
        };
        let client = crate::FloxhubClient::new(config).unwrap();
        let result = client.list_builds(&BuildFilters::default()).await.unwrap();

        mock.assert();
        assert_eq!(result, ResultsPage {
            results: vec![],
            count: Some(0),
        });
    }

    /// Verify extra headers are forwarded on factory requests.
    #[tokio::test]
    async fn extra_headers_set_on_all_requests() {
        let mut extra_headers: BTreeMap<String, String> = BTreeMap::new();
        extra_headers.insert("flox-test".to_string(), "test-value".to_string());
        extra_headers.insert("flox-test2".to_string(), "test-value2".to_string());

        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.header("flox-test", "test-value")
                .and(|when| when.header("flox-test2", "test-value2"));
            then.status(200).json_body(json!({
                "builds": [],
                "page": 1,
                "page_size": 20,
                "total": 0
            }));
        });

        let config = FloxhubClientConfig {
            extra_headers,
            ..client_config(&server.base_url())
        };

        let client = crate::FloxhubClient::new(config).unwrap();
        let _ = client.list_builds(&BuildFilters::default()).await;
        mock.assert();
    }

    // -------------------------------------------------------------------------
    // FactoryClientError / MapApiErrorExt tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn map_api_error_ok() {
        let expected = 1234u32;
        let result: Result<u32, APIError<ErrorResponse>> = Ok(expected);
        let mapped = result.map_api_error().await.unwrap();
        assert_eq!(mapped, expected);
    }

    #[tokio::test]
    async fn map_api_error_known_error_response() {
        let status = StatusCode::FORBIDDEN;
        let error_body = ErrorResponse {
            detail: "context specific message".to_string(),
        };

        let mut headers = HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        let resp_val = ResponseValue::new(error_body.clone(), status, headers);

        let result: Result<(), APIError<ErrorResponse>> = Err(APIError::ErrorResponse(resp_val));
        let err = result.map_api_error().await.unwrap_err();
        assert_eq!(err.to_string(), "403 Forbidden: context specific message");
    }

    #[tokio::test]
    async fn map_api_error_unexpected_response_parsed() {
        let status = StatusCode::FORBIDDEN;
        let body = serde_json::json!({
            "detail": "context specific message",
        });
        let resp = http::Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(body.to_string())
            .unwrap()
            .into();

        let result: Result<(), APIError<ErrorResponse>> = Err(APIError::UnexpectedResponse(resp));
        let err = result.map_api_error().await.unwrap_err();
        assert_eq!(err.to_string(), "403 Forbidden: context specific message");
    }

    #[tokio::test]
    async fn map_api_error_unexpected_response_unparsed_text() {
        let status = StatusCode::FORBIDDEN;
        let body = "not valid JSON";
        let resp = http::Response::builder()
            .status(status)
            .body(body.to_string())
            .unwrap()
            .into();

        let result: Result<(), APIError<ErrorResponse>> = Err(APIError::UnexpectedResponse(resp));
        let err = result.map_api_error().await.unwrap_err();
        assert_eq!(err.to_string(), "403 Forbidden");
    }

    #[tokio::test]
    async fn map_api_error_unexpected_response_unparsed_json() {
        let status = StatusCode::FORBIDDEN;
        let body = serde_json::json!({
            "something": "else",
        });
        let resp = http::Response::builder()
            .status(status)
            .body(body.to_string())
            .unwrap()
            .into();

        let result: Result<(), APIError<ErrorResponse>> = Err(APIError::UnexpectedResponse(resp));
        let err = result.map_api_error().await.unwrap_err();
        assert_eq!(err.to_string(), "403 Forbidden");
    }

    #[tokio::test]
    async fn map_api_error_does_not_collapse_404_error_response() {
        // The shared funnel no longer treats a 404 as a missing resource; only
        // resource-specific callers (get_build) do that. A 404 here stays an
        // APIError carrying the server detail.
        let status = StatusCode::NOT_FOUND;
        let error_body = ErrorResponse {
            detail: "Build not found".to_string(),
        };

        let mut headers = HeaderMap::new();
        headers.insert("content-type", "application/json".parse().unwrap());
        let resp_val = ResponseValue::new(error_body, status, headers);

        let result: Result<(), APIError<ErrorResponse>> = Err(APIError::ErrorResponse(resp_val));
        let err = result.map_api_error().await.unwrap_err();
        assert_eq!(err.to_string(), "404 Not Found: Build not found");
    }

    #[tokio::test]
    async fn map_api_error_does_not_collapse_404_unexpected_response() {
        let status = StatusCode::NOT_FOUND;
        let resp = http::Response::builder()
            .status(status)
            .body("Build not found".to_string())
            .unwrap()
            .into();

        let result: Result<(), APIError<ErrorResponse>> = Err(APIError::UnexpectedResponse(resp));
        let err = result.map_api_error().await.unwrap_err();
        assert_eq!(err.to_string(), "404 Not Found");
    }

    #[tokio::test]
    async fn map_api_error_communication_error_is_transport() {
        // A deterministic transport failure: port 0 is never a valid
        // connection target, so the request always fails at the transport
        // layer, with no reliance on a particular port being closed.
        let transport_err = reqwest::get("http://127.0.0.1:0").await.unwrap_err();

        let result: Result<(), APIError<ErrorResponse>> =
            Err(APIError::CommunicationError(transport_err));
        let err = result.map_api_error().await.unwrap_err();
        assert!(
            matches!(err, FactoryClientError::Transport(_)),
            "expected Transport, got {err:?}"
        );
        // The classification drops progenitor's URL-bearing transport text.
        assert_eq!(err.to_string(), "could not reach the Flox Factory");
    }

    #[tokio::test]
    async fn get_build_maps_404_to_not_found() {
        // On a resource-specific endpoint a 404 unambiguously means the build
        // does not exist, so get_build classifies it as NotFound.
        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.method("GET")
                .path(format!("/api/v1/factory/builds/{BUILD_ID}"));
            then.status(404)
                .json_body(json!({ "detail": "Build not found" }));
        });

        let config = FloxhubClientConfig {
            base_url: server.base_url(),
            ..client_config(&server.base_url())
        };
        let client = crate::FloxhubClient::new(config).unwrap();
        let err = client.get_build(BUILD_ID).await.unwrap_err();

        mock.assert();
        assert!(
            matches!(err, FactoryClientError::NotFound),
            "expected NotFound, got {err:?}"
        );
    }

    // -------------------------------------------------------------------------
    // cancel_build / classify_build_error
    //
    // Each test drives `cancel_build` against a mock and asserts the DELETE
    // outcome maps to the right `FactoryClientError` variant — the exhaustive
    // coverage of the classification the verb's exit-code space depends on.
    // -------------------------------------------------------------------------

    /// The build ID every single-build mock in this module serves.
    const BUILD_ID: i64 = 7;

    /// A complete `BuildResponse` body with the given effective `status`, built
    /// from the typed `BuildResponse` and serialized so the wire fixture fails
    /// loudly if a required field is added or changed, rather than silently
    /// omitting it and parsing a degenerate body.
    fn valid_build_json(status: EffectiveBuildStatus) -> serde_json::Value {
        serde_json::to_value(BuildResponse {
            attr_path: "hello".to_string(),
            build_id: BUILD_ID,
            build_type: "nixpkgs".to_string(),
            catalog_name: "my-catalog".to_string(),
            created_at: "2025-01-01T00:00:00Z".parse().unwrap(),
            exit_code: None,
            nixpkgs_revision: "deadbeef1234567890deadbeef1234567890dead".to_string(),
            source_commit_sha: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2".to_string(),
            source_repo_url: "https://github.com/example/repo".to_string(),
            status,
            system: "x86_64-linux".to_string(),
            task: None,
        })
        .expect("BuildResponse serializes")
    }

    /// Start a mock server returning `status`/`body` for the single-build DELETE,
    /// then call `cancel_build` and return its result.
    async fn cancel_build_against_mock(
        status: u16,
        body: serde_json::Value,
    ) -> Result<BuildResponse, FactoryClientError> {
        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.method("DELETE")
                .path(format!("/api/v1/factory/builds/{BUILD_ID}"));
            then.status(status).json_body(body);
        });

        let config = FloxhubClientConfig {
            base_url: server.base_url(),
            ..client_config(&server.base_url())
        };
        let client = crate::FloxhubClient::new(config).unwrap();
        let result = client.cancel_build(BUILD_ID).await;

        mock.assert();
        result
    }

    #[tokio::test]
    async fn cancel_build_returns_build_on_200() {
        // A satisfied cancel returns the build; the caller reads the effective
        // `status` to tell a fresh cancellation from an already-terminal build.
        let build =
            cancel_build_against_mock(200, valid_build_json(EffectiveBuildStatus::Cancelled))
                .await
                .unwrap();
        let expected = BuildResponse {
            attr_path: "hello".to_string(),
            build_id: BUILD_ID,
            build_type: "nixpkgs".to_string(),
            catalog_name: "my-catalog".to_string(),
            created_at: "2025-01-01T00:00:00Z".parse().unwrap(),
            exit_code: None,
            nixpkgs_revision: "deadbeef1234567890deadbeef1234567890dead".to_string(),
            source_commit_sha: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2".to_string(),
            source_repo_url: "https://github.com/example/repo".to_string(),
            status: EffectiveBuildStatus::Cancelled,
            system: "x86_64-linux".to_string(),
            task: None,
        };
        assert_eq!(build, expected);
    }

    #[tokio::test]
    async fn cancel_build_maps_404_to_not_found() {
        let err = cancel_build_against_mock(404, json!({ "detail": "Build not found" }))
            .await
            .unwrap_err();
        assert!(
            matches!(err, FactoryClientError::NotFound),
            "expected NotFound, got {err:?}"
        );
    }

    #[tokio::test]
    async fn cancel_build_maps_401_to_auth_rejected() {
        let err = cancel_build_against_mock(401, json!({ "detail": "Unauthorized" }))
            .await
            .unwrap_err();
        assert!(
            matches!(err, FactoryClientError::AuthRejected(_)),
            "expected AuthRejected, got {err:?}"
        );
    }

    #[tokio::test]
    async fn cancel_build_maps_403_to_auth_rejected() {
        let err = cancel_build_against_mock(403, json!({ "detail": "Forbidden" }))
            .await
            .unwrap_err();
        assert!(
            matches!(err, FactoryClientError::AuthRejected(_)),
            "expected AuthRejected, got {err:?}"
        );
    }

    #[tokio::test]
    async fn cancel_build_maps_502_to_server() {
        let err =
            cancel_build_against_mock(502, json!({ "detail": "Build Coordinator unreachable" }))
                .await
                .unwrap_err();
        assert!(
            matches!(err, FactoryClientError::Server(_)),
            "expected Server, got {err:?}"
        );
        // The typed variant still renders the server's own `detail`, so callers
        // like `flox factory status` keep the diagnostic context.
        assert!(
            err.to_string().contains("Build Coordinator unreachable"),
            "expected the server detail to survive, got {err}"
        );
    }

    #[tokio::test]
    async fn cancel_build_maps_422_to_server() {
        let err = cancel_build_against_mock(422, json!({ "detail": "Unprocessable" }))
            .await
            .unwrap_err();
        assert!(
            matches!(err, FactoryClientError::Server(_)),
            "expected Server, got {err:?}"
        );
    }

    #[tokio::test]
    async fn cancel_build_maps_200_wrong_shape_to_api_error() {
        // A 200 whose body is not a `BuildResponse` degrades to the generic API
        // error rather than escalating to a bespoke variant; the verb reports it
        // as a retryable service error.
        let err = cancel_build_against_mock(200, json!({ "unexpected": "shape" }))
            .await
            .unwrap_err();
        assert!(
            matches!(err, FactoryClientError::APIError(_)),
            "expected APIError, got {err:?}"
        );
    }

    #[tokio::test]
    async fn cancel_build_maps_connection_failure_to_transport() {
        // Port 0 is never a valid connection target, so the DELETE always fails
        // at the transport layer regardless of which ports are closed.
        let config = FloxhubClientConfig {
            base_url: "http://127.0.0.1:0".to_string(),
            ..client_config("http://127.0.0.1:0")
        };
        let client = crate::FloxhubClient::new(config).unwrap();
        let err = client.cancel_build(7).await.unwrap_err();
        assert!(
            matches!(err, FactoryClientError::Transport(_)),
            "expected Transport, got {err:?}"
        );
    }

    #[tokio::test]
    async fn get_build_logs_maps_404_to_not_found() {
        // The logs endpoint returns 404 when the build does not exist, was
        // never dispatched (no task), or the coordinator itself 404s. All three
        // mean "no logs available", so get_build_logs classifies the 404 as
        // NotFound and lets the verb render a cause-agnostic message.
        let server = MockServer::start_async().await;
        let mock = server.mock(|when, then| {
            when.method("GET").path("/api/v1/factory/builds/7/logs");
            then.status(404)
                .json_body(json!({ "detail": "Build not found" }));
        });

        let config = FloxhubClientConfig {
            base_url: server.base_url(),
            ..client_config(&server.base_url())
        };
        let client = crate::FloxhubClient::new(config).unwrap();
        // `ByteStream` is not `Debug`, so destructure rather than `unwrap_err`.
        let Err(err) = client.get_build_logs(7).await else {
            panic!("expected an error, got an Ok stream");
        };

        mock.assert();
        assert!(
            matches!(err, FactoryClientError::NotFound),
            "expected NotFound, got {err:?}"
        );
    }

    #[tokio::test]
    async fn list_builds_404_is_not_collapsed_to_not_found() {
        // A 404 on the listing endpoint is a route or configuration error, not
        // a missing resource, so it is left as the underlying API error rather
        // than reported as NotFound.
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method("GET").path("/api/v1/factory/builds");
            then.status(404).json_body(json!({ "detail": "Not Found" }));
        });

        let config = FloxhubClientConfig {
            base_url: server.base_url(),
            ..client_config(&server.base_url())
        };
        let client = crate::FloxhubClient::new(config).unwrap();
        let err = client
            .list_builds(&BuildFilters::default())
            .await
            .unwrap_err();

        assert!(
            matches!(err, FactoryClientError::APIError(_)),
            "expected APIError, got {err:?}"
        );
    }

    #[tokio::test]
    async fn map_api_error_other() {
        let msg = "something bad".to_string();
        let result: Result<(), APIError<ErrorResponse>> =
            Err(APIError::InvalidRequest(msg.clone()));

        let err = result.map_api_error().await.unwrap_err();
        assert_eq!(err.to_string(), "Invalid Request: something bad");
    }

    /// Verify that a `list_builds` call routes through the shared MockGuard
    /// in Replay mode — proving that the factory inner client respects
    /// `FloxhubMockMode::Replay` without extra wiring.
    #[tokio::test]
    async fn list_builds_routes_through_mock_guard_in_replay_mode() {
        // Use a temporary directory to write and replay a recording.
        let tmp = tempfile::tempdir().unwrap();
        let recording_path = tmp.path().join("factory_mock.json");

        // Record phase: start a real mock server, configure FloxhubClient
        // in Record mode, call list_builds, and let the guard write the file.
        {
            let server = MockServer::start_async().await;
            let _mock = server.mock(|when, then| {
                when.method("GET").path("/api/v1/factory/builds");
                then.status(200).json_body(json!({
                    "builds": [],
                    "page": 1,
                    "page_size": 20,
                    "total": 0
                }));
            });

            let config = FloxhubClientConfig {
                base_url: server.base_url(),
                mock_mode: FloxhubMockMode::Record(recording_path.clone()),
                ..client_config(&server.base_url())
            };
            let client = crate::FloxhubClient::new(config).unwrap();
            let result = client.list_builds(&BuildFilters::default()).await.unwrap();
            assert_eq!(result, ResultsPage {
                results: vec![],
                count: Some(0),
            });
            // Drop client to flush the recording.
        }

        // The recording file must exist before we replay.
        assert!(
            recording_path.exists(),
            "recording file not written: {:?}",
            recording_path
        );

        // Replay phase: FloxhubClient in Replay mode, no real server needed.
        {
            let config = FloxhubClientConfig {
                // Any URL; the mock guard intercepts the request.
                base_url: "http://localhost:0".to_string(),
                mock_mode: FloxhubMockMode::Replay(recording_path),
                ..client_config("http://localhost:0")
            };
            let client = crate::FloxhubClient::new(config).unwrap();
            let result = client.list_builds(&BuildFilters::default()).await.unwrap();
            assert_eq!(result, ResultsPage {
                results: vec![],
                count: Some(0),
            });
        }
    }
}
