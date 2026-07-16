//! The identity behind a FloxHub credential, and how it is resolved.
//!
//! [`UserIdentity`] is resolved lazily: a [`LazyIdentity`] wraps the
//! resolution function together with its once-per-process result, and is
//! bound to a token at construction. This crate defines the contract only —
//! the production resolution against `/me` lives with the FloxHub HTTP
//! client (`floxhub-client`), keeping this crate free of any transport
//! concerns.

use std::sync::{Arc, LazyLock};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use thiserror::Error;

/// The identity behind a credential.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct UserIdentity {
    pub user_id: String,
    pub handle: String,
    /// Wall-clock expiry of the presenting credential;
    /// `None` when it never expires.
    pub expires_at: Option<DateTime<Utc>>,
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

/// A lazily resolved identity, shared across clones of its credential.
///
/// The resolution function runs at most once per process, on first use; the
/// outcome — success or failure — is cached.
pub type LazyIdentity = Arc<
    LazyLock<
        Result<UserIdentity, IdentityError>,
        Box<dyn FnOnce() -> Result<UserIdentity, IdentityError> + Send + Sync>,
    >,
>;

/// Wrap a resolution function as a [`LazyIdentity`].
pub fn lazy_identity(
    resolve: impl FnOnce() -> Result<UserIdentity, IdentityError> + Send + Sync + 'static,
) -> LazyIdentity {
    Arc::new(LazyLock::new(Box::new(resolve)))
}

/// Test fixtures for identity resolution.
///
/// Intentionally not behind `#[cfg(test)]` so that other crates' (also
/// non-gated) test helpers can use them without enabling a feature.
/// Nothing here should be used in production code.
pub mod test_helpers {
    use super::*;

    /// A lazy identity that resolves to the given identity.
    pub fn static_identity(identity: UserIdentity) -> LazyIdentity {
        lazy_identity(move || Ok(identity))
    }

    /// A lazy identity that fails as if the server rejected the credential.
    pub fn unauthorized_identity() -> LazyIdentity {
        lazy_identity(|| Err(IdentityError::Unauthorized))
    }

    /// A lazy identity that fails as if the server were unreachable.
    pub fn unreachable_identity() -> LazyIdentity {
        lazy_identity(|| Err(IdentityError::Other("server unreachable".to_string())))
    }

    /// A resolution function for `AuthContext::from_mode` in tests that must
    /// never resolve.
    pub fn unreachable_resolve(_token: String) -> Result<UserIdentity, IdentityError> {
        Err(IdentityError::Other("server unreachable".to_string()))
    }

    /// An identity for `handle` that never expires.
    pub fn test_identity(handle: &str) -> UserIdentity {
        UserIdentity {
            user_id: format!("test|{handle}"),
            handle: handle.to_string(),
            expires_at: None,
        }
    }
}
