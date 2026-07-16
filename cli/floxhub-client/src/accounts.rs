//! The accounts surface of [`FloxhubClient`], hand-written.
//!
//! `GET /api/v1/accounts/me` reports who the presented credential belongs to
//! and when it expires. This module provides the production
//! [`identity_resolver`] injected into personal access tokens, keeping the
//! [`crate::auth`] module free of transport concerns.
//!
//! TODO: generate this client from the accounts service's OpenAPI schema
//! (the service emits a 3.0.2 schema for exactly this purpose) and replace
//! the hand-written request, once the CLI grows beyond this one endpoint.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::auth::{IdentityError, UserIdentity};
use crate::client::build_http_client;

#[derive(Debug, Error)]
pub enum MeError {
    #[error("token is invalid, expired, or revoked")]
    Unauthorized,
    #[error("unexpected response from FloxHub ({0})")]
    UnexpectedStatus(reqwest::StatusCode),
    #[error("could not reach FloxHub")]
    Request(#[from] reqwest::Error),
    #[error("could not build HTTP client: {0}")]
    Client(String),
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
    /// Build a standalone client with its own connection pool.
    pub fn new(base_url: &str) -> Result<Self, MeError> {
        let http_client =
            build_http_client(&BTreeMap::new(), None, base_url).map_err(MeError::Client)?;
        Ok(Self::new_with_client(base_url, http_client))
    }

    /// Build with an existing reqwest client; clones share its pool.
    pub(crate) fn new_with_client(base_url: &str, http_client: reqwest::Client) -> Self {
        Self {
            base_url: base_url.to_string(),
            http_client,
        }
    }

    /// Fetch the identity behind `token` from `GET /api/v1/accounts/me`.
    ///
    /// The credential being verified is an explicit input rather than
    /// ambient client state, so this works without any configured auth.
    pub async fn me(&self, token: &str) -> Result<UserIdentity, MeError> {
        let url = format!("{}/api/v1/accounts/me", self.base_url.trim_end_matches('/'));
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

/// The production identity resolution for
/// [`AuthContext::from_mode`](crate::auth::AuthContext::from_mode): fetch
/// the identity from `{api_url}/api/v1/accounts/me`, blocking the calling
/// thread.
///
/// Constructs a standalone [`AccountsApiClient`] on first use — the shared
/// [`FloxhubClient`](crate::FloxhubClient) the CLI holds is itself built
/// *from* the auth context, so it cannot be captured here.
pub fn identity_resolver(
    api_url: &str,
) -> impl FnOnce(String) -> Result<UserIdentity, IdentityError> + Send + Sync + 'static {
    let api_url = api_url.to_string();
    move |token| {
        fetch_me_blocking(&api_url, &token).map_err(|err| match err {
            MeError::Unauthorized => IdentityError::Unauthorized,
            other => IdentityError::Other(other.to_string()),
        })
    }
}

/// Fetch the identity behind `token` from `/me`, blocking the calling thread.
///
/// The request runs on its own thread with its own runtime so that it can
/// block safely from both sync and async (tokio) callers.
pub fn fetch_me_blocking(api_url: &str, token: &str) -> Result<UserIdentity, MeError> {
    let client = AccountsApiClient::new(api_url)?;
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build runtime for /me request")
                    .block_on(client.me(token))
            })
            .join()
            .expect("/me request thread panicked")
    })
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
                .path("/api/v1/accounts/me")
                .header("authorization", "bearer flox_pat_secret");
            then.status(200).json_body(serde_json::json!({
                "user_id": "auth0|123",
                "handle": "testuser",
                "expires_at": "2027-01-01T00:00:00Z",
            }));
        });

        let client = AccountsApiClient::new(&server.base_url()).unwrap();
        let identity = client.me("flox_pat_secret").await.unwrap();

        mock.assert();
        assert_eq!(identity, UserIdentity {
            user_id: "auth0|123".to_string(),
            handle: "testuser".to_string(),
            expires_at: Some("2027-01-01T00:00:00Z".parse().unwrap()),
        });
    }

    #[test]
    fn identity_resolver_maps_401_to_unauthorized() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/api/v1/accounts/me");
            then.status(401)
                .json_body(serde_json::json!({"detail": "unauthorized"}));
        });

        let resolve = identity_resolver(&server.base_url());
        let err = resolve("flox_pat_bad".to_string()).unwrap_err();

        assert!(matches!(err, IdentityError::Unauthorized));
    }

    #[test]
    fn identity_resolver_maps_other_failures_to_other() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/api/v1/accounts/me");
            then.status(500);
        });

        let resolve = identity_resolver(&server.base_url());
        let err = resolve("flox_pat_secret".to_string()).unwrap_err();

        assert!(matches!(err, IdentityError::Other(_)));
    }
}
