//! Authentication for catalog client requests
//!
//! This module provides credential creation via compile-time selection of
//! authentication method. The available methods are:
//!
//! - Default (no feature): Auth0 authentication only (no Kerberos dependencies)
//! - `floxhub-authn-kerberos`: Kerberos authentication via GSSAPI

use flox_config::AuthnMode as ConfigAuthnMode;
use serde::{Deserialize, Serialize};

mod auth_context;
mod auth_context_factory;

pub use auth_context::{AuthContext, AuthFailure, AuthHeaderError, KerberosMaterial};

/// Errors from authentication validation (internal, used by Kerberos credential acquisition).
#[cfg(feature = "floxhub-authn-kerberos")]
#[derive(Debug, Clone, thiserror::Error)]
pub(crate) enum AuthError {
    #[error("{0}")]
    NotAuthenticated(String),
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

/// Error returned by [AuthnMode::from_config] when the configured mode is not
/// compiled into this build.
#[derive(Debug, Clone, thiserror::Error)]
pub enum UnsupportedAuthnModeError {
    #[error("Kerberos authentication is not supported by this build.")]
    Kerberos,
}

impl AuthnMode {
    /// Resolve the configured authn mode to the client's, applying the
    /// compiled-in default when unset.
    ///
    /// The config enum always parses both modes; this enum only carries the
    /// modes compiled into this build.
    pub fn from_config(
        configured: Option<&ConfigAuthnMode>,
    ) -> Result<Self, UnsupportedAuthnModeError> {
        match configured {
            None => Ok(AuthnMode::default()),
            Some(ConfigAuthnMode::Auth0) => Ok(AuthnMode::Auth0),
            #[cfg(feature = "floxhub-authn-kerberos")]
            Some(ConfigAuthnMode::Kerberos) => Ok(AuthnMode::Kerberos),
            #[cfg(not(feature = "floxhub-authn-kerberos"))]
            Some(ConfigAuthnMode::Kerberos) => Err(UnsupportedAuthnModeError::Kerberos),
        }
    }
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
