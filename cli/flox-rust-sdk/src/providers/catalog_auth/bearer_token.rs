//! Bearer token authentication strategy

use reqwest::header::{self, HeaderMap, HeaderValue};
use tracing::debug;

use super::AuthStrategy;
use crate::providers::catalog::CatalogClientConfig;

/// Bearer token authentication strategy
///
/// Uses a bearer token (typically from FloxHub) for authentication.
pub struct BearerTokenAuthStrategy;

impl AuthStrategy for BearerTokenAuthStrategy {
    fn add_auth_headers(header_map: &mut HeaderMap, config: &CatalogClientConfig) {
        let Some(token) = &config.floxhub_token else {
            return;
        };

        let auth_value = format!("bearer {}", token);
        let Ok(value) = HeaderValue::from_str(&auth_value) else {
            tracing::warn!("Failed to create header value from bearer token");
            return;
        };
        header_map.insert(header::AUTHORIZATION, value);
        debug!("Added bearer token authorization header");
    }
}
