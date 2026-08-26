use std::collections::BTreeMap;

use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::utils::{HEADER_DEVICE_UUID, HEADER_INVOCATION_ID, INVOCATION_SOURCES};
use floxhub_client::{
    AuthContext,
    FloxhubClient,
    FloxhubClientConfig,
    FloxhubMockMode,
    UnauthenticatedResolveHook,
};
use tracing::debug;
use uuid::Uuid;

/// Initialize the FloxHub API client.
///
/// - Reads the catalog URL from config (defaults to the production catalog URL)
/// - Configures mock replay mode if `_FLOX_USE_CATALOG_MOCK` is set
/// - Includes device and invocation UUID headers when metrics are enabled
/// - Includes invocation-source headers when available
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
    invocation_id: Uuid,
    on_unauthenticated_resolve: Option<UnauthenticatedResolveHook>,
) -> Result<FloxhubClient, anyhow::Error> {
    let mut extra_headers = BTreeMap::new();

    // Propagate the metrics and invocation UUIDs when metrics are enabled.
    if let Some(uuid) = metrics_device_uuid {
        extra_headers.insert(HEADER_DEVICE_UUID.to_string(), uuid.to_string());
        extra_headers.insert(HEADER_INVOCATION_ID.to_string(), invocation_id.to_string());
    }
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
        on_unauthenticated_resolve,
    };

    debug!("using catalog client with url: {}", client_config.base_url);
    Ok(FloxhubClient::new(client_config)?)
}

#[cfg(test)]
mod tests {
    use floxhub_client::CatalogClientTrait;
    use httpmock::MockServer;

    use super::*;

    #[tokio::test]
    async fn catalog_requests_include_the_cli_invocation_id() {
        let invocation_id = Uuid::nil();
        let server = MockServer::start_async().await;
        let request = server.mock(|when, then| {
            when.header("flox-invocation-id", invocation_id.to_string());
            then.status(500);
        });

        let client = init_floxhub_client(
            server.base_url(),
            AuthContext::new_from_token(None),
            Some(Uuid::new_v4()),
            invocation_id,
            None,
        )
        .expect("client initializes");
        let _ = client.package_versions("hello").await;

        request.assert();
    }

    #[tokio::test]
    async fn catalog_requests_omit_the_cli_invocation_id_when_metrics_are_disabled() {
        let server = MockServer::start_async().await;
        let request = server.mock(|when, then| {
            when.header_missing("flox-invocation-id");
            then.status(500);
        });

        let client = init_floxhub_client(
            server.base_url(),
            AuthContext::new_from_token(None),
            None,
            Uuid::nil(),
            None,
        )
        .expect("client initializes");
        let _ = client.package_versions("hello").await;

        request.assert();
    }
}
