//! Shared catalog interaction layer for the FloxHub catalog API.
//!
//! This crate provides:
//! - HTTP client construction with bearer token authentication
//! - Complete catalog API trait ([`ClientTrait`]) with HTTP implementation
//! - Catalog domain types (`PackageGroup`, `ResolvedPackageGroup`, etc.)
//! - Operation-specific error types
//! - Common error handling for catalog API operations
//! - Mock server infrastructure for integration testing (feature-gated)
//! - Re-exports of `catalog-api-v1` types for consumers
//!
//! ## Usage
//!
//! ```ignore
//! use flox_catalog::{
//!     CatalogClient, CatalogClientConfig, CatalogMockMode, ClientTrait,
//! };
//!
//! let config = CatalogClientConfig {
//!     catalog_url: "https://api.flox.dev".to_string(),
//!     floxhub_token: Some(token),
//!     extra_headers: BTreeMap::new(),
//!     mock_mode: CatalogMockMode::None,
//! };
//!
//! let client = CatalogClient::new(config)?;
//! let results = client.search("curl", system, None).await?;
//! ```

mod client;
mod config;
pub mod error;
pub mod types;

pub(crate) mod mock;

// Re-export catalog-api-v1 types for consumers.
// This allows consumers to depend only on catalog-client, not directly on catalog-api-v1.
pub use catalog_api_v1::{
    Client as ApiClient,
    Error as ApiError,
    ResponseValue as ApiResponseValue,
};
#[cfg(any(test, feature = "tests"))]
// Client
pub use client::EMPTY_SEARCH_RESPONSE;
pub use client::{str_to_catalog_name, str_to_package_name, CatalogClient, ClientTrait};
pub use config::{CatalogClientConfig, CatalogMockMode};
// Errors
pub use error::{
    CatalogClientError,
    MapApiErrorExt,
    PublishError,
    ResolveError,
    SearchError,
    VersionsError,
};
// Types (re-exported from types module for convenience)
pub use types::{
    // Base catalog
    BaseCatalogInfo,
    BaseCatalogUrl,
    BaseCatalogUrlError,
    CatalogPage,
    CatalogStoreConfig,
    CatalogStoreConfigNixCopy,
    CatalogStoreConfigPublisher,
    PackageBuild,
    PackageDescriptor,
    PackageDetails,
    // Package types
    PackageGroup,
    PackageResolutionInfo,
    PageInfo,
    // API type aliases
    PublishResponse,
    ResolutionMessage,
    // Resolved types
    ResolvedPackageGroup,
    ResultCount,
    // Result types
    ResultsPage,
    SearchLimit,
    SearchResult,
    SearchResults,
    StabilityInfo,
    StoreInfo,
    StoreInfoRequest,
    StoreInfoResponse,
    StorepathStatusResponse,
    UserBuildPublish,
    UserDerivationInfo,
};
