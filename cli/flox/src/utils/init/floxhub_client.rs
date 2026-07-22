use std::collections::BTreeMap;

use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::utils::{HEADER_DEVICE_UUID, INVOCATION_SOURCES};
use floxhub_client::{AuthContext, FloxhubClient, FloxhubClientConfig, FloxhubMockMode};
use tracing::debug;
use uuid::Uuid;

/// Initialize the FloxHub API client.
///
/// - Reads the catalog URL from config (defaults to the production catalog URL)
/// - Configures mock replay mode if `_FLOX_USE_CATALOG_MOCK` is set
/// - Includes device UUID and invocation-source headers when available
/// - Pins `resolve()` requests to a stability channel if
///   `_FLOX_RESOLVE_STABILITY` is set (test/regen-only, not user-facing)
///
/// `base_url` is the API base the generated client joins request paths onto
/// (e.g. `<base>/api/v1/catalog/...`); pass [`flox_core::floxhub::Floxhub::api_url_str`]
/// so any trailing slash is already trimmed.
pub fn init_floxhub_client(
    base_url: String,
    auth_context: AuthContext,
    metrics_device_uuid: Option<Uuid>,
) -> Result<FloxhubClient, anyhow::Error> {
    let mut extra_headers = BTreeMap::new();

    // Propagate the metrics UUID to catalog-server if metrics are enabled.
    match metrics_device_uuid {
        Some(uuid) => extra_headers.insert(HEADER_DEVICE_UUID.to_string(), uuid.to_string()),
        None => Default::default(),
    };
    // Add invocation sources header if any sources are detected
    if !INVOCATION_SOURCES.is_empty() {
        let sources_str = INVOCATION_SOURCES.join(",");
        extra_headers.insert("flox-invocation-source".to_string(), sources_str);
    };

    let mock_mode = FloxhubMockMode::default_from_env();

    let client_config = FloxhubClientConfig {
        base_url,
        extra_headers,
        mock_mode,
        auth_context,
        user_agent: Some(format!("flox-cli/{}", &*FLOX_VERSION)),
        stability: FloxhubClientConfig::stability_from_env(),
    };

    debug!("using catalog client with url: {}", client_config.base_url);
    Ok(FloxhubClient::new(client_config)?)
}
