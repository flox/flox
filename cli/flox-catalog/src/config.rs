//! Configuration types for catalog client construction.

use std::collections::BTreeMap;
use std::path::PathBuf;

/// Configuration for catalog client construction.
#[derive(Debug, Clone)]
pub struct CatalogClientConfig {
    /// Base URL for the catalog API.
    pub catalog_url: String,
    /// Optional bearer token for FloxHub authentication.
    pub floxhub_token: Option<String>,
    /// Additional headers to include in requests.
    pub extra_headers: BTreeMap<String, String>,
    /// Mock mode for testing.
    pub mock_mode: CatalogMockMode,
}

/// Mock recording/replay mode for integration testing.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub enum CatalogMockMode {
    /// Use a real server without any mock recording or replaying.
    #[default]
    None,
    /// Proxy via a mock server and record interactions to a path.
    Record(PathBuf),
    /// Replay interactions from a path using a mock server.
    Replay(PathBuf),
}
