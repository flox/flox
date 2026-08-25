//! Per-deployment login discovery.
//!
//! A FloxHub deployment advertises where the CLI should log in: an
//! unauthenticated RFC 9728 protected-resource metadata document at
//! `{floxhub_url}/.well-known/oauth-protected-resource` names the
//! deployment's OIDC issuer (`authorization_servers`, exactly one entry),
//! the CLI's public client id (`client_id`, an extension member), and
//! optionally the `audience` value to send with the device-code request
//! (an Auth0 extension; absent on Dex deployments). The issuer's own
//! OIDC discovery then supplies the device-authorization and token
//! endpoints. FloxHub is not in the token path: after discovery the CLI
//! speaks standard OIDC directly to the issuer.
//!
//! Deployments without the document — today's SaaS until it is deployed
//! there, and forks that never serve it — answer the probe with a 404 or
//! with the web UI's `index.html`, which [`discover_login_config`]
//! reports as [`None`] so the caller can fall back to its compiled-in
//! login configuration.

use std::time::Duration;

use serde::Deserialize;
use thiserror::Error;
use tracing::debug;
use url::Url;

/// RFC 8628 grant type identifier for the device authorization grant.
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Bound on each discovery request. Login is interactive, so a hung probe
/// must fail rather than hang the prompt.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);

/// The login configuration a deployment advertises, fully resolved:
/// members of the protected-resource document plus the endpoints from the
/// issuer's OIDC discovery.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveredLoginConfig {
    /// The CLI's public client id at this deployment's issuer.
    pub client_id: String,
    /// Auth0 `audience` parameter for the device-code request; sent
    /// exactly when advertised.
    pub audience: Option<String>,
    pub device_authorization_endpoint: Url,
    pub token_endpoint: Url,
}

/// Why discovery failed. Absence of the document is not a failure — see
/// [`discover_login_config`] — so every variant means the deployment
/// advertised a login configuration that could not be used.
#[derive(Debug, Error)]
pub enum LoginDiscoveryError {
    #[error("could not fetch the login configuration from {url}")]
    ProbeConnection {
        url: Url,
        #[source]
        source: reqwest::Error,
    },
    #[error(
        "the login configuration advertises {count} authorization servers; expected exactly one"
    )]
    NotExactlyOneAuthorizationServer { count: usize },
    #[error("the login configuration does not name a client id")]
    MissingClientId,
    #[error("could not fetch OIDC discovery from {url}")]
    OidcDiscovery {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error(
        "the OIDC issuer reports itself as '{reported}' but the login configuration advertises '{advertised}'"
    )]
    IssuerMismatch {
        advertised: String,
        reported: String,
    },
    #[error("the OIDC issuer '{issuer}' does not advertise the device authorization grant")]
    DeviceGrantNotSupported { issuer: String },
    #[error("the OIDC issuer '{issuer}' does not advertise a token endpoint")]
    MissingTokenEndpoint { issuer: String },
}

/// The members of the protected-resource document the CLI consumes.
/// `authorization_servers` doubles as the presence test: a 2xx body that
/// deserializes with it present is the document, anything else is
/// absence.
#[derive(Debug, Deserialize)]
struct ProtectedResourceMetadata {
    authorization_servers: Vec<String>,
    client_id: Option<String>,
    audience: Option<String>,
}

/// The members of the issuer's OIDC discovery document the CLI consumes.
#[derive(Debug, Deserialize)]
struct OidcProviderMetadata {
    issuer: String,
    token_endpoint: Option<Url>,
    device_authorization_endpoint: Option<Url>,
    grant_types_supported: Option<Vec<String>>,
}

