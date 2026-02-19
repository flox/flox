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

use crate::providers::catalog::CatalogClientConfig;

/// Strategy pattern for authentication header insertion
#[enum_dispatch]
pub trait AuthStrategy {
    /// Add authorization headers to the provided HeaderMap
    ///
    /// # Arguments
    /// * `header_map` - The header map to modify
    fn add_auth_headers(&self, header_map: &mut HeaderMap);
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
    /// Convert this auth method to the appropriate strategy with config data
    pub fn to_strategy(&self, config: &CatalogClientConfig) -> AuthStrategies {
        match self {
            AuthMethod::Auth0 => {
                AuthStrategies::Auth0(Auth0AuthStrategy::new(config.floxhub_token.clone()))
            },
            #[cfg(feature = "floxhub-authn-kerberos")]
            AuthMethod::Kerberos => {
                AuthStrategies::Kerberos(KerberosAuthStrategy::new(config.catalog_url.clone()))
            },
        }
    }
}

#[enum_dispatch(AuthStrategy)]
pub enum AuthStrategies {
    /// Auth0 authentication (default)
    Auth0(Auth0AuthStrategy),
    /// Kerberos authentication
    #[cfg(feature = "floxhub-authn-kerberos")]
    Kerberos(KerberosAuthStrategy),
}

/// Authentication manager that provides a static method for adding auth headers
pub struct AuthManager;

impl AuthManager {
    /// Add authentication headers using the configured auth method
    ///
    /// This static method creates the appropriate strategy based on the config
    /// and delegates to it via enum_dispatch.
    pub fn add_auth_headers(header_map: &mut HeaderMap, config: &CatalogClientConfig) {
        let strategy = config.auth_method.to_strategy(config);
        strategy.add_auth_headers(header_map);
    }
}
