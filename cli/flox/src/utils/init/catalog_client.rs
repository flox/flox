use std::collections::BTreeMap;
use std::path::PathBuf;

use flox_catalog::{CatalogClient, CatalogClientConfig, CatalogMockMode};
use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::providers::catalog::{
    Client,
    DEFAULT_CATALOG_URL,
    FLOX_CATALOG_DUMP_DATA_VAR,
    FLOX_CATALOG_MOCK_DATA_VAR,
};
use tracing::debug;

use crate::config::Config;
use crate::utils::metrics::read_metrics_uuid;

/// Initialize the Catalog API client
///
/// - Initialize a mock client if the `_FLOX_USE_CATALOG_MOCK` environment variable is set to `true`
/// - Initialize a real client otherwise
pub fn init_catalog_client(config: &Config) -> Result<Client, anyhow::Error> {
    let extra_headers = {
        let mut headers = BTreeMap::new();
        // Propagate the metrics UUID to catalog-server if metrics are enabled.
        if !config.flox.disable_metrics {
            headers.insert(
                "flox-device-uuid".to_string(),
                read_metrics_uuid(config).unwrap().to_string(),
            );
        }
        headers
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
        user_agent: Some(format!("flox-cli/{}", &*FLOX_VERSION)),
    };

    debug!(
        "using catalog client with url: {}",
        client_config.catalog_url
    );
    Ok(CatalogClient::new(client_config)?.into())
}
