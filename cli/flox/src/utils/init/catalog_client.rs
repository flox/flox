use std::collections::BTreeMap;
use std::path::PathBuf;

use flox_rust_sdk::providers::catalog::{
    CatalogClient,
    CatalogClientConfig,
    CatalogMockMode,
    Client,
    FLOX_CATALOG_DUMP_DATA_VAR,
    FLOX_CATALOG_MOCK_DATA_VAR,
};
use tracing::debug;

use crate::config::Config;
use crate::utils::metrics::read_metrics_uuid;

pub const DEFAULT_CATALOG_URL: &str = "https://api.flox.dev";

/// Initialize the Catalog API client
///
/// - Initialize a mock client if the `_FLOX_USE_CATALOG_MOCK` environment variable is set to `true`
/// - Initialize a real client otherwise
pub fn init_catalog_client(config: &Config) -> Result<Client, anyhow::Error> {
    let extra_headers = {
        // If metrics are not disabled, pass along the metrics UUID so it can be
        // sent in catalog request headers, as well as the Sentry span info
        if !config.flox.disable_metrics {
            let mut metrics_headers = BTreeMap::new();
            metrics_headers.insert(
                "flox-device-uuid".to_string(),
                read_metrics_uuid(config).unwrap().to_string(),
            );

            if let Some(span) = sentry::configure_scope(|scope| scope.get_span()) {
                for (k, v) in span.iter_headers() {
                    metrics_headers.insert(k.to_string(), v);
                }
            }
            metrics_headers
        } else {
            Default::default()
        }
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
    };

    debug!(
        "using catalog client with url: {}",
        client_config.catalog_url
    );
    Ok(CatalogClient::new(client_config).into())
}