/// Discover where to log in against the deployment at `floxhub_url`.
///
/// Returns `Ok(None)` when the deployment does not advertise a login
/// configuration: the probe answered with anything other than a 2xx
/// protected-resource document (404, the web UI's SPA fallback, an error
/// page). Only a connection-level failure of the probe is an error — a
/// login against an unreachable FloxHub cannot complete anyway, because
/// the handle resolves from the same server.
///
/// A present document that cannot be used — no client id, more than one
/// authorization server, an issuer whose OIDC discovery fails or that
/// does not offer the device grant — is an error rather than absence:
/// falling back to the compiled-in configuration would silently log the
/// user in against the wrong deployment's issuer.
pub async fn discover_login_config(
    floxhub_url: &Url,
) -> Result<Option<DiscoveredLoginConfig>, LoginDiscoveryError> {
    let client = reqwest::Client::builder()
        .timeout(DISCOVERY_TIMEOUT)
        .build()
        .expect("failed to build discovery HTTP client");

    let probe_url = append_path_segments(floxhub_url, &[".well-known", "oauth-protected-resource"]);
    let response = client.get(probe_url.clone()).send().await.map_err(|err| {
        LoginDiscoveryError::ProbeConnection {
            url: probe_url.clone(),
            source: err,
        }
    })?;

    if !response.status().is_success() {
        debug!(url = %probe_url, status = %response.status(), "no login configuration served");
        return Ok(None);
    }
    let Ok(document) = response.json::<ProtectedResourceMetadata>().await else {
        debug!(url = %probe_url, "response is not a login configuration document");
        return Ok(None);
    };
    debug!(url = %probe_url, ?document, "fetched login configuration");

    let [issuer] = document.authorization_servers.as_slice() else {
        return Err(LoginDiscoveryError::NotExactlyOneAuthorizationServer {
            count: document.authorization_servers.len(),
        });
    };
    let client_id = document
        .client_id
        .ok_or(LoginDiscoveryError::MissingClientId)?;

    // Append-form only (RFC 8414 path-insertion does not exist on a
    // path-bearing issuer like Dex's `https://<host>/dex`).
    let oidc_url = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );
    let oidc = async {
        client
            .get(&oidc_url)
            .send()
            .await?
            .error_for_status()?
            .json::<OidcProviderMetadata>()
            .await
    }
    .await
    .map_err(|err| LoginDiscoveryError::OidcDiscovery {
        url: oidc_url.clone(),
        source: err,
    })?;
    debug!(url = %oidc_url, ?oidc, "fetched OIDC provider metadata");

    // The issuer must vouch for itself, modulo the trailing slash: Auth0
    // self-reports with one, and the advertisement may carry either form.
    if oidc.issuer.trim_end_matches('/') != issuer.trim_end_matches('/') {
        return Err(LoginDiscoveryError::IssuerMismatch {
            advertised: issuer.clone(),
            reported: oidc.issuer,
        });
    }

    // The endpoint's presence advertises the grant; grant_types_supported
    // is only a veto because RFC 8414 makes the member optional (Auth0
    // omits it while supporting the grant).
    let device_grant_listed = oidc
        .grant_types_supported
        .is_none_or(|grants| grants.iter().any(|grant| grant == DEVICE_CODE_GRANT));
    let device_authorization_endpoint = match oidc.device_authorization_endpoint {
        Some(endpoint) if device_grant_listed => endpoint,
        _ => {
            return Err(LoginDiscoveryError::DeviceGrantNotSupported {
                issuer: issuer.clone(),
            });
        },
    };
    let token_endpoint =
        oidc.token_endpoint
            .ok_or_else(|| LoginDiscoveryError::MissingTokenEndpoint {
                issuer: issuer.clone(),
            })?;

    Ok(Some(DiscoveredLoginConfig {
        client_id,
        audience: document.audience,
        device_authorization_endpoint,
        token_endpoint,
    }))
}

/// Append `segments` to `base`'s path, preserving any existing path prefix
/// (`https://host/floxhub` + `.well-known/x` → `https://host/floxhub/.well-known/x`).
/// `Url::join` would instead replace the last segment.
fn append_path_segments(base: &Url, segments: &[&str]) -> Url {
    let mut url = base.clone();
    url.path_segments_mut()
        .expect("FloxHub URLs are http(s) and can be a base")
        .pop_if_empty()
        .extend(segments);
    url
}

#[cfg(test)]
mod tests {
    use httpmock::MockServer;
    use serde_json::json;

    use super::*;

    const PROBE_PATH: &str = "/.well-known/oauth-protected-resource";

