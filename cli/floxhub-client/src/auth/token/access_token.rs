//! [`AccessToken`] — an opaque `flox_`-prefixed token.

use crate::auth::identity;

/// Prefix identifying an opaque FloxHub access token. Individual kinds carry
/// a longer prefix (`flox_pat_` personal access tokens, service account
/// tokens to come), but the CLI treats them uniformly and never parses them.
pub(crate) const ACCESS_TOKEN_PREFIX: &str = "flox_";

/// An opaque access token (`flox_…`) authenticating a caller with FloxHub —
/// a `flox_pat_` personal access token today; service account tokens to
/// come.
///
/// The CLI cannot decode identity from an opaque token; it is resolved via
/// `GET /api/v1/accounts/me` (`FloxhubClient::resolve_identity`) and cached
/// process-wide, keyed by the secret. Until resolution succeeds,
/// [`Self::handle`] returns `None` — the server's 401 is the authoritative
/// backstop.
#[derive(Clone)]
pub struct AccessToken {
    /// The entire token as a string.
    token: String,
}

impl AccessToken {
    /// Create an opaque token from a string; nothing is parsed or validated.
    pub fn new(token: String) -> Self {
        AccessToken { token }
    }

    /// Return the token as a string.
    pub fn secret(&self) -> &str {
        &self.token
    }

    /// Return the resolved handle; `None` until resolution has succeeded.
    /// Reads the process-wide cache — never blocks, never touches the
    /// network.
    pub fn handle(&self) -> Option<String> {
        identity::cached_identity(&self.token).map(|identity| identity.handle)
    }
}

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccessToken")
            .field("identity", &identity::cached_identity(&self.token))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::identity::test_helpers::test_identity;

    #[test]
    fn opaque_token_handle_reads_the_identity_cache() {
        let token = AccessToken::new("flox_pat_handle-cache-test".to_string());
        assert_eq!(token.handle(), None, "handle is unknown before resolution");

        identity::cache_identity(token.secret(), &test_identity("testuser"));
        assert_eq!(token.handle(), Some("testuser".to_string()));
    }

    #[test]
    fn opaque_token_debug_redacts_the_secret() {
        let token = AccessToken::new("flox_pat_debug-test".to_string());
        assert!(!format!("{token:?}").contains("flox_pat_debug-test"));
    }
}
