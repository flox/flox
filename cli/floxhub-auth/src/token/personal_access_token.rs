//! [`PersonalAccessToken`] — an opaque `flox_pat_` token whose identity is
//! resolved lazily and cached in memory.

use crate::identity::{IdentityError, LazyIdentity, UserIdentity};

/// Prefix identifying a FloxHub personal access token.
pub const PAT_PREFIX: &str = "flox_pat_";

/// An opaque token (`flox_pat_…` personal access token) authenticating a user
/// with FloxHub.
///
/// The CLI cannot decode identity from an opaque token. Identity is a
/// [`LazyIdentity`] bound at construction: resolved on first use by
/// [`Self::resolve_identity`], at most once per process, shared across
/// clones. Until resolution succeeds, [`Self::handle`] returns `None` and
/// [`Self::is_expired`] returns `false` — the server's 401 is the
/// authoritative backstop.
#[derive(Clone)]
pub struct PersonalAccessToken {
    /// The entire token as a string.
    token: String,
    /// The lazily resolved identity behind the token.
    identity: LazyIdentity,
}

impl PersonalAccessToken {
    /// Create an opaque token from a string; nothing is parsed or validated.
    pub fn new(token: String, identity: LazyIdentity) -> Self {
        PersonalAccessToken { token, identity }
    }

    /// Return the token as a string.
    pub fn secret(&self) -> &str {
        &self.token
    }

    /// Return the resolved handle; `None` until [`Self::resolve_identity`]
    /// has succeeded.
    pub fn handle(&self) -> Option<&str> {
        std::sync::LazyLock::get(&self.identity)
            .and_then(|resolved| resolved.as_ref().ok())
            .map(|identity| identity.handle.as_str())
    }

    /// Whether the resolved `expires_at` is in the past.
    ///
    /// `false` when unresolved or when the token never expires.
    pub fn is_expired(&self) -> bool {
        let expires_at = std::sync::LazyLock::get(&self.identity)
            .and_then(|resolved| resolved.as_ref().ok())
            .and_then(|identity| identity.expires_at);
        match expires_at {
            Some(expires_at) => expires_at < chrono::Utc::now(),
            None => false,
        }
    }

    /// Resolve the identity, blocking the calling thread on first use. The
    /// outcome — success or failure — is resolved at most once per process.
    pub fn resolve_identity(&self) -> Result<&UserIdentity, IdentityError> {
        std::sync::LazyLock::force(&self.identity)
            .as_ref()
            .map_err(Clone::clone)
    }
}

impl std::fmt::Debug for PersonalAccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PersonalAccessToken")
            .field("identity", &std::sync::LazyLock::get(&self.identity))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::identity::lazy_identity;
    use crate::identity::test_helpers::{static_identity, test_identity, unreachable_identity};

    #[test]
    fn opaque_token_resolves_and_caches_identity() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let token = PersonalAccessToken::new(
            "flox_pat_secret".to_string(),
            lazy_identity(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok(test_identity("testuser"))
            }),
        );
        assert_eq!(token.handle(), None, "handle is unknown before resolution");

        token.resolve_identity().unwrap();
        // A clone shares the lazy identity, and a second resolve does not
        // re-resolve.
        token.clone().resolve_identity().unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(token.handle(), Some("testuser"));
    }

    #[test]
    fn opaque_token_resolution_failure_is_final_for_the_process() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let token = PersonalAccessToken::new(
            "flox_pat_secret".to_string(),
            lazy_identity(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Err(IdentityError::Other("server unreachable".to_string()))
            }),
        );

        token
            .resolve_identity()
            .expect_err("an unreachable server should fail resolution");
        token
            .resolve_identity()
            .expect_err("the outcome is cached; the failure persists");

        // The resolution function ran exactly once; the failure is cached.
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(token.handle(), None);
    }

    #[test]
    fn opaque_token_expiry_reads_the_resolved_identity() {
        let identity = UserIdentity {
            expires_at: Some(chrono::Utc::now() - chrono::Duration::hours(1)),
            ..test_identity("testuser")
        };
        let token =
            PersonalAccessToken::new("flox_pat_secret".to_string(), static_identity(identity));
        assert!(
            !token.is_expired(),
            "unresolved token is not reported expired"
        );

        token.resolve_identity().unwrap();
        assert!(token.is_expired(), "past expires_at is reported expired");
    }

    #[test]
    fn opaque_token_without_expiry_never_expires() {
        let token = PersonalAccessToken::new(
            "flox_pat_secret".to_string(),
            static_identity(test_identity("testuser")),
        );
        token.resolve_identity().unwrap();
        assert!(!token.is_expired());
    }

    #[test]
    fn opaque_token_debug_redacts_the_secret() {
        let token = PersonalAccessToken::new("flox_pat_secret".to_string(), unreachable_identity());
        assert!(!format!("{token:?}").contains("flox_pat_secret"));
    }
}
