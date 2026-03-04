use std::collections::BTreeMap;
use std::path::PathBuf;

use flox_rust_sdk::providers::catalog::{
    CatalogClient,
    CatalogClientConfig,
    CatalogMockMode,
    Client,
    DEFAULT_CATALOG_URL,
    FLOX_CATALOG_DUMP_DATA_VAR,
    FLOX_CATALOG_MOCK_DATA_VAR,
};
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
    // Propagate the metrics UUID to catalog-server if metrics are enabled.
    let extra_headers = match metrics_device_uuid {
        Some(uuid) => BTreeMap::from([("flox-device-uuid".to_string(), uuid.to_string())]),
        None => Default::default(),
    };

    let mock_mode = if let Ok(path_str) = std::env::var(FLOX_CATALOG_MOCK_DATA_VAR) {
        let path = PathBuf::from(path_str);
        CatalogMockMode::Replay(path)
    } else if let Ok(path_str) = std::env::var(FLOX_CATALOG_DUMP_DATA_VAR) {
        let path = PathBuf::from(path_str);
        CatalogMockMode::Record(path)
    } else {
        CatalogMockMode::None
    };

    let client_config = CatalogClientConfig {
        catalog_url: config
            .flox
            .catalog_url
            .clone()
            .unwrap_or_else(|| DEFAULT_CATALOG_URL.to_string()),
        floxhub_token: config.flox.floxhub_token.clone(),
        extra_headers,
        mock_mode,
        auth_method: config.flox.floxhub_authn_mode.clone(),
    };

    debug!(
        "using catalog client with url: {}",
        client_config.catalog_url
    );
    Ok(CatalogClient::new(client_config).into())
}
