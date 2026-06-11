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

use factory_api_v1::types::{BuildResponse, ErrorResponse};
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

        if let APIError::UnexpectedResponse(resp) = err {
            return parse_api_error(resp).await;
        }

        Err(FactoryClientError::APIError(err))
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
    /// Return all builds across pages, optionally filtered by status.
    async fn list_builds(
        &self,
        status: Option<&str>,
    ) -> Result<ResultsPage<BuildResponse>, FactoryClientError>;

    /// Fetch a single build by its numeric ID.
    async fn get_build(&self, build_id: i64) -> Result<BuildResponse, FactoryClientError>;

    /// Proxy the raw log stream for a build.
    async fn get_build_logs(&self, build_id: i64) -> Result<ByteStream, FactoryClientError>;
}

// ---------------------------------------------------------------------------
// FactoryClientTrait implementation for FloxhubClient
// ---------------------------------------------------------------------------

impl FactoryClientTrait for crate::FloxhubClient {
    async fn list_builds(
        &self,
        status: Option<&str>,
    ) -> Result<ResultsPage<BuildResponse>, FactoryClientError> {
        let stream = make_depaging_stream(
            |page_number, page_size| async move {
                let response = self
                    .factory
                    .list_builds_api_v1_factory_builds_get(
                        Some(page_number as u64),
                        NonZeroU64::new(page_size as u64),
                        status,
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
        Ok(self
            .factory
            .get_build_api_v1_factory_builds_build_id_get(build_id)
            .await
            .map_api_error()
            .await?
            .into_inner())
    }

    async fn get_build_logs(&self, build_id: i64) -> Result<ByteStream, FactoryClientError> {
        Ok(self
            .factory
            .get_build_logs_api_v1_factory_builds_build_id_logs_get(build_id)
            .await
            .map_api_error()
            .await?
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
    use crate::{AuthContext, FloxhubClientConfig, FloxhubMockMode, FloxhubToken};

    /// Build a fake JWT for use in tests.
    pub fn make_test_token(handle: &str) -> FloxhubToken {
        use std::str::FromStr;

        let claims = serde_json::json!({
            "https://flox.dev/handle": handle,
            "exp": 9999999999_i64
        });
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret("secret".as_ref()),
        )
        .unwrap();
        FloxhubToken::from_str(&token).unwrap()
    }

    /// Exercise `list_builds` against a mock server, asserting:
    /// 1. The Authorization header carries the token supplied at construction.
    /// 2. The paginated response collapses into a `ResultsPage` correctly.
    #[tokio::test]
    async fn list_builds_sends_auth_header_and_deserialises_response() {
        let server = MockServer::start_async().await;

        let token = make_test_token("testuser");
        let expected_header = format!("bearer {}", token.secret());
        let auth = AuthContext::Auth0(Some(token));

        let mock = server.mock(|when, then| {
            when.method("GET")
                .path("/api/v1/factory/builds")
                .header("authorization", &expected_header);
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
        let result = client.list_builds(None).await.unwrap();

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
        let _ = client.list_builds(None).await;
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
            let result = client.list_builds(None).await.unwrap();
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
            let result = client.list_builds(None).await.unwrap();
            assert_eq!(result, ResultsPage {
                results: vec![],
                count: Some(0),
            });
        }
    }
}
