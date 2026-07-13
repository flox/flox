//! FloxHub SDK client — catalog and factory API surfaces.
//!
//! This crate abstracts over both generated API crates:
//! - `catalog-api-v1` — via [`FloxhubClient`] and the catalog [`CatalogClientTrait`]
//! - `factory-api-v1` — via [`FactoryClientTrait`] implemented for
//!   [`FloxhubClient`]
//!
//! Both clients share a single construction path inside [`FloxhubClient`]:
//! one reqwest client, one auth pre-request hook, and one record/replay
//! [`mock::MockGuard`] cover all outgoing requests.
//!
//! ## Usage
//!
//! ```ignore
//! use floxhub_client::{
//!     FloxhubClient, FloxhubClientConfig,
//!     FloxhubMockMode, CatalogClientTrait, AuthContext,
//!     FactoryClientTrait,
//! };
//!
//! let config = FloxhubClientConfig {
//!     base_url: "https://api.flox.dev".to_string(),
//!     extra_headers: BTreeMap::new(),
//!     mock_mode: FloxhubMockMode::None,
//!     auth_context: AuthContext::from_mode(&Default::default(), floxhub_token, "https://api.flox.dev")?,
//!     user_agent: Some("flox-cli/1.0".to_string()),
//!     stability: FloxhubClientConfig::stability_from_env(),
//! };
//!
//! let client = FloxhubClient::new(config)?;
//! let results = client.search("curl", system, None).await?;
//! let builds = client.list_builds(None).await?;
//! ```

pub mod client;
mod config;
mod error;
mod factory;
mod types;

pub(crate) mod mock;

pub const DEFAULT_CATALOG_URL: &str = "https://api.flox.dev";
pub const FLOX_CATALOG_MOCK_DATA_VAR: &str = "_FLOX_USE_CATALOG_MOCK";
pub const FLOX_CATALOG_DUMP_DATA_VAR: &str = "_FLOX_CATALOG_DUMP_RESPONSE_FILE";
/// Sets `PackageGroup.stability` on the wire during mock recording runs.
/// Test/regen-only — not a user-facing interface. See Justfile
/// `gen-unit-data-no-publish` for usage.
pub const FLOX_RESOLVE_STABILITY_VAR: &str = "_FLOX_RESOLVE_STABILITY";

// Re-export catalog-api-v1 types for consumers.
// This allows consumers to depend only on floxhub-client, not directly on catalog-api-v1.
// Re-export the authentication types so consumers can keep depending only
// on floxhub-client.
pub use catalog_api_v1::{
    Client as ApiClient,
    Error as ApiError,
    ResponseValue as ApiResponseValue,
};
// Client
#[cfg(any(test, feature = "tests"))]
pub use client::EMPTY_SEARCH_RESPONSE;
pub use client::{
    CatalogClientTrait,
    CheckBuildQuery,
    FloxhubClient,
    str_to_catalog_name,
    str_to_package_name,
};
pub use config::{FloxhubClientConfig, FloxhubMockMode};
// Errors
pub use error::*;
// Re-export factory types so consumers depend only on floxhub-client.
pub use factory::{
    FactoryClientError,
    FactoryClientTrait,
    MapApiErrorExt as FactoryMapApiErrorExt,
};
pub use factory_api_v1::types::{
    BuildResponse,
    ErrorResponse as FactoryErrorResponse,
    Status as FactoryStatus,
};
// Re-export factory-api-v1 types for consumers.
pub use factory_api_v1::{
    ByteStream as FactoryByteStream,
    Error as FactoryApiError,
    ResponseValue as FactoryApiResponseValue,
};
pub use floxhub_auth::*;
// Types (re-exported from types module for convenience)
pub use types::*;
