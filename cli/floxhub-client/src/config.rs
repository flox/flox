//! Configuration types for FloxhubClient construction.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::AuthContext;

/// Configuration for FloxHub client construction.
///
/// The `base_url` and auth/header fields here are shared by both the catalog
/// and factory inner clients inside [`crate::FloxhubClient`]; the two surfaces
/// share a base URL and authentication scheme on FloxHub.
#[derive(Debug, Clone)]
pub struct FloxhubClientConfig {
    /// Base URL for the catalog and factory APIs.
    pub base_url: String,
    /// Additional headers to include in requests.
    pub extra_headers: BTreeMap<String, String>,
    /// Mock mode for testing.
    pub mock_mode: FloxhubMockMode,
    pub auth_context: AuthContext,
    pub user_agent: Option<String>,
}

/// Mock recording/replay mode for integration testing.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub enum FloxhubMockMode {
    /// Use a real server without any mock recording or replaying.
    #[default]
    None,
    /// Proxy via a mock server and record interactions to a path.
    Record(PathBuf),
    /// Replay interactions from a path using a mock server.
    Replay(PathBuf),
}

impl FloxhubMockMode {
    pub fn default_from_env() -> Self {
        if let Ok(path_str) = std::env::var(crate::FLOX_CATALOG_MOCK_DATA_VAR) {
            let path = PathBuf::from(path_str);
            FloxhubMockMode::Replay(path)
        } else if let Ok(path_str) = std::env::var(crate::FLOX_CATALOG_DUMP_DATA_VAR) {
            let path = PathBuf::from(path_str);
            FloxhubMockMode::Record(path)
        } else {
            FloxhubMockMode::None
        }
    }
}
