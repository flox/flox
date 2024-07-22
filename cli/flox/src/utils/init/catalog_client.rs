use std::path::PathBuf;

use anyhow::bail;
use flox_rust_sdk::providers::catalog::{
    CatalogClient,
    Client,
    MockClient,
    FLOX_CATALOG_MOCK_DATA_VAR,
};
use flox_rust_sdk::utils::traceable_path;
use tracing::debug;

use crate::config::Config;
use crate::utils::metrics::read_metrics_uuid;

/// Initialize the Catalog API client
///
/// - Return [None] if the Catalog API is disabled through the feature flag
/// - Initialize a mock client if the `_FLOX_USE_CATALOG_MOCK` environment variable is set to `true`
/// - Initialize a real client otherwise
pub fn init_catalog_client(config: &Config) -> Result<Option<Client>, anyhow::Error> {
    // Do not initialize a client if the Catalog API is disabled
    if !*config.features.clone().unwrap_or_default().use_catalog {
        debug!("catalog feature is disabled, skipping client initialization");
        return Ok(None);
    }

    // If metrics are not disabled, pass along the metrics UUID so it can be
    // sent in catalog request headers.
    let device_uuid_for_catalog = if config.flox.disable_metrics {
        Option::None
    } else {
        Some(read_metrics_uuid(config).unwrap())
    };
    // Pass in a bool if we are running in CI, so requests can reflect this in the headers
    let running_in_ci = std::env::var("CI").is_ok();

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
        Ok(Some(Client::Mock(MockClient::new(Some(path))?)))
    } else if let Some(ref catalog_url) = config.flox.catalog_url {
        debug!("using catalog client with url: {}", catalog_url);
        Ok(Some(Client::Catalog(CatalogClient::new(
            Some(catalog_url),
            device_uuid_for_catalog,
            running_in_ci,
        ))))
    } else {
        debug!("using production catalog client");
        Ok(Some(Client::Catalog(CatalogClient::new(
            None,
            device_uuid_for_catalog,
            running_in_ci,
        ))))
    }
}
