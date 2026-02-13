//! Catalog client wrapper around the auto-generated API client.

use std::fmt::Debug;
use std::str::FromStr;
use std::time::Duration;

use catalog_api_v1::Client as APIClient;
use reqwest::header::{self, HeaderMap};
use tracing::debug;

use crate::config::CatalogClientConfig;
use crate::error::CatalogClientError;
#[cfg(feature = "mock")]
use crate::mock::MockGuard;

/// A client for the catalog service.
///
/// This is a wrapper around the auto-generated APIClient that handles:
/// - HTTP client configuration with timeouts
/// - Bearer token authentication for FloxHub
/// - Mock server recording/replay for testing (feature-gated)
pub struct CatalogClient {
    client: APIClient,
    config: CatalogClientConfig,
    #[cfg(feature = "mock")]
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
        #[cfg(feature = "mock")]
        let mock_guard = MockGuard::new(&config);
        #[cfg(feature = "mock")]
        let effective_url = mock_guard
            .as_ref()
            .map(|m| m.url())
            .unwrap_or_else(|| config.catalog_url.clone());
        #[cfg(not(feature = "mock"))]
        let effective_url = config.catalog_url.clone();

        let http_client = build_http_client(&config)?;
        let client = APIClient::new_with_client(&effective_url, http_client);

        Ok(Self {
            client,
            config,
            #[cfg(feature = "mock")]
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

    reqwest::Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| CatalogClientError::Other(e.to_string()))
}
