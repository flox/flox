use std::collections::BTreeMap;
use std::str::FromStr;

use flox_rust_sdk::flox::{FLOX_VERSION, Floxhub};
use flox_rust_sdk::utils::{HEADER_DEVICE_UUID, INVOCATION_SOURCES};
use floxhub_client::{
    AuthContext,
    FloxhubClient,
    FloxhubClientConfig,
    FloxhubMockMode,
    FloxhubToken,
};
use tracing::debug;
use uuid::Uuid;

use crate::config::Config;

/// Initialize the FloxHub API client.
///
/// - The catalog URL is an explicit `catalog_url`/`FLOX_CATALOG_URL` override
///   if set, otherwise derived from the FloxHub base via
///   [`Floxhub::catalog_url`] (hosted realm → `api.flox.dev`; any other base →
///   the base, with the client appending `/api/v1/catalog`).
/// - Configures mock replay mode if `_FLOX_USE_CATALOG_MOCK` is set
/// - Includes device UUID and invocation-source headers when available
pub fn init_floxhub_client(
    config: &Config,
    floxhub: &Floxhub,
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
        base_url: config
            .flox
            .catalog_url
            .clone()
            .unwrap_or_else(|| Floxhub::catalog_url(floxhub.base_url()).to_string()),
        extra_headers,
        mock_mode,
        auth_context: AuthContext::from_mode(
            &config.flox.floxhub_authn_mode,
            config.flox.floxhub_token.as_deref().and_then(|s| {
                if s.is_empty() {
                    None
                } else {
                    FloxhubToken::from_str(s).ok()
                }
            }),
        ),
        user_agent: Some(format!("flox-cli/{}", &*FLOX_VERSION)),
    };

    debug!("using catalog client with url: {}", client_config.base_url);
    Ok(FloxhubClient::new(client_config)?)
}
