//! FloxHub SDK client — catalog and (later) factory API surfaces.
//!
//! This crate provides:
//! - HTTP client construction with bearer token authentication
//! - Complete catalog API trait ([`CatalogClientTrait`]) with HTTP implementation
//! - Catalog domain types (`PackageGroup`, `ResolvedPackageGroup`, etc.)
//! - Operation-specific error types
//! - Common error handling for catalog API operations
//! - Mock server infrastructure for integration testing (feature-gated)
//! - Re-exports of `catalog-api-v1` types for consumers
//!
//! ## Usage
//!
//! ```ignore
//! use floxhub_client::{
//!     FloxhubClient, FloxhubClientConfig,
//!     FloxhubMockMode, CatalogClientTrait, AuthContext,
//! };
//!
//! let config = FloxhubClientConfig {
//!     base_url: "https://api.flox.dev".to_string(),
//!     extra_headers: BTreeMap::new(),
//!     mock_mode: FloxhubMockMode::None,
//!     auth_context: AuthContext::from_mode(&Default::default(), floxhub_token),
//!     user_agent: Some("flox-cli/1.0".to_string()),
//! };
//!
//! let client = FloxhubClient::new(config)?;
//! let results = client.search("curl", system, None).await?;
//! ```

mod auth;
mod client;
mod config;
mod error;
mod token;
mod types;

pub(crate) mod mock;

pub const DEFAULT_CATALOG_URL: &str = "https://api.flox.dev";
pub const FLOX_CATALOG_MOCK_DATA_VAR: &str = "_FLOX_USE_CATALOG_MOCK";
pub const FLOX_CATALOG_DUMP_DATA_VAR: &str = "_FLOX_CATALOG_DUMP_RESPONSE_FILE";

// Re-export catalog-api-v1 types for consumers.
// This allows consumers to depend only on floxhub-client, not directly on catalog-api-v1.
pub use auth::{AuthContext, AuthFailure, AuthHeaderError, AuthnMode, KerberosMaterial};
pub use catalog_api_v1::{
    Client as ApiClient,
    Error as ApiError,
    ResponseValue as ApiResponseValue,
};
#[cfg(any(test, feature = "tests"))]
// Client
pub use client::EMPTY_SEARCH_RESPONSE;
pub use client::{CatalogClientTrait, FloxhubClient, str_to_catalog_name, str_to_package_name};
pub use config::{FloxhubClientConfig, FloxhubMockMode};
// Errors
pub use error::*;
pub use token::{FloxhubToken, FloxhubTokenError};
// Types (re-exported from types module for convenience)
pub use types::*;
