//! Configuration types for catalog client construction.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::Credential;

/// Configuration for catalog client construction.
#[derive(Debug, Clone)]
pub struct CatalogClientConfig {
    /// Base URL for the catalog API.
    pub catalog_url: String,
    /// Additional headers to include in requests.
    pub extra_headers: BTreeMap<String, String>,
    /// Mock mode for testing.
    pub mock_mode: CatalogMockMode,
    pub credential: Credential,
    pub user_agent: Option<String>,
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

impl CatalogMockMode {
    pub fn default_from_env() -> Self {
        if let Ok(path_str) = std::env::var(crate::FLOX_CATALOG_MOCK_DATA_VAR) {
            let path = PathBuf::from(path_str);
            CatalogMockMode::Replay(path)
        } else if let Ok(path_str) = std::env::var(crate::FLOX_CATALOG_DUMP_DATA_VAR) {
            let path = PathBuf::from(path_str);
            CatalogMockMode::Record(path)
        } else {
            CatalogMockMode::None
        }
    }
}
