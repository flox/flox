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

/// Initialize the Catalog API client
///
/// - Return [None] if the Catalog API is disabled through the feature flag
/// - Initialize a mock client if the `_FLOX_USE_CATALOG_MOCK` environment variable is set to `true`
/// - Initialize a real client otherwise
pub fn init_catalog_client(config: &Config) -> Result<Option<Client>, anyhow::Error> {
    // Do not initialize a client if the Catalog API is disabled
    if !config.features.clone().unwrap_or_default().use_catalog {
        debug!("catalog feature is disabled, skipping client initialization");
        return Ok(None);
    }

    // if $_FLOX_USE_CATALOG_MOCK is set to a path to mock data, use the mock client
    let mock_data_path = if let Ok(path_str) = std::env::var(FLOX_CATALOG_MOCK_DATA_VAR) {
        let path = PathBuf::from(path_str);
        if path.exists() {
            Some(path)
        } else {
            bail!("path to mock data file doesn't exist: {}", path.display());
        }
    } else {
        None
    };
    if mock_data_path.is_some() {
        debug!(
            mock_data_path = mock_data_path.clone().map(|p| traceable_path(&p)),
            "using mock catalog client"
        );
        Ok(Some(Client::Mock(MockClient::new(
            mock_data_path.as_ref(),
        )?)))
    } else {
        debug!("using production catalog client");
        Ok(Some(Client::Catalog(CatalogClient::default())))
    }
}
