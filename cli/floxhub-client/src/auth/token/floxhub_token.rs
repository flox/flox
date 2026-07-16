//! [`FloxhubToken`] — a parsed Auth0 JWT that authenticates a user with
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
    /// The OIDC subject identifier — an opaque, pseudonymous id
    /// (e.g. `github|3670948`) stable across the user's lifetime.
    /// Declaring the claim here means a non-string `sub` fails the whole
    /// token parse — deliberate, since OIDC requires `sub` to be a string.
    sub: Option<String>,
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

    /// Return the OIDC `sub` claim, if present and non-empty.
    ///
    /// An opaque, pseudonymous subject identifier (`github|3670948`,
    /// `auth0|…`) — never the handle, email, or display name. Used for
    /// telemetry attribution; stable across the user's lifetime, so it
    /// remains meaningful even when the token has expired.
    pub fn sub(&self) -> Option<&str> {
        self.token_data.sub.as_deref().filter(|s| !s.is_empty())
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

/// Test fixtures for [FloxhubToken].
///
/// Intentionally not behind `#[cfg(test)]` so that other crates' (also
/// non-gated) test helpers can use them without enabling a feature.
/// Nothing here should be used in production code.
pub mod test_helpers {
    /// A fake FloxHub token
    ///
    /// {
    ///  "typ": "JWT",
    ///  "alg": "HS256"
    /// }
    /// .
    /// {
    ///   "https://flox.dev/handle": "test"
    ///   "exp": 9999999999,                // 2286-11-20T17:46:39+00:00
    /// }
    /// .
    /// AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
    pub const FAKE_TOKEN: &str = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjk5OTk5OTk5OTl9.6-nbzFzQEjEX7dfWZFLE-I_qW2N_-9W2HFzzfsquI74";

    /// A fake floxhub token, that is expired
    ///
    /// {
    ///  "typ": "JWT",
    ///  "alg": "HS256"
    /// }
    /// .
    /// {
    ///   "https://flox.dev/handle": "test"
    ///   "exp": 1704063600,                // 2024-01-01T00:00:00+00:00
    /// }
    /// .
    /// AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
    pub const FAKE_EXPIRED_TOKEN: &str = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjE3MDQwNjM2MDB9.-5VCofPtmYQuvh21EV1nEJhTFV_URkRP0WFu4QDPFxY";

    /// A fake FloxHub token carrying an OIDC `sub` claim, plus the PII
    /// claims a real token also carries (which [`FloxhubToken::sub`]
    /// must never return)
    ///
    /// {
    ///  "typ": "JWT",
    ///  "alg": "HS256"
    /// }
    /// .
    /// {
    ///   "https://flox.dev/handle": "test",
    ///   "exp": 9999999999,                // 2286-11-20T17:46:39+00:00
    ///   "sub": "github|424242",
    ///   "email": "test@example.com",
    ///   "name": "Test User"
    /// }
    /// .
    /// AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
    pub const FAKE_TOKEN_WITH_SUB: &str = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjk5OTk5OTk5OTksInN1YiI6ImdpdGh1Ynw0MjQyNDIiLCJlbWFpbCI6InRlc3RAZXhhbXBsZS5jb20iLCJuYW1lIjoiVGVzdCBVc2VyIn0.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    /// An expired fake FloxHub token carrying an OIDC `sub` claim
    ///
    /// {
    ///  "typ": "JWT",
    ///  "alg": "HS256"
    /// }
    /// .
    /// {
    ///   "https://flox.dev/handle": "test",
    ///   "exp": 1704063600,                // 2024-01-01T00:00:00+00:00
    ///   "sub": "github|424242"
    /// }
    /// .
    /// AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
    pub const FAKE_EXPIRED_TOKEN_WITH_SUB: &str = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjE3MDQwNjM2MDAsInN1YiI6ImdpdGh1Ynw0MjQyNDIifQ.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
}
