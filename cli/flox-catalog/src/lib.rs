//! Shared HTTP client infrastructure for the FloxHub catalog API.
//!
//! This crate provides:
//! - HTTP client construction with bearer token authentication
//! - Common error handling for catalog API operations
//! - Mock server infrastructure for integration testing (feature-gated)
//! - Re-exports of `catalog-api-v1` types for consumers
//!
//! ## Usage
//!
//! ```ignore
//! use flox_catalog::{CatalogClient, CatalogClientConfig, CatalogMockMode};
//!
//! let config = CatalogClientConfig {
//!     catalog_url: "https://api.flox.dev".to_string(),
//!     floxhub_token: Some(token),
//!     extra_headers: BTreeMap::new(),
//!     mock_mode: CatalogMockMode::None,
//! };
//!
//! let client = CatalogClient::new(config)?;
//! let response = client.api().resolve(...).await;
//! ```

mod client;
mod config;
mod error;

#[cfg(feature = "mock")]
pub(crate) mod mock;

// Public exports
// Re-export catalog-api-v1 types for consumers.
// This allows consumers to depend only on catalog-client, not directly on catalog-api-v1.
pub use catalog_api_v1::{Client as ApiClient, Error as ApiError, types};
pub use client::CatalogClient;
pub use config::{CatalogClientConfig, CatalogMockMode};
pub use error::{CatalogClientError, MapApiErrorExt};
