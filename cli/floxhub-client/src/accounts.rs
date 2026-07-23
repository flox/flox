//! The accounts surface of [`FloxhubClient`](crate::FloxhubClient),
//! hand-written.
//!
//! `GET /api/v1/accounts/me` reports who the presented credential belongs to
//! and when it expires.
//!
//! TODO: generate this client from the accounts service's OpenAPI schema
//! (the service emits a 3.0.2 schema for exactly this purpose) and replace
//! the hand-written request, once the CLI grows beyond this one endpoint.

use thiserror::Error;

use crate::auth::UserIdentity;

#[derive(Debug, Error)]
pub enum MeError {
    #[error("token is invalid, expired, or revoked")]
    Unauthorized,
    #[error("unexpected response from FloxHub ({0})")]
    UnexpectedStatus(reqwest::StatusCode),
    #[error("could not reach FloxHub")]
    Request(#[from] reqwest::Error),
}

/// Accounts inner client, mirroring the generated `CatalogApiClient` and
/// `FactoryApiClient` shape so the eventual generated client is a drop-in
/// replacement.
#[derive(Debug, Clone)]
pub struct AccountsApiClient {
    base_url: String,
    http_client: reqwest::Client,
}

impl AccountsApiClient {
    /// Build with an existing reqwest client; clones share its pool.
    pub(crate) fn new_with_client(base_url: &str, http_client: reqwest::Client) -> Self {
        Self {
            base_url: base_url.to_string(),
            http_client,
        }
    }

    /// Fetch the identity behind `token` from the accounts service's
    /// `GET /api/v1/accounts/me`.
    ///
    /// The public API gateway exposes the accounts service under the
    /// `/accounts` prefix and forwards the service's native path, so the
    /// public URL is `{base}/accounts/api/v1/accounts/me`.
    ///
    /// The credential being verified is an explicit input rather than
    /// ambient client state, so this works without any configured auth.
    pub async fn me(&self, token: &str) -> Result<UserIdentity, MeError> {
        let url = format!(
            "{}/accounts/api/v1/accounts/me",
            self.base_url.trim_end_matches('/')
        );
        let response = self
            .http_client
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
}

#[cfg(test)]
mod tests {
    use httpmock::MockServer;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn me_parses_identity_with_expiry() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/accounts/api/v1/accounts/me")
                .header("authorization", "bearer flox_pat_secret");
            then.status(200).json_body(serde_json::json!({
                "user_id": "auth0|123",
                "handle": "testuser",
                "expires_at": "2027-01-01T00:00:00Z",
            }));
        });

        let client = AccountsApiClient::new_with_client(&server.base_url(), reqwest::Client::new());
        let identity = client.me("flox_pat_secret").await.unwrap();

        mock.assert();
        assert_eq!(identity, UserIdentity {
            handle: "testuser".to_string(),
            expires_at: Some("2027-01-01T00:00:00Z".parse().unwrap()),
        });
    }
}
