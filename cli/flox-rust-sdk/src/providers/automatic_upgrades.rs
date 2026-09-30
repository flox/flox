//! FloxHub's automatic upgrade settings for an environment.
//!
//! The settings are fetched from FloxHub by `flox list --all --upstream` and
//! by the upgrade check that `flox activate` runs in the background, and are
//! cached so that `flox list --all` shows them without contacting FloxHub.
//!
//! The cache belongs to the environment on FloxHub rather than to a local
//! copy, so every copy of the environment shows the settings of the last
//! fetch. Each fetch replaces the whole file atomically, so concurrent
//! writers need no lock: the last one wins.

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, NaiveDate, SubsecRound, Utc};
use flox_core::data::environment_ref::{RemoteEnvironmentRef, RemoteEnvironmentRefError};
use flox_core::{WriteError, write_atomically};
use floxhub_client::{
    AutomaticUpgradeResult as WireResult,
    AutomaticUpgradesRequestError,
    AutomaticUpgradesResponse,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;
use url::Url;

use crate::flox::Flox;
use crate::models::environment::generations::GenerationId;
use crate::models::environment::{ConcreteEnvironment, ManagedPointer};

const CACHE_FILE_NAME: &str = "automatic-upgrades.json";

/// FloxHub's automatic upgrade settings for an environment
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutomaticUpgrades {
    /// Whether FloxHub upgrades the environment on a schedule
    pub enabled: bool,
    /// The schedule, such as `weekly`, as an opaque name that FloxHub may
    /// extend. `None` when the environment has no schedule.
    pub cadence: Option<String>,
    /// The next UTC date the cadence is due
    pub next_due_date: Option<NaiveDate>,
    /// The newest upgrade FloxHub attempted on its own
    pub last_run: Option<AutomaticUpgradeRun>,
    /// The environment FloxHub upgrades this one from, instead of the catalog
    pub upgrade_source: Option<RemoteEnvironmentRef>,
}

/// An upgrade FloxHub attempted on its own
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AutomaticUpgradeRun {
    pub timestamp: DateTime<Utc>,
    pub result: AutomaticUpgradeResult,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomaticUpgradeResult {
    /// The upgrade created a generation
    Upgrade(GenerationId),
    /// No upgrades were available
    Noop,
    Failure,
    /// FloxHub failed to run the upgrade
    Error,
    /// A result this version of Flox doesn't know about
    Unknown,
}

/// The automatic upgrade settings of the last fetch
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CachedAutomaticUpgrades {
    /// The FloxHub the settings were fetched from.
    /// The cache is keyed by owner and name only,
    /// which another FloxHub may reuse for a different environment.
    pub floxhub_url: Url,
    pub fetched_at: DateTime<Utc>,
    pub state: AutomaticUpgradesState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", content = "settings", rename_all = "snake_case")]
pub enum AutomaticUpgradesState {
    Fetched(AutomaticUpgrades),
    /// FloxHub answered without the settings,
    /// for example because it's too old to serve them
    /// or the credential can't read them.
    Unavailable,
}

impl TryFrom<AutomaticUpgradesResponse> for AutomaticUpgrades {
    type Error = RemoteEnvironmentRefError;

    fn try_from(response: AutomaticUpgradesResponse) -> Result<Self, Self::Error> {
        let upgrade_source = response
            .upgrade_source
            .map(|source| {
                Ok::<_, RemoteEnvironmentRefError>(RemoteEnvironmentRef::from_parts(
                    source.owner.parse()?,
                    source.name.parse()?,
                ))
            })
            .transpose()?;
        let last_run = response.last_automatic_upgrade.map(|run| {
            let generation = run
                .generation
                .and_then(|generation| usize::try_from(generation).ok())
                .map(GenerationId::from);
            let result = match (run.result, generation) {
                (WireResult::Upgrade, Some(generation)) => {
                    AutomaticUpgradeResult::Upgrade(generation)
                },
                // An upgrade that names no generation can't be matched
                // against the environment's history.
                (WireResult::Upgrade, None) | (WireResult::Unknown, _) => {
                    AutomaticUpgradeResult::Unknown
                },
                (WireResult::Noop, _) => AutomaticUpgradeResult::Noop,
                (WireResult::Failure, _) => AutomaticUpgradeResult::Failure,
                (WireResult::Error, _) => AutomaticUpgradeResult::Error,
            };
            AutomaticUpgradeRun {
                timestamp: run.timestamp,
                result,
            }
        });
        Ok(Self {
            enabled: response.enabled,
            cadence: response.cadence,
            next_due_date: response.next_due_date,
            last_run,
            upgrade_source,
        })
    }
}

/// Fetch the automatic upgrade settings of a FloxHub environment and cache
/// them.
///
/// Returns `Ok(None)` for path environments and for environments on a
/// FloxHub other than the configured one, whose credential must not be sent
/// elsewhere.
/// A response without the settings is cached as
/// [AutomaticUpgradesState::Unavailable].
/// If FloxHub can't be reached, the cache is left as it is.
pub async fn refresh(
    flox: &Flox,
    environment: &ConcreteEnvironment,
) -> Result<Option<CachedAutomaticUpgrades>, AutomaticUpgradesRequestError> {
    let Some(pointer) = configured_floxhub_pointer(flox, environment) else {
        return Ok(None);
    };

    let response = flox
        .floxhub_client
        .floxem()
        .automatic_upgrades(pointer.owner.as_str(), &pointer.name.to_string())
        .await;
    let state = match response {
        Ok(response) => match AutomaticUpgrades::try_from(response) {
            Ok(settings) => AutomaticUpgradesState::Fetched(settings),
            Err(err) => {
                debug!(error = %err, "FloxHub returned an invalid upgrade source");
                AutomaticUpgradesState::Unavailable
            },
        },
        Err(
            err @ (AutomaticUpgradesRequestError::UnexpectedStatus(_)
            | AutomaticUpgradesRequestError::Decode(_)),
        ) => {
            debug!(error = %err, "FloxHub didn't provide automatic upgrade settings");
            AutomaticUpgradesState::Unavailable
        },
        Err(err) => return Err(err),
    };

    let cached = CachedAutomaticUpgrades {
        floxhub_url: pointer.floxhub_base_url.clone(),
        fetched_at: Utc::now().trunc_subsecs(0),
        state,
    };
    if let Some(path) = cache_path(flox, pointer)
        && let Err(err) = write_cache(&path, &cached)
    {
        debug!(error = %err, ?path, "Failed to cache automatic upgrade settings");
    }
    Ok(Some(cached))
}

/// The automatic upgrade settings of the last fetch,
/// or `None` if they were never fetched or can't be read.
pub fn read_cached(
    flox: &Flox,
    environment: &ConcreteEnvironment,
) -> Option<CachedAutomaticUpgrades> {
    let pointer = configured_floxhub_pointer(flox, environment)?;
    let path = cache_path(flox, pointer)?;
    let contents = match fs::read(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == ErrorKind::NotFound => return None,
        Err(err) => {
            debug!(error = %err, ?path, "Failed to read cached automatic upgrade settings");
            return None;
        },
    };
    let cached: CachedAutomaticUpgrades = serde_json::from_slice(&contents)
        .inspect_err(
            |err| debug!(error = %err, ?path, "Failed to parse cached automatic upgrade settings"),
        )
        .ok()?;
    if cached.floxhub_url != pointer.floxhub_base_url {
        debug!(
            cached_floxhub = %cached.floxhub_url,
            ?path,
            "Cached automatic upgrade settings are from another FloxHub"
        );
        return None;
    }
    Some(cached)
}

/// Whether a FloxHub environment is on the configured FloxHub.
///
/// Credentials are stored for the configured FloxHub, so the settings are
/// only fetched for environments on it.
pub fn is_on_configured_floxhub(flox: &Flox, pointer: &ManagedPointer) -> bool {
    pointer.floxhub_base_url == *flox.floxhub.base_url()
}

/// The pointer of a FloxHub environment on the configured FloxHub
fn configured_floxhub_pointer<'a>(
    flox: &Flox,
    environment: &'a ConcreteEnvironment,
) -> Option<&'a ManagedPointer> {
    let pointer = match environment {
        ConcreteEnvironment::Path(_) => return None,
        ConcreteEnvironment::Managed(environment) => environment.pointer(),
        ConcreteEnvironment::Remote(environment) => environment.pointer(),
    };
    if !is_on_configured_floxhub(flox, pointer) {
        debug!(
            environment_floxhub = %pointer.floxhub_base_url,
            configured_floxhub = %flox.floxhub.base_url(),
            "Environment isn't on the configured FloxHub"
        );
        return None;
    }
    Some(pointer)
}

/// `<cache dir>/floxhub/<owner>/<name>/automatic-upgrades.json`
///
/// `None` if the owner or name isn't usable as a directory name.
fn cache_path(flox: &Flox, pointer: &ManagedPointer) -> Option<PathBuf> {
    let owner = pointer.owner.to_string();
    let name = pointer.name.to_string();
    // Owners and names only exclude spaces and slashes,
    // so `..` would otherwise leave the cache directory.
    let is_directory_name = |part: &str| {
        let mut components = Path::new(part).components();
        matches!(
            (components.next(), components.next()),
            (Some(Component::Normal(_)), None)
        )
    };
    if !is_directory_name(&owner) || !is_directory_name(&name) {
        debug!(%owner, %name, "Not caching automatic upgrade settings");
        return None;
    }
    Some(
        flox.cache_dir
            .join("floxhub")
            .join(owner)
            .join(name)
            .join(CACHE_FILE_NAME),
    )
}

#[derive(Debug, Error)]
enum WriteCacheError {
    #[error("failed to create the cache directory")]
    CreateDir(#[source] std::io::Error),
    #[error("failed to serialize the settings")]
    Serialize(#[source] serde_json::Error),
    #[error("failed to write the cache file")]
    Write(#[source] WriteError),
}

fn write_cache(path: &Path, cached: &CachedAutomaticUpgrades) -> Result<(), WriteCacheError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(WriteCacheError::CreateDir)?;
    }
    let contents = serde_json::to_vec(cached).map_err(WriteCacheError::Serialize)?;
    write_atomically(path, contents).map_err(WriteCacheError::Write)
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use flox_core::floxhub::Floxhub;
    use floxhub_client::client::test_helpers::client_config;
    use floxhub_client::{AuthContext, FloxhubClient, FloxhubClientConfig};
    use httpmock::MockServer;
    use serde_json::json;
    use url::Url;

    use super::*;
    use crate::flox::test_helpers::flox_instance_with_optional_floxhub;
    use crate::models::environment::managed_environment::test_helpers::mock_managed_environment_unlocked;

    const PATH: &str = "/floxem/api/v1/environment/owner/name/automatic-upgrades";

    /// A managed environment `owner/name` on the configured FloxHub,
    /// whose floxEM is `floxem_url`
    fn managed_environment(floxem_url: &str) -> (Flox, ConcreteEnvironment, tempfile::TempDir) {
        let owner = "owner".parse().unwrap();
        let (mut flox, tempdir) = flox_instance_with_optional_floxhub(Some(&owner));
        let environment = mock_managed_environment_unlocked(&flox, "version = 1", owner);
        flox.floxhub_client = FloxhubClient::new(FloxhubClientConfig {
            floxem_url: Url::parse(floxem_url).unwrap(),
            auth_context: AuthContext::new_from_token(Some("flox_pat_secret")),
            ..client_config("http://localhost:0")
        })
        .unwrap();
        (flox, ConcreteEnvironment::Managed(environment), tempdir)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refresh_caches_fetched_settings() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PATH);
            then.status(200).json_body(json!({
                "enabled": true,
                "cadence": "daily",
                "nextDueDate": "2026-10-01",
                "lastAutomaticUpgrade": {
                    "timestamp": "2026-09-30T00:17:03.5+00:00",
                    "result": "noop",
                    "generation": null,
                },
                "upgradeSource": {"owner": "acme", "name": "app-qa"},
            }));
        });
        let (flox, environment, _tempdir) = managed_environment(&server.url("/floxem"));

        let cached = refresh(&flox, &environment).await.unwrap().unwrap();

        assert_eq!(cached, CachedAutomaticUpgrades {
            floxhub_url: flox.floxhub.base_url().clone(),
            fetched_at: cached.fetched_at,
            state: AutomaticUpgradesState::Fetched(AutomaticUpgrades {
                enabled: true,
                cadence: Some("daily".to_string()),
                next_due_date: NaiveDate::from_ymd_opt(2026, 10, 1),
                last_run: Some(AutomaticUpgradeRun {
                    timestamp: Utc.with_ymd_and_hms(2026, 9, 30, 0, 17, 3).unwrap(),
                    result: AutomaticUpgradeResult::Noop,
                }),
                upgrade_source: Some("acme/app-qa".parse().unwrap()),
            }),
        });
        assert_eq!(read_cached(&flox, &environment), Some(cached));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refresh_caches_unavailable_settings_for_other_statuses() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PATH);
            then.status(404).json_body(json!({"detail": "Not Found"}));
        });
        let (flox, environment, _tempdir) = managed_environment(&server.url("/floxem"));

        let cached = refresh(&flox, &environment).await.unwrap().unwrap();

        assert_eq!(cached, CachedAutomaticUpgrades {
            floxhub_url: flox.floxhub.base_url().clone(),
            fetched_at: cached.fetched_at,
            state: AutomaticUpgradesState::Unavailable,
        });
        assert_eq!(read_cached(&flox, &environment), Some(cached));
    }

    /// Only an upgrade that names its generation can be compared with
    /// FloxHub's history, and a response that can't be used is cached as
    /// unavailable.
    #[tokio::test(flavor = "multi_thread")]
    async fn refresh_converts_the_response() {
        let server = MockServer::start();
        let (flox, environment, _tempdir) = managed_environment(&server.url("/floxem"));
        let upgrade = |generation: Option<u64>| {
            json!({
                "enabled": false,
                "lastAutomaticUpgrade": {
                    "timestamp": "2026-09-28T00:17:03+00:00",
                    "result": "upgrade",
                    "generation": generation,
                },
            })
            .to_string()
        };
        let last_run = |result| {
            AutomaticUpgradesState::Fetched(AutomaticUpgrades {
                enabled: false,
                cadence: None,
                next_due_date: None,
                last_run: Some(AutomaticUpgradeRun {
                    timestamp: Utc.with_ymd_and_hms(2026, 9, 28, 0, 17, 3).unwrap(),
                    result,
                }),
                upgrade_source: None,
            })
        };

        let cases = [
            (
                "an upgrade",
                upgrade(Some(7)),
                last_run(AutomaticUpgradeResult::Upgrade(7.into())),
            ),
            (
                "an upgrade without a generation",
                upgrade(None),
                last_run(AutomaticUpgradeResult::Unknown),
            ),
            (
                "an upgrade source that isn't an environment",
                json!({"enabled": true, "upgradeSource": {"owner": "a b", "name": "c"}})
                    .to_string(),
                AutomaticUpgradesState::Unavailable,
            ),
            (
                "a body that isn't JSON",
                "<html></html>".to_string(),
                AutomaticUpgradesState::Unavailable,
            ),
        ];

        for (case, body, state) in cases {
            let mut mock = server.mock(|when, then| {
                when.method(httpmock::Method::GET).path(PATH);
                then.status(200).body(&body);
            });

            let cached = refresh(&flox, &environment).await.unwrap().unwrap();

            assert_eq!(
                cached,
                CachedAutomaticUpgrades {
                    floxhub_url: flox.floxhub.base_url().clone(),
                    fetched_at: cached.fetched_at,
                    state,
                },
                "{case}"
            );
            mock.delete();
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refresh_keeps_the_cache_when_floxhub_is_unreachable() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PATH);
            then.status(200).json_body(json!({"enabled": false}));
        });
        let (mut flox, environment, _tempdir) = managed_environment(&server.url("/floxem"));
        let cached = refresh(&flox, &environment).await.unwrap().unwrap();

        flox.floxhub_client = FloxhubClient::new(client_config("http://localhost:0")).unwrap();
        let result = refresh(&flox, &environment).await;

        assert!(
            matches!(result, Err(AutomaticUpgradesRequestError::Request(_))),
            "{result:?}"
        );
        assert_eq!(read_cached(&flox, &environment), Some(cached));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refresh_and_read_cached_skip_environments_on_another_floxhub() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET).path(PATH);
            then.status(200).json_body(json!({"enabled": true}));
        });
        let (mut flox, environment, _tempdir) = managed_environment(&server.url("/floxem"));
        // Cached while the environment's FloxHub is the configured one
        let cached = refresh(&flox, &environment).await.unwrap().unwrap();
        assert_eq!(read_cached(&flox, &environment), Some(cached));

        flox.floxhub = Floxhub::new(
            Url::parse("https://floxhub.example.internal").unwrap(),
            None,
            None,
        )
        .unwrap();

        assert_eq!(refresh(&flox, &environment).await.unwrap(), None);
        assert_eq!(read_cached(&flox, &environment), None);
        mock.assert_calls(1);
    }

    /// Another FloxHub may have an environment of the same owner and name.
    #[test]
    fn read_cached_ignores_settings_cached_from_another_floxhub() {
        let (flox, environment, _tempdir) = managed_environment("http://localhost:0/floxem");
        let pointer = configured_floxhub_pointer(&flox, &environment).unwrap();
        let path = cache_path(&flox, pointer).unwrap();
        let cached_from = |floxhub_url: Url| CachedAutomaticUpgrades {
            floxhub_url,
            fetched_at: Utc::now().trunc_subsecs(0),
            state: AutomaticUpgradesState::Unavailable,
        };

        let own = cached_from(pointer.floxhub_base_url.clone());
        write_cache(&path, &own).unwrap();
        assert_eq!(read_cached(&flox, &environment), Some(own));

        let other = cached_from(Url::parse("https://floxhub.example.internal").unwrap());
        write_cache(&path, &other).unwrap();
        assert_eq!(read_cached(&flox, &environment), None);
    }
}
