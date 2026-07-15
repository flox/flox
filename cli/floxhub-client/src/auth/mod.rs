//! Authentication for catalog client requests
//!
//! The available methods are:
//!
//! - Auth0 authentication
//! - Kerberos authentication via GSSAPI

use serde::{Deserialize, Serialize};

mod auth_context;
mod auth_context_factory;

pub use auth_context::{AuthContext, AuthFailure, AuthHeaderError, KerberosMaterial};

/// Errors from authentication validation (internal, used by Kerberos credential acquisition).
#[derive(Debug, Clone, thiserror::Error)]
pub(crate) enum AuthError {
    #[error("{0}")]
    NotAuthenticated(String),
}

/// Available authentication methods
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthnMode {
    /// Auth0 authentication
    #[default]
    Auth0,
    /// Kerberos authentication
    Kerberos,
}
