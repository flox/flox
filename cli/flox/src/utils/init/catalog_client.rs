use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::bail;
use flox_rust_sdk::providers::catalog::{
    CatalogClient,
    CatalogClientConfig,
    Client,
    FLOX_CATALOG_MOCK_DATA_VAR,
    MockClient,
};
use flox_rust_sdk::utils::logging::traceable_path;
use tracing::debug;

use crate::config::Config;
use crate::utils::metrics::read_metrics_uuid;

pub const DEFAULT_CATALOG_URL: &str = "https://api.flox.dev";

/// Initialize the Catalog API client
///
/// - Return [None] if the Catalog API is disabled through the feature flag
/// - Initialize a mock client if the `_FLOX_USE_CATALOG_MOCK` environment variable is set to `true`
/// - Initialize a real client otherwise
pub fn init_catalog_client(config: &Config) -> Result<Client, anyhow::Error> {
    // if $_FLOX_USE_CATALOG_MOCK is set to a path to mock data, use the mock client
    if let Ok(path_str) = std::env::var(FLOX_CATALOG_MOCK_DATA_VAR) {
        let path = PathBuf::from(path_str);
        if !path.exists() {
            bail!("path to mock data file doesn't exist: {}", path.display());
        }

        debug!(
            mock_data_path = traceable_path(&path),
            "using mock catalog client"
        );
        Ok(MockClient::new(Some(path))?.into())
    } else {
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

        let client_config = CatalogClientConfig {
            catalog_url: config
                .flox
                .catalog_url
                .clone()
                .unwrap_or_else(|| DEFAULT_CATALOG_URL.to_string()),
            floxhub_token: config.flox.floxhub_token.clone(),
            extra_headers,
        };

        debug!(
            "using catalog client with url: {}",
            client_config.catalog_url
        );
        Ok(CatalogClient::new(client_config).into())
    }
}
