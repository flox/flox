//! Error handling for catalog API operations.

use catalog_api_v1::types::{self as api_types, error as api_error};
use catalog_api_v1::{Error as APIError, ResponseValue};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Operation-specific errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("invalid search term")]
    InvalidSearchTerm(#[source] api_error::ConversionError),
    #[error("catalog error")]
    CatalogClientError(#[from] CatalogClientError),
}

#[derive(Debug, Error)]
pub enum ResolveError {
    #[error("catalog error")]
    CatalogClientError(#[from] CatalogClientError),
}

#[derive(Debug, Error)]
pub enum VersionsError {
    #[error("catalog error")]
    CatalogClientError(#[from] CatalogClientError),
    #[error("package not found")]
    NotFound,
}

#[derive(Debug, Error)]
pub enum PublishError {
    #[error("catalog error")]
    CatalogClientError(#[from] CatalogClientError),
    #[error("catalog does not have a store configured")]
    UnconfiguredCatalog,
}

/// Alias to type representing expected errors that are in the API spec.
pub type ApiErrorResponse = api_types::ErrorResponse;
pub type ApiErrorResponseValue = ResponseValue<ApiErrorResponse>;

/// Common error type for catalog API operations.
///
/// This error type wraps errors from the generated `catalog-api-v1` crate.
/// SDK-specific operation errors (ResolveError, SearchError, etc.) wrap this type.
#[derive(Debug, Error)]
pub enum CatalogClientError {
    #[error("system not supported by catalog")]
    UnsupportedSystem(#[source] api_error::ConversionError),
    #[error("{}", fmt_api_error(.0))]
    APIError(APIError<api_types::ErrorResponse>),
    #[error("{}", .0)]
    StabilityError(String),
    #[error("{}", .0)]
    Other(String),
}

/// Extension trait for converting API errors into client errors.
pub trait MapApiErrorExt<T> {
    /// Consumes a `Result<T, APIError<ApiErrorResponse>>`, maps any APIError
    /// into `CatalogClientError`, and returns `Ok(T)` or `Err(...)`.
    fn map_api_error(
        self,
    ) -> impl std::future::Future<Output = Result<T, CatalogClientError>> + Send;
}

impl<T: Send> MapApiErrorExt<T> for Result<T, APIError<ApiErrorResponse>> {
    async fn map_api_error(self) -> Result<T, CatalogClientError> {
        let err = match self {
            Ok(v) => return Ok(v),
            Err(err) => err,
        };

        // Attempt to parse errors that don't have status code enumerated in the
        // spec but still contain a `detail` field.
        if let APIError::UnexpectedResponse(resp) = err {
            return parse_api_error(resp).await;
        }

        Err(CatalogClientError::APIError(err))
    }
}

async fn parse_api_error<T>(resp: reqwest::Response) -> Result<T, CatalogClientError> {
    let status = resp.status();
    match ApiErrorResponseValue::from_response::<api_types::ErrorResponse>(resp).await {
        Ok(resp_parsed) => Err(CatalogClientError::APIError(APIError::ErrorResponse(
            resp_parsed,
        ))),
        Err(_) => {
            // We couldn't parse but consumed the response body, which we don't
            // format anyway because it may contain HTML garbage, so recreate a
            // response with the right status.
            let resp_bare = http::Response::builder()
                .status(status)
                .body("response body omitted by error parsing")
                .expect("failed to rebuild response while parsing error response")
                .into();
            Err(CatalogClientError::APIError(APIError::UnexpectedResponse(
                resp_bare,
            )))
        },
    }
}

fn fmt_api_error(api_error: &APIError<api_types::ErrorResponse>) -> String {
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

#[cfg(test)]
mod tests {
    use catalog_api_v1::types::ErrorResponse;
    use http::{HeaderMap, StatusCode};
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn map_api_error_ok() {
        let expected = 1234;
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
        let body = json!({
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
        let body = json!({
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
}
