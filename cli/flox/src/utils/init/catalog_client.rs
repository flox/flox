use std::collections::BTreeMap;
use std::str::FromStr;

use flox_catalog::{
    AuthContext,
    CatalogClient,
    CatalogClientConfig,
    CatalogMockMode,
    DEFAULT_CATALOG_URL,
    FloxhubToken,
};
use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::providers::catalog::Client;
use flox_rust_sdk::utils::{HEADER_DEVICE_UUID, INVOCATION_SOURCES};
use tracing::debug;
use uuid::Uuid;

use crate::config::Config;

/// Initialize the Catalog API client
///
/// - Initialize a mock client if the `_FLOX_USE_CATALOG_MOCK` environment variable is set to `true`
/// - Initialize a real client otherwise
pub fn init_catalog_client(
    config: &Config,
    metrics_device_uuid: Option<Uuid>,
) -> Result<Client, anyhow::Error> {
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

    let mock_mode = CatalogMockMode::default_from_env();

    let client_config = CatalogClientConfig {
        catalog_url: config
            .flox
            .catalog_url
            .clone()
            .unwrap_or_else(|| DEFAULT_CATALOG_URL.to_string()),
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

    debug!(
        "using catalog client with url: {}",
        client_config.catalog_url
    );
    Ok(CatalogClient::new(client_config)?.into())
}
