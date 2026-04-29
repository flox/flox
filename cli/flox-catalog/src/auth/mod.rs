//! Authentication for catalog client requests
//!
//! This module provides credential creation via compile-time selection of
//! authentication method. The available methods are:
//!
//! - Default (no feature): Auth0 authentication only (no Kerberos dependencies)
//! - `floxhub-authn-kerberos`: Kerberos authentication via GSSAPI

use serde::{Deserialize, Serialize};

mod auth_context;
mod auth_context_factory;

pub use auth_context::{AuthContext, AuthFailure};

/// Errors from authentication validation
#[derive(Debug, Clone, thiserror::Error)]
pub enum AuthError {
    #[error("{0}")]
    NotAuthenticated(String),
    #[error("{message}")]
    Expired { handle: String, message: String },
}

/// Available authentication methods
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthnMode {
    /// Auth0 authentication
    Auth0,
    /// Kerberos authentication
    #[cfg(feature = "floxhub-authn-kerberos")]
    Kerberos,
}

#[cfg(not(feature = "floxhub-authn-kerberos"))]
#[allow(clippy::derivable_impls)]
impl Default for AuthnMode {
    fn default() -> Self {
        AuthnMode::Auth0
    }
}

#[cfg(feature = "floxhub-authn-kerberos")]
#[allow(clippy::derivable_impls)]
impl Default for AuthnMode {
    fn default() -> Self {
        AuthnMode::Kerberos
    }
}
