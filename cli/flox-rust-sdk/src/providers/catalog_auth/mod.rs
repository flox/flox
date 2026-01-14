//! Authentication strategies for catalog client requests
//!
//! This module provides compile-time selection of authentication strategies
//! via Cargo features. The available strategies are:
//!
//! - Default (no feature): Bearer token authentication only (no GSSAPI dependencies)
//! - `catalog-auth-gssapi`: GSSAPI/Kerberos authentication

use cfg_if::cfg_if;
use reqwest::header::HeaderMap;

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

// Conditionally compile authentication modules and type alias based on features
cfg_if! {
    if #[cfg(feature = "catalog-auth-gssapi")] {
        mod gssapi;
        use gssapi::GssapiAuthStrategy;

        /// Type alias for build-time injection: GSSAPI authentication strategy
        pub type CatalogAuthStrategy = GssapiAuthStrategy;
    } else {
        mod bearer_token;
        use bearer_token::BearerTokenAuthStrategy;

        /// Type alias for build-time injection: Bearer token authentication strategy
        pub type CatalogAuthStrategy = BearerTokenAuthStrategy;
    }
}
