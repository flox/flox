//! Authentication strategies for catalog client requests
//!
//! This module provides compile-time selection of authentication strategies
//! via Cargo features. The available strategies are:
//!
//! - Default (no feature): Bearer token authentication only (no GSSAPI dependencies)
//! - `catalog-auth-gssapi`: GSSAPI/Kerberos authentication

use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::providers::catalog::CatalogClientConfig;

/// Strategy pattern for authentication header insertion
pub trait AuthStrategy {
    /// Add authorization headers to the provided HeaderMap
    ///
    /// # Arguments
    /// * `header_map` - The header map to modify
    /// * `config` - The catalog client configuration (may be needed for certain auth methods)
    fn add_auth_headers(header_map: &mut HeaderMap, config: &CatalogClientConfig);
}

// Always include bearer token strategy
mod bearer_token;
use bearer_token::BearerTokenAuthStrategy;

// Conditionally include GSSAPI strategy
#[cfg(feature = "catalog-auth-gssapi")]
mod gssapi;
#[cfg(feature = "catalog-auth-gssapi")]
use gssapi::GssapiAuthStrategy;

/// Available authentication methods
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuthMethod {
    #[default]
    /// Bearer token authentication (default)
    Bearer,
    /// GSSAPI/Kerberos authentication
    #[cfg(feature = "catalog-auth-gssapi")]
    Gssapi,
}

impl AuthStrategy for AuthMethod {
    fn add_auth_headers(header_map: &mut HeaderMap, config: &CatalogClientConfig) {
        match &config.auth_method {
            AuthMethod::Bearer => BearerTokenAuthStrategy::add_auth_headers(header_map, config),
            #[cfg(feature = "catalog-auth-gssapi")]
            AuthMethod::Gssapi => GssapiAuthStrategy::add_auth_headers(header_map, config),
        }
    }
}
