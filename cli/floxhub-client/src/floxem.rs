//! The environment manager (floxEM) surface of
//! [`FloxhubClient`](crate::FloxhubClient), hand-written.
//!
//! `GET /api/v1/environment/{owner}/{name}/automatic-upgrades` reports
//! FloxHub's automatic upgrade settings for an environment.
//!
//! TODO: generate this client from floxEM's OpenAPI schema
//! (`floxem_oas302.json`, a 3.0.2 schema emitted for generated clients) and
//! replace the hand-written request, once the CLI grows beyond this one
//! endpoint.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, SubsecRound, Utc};
use reqwest::StatusCode;
use reqwest::header::HeaderValue;
use serde::{Deserialize, Deserializer};
use thiserror::Error;
use url::Url;

use crate::auth::AuthContext;

/// floxEM reads the caller's identity from this header;
/// it ignores `Authorization`.
const FLOX_GITHUB_TOKEN_HEADER: &str = "x-flox-github-token";

/// Callers show the settings as a detail and carry on without them,
/// so don't wait long for them.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

/// FloxHub's automatic upgrade settings for an environment.
///
/// Every field has a default, so responses from older servers that lack a
/// field still parse.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AutomaticUpgradesResponse {
    /// `false` when the environment has no upgrade schedule
    pub enabled: bool,
    /// The schedule, such as `weekly`, as an opaque name.
    /// `None` when the environment has no upgrade schedule.
    pub cadence: Option<String>,
    /// The next UTC date the cadence is due
    pub next_due_date: Option<NaiveDate>,
    /// The newest upgrade attempt FloxHub made on its own
    pub last_automatic_upgrade: Option<LastAutomaticUpgrade>,
    /// The environment FloxHub upgrades this one from,
    /// if the caller can read it
    pub upgrade_source: Option<UpgradeSource>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastAutomaticUpgrade {
    /// Truncated to whole seconds, like the timestamps of generations
    #[serde(deserialize_with = "deserialize_whole_seconds")]
    pub timestamp: DateTime<Utc>,
    pub result: AutomaticUpgradeResult,
    /// The generation an upgrade created; `None` for other results
    #[serde(default)]
    pub generation: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutomaticUpgradeResult {
    Upgrade,
    /// No upgrades were available
    Noop,
    Failure,
    /// FloxHub failed to run the upgrade
    Error,
    /// A result this client doesn't know about yet
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct UpgradeSource {
    pub owner: String,
    pub name: String,
}

fn deserialize_whole_seconds<'de, D>(deserializer: D) -> Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(DateTime::<Utc>::deserialize(deserializer)?.trunc_subsecs(0))
}

#[derive(Debug, Error)]
pub enum AutomaticUpgradesRequestError {
    #[error("unexpected response from FloxHub ({0})")]
    UnexpectedStatus(StatusCode),
    #[error("could not reach FloxHub")]
    Request(#[source] reqwest::Error),
    #[error("could not parse the response from FloxHub")]
    Decode(#[source] serde_json::Error),
    #[error("invalid FloxHub URL '{0}'")]
    InvalidUrl(Url),
}

/// floxEM inner client, sharing the authentication hook of the catalog and
/// factory clients.
///
/// `http_client` must not follow redirects,
/// or the token in `X-Flox-Github-Token` could reach another host.
#[derive(Clone)]
pub struct FloxemApiClient {
    base_url: Url,
    http_client: reqwest::Client,
    auth_context: AuthContext,
    pre_request: Arc<dyn Fn(&mut reqwest::Request) + Send + Sync>,
}

impl fmt::Debug for FloxemApiClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FloxemApiClient")
            .field("base_url", &self.base_url.as_str())
            .finish_non_exhaustive()
    }
}

impl FloxemApiClient {
    pub(crate) fn new_with_client(
        base_url: Url,
        http_client: reqwest::Client,
        auth_context: AuthContext,
        pre_request: Arc<dyn Fn(&mut reqwest::Request) + Send + Sync>,
    ) -> Self {
        Self {
            base_url,
            http_client,
            auth_context,
            pre_request,
        }
    }

    /// Fetch FloxHub's automatic upgrade settings for `owner/name`.
    ///
    /// Only the `/api/v1` mount serves this route.
    pub async fn automatic_upgrades(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<AutomaticUpgradesResponse, AutomaticUpgradesRequestError> {
        let mut url = self.base_url.clone();
        url.path_segments_mut()
            .map_err(|()| AutomaticUpgradesRequestError::InvalidUrl(self.base_url.clone()))?
            .pop_if_empty()
            .extend([
                "api",
                "v1",
                "environment",
                owner,
                name,
                "automatic-upgrades",
            ]);

        // A per-request timeout bounds connecting as well as the response.
        let mut request = self
            .http_client
            .get(url)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(AutomaticUpgradesRequestError::Request)?;
        (self.pre_request)(&mut request);
        if let Some(secret) = self.auth_context.token_secret()
            && let Ok(mut value) = HeaderValue::from_str(secret)
        {
            value.set_sensitive(true);
            request
                .headers_mut()
                .insert(FLOX_GITHUB_TOKEN_HEADER, value);
        }

        let response = self
            .http_client
            .execute(request)
            .await
            .map_err(AutomaticUpgradesRequestError::Request)?;
        let status = response.status();
        if status != StatusCode::OK {
            return Err(AutomaticUpgradesRequestError::UnexpectedStatus(status));
        }
        let body = response
            .bytes()
            .await
            .map_err(AutomaticUpgradesRequestError::Request)?;
        serde_json::from_slice(&body).map_err(AutomaticUpgradesRequestError::Decode)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use httpmock::MockServer;
    use indoc::formatdoc;
    use serde_json::json;

    use super::*;
    use crate::FloxhubClient;
    use crate::client::test_helpers::client_config;
    use crate::config::{FloxhubClientConfig, FloxhubMockMode};

    const PATH: &str = "/floxem/api/v1/environment/owner/name/automatic-upgrades";

    fn client(server: &MockServer) -> FloxhubClient {
        FloxhubClient::new(FloxhubClientConfig {
            floxem_url: Url::parse(&server.url("/floxem")).unwrap(),
            auth_context: AuthContext::new_from_token(Some("flox_pat_secret")),
            ..client_config("http://localhost:0")
        })
        .unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn automatic_upgrades_parses_the_full_document() {
        let server = MockServer::start();
        // floxEM reads the credential from `x-flox-github-token`;
        // `authorization` comes from the hook shared with the other clients.
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path(PATH)
                .header("authorization", "bearer flox_pat_secret")
                .header("x-flox-github-token", "flox_pat_secret");
            then.status(200).json_body(json!({
                "enabled": true,
                "cadence": "weekly",
                "nextDueDate": "2026-10-05",
                "lastAutomaticUpgrade": {
                    "timestamp": "2026-09-28T00:17:03.123456+00:00",
                    "result": "upgrade",
                    "generation": 7,
                },
                "upgradeSource": {"owner": "acme", "name": "app-qa"},
            }));
        });

        let response = client(&server)
            .floxem()
            .automatic_upgrades("owner", "name")
            .await
            .unwrap();

        mock.assert();
        assert_eq!(response, AutomaticUpgradesResponse {
            enabled: true,
            cadence: Some("weekly".to_string()),
            next_due_date: Some(NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()),
            last_automatic_upgrade: Some(LastAutomaticUpgrade {
                timestamp: Utc.with_ymd_and_hms(2026, 9, 28, 0, 17, 3).unwrap(),
                result: AutomaticUpgradeResult::Upgrade,
                generation: Some(7),
            }),
            upgrade_source: Some(UpgradeSource {
                owner: "acme".to_string(),
                name: "app-qa".to_string(),
            }),
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn automatic_upgrades_defaults_missing_fields_and_unknown_results() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PATH);
            then.status(200).json_body(json!({
                "enabled": false,
                "lastAutomaticUpgrade": {
                    "timestamp": "2026-09-28T02:17:03+02:00",
                    "result": "skipped",
                },
            }));
        });

        let response = client(&server)
            .floxem()
            .automatic_upgrades("owner", "name")
            .await
            .unwrap();

        assert_eq!(response, AutomaticUpgradesResponse {
            enabled: false,
            cadence: None,
            next_due_date: None,
            last_automatic_upgrade: Some(LastAutomaticUpgrade {
                timestamp: Utc.with_ymd_and_hms(2026, 9, 28, 0, 17, 3).unwrap(),
                result: AutomaticUpgradeResult::Unknown,
                generation: None,
            }),
            upgrade_source: None,
        });
    }

    /// Replay serves every surface from one mock server,
    /// so floxEM requests keep their `/floxem` prefix there.
    #[tokio::test(flavor = "multi_thread")]
    async fn automatic_upgrades_routes_through_mock_guard_in_replay_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let recording_path = tmp.path().join("floxem.yaml");
        let recording = formatdoc! {"
            when:
              method: GET
              path: {PATH}
            then:
              status: 200
              body: '{{\"enabled\":false}}'
        "};
        std::fs::write(&recording_path, recording).unwrap();

        let client = FloxhubClient::new(FloxhubClientConfig {
            mock_mode: FloxhubMockMode::Replay(recording_path),
            ..client_config("http://localhost:0")
        })
        .unwrap();
        let response = client
            .floxem()
            .automatic_upgrades("owner", "name")
            .await
            .unwrap();

        assert_eq!(response, AutomaticUpgradesResponse::default());
    }

    /// reqwest strips only its own sensitive headers on a redirect to
    /// another host, so a redirect must not carry `x-flox-github-token` there.
    #[tokio::test(flavor = "multi_thread")]
    async fn automatic_upgrades_does_not_follow_redirects() {
        let server = MockServer::start();
        let other_host = MockServer::start();
        let elsewhere = other_host.mock(|when, then| {
            when.any_request();
            then.status(200).json_body(json!({}));
        });
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PATH);
            then.status(302)
                .header("location", other_host.url("/login"));
        });

        let err = client(&server)
            .floxem()
            .automatic_upgrades("owner", "name")
            .await
            .unwrap_err();

        assert!(
            matches!(
                err,
                AutomaticUpgradesRequestError::UnexpectedStatus(StatusCode::FOUND)
            ),
            "{err:?}"
        );
        elsewhere.assert_calls(0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn automatic_upgrades_rejects_other_statuses_and_bodies() {
        let server = MockServer::start();
        let mut not_found = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PATH);
            then.status(404).json_body(json!({"detail": "Not Found"}));
        });
        let client = client(&server);

        let err = client
            .floxem()
            .automatic_upgrades("owner", "name")
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                AutomaticUpgradesRequestError::UnexpectedStatus(StatusCode::NOT_FOUND)
            ),
            "{err:?}"
        );

        not_found.delete();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PATH);
            then.status(200);
        });
        let err = client
            .floxem()
            .automatic_upgrades("owner", "name")
            .await
            .unwrap_err();
        assert!(
            matches!(err, AutomaticUpgradesRequestError::Decode(_)),
            "{err:?}"
        );
    }
}
