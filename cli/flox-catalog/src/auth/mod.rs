//! Authentication strategies for catalog client requests
//!
//! This module provides compile-time selection of authentication strategies
//! via Cargo features. The available strategies are:
//!
//! - Default (no feature): Auth0 authentication only (no Kerberos dependencies)
//! - `floxhub-authn-kerberos`: Kerberos authentication via GSSAPI

use enum_dispatch::enum_dispatch;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::token::FloxhubToken;

/// Errors from authentication validation
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("{0}")]
    NotAuthenticated(String),
}

/// Strategy pattern for authentication header insertion
#[enum_dispatch]
pub trait AuthStrategy {
    /// Add authorization headers to the provided HeaderMap
    ///
    /// # Arguments
    /// * `header_map` - The header map to modify
    fn add_auth_headers(&self, header_map: &mut HeaderMap);

    /// Validate that auth is available and return the user's handle.
    fn get_handle(&self) -> Result<String, AuthError>;
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

impl AuthMethod {
    /// Convert this auth method to the appropriate strategy with the required data.
    ///
    /// Each strategy uses different data:
    /// - Auth0 needs the FloxHub token for bearer authentication
    /// - Kerberos needs the catalog URL for SPNEGO service principal resolution
    pub fn to_strategy(
        &self,
        floxhub_token: Option<FloxhubToken>,
        #[cfg_attr(not(feature = "floxhub-authn-kerberos"), allow(unused_variables))]
        catalog_url: String,
    ) -> AuthStrategies {
        match self {
            AuthMethod::Auth0 => AuthStrategies::Auth0(Auth0AuthStrategy::new(floxhub_token)),
            #[cfg(feature = "floxhub-authn-kerberos")]
            AuthMethod::Kerberos => {
                AuthStrategies::Kerberos(KerberosAuthStrategy::new(catalog_url))
            },
        }
    }
}

#[derive(Debug)]
#[enum_dispatch(AuthStrategy)]
pub enum AuthStrategies {
    /// Auth0 authentication (default)
    Auth0(Auth0AuthStrategy),
    /// Kerberos authentication
    #[cfg(feature = "floxhub-authn-kerberos")]
    Kerberos(KerberosAuthStrategy),
}
