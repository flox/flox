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
    /// Stability pin applied to every outgoing `PackageGroup` in
    /// `resolve()`. Test/regen-only — not a user-facing interface. See
    /// [`crate::FLOX_RESOLVE_STABILITY_VAR`] and [`Self::stability_from_env`].
    pub stability: Option<String>,
}

impl FloxhubClientConfig {
    /// Read the test/regen-only stability pin from
    /// [`crate::FLOX_RESOLVE_STABILITY_VAR`]. Empty string is treated as
    /// unset, matching [`FloxhubMockMode::default_from_env`].
    ///
    /// Call this once at client construction time and store the result on
    /// the config's `stability` field; `resolve()` applies it to every
    /// outgoing package group.
    pub fn stability_from_env() -> Option<String> {
        std::env::var(crate::FLOX_RESOLVE_STABILITY_VAR)
            .ok()
            .filter(|s| !s.is_empty())
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stability_from_env_unset_gives_none() {
        temp_env::with_var(crate::FLOX_RESOLVE_STABILITY_VAR, None::<&str>, || {
            assert_eq!(FloxhubClientConfig::stability_from_env(), None);
        });
    }

    #[test]
    fn stability_from_env_empty_gives_none() {
        temp_env::with_var(crate::FLOX_RESOLVE_STABILITY_VAR, Some(""), || {
            assert_eq!(FloxhubClientConfig::stability_from_env(), None);
        });
    }

    #[test]
    fn stability_from_env_set_gives_some() {
        temp_env::with_var(crate::FLOX_RESOLVE_STABILITY_VAR, Some("lts"), || {
            assert_eq!(
                FloxhubClientConfig::stability_from_env(),
                Some("lts".to_string())
            );
        });
    }
}
