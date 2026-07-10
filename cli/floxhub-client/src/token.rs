//! FloxHub authentication tokens
//!
//! Provides [`FloxhubToken`] — a parsed JWT that authenticates a user with
//! FloxHub.  The token is decoded (without signature verification) at
//! construction time so that the handle and expiration are available cheaply.
//!
//! Also provides [`PersonalAccessToken`] — a `flox_pat_` personal access token whose
//! identity is resolved lazily via `/me` and cached in memory.  A token
//! string is routed to one of the two forms by
//! [`AuthContext::from_mode`](crate::AuthContext::from_mode).

use std::str::FromStr;
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use serde_with::DeserializeFromStr;
use thiserror::Error;

use crate::accounts::{self, MeError, UserIdentity};

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

    /// A fake FloxHub token whose `sub` claim is present but empty
    ///
    /// {
    ///  "typ": "JWT",
    ///  "alg": "HS256"
    /// }
    /// .
    /// {
    ///   "https://flox.dev/handle": "test",
    ///   "exp": 9999999999,                // 2286-11-20T17:46:39+00:00
    ///   "sub": ""
    /// }
    /// .
    /// AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
    pub const FAKE_TOKEN_WITH_EMPTY_SUB: &str = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjk5OTk5OTk5OTksInN1YiI6IiJ9.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::test_helpers::{FAKE_TOKEN, FAKE_TOKEN_WITH_EMPTY_SUB, FAKE_TOKEN_WITH_SUB};
    use super::*;

    /// The accessor returns exactly the `sub` claim — never the handle,
    /// email, or display name the same payload carries.
    #[test]
    fn sub_returns_only_the_sub_claim() {
        let token = FloxhubToken::from_str(FAKE_TOKEN_WITH_SUB).expect("token parses");
        let sub = token.sub().expect("sub present");
        assert_eq!(sub, "github|424242");
        assert!(!sub.contains('@'), "must never be an email");
        assert_ne!(sub, token.handle(), "must never be the handle");
    }

    /// A present-but-empty `sub` normalizes to `None` — an empty
    /// `auth_subject` must never reach the wire.
    #[test]
    fn sub_is_none_when_claim_empty() {
        let token = FloxhubToken::from_str(FAKE_TOKEN_WITH_EMPTY_SUB).expect("token parses");
        assert_eq!(token.sub(), None);
    }

    /// Tokens predating the `sub` claim still parse; the accessor just
    /// returns `None`.
    #[test]
    fn sub_is_none_when_claim_absent() {
        let token = FloxhubToken::from_str(FAKE_TOKEN).expect("token parses");
        assert_eq!(token.sub(), None);
    }
}

/// Prefix identifying a FloxHub personal access token.
pub const PAT_PREFIX: &str = "flox_pat_";

/// An opaque token (`flox_pat_…` personal access token) authenticating a user
/// with FloxHub.
///
/// The CLI cannot decode identity from an opaque token. Identity (`handle`,
/// `expires_at`) is fetched lazily from `GET {api_url}/api/v1/accounts/me` by
/// [`Self::resolve_identity`] and cached in memory for the lifetime of the
/// process; clones share the cache. Until resolution succeeds, [`Self::handle`]
/// returns `None` and [`Self::is_expired`] returns `false` — the server's 401
/// is the authoritative backstop.
#[derive(Clone)]
pub struct PersonalAccessToken {
    /// The entire token as a string.
    token: String,
    /// Base URL of the FloxHub API this token authenticates against.
    api_url: String,
    /// Identity resolved from `/me`; shared across clones.
    identity: Arc<OnceLock<UserIdentity>>,
}

impl PersonalAccessToken {
    /// Create an opaque token from a string; nothing is parsed or validated.
    pub fn new(token: String, api_url: String) -> Self {
        PersonalAccessToken {
            token,
            api_url,
            identity: Arc::new(OnceLock::new()),
        }
    }

