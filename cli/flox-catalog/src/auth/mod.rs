//! Authentication strategies for catalog client requests
//!
//! This module provides compile-time selection of authentication strategies
//! via Cargo features. The available strategies are:
//!
//! - Default (no feature): Auth0 authentication only (no Kerberos dependencies)
//! - `floxhub-authn-kerberos`: Kerberos authentication via GSSAPI

use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::token::FloxhubToken;

/// Errors from authentication validation
#[derive(Debug, Clone, thiserror::Error)]
pub enum AuthError {
    #[error("{0}")]
    NotAuthenticated(String),
    #[error("{message}")]
    Expired { handle: String, message: String },
}

/// Strategy pattern for authentication header insertion
pub trait AuthStrategy: Send + Sync + std::fmt::Debug {
    /// Add authorization headers to the provided HeaderMap
    fn add_auth_headers(&self, header_map: &mut HeaderMap);

    /// Validate that auth is available and return the user's handle.
    fn get_handle(&self) -> Result<String, AuthError>;

    /// Return the authentication method this strategy implements.
    fn auth_method(&self) -> AuthMethod;
}

/// Construct the appropriate strategy for the given [`AuthMethod`].
///
/// Each strategy uses different data:
/// - Auth0 needs the FloxHub token for bearer authentication
/// - Kerberos needs the catalog URL for SPNEGO service principal resolution
pub fn auth_strategy_from_method(
    method: &AuthMethod,
    floxhub_token: Option<FloxhubToken>,
    #[cfg_attr(not(feature = "floxhub-authn-kerberos"), allow(unused_variables))]
    catalog_url: String,
) -> std::sync::Arc<dyn AuthStrategy> {
    match method {
        AuthMethod::Auth0 => std::sync::Arc::new(Auth0AuthStrategy::new(floxhub_token)),
        #[cfg(feature = "floxhub-authn-kerberos")]
        AuthMethod::Kerberos => std::sync::Arc::new(KerberosAuthStrategy::new(catalog_url)),
    }
}

// Always include Auth0 strategy
mod auth0;
use auth0::Auth0AuthStrategy;

// Conditionally include Kerberos strategy
#[cfg(feature = "floxhub-authn-kerberos")]
mod kerberos;
#[cfg(feature = "floxhub-authn-kerberos")]
use kerberos::KerberosAuthStrategy;

/// Available authentication methods
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    /// Auth0 authentication
    Auth0,
    /// Kerberos authentication
    #[cfg(feature = "floxhub-authn-kerberos")]
    Kerberos,
}

#[cfg(not(feature = "floxhub-authn-kerberos"))]
#[allow(clippy::derivable_impls)]
impl Default for AuthMethod {
    fn default() -> Self {
        AuthMethod::Auth0
    }
}

#[cfg(feature = "floxhub-authn-kerberos")]
#[allow(clippy::derivable_impls)]
impl Default for AuthMethod {
    fn default() -> Self {
        AuthMethod::Kerberos
    }
}
