//! The identity behind a FloxHub credential.
//!
//! [`UserIdentity`] cannot be derived locally from an opaque token; it is
//! resolved through the FloxHub client (`GET /api/v1/accounts/me`) at the
//! point of use and cached process-wide, keyed by token secret — a token's
//! identity never changes. This module defines the data contract and the
//! cache only — it carries no transport.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use thiserror::Error;

/// Placeholder handle shown when a credential could not be verified (e.g.
/// FloxHub was unreachable). The server is the authority for authn/authz,
/// so an unknown handle is a display concern, never an access decision.
pub const UNKNOWN_HANDLE: &str = "UNKNOWN";

/// The identity behind a credential.
///
/// Uniform across credential kinds: derived from JWT claims for Auth0,
/// resolved from `/me` for a personal access token, and from the principal
/// for Kerberos.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct UserIdentity {
    pub handle: String,
    /// Wall-clock expiry of the presenting credential;
    /// `None` when it never expires.
    pub expires_at: Option<DateTime<Utc>>,
}

impl UserIdentity {
    /// Whether the credential behind this identity has expired. What that
    /// means is the caller's decision — e.g. a failure when gating
    /// authentication, or merely diagnostic context elsewhere.
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .is_some_and(|expires_at| expires_at < Utc::now())
    }
}

/// Why an identity could not be resolved.
#[derive(Debug, Clone, Error)]
pub enum IdentityError {
    /// The server rejected the credential (invalid, expired, or revoked).
    #[error("token is invalid, expired, or revoked")]
    Unauthorized,
    /// Resolution failed for another reason (e.g. the server was
    /// unreachable); the credential may still authenticate requests.
    #[error("{0}")]
    Other(String),
}

/// Process-wide cache of identities resolved for opaque tokens, keyed by
/// token secret. A token's identity never changes, so a successful
/// resolution is cached for the process. Failures are not cached — a later
/// call retries.
static RESOLVED_IDENTITIES: LazyLock<Mutex<HashMap<String, UserIdentity>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Read the cached identity for `secret`, if resolution has succeeded.
pub(crate) fn cached_identity(secret: &str) -> Option<UserIdentity> {
    RESOLVED_IDENTITIES
        .lock()
        .expect("identity cache is never poisoned")
        .get(secret)
        .cloned()
}

/// Cache a successfully resolved identity for `secret`.
pub(crate) fn cache_identity(secret: &str, identity: &UserIdentity) {
    RESOLVED_IDENTITIES
        .lock()
        .expect("identity cache is never poisoned")
        .insert(secret.to_string(), identity.clone());
}

/// Test fixtures for identity resolution.
#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    use super::*;

    /// An identity for `handle` that never expires.
    pub fn test_identity(handle: &str) -> UserIdentity {
        UserIdentity {
            handle: handle.to_string(),
            expires_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_helpers::test_identity;
    use super::*;

    // NOTE: the cache is process-wide state shared by every test in this
    // binary — each test must use a unique secret.

    #[test]
    fn identity_cache_round_trips_per_secret() {
        assert_eq!(cached_identity("flox_pat_cache-round-trip-test"), None);

        cache_identity("flox_pat_cache-round-trip-test", &test_identity("testuser"));

        assert_eq!(
            cached_identity("flox_pat_cache-round-trip-test").unwrap(),
            test_identity("testuser")
        );
    }
}
