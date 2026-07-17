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

/// Return the cached identity for `secret`, running `resolve` to fill the
/// cache when it has not been resolved yet. A failed resolution is returned
/// but not cached.
///
/// The lock is not held while resolving — the request can take seconds and
/// must not block concurrent readers of the cache.
pub(crate) fn resolve_and_cache(
    secret: &str,
    resolve: impl FnOnce(&str) -> Result<UserIdentity, IdentityError>,
) -> Result<UserIdentity, IdentityError> {
    if let Some(identity) = cached_identity(secret) {
        return Ok(identity);
    }
    let identity = resolve(secret)?;
    RESOLVED_IDENTITIES
        .lock()
        .expect("identity cache is never poisoned")
        .insert(secret.to_string(), identity.clone());
    Ok(identity)
}

/// Test fixtures for identity resolution.
///
/// Intentionally not behind `#[cfg(test)]` so that other crates' (also
/// non-gated) test helpers can use them without enabling a feature.
/// Nothing here should be used in production code.
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::test_helpers::test_identity;
    use super::*;

    // NOTE: the cache is process-wide state shared by every test in this
    // binary — each test must use a unique secret.

    #[test]
    fn identity_resolves_and_caches_per_secret() {
        let calls = AtomicUsize::new(0);
        let resolve = |_: &str| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(test_identity("testuser"))
        };

        resolve_and_cache("flox_pat_success-cache-test", resolve).unwrap();
        resolve_and_cache("flox_pat_success-cache-test", resolve).unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            cached_identity("flox_pat_success-cache-test").unwrap(),
            test_identity("testuser")
        );
    }

    #[test]
    fn identity_resolution_failures_are_retried() {
        let calls = AtomicUsize::new(0);

        resolve_and_cache("flox_pat_retry-test", |_: &str| {
            calls.fetch_add(1, Ordering::SeqCst);
            Err(IdentityError::Other("server unreachable".to_string()))
        })
        .expect_err("an unreachable server should fail resolution");
        assert_eq!(
            cached_identity("flox_pat_retry-test"),
            None,
            "a failure is not cached"
        );

        // The next call retries — and its success is cached.
        resolve_and_cache("flox_pat_retry-test", |_: &str| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(test_identity("testuser"))
        })
        .unwrap();
        resolve_and_cache("flox_pat_retry-test", |_: &str| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(test_identity("testuser"))
        })
        .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
