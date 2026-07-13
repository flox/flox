//! Hand-written accounts API requests.
//!
//! `GET /api/v1/accounts/me` reports who the presented credential belongs to
//! and when it expires. Hand-written per the service-token spec; revisit
//! OpenAPI codegen when the CLI needs more accounts endpoints.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use thiserror::Error;

/// Identity of the authenticated caller as reported by
/// `GET /api/v1/accounts/me`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct UserIdentity {
    pub user_id: String,
    pub handle: String,
    /// Wall-clock expiry of the presenting credential;
    /// `None` when it never expires.
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Error)]
pub enum MeError {
    #[error("token is invalid, expired, or revoked")]
    Unauthorized,
    #[error("unexpected response from FloxHub ({0})")]
    UnexpectedStatus(reqwest::StatusCode),
    #[error("could not reach FloxHub")]
    Request(#[from] reqwest::Error),
}

/// Fetch the identity behind `token` from `{api_url}/api/v1/accounts/me`.
///
/// Uses a one-off HTTP client: this runs at most once per process (the result
/// is cached by [`crate::PersonalAccessToken`]), and the shared FloxHub API
/// client is constructed *from* the auth context, so it cannot be used here.
pub async fn fetch_me(api_url: &str, token: &str) -> Result<UserIdentity, MeError> {
    let url = format!("{}/api/v1/accounts/me", api_url.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .get(url)
        .header(reqwest::header::AUTHORIZATION, format!("bearer {token}"))
        .send()
        .await?;
    match response.status() {
        reqwest::StatusCode::OK => Ok(response.json().await?),
        reqwest::StatusCode::UNAUTHORIZED => Err(MeError::Unauthorized),
        status => Err(MeError::UnexpectedStatus(status)),
    }
}

#[cfg(test)]
mod tests {
    use httpmock::MockServer;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn fetch_me_parses_identity_with_expiry() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/api/v1/accounts/me")
                .header("authorization", "bearer flox_pat_secret");
            then.status(200).json_body(serde_json::json!({
                "user_id": "auth0|123",
                "handle": "testuser",
                "expires_at": "2027-01-01T00:00:00Z",
            }));
        });

        let identity = fetch_me(&server.base_url(), "flox_pat_secret")
            .await
            .unwrap();

        mock.assert();
        assert_eq!(identity, UserIdentity {
            user_id: "auth0|123".to_string(),
            handle: "testuser".to_string(),
            expires_at: Some("2027-01-01T00:00:00Z".parse().unwrap()),
        });
    }
}