    /// Return the token as a string.
    pub fn secret(&self) -> &str {
        &self.token
    }

    /// Return the cached handle; `None` until [`Self::resolve_identity`]
    /// has succeeded.
    pub fn handle(&self) -> Option<&str> {
        self.identity.get().map(|identity| identity.handle.as_str())
    }

    /// Whether the cached `expires_at` is in the past.
    ///
    /// `false` when unresolved or when the token never expires.
    pub fn is_expired(&self) -> bool {
        match self.identity.get().and_then(|identity| identity.expires_at) {
            Some(expires_at) => expires_at < chrono::Utc::now(),
            None => false,
        }
    }

    /// Fetch and cache the identity from `/me`, blocking the calling thread;
    /// at most one successful fetch per process. Errors are returned but not
    /// cached, so a later call retries.
    pub fn resolve_identity(&self) -> Result<&UserIdentity, MeError> {
        if let Some(identity) = self.identity.get() {
            return Ok(identity);
        }
        // The fetch runs on its own thread with its own runtime so that this
        // can block safely from both sync and async (tokio) callers.
        let identity = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("failed to build runtime for /me request")
                        .block_on(accounts::fetch_me(&self.api_url, &self.token))
                })
                .join()
                .expect("/me request thread panicked")
        })?;
        Ok(self.identity.get_or_init(|| identity))
    }
}

impl std::fmt::Debug for PersonalAccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersonalAccessToken")
            .field("identity", &self.identity.get())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use httpmock::MockServer;

    use super::*;

    fn identity_response(handle: &str, expires_at: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "user_id": "auth0|123",
            "handle": handle,
            "expires_at": expires_at,
        })
    }

    #[test]
    fn opaque_token_resolves_and_caches_identity() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/api/v1/accounts/me");
            then.status(200)
                .json_body(identity_response("testuser", Some("2027-01-01T00:00:00Z")));
        });

        let token = PersonalAccessToken::new("flox_pat_secret".to_string(), server.base_url());
        assert_eq!(token.handle(), None, "handle is unknown before resolution");

        token.resolve_identity().unwrap();
        // A clone shares the cache, and a second resolve does not refetch.
        token.clone().resolve_identity().unwrap();

        mock.assert_calls(1);
        assert_eq!(token.handle(), Some("testuser"));
    }

    #[test]
    fn opaque_token_resolve_error_is_not_cached() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/api/v1/accounts/me");
            then.status(500);
        });

        let token = PersonalAccessToken::new("flox_pat_secret".to_string(), server.base_url());
        token.resolve_identity().unwrap_err();
        token.resolve_identity().unwrap_err();

        // Both attempts hit the server: failures don't poison the cache.
        mock.assert_calls(2);
        assert_eq!(token.handle(), None);
    }

    #[test]
    fn opaque_token_expiry_reads_the_cache() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/api/v1/accounts/me");
            then.status(200)
                .json_body(identity_response("testuser", Some("2000-01-01T00:00:00Z")));
        });

        let token = PersonalAccessToken::new("flox_pat_secret".to_string(), server.base_url());
        assert!(
            !token.is_expired(),
            "unresolved token is not reported expired"
        );

        token.resolve_identity().unwrap();
        assert!(token.is_expired(), "past expires_at is reported expired");
    }

    #[test]
    fn opaque_token_without_expiry_never_expires() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/api/v1/accounts/me");
            then.status(200)
                .json_body(identity_response("testuser", None));
        });

        let token = PersonalAccessToken::new("flox_pat_secret".to_string(), server.base_url());
        token.resolve_identity().unwrap();
        assert!(!token.is_expired());
    }

    #[test]
    fn opaque_token_debug_redacts_the_secret() {
        let token = PersonalAccessToken::new(
            "flox_pat_secret".to_string(),
            "https://not_used".to_string(),
        );
        assert!(!format!("{token:?}").contains("flox_pat_secret"));
    }
}
