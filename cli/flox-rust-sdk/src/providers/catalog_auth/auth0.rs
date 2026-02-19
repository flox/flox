//! Auth0 authentication strategy

use reqwest::header::{self, HeaderMap, HeaderValue};
use tracing::debug;

use super::AuthStrategy;

/// Auth0 authentication strategy
///
/// Uses a bearer token from Auth0 (typically from FloxHub) for authentication.
pub struct Auth0AuthStrategy {
    auth0_token: Option<String>,
}

impl Auth0AuthStrategy {
    pub fn new(auth0_token: Option<String>) -> Self {
        Self { auth0_token }
    }
}

impl AuthStrategy for Auth0AuthStrategy {
    fn add_auth_headers(&self, header_map: &mut HeaderMap) {
        let Some(ref token) = self.auth0_token else {
            return;
        };

        let auth_value = format!("bearer {}", token);
        let Ok(value) = HeaderValue::from_str(&auth_value) else {
            tracing::warn!("Failed to create header value from Auth0 bearer token");
            return;
        };
        header_map.insert(header::AUTHORIZATION, value);
        debug!("Added Auth0 bearer token authorization header");
    }
}