    /// Serve a document at `probe_path` advertising `issuer`, and the
    /// issuer's OIDC discovery under the issuer's own path.
    fn mock_deployment(
        server: &MockServer,
        document: serde_json::Value,
        issuer_path: &str,
        oidc_overrides: serde_json::Value,
    ) {
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PROBE_PATH);
            then.status(200).json_body(document);
        });
        let issuer = server.url(issuer_path);
        let mut oidc = json!({
            "issuer": issuer,
            "token_endpoint": format!("{issuer}/token"),
            "device_authorization_endpoint": format!("{issuer}/device/code"),
        });
        oidc.as_object_mut()
            .unwrap()
            .extend(oidc_overrides.as_object().unwrap().clone());
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path(format!("{issuer_path}/.well-known/openid-configuration"));
            then.status(200).json_body(oidc);
        });
    }

    fn base_url(server: &MockServer) -> Url {
        Url::parse(&server.base_url()).unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn absent_on_404() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PROBE_PATH);
            then.status(404);
        });

        let result = discover_login_config(&base_url(&server)).await.unwrap();
        assert_eq!(result, None);
    }

    /// Deployments without the document fall through to the web UI, which
    /// answers 200 with the SPA's index.html.
    #[tokio::test(flavor = "multi_thread")]
    async fn absent_on_spa_fallback_html() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PROBE_PATH);
            then.status(200)
                .header("content-type", "text/html")
                .body("<!doctype html><html><body>FloxHub</body></html>");
        });

        let result = discover_login_config(&base_url(&server)).await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn absent_on_json_that_is_not_the_document() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PROBE_PATH);
            then.status(200).json_body(json!({"detail": "who dis"}));
        });

        let result = discover_login_config(&base_url(&server)).await.unwrap();
        assert_eq!(result, None);
    }

    /// A non-2xx answer is absence even when the body looks like the
    /// document: only a successful response advertises a configuration.
    #[tokio::test(flavor = "multi_thread")]
    async fn absent_on_error_status_with_document_body() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PROBE_PATH);
            then.status(500).json_body(json!({
                "authorization_servers": ["https://issuer.example"],
                "client_id": "cli",
            }));
        });

        let result = discover_login_config(&base_url(&server)).await.unwrap();
        assert_eq!(result, None);
    }

    /// A connection failure is an error, not absence: nothing was learned
    /// about the deployment, and a login against an unreachable FloxHub
    /// cannot complete anyway.
    #[tokio::test(flavor = "multi_thread")]
    async fn errors_when_floxhub_is_unreachable() {
        // Bind to learn a free port, then drop the listener so nothing
        // answers on it.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let url = Url::parse(&format!("http://127.0.0.1:{port}")).unwrap();

        let err = discover_login_config(&url).await.unwrap_err();
        assert!(matches!(err, LoginDiscoveryError::ProbeConnection { .. }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn errors_on_multiple_authorization_servers() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PROBE_PATH);
            then.status(200).json_body(json!({
                "authorization_servers": ["https://a.example", "https://b.example"],
                "client_id": "cli",
            }));
        });

        let err = discover_login_config(&base_url(&server)).await.unwrap_err();
        assert!(matches!(
            err,
            LoginDiscoveryError::NotExactlyOneAuthorizationServer { count: 2 }
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn errors_on_empty_authorization_servers() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PROBE_PATH);
            then.status(200).json_body(json!({
                "authorization_servers": [],
                "client_id": "cli",
            }));
        });

        let err = discover_login_config(&base_url(&server)).await.unwrap_err();
        assert!(matches!(
            err,
            LoginDiscoveryError::NotExactlyOneAuthorizationServer { count: 0 }
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn errors_on_missing_client_id() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PROBE_PATH);
            then.status(200).json_body(json!({
                "authorization_servers": ["https://issuer.example"],
            }));
        });

        let err = discover_login_config(&base_url(&server)).await.unwrap_err();
        assert!(matches!(err, LoginDiscoveryError::MissingClientId));
    }

    /// The Auth0-shaped happy path: root issuer self-reporting with a
    /// trailing slash, audience advertised.
    #[tokio::test(flavor = "multi_thread")]
    async fn discovers_auth0_shaped_deployment() {
        let server = MockServer::start();
        let issuer_with_slash = format!("{}/", server.base_url());
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PROBE_PATH);
            then.status(200).json_body(json!({
                "resource": "https://api.flox.example",
                "authorization_servers": [issuer_with_slash],
                "scopes_supported": ["openid", "profile", "email"],
                "client_id": "auth0-cli-client",
                "audience": "https://hub.flox.example/api",
            }));
        });
        let issuer = server.base_url();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/.well-known/openid-configuration");
            then.status(200).json_body(json!({
                "issuer": format!("{issuer}/"),
                "token_endpoint": format!("{issuer}/oauth/token"),
                "device_authorization_endpoint": format!("{issuer}/oauth/device/code"),
            }));
        });

        let config = discover_login_config(&base_url(&server))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(config, DiscoveredLoginConfig {
            client_id: "auth0-cli-client".to_string(),
            audience: Some("https://hub.flox.example/api".to_string()),
            device_authorization_endpoint: Url::parse(&format!("{issuer}/oauth/device/code"))
                .unwrap(),
            token_endpoint: Url::parse(&format!("{issuer}/oauth/token")).unwrap(),
        });
    }

    /// The Dex-shaped happy path: a path-bearing issuer (append-form
    /// discovery URL), no audience, device grant listed explicitly.
    #[tokio::test(flavor = "multi_thread")]
    async fn discovers_dex_shaped_deployment() {
        let server = MockServer::start();
        let issuer = server.url("/dex");
        mock_deployment(
            &server,
            json!({
                "resource": server.base_url(),
                "authorization_servers": [issuer],
                "client_id": "floxhub-cli",
            }),
            "/dex",
            json!({
                "grant_types_supported": ["authorization_code", DEVICE_CODE_GRANT],
            }),
        );

        let config = discover_login_config(&base_url(&server))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(config, DiscoveredLoginConfig {
            client_id: "floxhub-cli".to_string(),
            audience: None,
            device_authorization_endpoint: Url::parse(&format!("{issuer}/device/code")).unwrap(),
            token_endpoint: Url::parse(&format!("{issuer}/token")).unwrap(),
        });
    }

    /// The probe preserves a path prefix on the FloxHub URL, as an on-prem
    /// deployment served under one may carry.
    #[tokio::test(flavor = "multi_thread")]
    async fn probes_under_a_path_prefixed_floxhub_url() {
        let server = MockServer::start();
        let issuer = server.url("/dex");
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/floxhub/.well-known/oauth-protected-resource");
            then.status(200).json_body(json!({
                "authorization_servers": [issuer],
                "client_id": "floxhub-cli",
            }));
        });
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/dex/.well-known/openid-configuration");
            then.status(200).json_body(json!({
                "issuer": issuer,
                "token_endpoint": format!("{issuer}/token"),
                "device_authorization_endpoint": format!("{issuer}/device/code"),
            }));
        });

        let floxhub_url = Url::parse(&server.url("/floxhub")).unwrap();
        let config = discover_login_config(&floxhub_url).await.unwrap().unwrap();
        assert_eq!(config.client_id, "floxhub-cli");
    }

    /// Advertised without a trailing slash, self-reported with one (and
    /// vice versa): both normalize to the same issuer.
    #[tokio::test(flavor = "multi_thread")]
    async fn issuer_check_normalizes_the_trailing_slash() {
        let server = MockServer::start();
        let issuer = server.url("/dex");
        mock_deployment(
            &server,
            json!({
                "authorization_servers": [format!("{issuer}/")],
                "client_id": "floxhub-cli",
            }),
            "/dex",
            json!({}),
        );

        let config = discover_login_config(&base_url(&server)).await.unwrap();
        assert!(config.is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn errors_when_the_issuer_reports_a_different_identity() {
        let server = MockServer::start();
        mock_deployment(
            &server,
            json!({
                "authorization_servers": [server.url("/dex")],
                "client_id": "floxhub-cli",
            }),
            "/dex",
            json!({"issuer": "https://impostor.example/dex"}),
        );

        let err = discover_login_config(&base_url(&server)).await.unwrap_err();
        assert!(matches!(err, LoginDiscoveryError::IssuerMismatch { .. }));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn errors_when_the_issuer_omits_the_device_endpoint() {
        let server = MockServer::start();
        let issuer = server.url("/dex");
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PROBE_PATH);
            then.status(200).json_body(json!({
                "authorization_servers": [issuer],
                "client_id": "floxhub-cli",
            }));
        });
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/dex/.well-known/openid-configuration");
            then.status(200).json_body(json!({
                "issuer": issuer,
                "token_endpoint": format!("{issuer}/token"),
            }));
        });

        let err = discover_login_config(&base_url(&server)).await.unwrap_err();
        assert!(matches!(
            err,
            LoginDiscoveryError::DeviceGrantNotSupported { .. }
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn errors_when_grant_types_exclude_the_device_grant() {
        let server = MockServer::start();
        mock_deployment(
            &server,
            json!({
                "authorization_servers": [server.url("/dex")],
                "client_id": "floxhub-cli",
            }),
            "/dex",
            json!({"grant_types_supported": ["authorization_code"]}),
        );

        let err = discover_login_config(&base_url(&server)).await.unwrap_err();
        assert!(matches!(
            err,
            LoginDiscoveryError::DeviceGrantNotSupported { .. }
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn errors_when_oidc_discovery_is_unavailable() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PROBE_PATH);
            then.status(200).json_body(json!({
                "authorization_servers": [server.url("/dex")],
                "client_id": "floxhub-cli",
            }));
        });
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/dex/.well-known/openid-configuration");
            then.status(404);
        });

        let err = discover_login_config(&base_url(&server)).await.unwrap_err();
        assert!(matches!(err, LoginDiscoveryError::OidcDiscovery { .. }));
    }
}
