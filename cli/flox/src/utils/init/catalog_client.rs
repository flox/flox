use flox_rust_sdk::providers::catalog::{CatalogClient, Client, MockClient};
use tracing::debug;

use crate::config::Config;

/// Initialize the Catalog API client
///
/// - Return [None] if the Catalog API is disabled through the feature flag
/// - Initialize a mock client if the `_FLOX_USE_CATALOG_MOCK` environment variable is set to `true`
/// - Initialize a real client otherwise
pub fn init_catalog_client(config: &Config) -> Option<Client> {
    // Do not initialize a client if the Catalog API is disabled
    if !config.features.clone().unwrap_or_default().use_catalog {
        debug!("catalog feature is disabled, skipping client initialization");
        return None;
    }

    // if $_FLOX_USE_CATALOG_MOCK is set to 'true', use the mock client
    let use_mock = std::env::var("_FLOX_USE_CATALOG_MOCK").is_ok_and(|val| val == "true");
    if use_mock {
        debug!("Using mock catalog client");
        // TODO: setup the "runtime" mock client, e.g. from a file
        Some(MockClient.into())
    } else {
        debug!("Using catalog client");
        Some(Client::Catalog(CatalogClient::default()))
    }
}
