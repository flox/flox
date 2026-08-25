//! [`FloxhubToken`] — the Auth0-shaped bearer credential: a JWT whose
//! claims carry the FloxHub handle and expiry. The token is decoded
//! (without signature verification) at construction time so that identity
//! and expiry are answered locally, with no request to FloxHub.
//!
//! A JWT without these claims is not an error — it is a [`BareToken`],
//! whose identity resolves from `/me` instead; [`AuthContext::new_from_token`]
//! routes between the two.
//!
//! [`BareToken`]: crate::auth::token::BareToken
//! [`AuthContext::new_from_token`]: crate::auth::AuthContext::new_from_token

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_with::DeserializeFromStr;

use crate::auth::token::InvalidTokenError;

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

/// A token authenticating a user with FloxHub, identifying its owner
/// locally through the Auth0 tenant's handle claim.
#[derive(Debug, Clone, DeserializeFromStr)]
pub struct FloxhubToken {
    /// The entire token as a string
    token: String,
    /// Assertions about the identity of the token's owner
    token_data: FloxTokenClaims,
}

impl FloxhubToken {
    /// Decode an Auth0-shaped JWT: the handle claim and `exp` are
    /// required. Errors for anything else — which may still be a valid
    /// credential of another kind (bare or opaque); routing is
    /// `AuthContext::new_from_token`'s job, not the caller's.
    pub fn new(token: String) -> Result<Self, InvalidTokenError> {
        // Client side we don't need to verify the signature,
        // as all privileged access is guarded server side.
        // We still decode the token to extract claims like handle and expiration.
        let decoded = jsonwebtoken::dangerous::insecure_decode::<FloxTokenClaims>(&token)
            .map_err(InvalidTokenError)?;

        Ok(FloxhubToken {
            token,
            token_data: decoded.claims,
        })
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

    /// The wall-clock expiry of the token, from the `exp` claim.
    pub fn expires_at(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(self.token_data.exp as i64, 0)
            .expect("the exp claim is a valid unix timestamp")
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
    type Err = InvalidTokenError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        FloxhubToken::new(s.to_string())
    }
}

/// Test fixtures for [FloxhubToken].
///
/// Nothing here should be used in production code.
#[cfg(any(test, feature = "tests"))]
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

#[cfg(test)]
mod tests {
    use super::test_helpers::*;
    use super::*;
    use crate::auth::token::test_helpers::FAKE_TOKEN_NO_HANDLE;

    #[test]
    fn auth0_shaped_jwt_answers_identity_locally() {
        let token = FloxhubToken::new(FAKE_TOKEN.to_string()).expect("token parses");
        assert_eq!(token.handle(), "test");
        assert!(!token.is_expired());
    }

    /// A JWT without the handle claim is not an invalid FloxhubToken so
    /// much as a different kind of credential — routing sends it to
    /// [`BareToken`](crate::auth::token::BareToken).
    #[test]
    fn jwt_without_handle_claim_is_rejected() {
        FloxhubToken::new(FAKE_TOKEN_NO_HANDLE.to_string()).unwrap_err();
    }

    #[test]
    fn non_jwt_is_rejected() {
        FloxhubToken::new("not-a-jwt".to_string()).unwrap_err();
    }

    #[test]
    fn expired_jwt_reports_expiry() {
        let token = FloxhubToken::new(FAKE_EXPIRED_TOKEN.to_string()).expect("token parses");
        assert!(token.is_expired());
    }
}
