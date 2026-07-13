//! [`PersonalAccessToken`] — an opaque `flox_pat_` token whose identity is
//! resolved lazily via `/me` and cached in memory.

use std::sync::{Arc, OnceLock};

use crate::accounts::{self, MeError, UserIdentity};

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
