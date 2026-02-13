//! Error handling for catalog API operations.

use catalog_api_v1::types::{self as api_types, error as api_error};
use catalog_api_v1::{Error as APIError, ResponseValue};
use thiserror::Error;

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
