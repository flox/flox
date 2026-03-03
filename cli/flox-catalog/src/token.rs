//! FloxHub authentication token
//!
//! Provides [`FloxhubToken`] — a parsed JWT that authenticates a user with
//! FloxHub.  The token is decoded (without signature verification) at
//! construction time so that the handle and expiration are available cheaply.

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_with::DeserializeFromStr;
use thiserror::Error;

/// Assertions about the owner of this token
#[derive(Debug, Clone, Deserialize)]
struct FloxTokenClaims {
    /// The FloxHub handle of the user this token belongs to
    #[serde(rename = "https://flox.dev/handle")]
    handle: String,
    /// The expiration time of the token (Unix timestamp)
    exp: usize,
}

/// A token authenticating a user with FloxHub
#[derive(Debug, Clone, DeserializeFromStr)]
pub struct FloxhubToken {
    /// The entire token as a string
    token: String,
    /// Assertions about the identity of the token's owner
    token_data: FloxTokenClaims,
}

impl FloxhubToken {
    /// Create a new floxhub token from a string
    pub fn new(token: String) -> Result<Self, FloxhubTokenError> {
        token.parse()
    }

    /// Return the token as a string
    pub fn secret(&self) -> &str {
        &self.token
    }

    /// Return the handle of the user the token belongs to
    pub fn handle(&self) -> &str {
        &self.token_data.handle
    }

    /// Returns whether the token has expired by checking the `exp` claim
    /// against the current time.
    pub fn is_expired(&self) -> bool {
        let now = {
            let start = std::time::SystemTime::now();
            start
                .duration_since(std::time::UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs() as usize
        };
        self.token_data.exp < now
    }
}

impl Serialize for FloxhubToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.token.serialize(serializer)
    }
}

impl FromStr for FloxhubToken {
    type Err = FloxhubTokenError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Client side we don't need to verify the signature,
        // as all privileged access is guarded server side.
        // We still decode the token to extract claims like handle and expiration.

        let token = jsonwebtoken::dangerous::insecure_decode::<FloxTokenClaims>(s)
            .map_err(FloxhubTokenError::InvalidToken)?;

        Ok(FloxhubToken {
            token: s.to_string(),
            token_data: token.claims,
        })
    }
}

#[derive(Debug, Clone, Error, Eq, PartialEq)]
pub enum FloxhubTokenError {
    #[error("invalid token")]
    InvalidToken(#[source] jsonwebtoken::errors::Error),
}
