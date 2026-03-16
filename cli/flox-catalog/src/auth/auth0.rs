//! Auth0 authentication strategy

use reqwest::header::{self, HeaderMap, HeaderValue};
use tracing::debug;

use super::{AuthError, AuthStrategy};
use crate::token::FloxhubToken;

/// Auth0 authentication strategy
///
/// Uses a bearer token from Auth0 (typically from FloxHub) for authentication.
/// The token is a JWT that contains the user's handle and expiration time.
#[derive(Debug, Clone)]
pub struct Auth0AuthStrategy {
    token: Option<FloxhubToken>,
}

impl Auth0AuthStrategy {
    pub fn new(token: Option<FloxhubToken>) -> Self {
        Self { token }
    }
}

impl AuthStrategy for Auth0AuthStrategy {
    fn add_auth_headers(&self, header_map: &mut HeaderMap) {
        let Some(ref token) = self.token else {
            return;
        };

        let auth_value = format!("bearer {}", token.secret());
        let Ok(value) = HeaderValue::from_str(&auth_value) else {
            tracing::warn!("Failed to create header value from Auth0 bearer token");
            return;
        };
        header_map.insert(header::AUTHORIZATION, value);
        debug!("Added Auth0 bearer token authorization header");
    }

    fn get_handle(&self) -> Result<String, AuthError> {
        let Some(ref token) = self.token else {
            return Err(AuthError::NotAuthenticated(
                "You are not logged in to FloxHub.\n\n\
                 To login you can either\n\
                 * login to FloxHub with 'flox auth login',\n\
                 * set the 'floxhub_token' field to '<your token>' in your config\n\
                 * set the '$FLOX_FLOXHUB_TOKEN=<your_token>' environment variable."
                    .to_string(),
            ));
        };

        if token.is_expired() {
            return Err(AuthError::Expired {
                handle: token.handle().to_string(),
                message: "Your FloxHub token has expired. To re-authenticate you can either:\n\n\
                 * login to FloxHub with 'flox auth login',\n\
                 * set the 'floxhub_token' field in your config to a fresh token,\n\
                 * set the '$FLOX_FLOXHUB_TOKEN' environment variable"
                    .to_string(),
            });
        }

        Ok(token.handle().to_string())
    }
}
