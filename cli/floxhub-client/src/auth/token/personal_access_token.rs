//! [`PersonalAccessToken`] — an opaque `flox_pat_` token.

use crate::auth::identity;

/// Prefix identifying a FloxHub personal access token.
pub const PAT_PREFIX: &str = "flox_pat_";

/// An opaque token (`flox_pat_…` personal access token) authenticating a user
/// with FloxHub.
///
/// The CLI cannot decode identity from an opaque token; it is resolved via
/// `GET /api/v1/accounts/me` (`FloxhubClient::resolve_identity`) and cached
/// process-wide, keyed by the secret. Until resolution succeeds,
/// [`Self::handle`] returns `None` — the server's 401 is the authoritative
/// backstop.
#[derive(Clone)]
pub struct PersonalAccessToken {
    /// The entire token as a string.
    token: String,
}

impl PersonalAccessToken {
    /// Create an opaque token from a string; nothing is parsed or validated.
    pub fn new(token: String) -> Self {
        PersonalAccessToken { token }
    }

    /// Return the token as a string.
    pub fn secret(&self) -> &str {
        &self.token
    }

    /// Return the resolved handle; `None` until resolution has succeeded.
    /// Reads the process-wide cache — never blocks, never touches the
    /// network.
    pub fn handle(&self) -> Option<String> {
        identity::cached_identity(&self.token)?
            .ok()
            .map(|identity| identity.handle)
    }
}

impl std::fmt::Debug for PersonalAccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersonalAccessToken")
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
        let token = PersonalAccessToken::new("flox_pat_handle-cache-test".to_string());
        assert_eq!(token.handle(), None, "handle is unknown before resolution");

        identity::resolve_and_cache(token.secret(), |_| Ok(test_identity("testuser"))).unwrap();
        assert_eq!(token.handle(), Some("testuser".to_string()));
    }

    #[test]
    fn opaque_token_debug_redacts_the_secret() {
        let token = PersonalAccessToken::new("flox_pat_debug-test".to_string());
        assert!(!format!("{token:?}").contains("flox_pat_debug-test"));
    }
}
