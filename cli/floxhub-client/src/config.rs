//! Configuration types for FloxhubClient construction.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::AuthContext;

/// Hook invoked by [`crate::FloxhubClient::resolve`] just before it contacts
/// the catalog `/resolve` endpoint without authentication material (see
/// [`AuthContext::is_unauthenticated`]).
///
/// The CLI installs a hook that warns that resolution will require
/// authentication in an upcoming release; rate limiting and output routing
/// live in that hook, not here. Consumers with no user to warn (tests, batch
/// tools) leave the config field unset. This call site is also where an
/// interactive "log in now?" prompt will live once catalog auth gating is
/// enforced server-side.
#[derive(Clone)]
pub struct UnauthenticatedResolveHook(Arc<dyn Fn() + Send + Sync>);

impl UnauthenticatedResolveHook {
    pub fn new(hook: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(hook))
    }

    pub fn call(&self) {
        (self.0)()
    }
}

impl std::fmt::Debug for UnauthenticatedResolveHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UnauthenticatedResolveHook")
    }
}

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
    /// Invoked when `resolve()` is called without authentication material;
    /// `None` disables the unauthenticated-resolve warning.
    pub on_unauthenticated_resolve: Option<UnauthenticatedResolveHook>,
}

impl FloxhubClientConfig {
    /// Read the test/regen-only stability pin from
    /// [`crate::FLOX_RESOLVE_STABILITY_VAR`]. Empty string is treated as
    /// unset.
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
