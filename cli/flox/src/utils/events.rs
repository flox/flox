//! Integration wrapper between the flox binary and the [`flox_events`] crate.
//!
//! The CLI emits two telemetry streams in parallel: the legacy
//! `subcommand_metric!` pipeline (`cli/flox/src/utils/metrics.rs`) and the
//! v2-events pipeline (this module + the [`flox_events`] crate). The two
//! stacks share no code and write separate on-disk buffers.
//! `config.flox.disable_metrics` silences both.
//!
//! Authenticated invocations additionally carry a pseudonymous subject
//! identifier as `auth_subject`, resolved by the caller from the auth
//! context (`AuthContext::user_subject`) — see
//! [`flox_events::EventsClient`] for the field's semantics.

use std::env;
use std::str::FromStr;
use std::sync::{LazyLock, OnceLock};

use flox_config::Config;
use flox_events::{EnvDetail, EventsClient, SharedMetadataTemplate};
use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment};
use flox_rust_sdk::utils::INVOCATION_SOURCES;
use tracing::debug;
use uuid::Uuid;

use crate::utils::metrics::read_metrics_uuid;

/// Stores the invocation_id resolved by [`resolve_invocation_id`] so detached
/// subprocess spawn sites can propagate it via [`FLOX_INVOCATION_ID_VAR`].
///
/// Kept out of the process environment so that an activated user shell does
/// not inherit it — `flox` commands run from inside an activated shell are
/// fresh top-level invocations.
static RESOLVED_INVOCATION_ID: OnceLock<Uuid> = OnceLock::new();

/// Env var carrying the parent flox process's invocation id across a
/// detached subprocess boundary.
pub const FLOX_INVOCATION_ID_VAR: &str = "FLOX_INVOCATION_ID";

static METRICS_EVENTS_URL_V2: LazyLock<String> = LazyLock::new(|| {
    std::env::var("_FLOX_METRICS_URL_V2_OVERRIDE")
        .unwrap_or(env!("METRICS_EVENTS_URL_V2").to_string())
});
static METRICS_EVENTS_API_KEY_V2: LazyLock<String> = LazyLock::new(|| {
    std::env::var("_FLOX_METRICS_API_KEY_V2_OVERRIDE")
        .unwrap_or(env!("METRICS_EVENTS_API_KEY_V2").to_string())
});

/// Resolve the invocation id for the current process.
///
/// If [`FLOX_INVOCATION_ID_VAR`] is set and parses as a UUID, the process
/// inherits it from a parent flox invocation so its v2 events join the
/// parent's stream. Otherwise a fresh v4 UUID is minted, marking this as a
/// top-level invocation.
pub fn resolve_invocation_id() -> Uuid {
    let resolved = match env::var(FLOX_INVOCATION_ID_VAR) {
        Ok(raw) => match Uuid::from_str(&raw) {
            Ok(uuid) => {
                debug!(invocation_id = %uuid, "inherited v2 invocation_id from FLOX_INVOCATION_ID");
                uuid
            },
            Err(err) => {
                debug!(error = %err, "FLOX_INVOCATION_ID set but unparseable; minting fresh id");
                Uuid::new_v4()
            },
        },
        Err(_) => Uuid::new_v4(),
    };
    let _ = RESOLVED_INVOCATION_ID.set(resolved);
    resolved
}

/// Return the invocation_id resolved by [`resolve_invocation_id`] earlier in
/// this process, if any. Detached subprocess spawn sites use this to set
/// [`FLOX_INVOCATION_ID_VAR`] on the child's `Command`.
pub fn current_invocation_id() -> Option<Uuid> {
    RESOLVED_INVOCATION_ID.get().copied()
}

/// Build the [`SharedMetadataTemplate`] stamped onto every v2 event emitted
/// by this process. The fields mirror the legacy
/// [`crate::utils::metrics::MetricEntry`] so downstream consumers can
/// reconstruct the existing columns.
fn shared_metadata_template() -> SharedMetadataTemplate {
    let linux_release = sys_info::linux_os_release().ok();
    SharedMetadataTemplate {
        flox_version: FLOX_VERSION.to_string(),
        os_family: sys_info::os_type()
            .ok()
            .map(|x| x.replace("Darwin", "Mac OS")),
        os_family_release: sys_info::os_release().ok(),
        os: linux_release.as_ref().and_then(|r| r.id.clone()),
        os_version: linux_release.and_then(|r| r.version_id),
        empty_flags: vec![],
        invocation_sources: INVOCATION_SOURCES.clone(),
    }
}

/// Try to build an [`EventsClient`] to install on the global
/// [`flox_events::EventsHub`].
///
/// Clients across invocations share an anonymous per-installation id via
/// [`read_metrics_uuid`].
///
/// Returns `None` if
/// a) metrics are disabled by config, or
/// b) reading the metrics uuid fails.
///
/// `auth_subject` is the caller-resolved pseudonymous subject identifier
/// (`AuthContext::user_subject`) — the returned client snapshots it at
/// construction (see [`flox_events::EventsClient`] for the snapshot
/// semantics).
pub fn build_events_client(
    config: &Config,
    invocation_id: Uuid,
    auth_subject: Option<String>,
) -> Option<EventsClient> {
    if config.flox.disable_metrics {
        debug!("v2 events: disable_metrics is true; not installing client");
        return None;
    }

    let device_id = match read_metrics_uuid(config) {
        Ok(uuid) => uuid,
        Err(err) => {
            debug!(error = %err, "v2 events: could not read metrics uuid; not installing client");
            return None;
        },
    };

    Some(EventsClient::new(
        device_id,
        &config.flox.data_dir,
        METRICS_EVENTS_URL_V2.clone(),
        METRICS_EVENTS_API_KEY_V2.clone(),
        invocation_id,
        auth_subject,
        shared_metadata_template(),
    ))
}

/// Build an [`EnvDetail`] for the supplied [`ConcreteEnvironment`], using the
/// same env-kind / env-ref mapping as the legacy
/// `environment_subcommand_metric!` macro. Shared across call sites so the
/// per-kind match is not duplicated.
pub fn env_detail_from_concrete(env: &ConcreteEnvironment) -> EnvDetail {
    match env {
        ConcreteEnvironment::Remote(environment) => {
            EnvDetail::new("remote", environment.env_ref().to_string())
        },
        ConcreteEnvironment::Managed(environment) => {
            EnvDetail::new("managed", environment.env_ref().to_string())
        },
        ConcreteEnvironment::Path(environment) => {
            EnvDetail::new("path", Environment::name(environment).to_string())
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use flox_config::FloxConfig;
    use flox_events::test_helpers::MockEventsConnection;
    use flox_events::{EVENTS_BUFFER_FILE_NAME, EventsHub, LifecycleFields};
    use serial_test::serial;
    use temp_env::with_var;
    use tempfile::TempDir;

    use super::*;

    /// A `Config` value pointing at a fresh tempdir, with metrics enabled
    /// and a pre-written metrics uuid so the wrapper has everything it
    /// needs to install a client.
    fn test_config_with_uuid(tempdir: &TempDir, uuid: Uuid) -> Config {
        let data_dir = tempdir.path().join("data");
        std::fs::create_dir_all(&data_dir).expect("data dir");
        std::fs::write(data_dir.join("metrics-uuid"), uuid.hyphenated().to_string())
            .expect("write metrics-uuid");
        test_config(tempdir, data_dir, /* disable_metrics */ false)
    }

    #[allow(deprecated)]
    fn test_config(tempdir: &TempDir, data_dir: PathBuf, disable_metrics: bool) -> Config {
        Config {
            flox: FloxConfig {
                cache_dir: tempdir.path().join("cache"),
                data_dir,
                state_dir: tempdir.path().join("state"),
                config_dir: tempdir.path().join("config"),
                disable_metrics,
                ..FloxConfig::default()
            },
            features: None,
        }
    }

    /// Ties the SDK's detection-token vocabulary to the payload's typed
    /// bools across the crate boundary: `flox-events` matches the
    /// `"ci"` / `"containerd"` tokens by literal and does not depend on
    /// `flox-rust-sdk`, so this is the only place a respelling on
    /// either side can fail a test.
    #[test]
    #[serial(v2_events_wrapper_env)]
    fn detection_tokens_drive_typed_payload_bools() {
        temp_env::with_vars(
            [("CI", Some("true")), ("FLOX_CONTAINERD", Some("1"))],
            || {
                let template = SharedMetadataTemplate {
                    flox_version: "0.0.0-test".to_string(),
                    os_family: None,
                    os_family_release: None,
                    os: None,
                    os_version: None,
                    empty_flags: vec![],
                    invocation_sources:
                        flox_rust_sdk::utils::invocation_sources::detect_invocation_sources(),
                };
                let payload = serde_json::to_value(template.into_payload("envs".to_string()))
                    .expect("payload serializes");
                assert_eq!(payload.get("in_ci"), Some(&serde_json::json!(true)));
                assert_eq!(payload.get("containerd"), Some(&serde_json::json!(true)));
                // The template above has `os_family_release: None`, so
                // the derived `kernel_version` must be absent — not
                // fabricated.
                assert_eq!(payload.get("kernel_version"), None);
            },
        );
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn resolve_invocation_id_returns_parent_id_when_env_set() {
        let parent = Uuid::new_v4();
        with_var(FLOX_INVOCATION_ID_VAR, Some(parent.to_string()), || {
            assert_eq!(resolve_invocation_id(), parent);
        });
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn resolve_invocation_id_mints_fresh_when_env_unset() {
        with_var(FLOX_INVOCATION_ID_VAR, None::<&str>, || {
            let a = resolve_invocation_id();
            let b = resolve_invocation_id();
            assert_ne!(a, Uuid::nil());
            assert_ne!(a, b, "consecutive mints should not collide");
        });
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn resolve_invocation_id_mints_fresh_when_env_unparseable() {
        with_var(FLOX_INVOCATION_ID_VAR, Some("not-a-uuid"), || {
            let id = resolve_invocation_id();
            assert_ne!(id, Uuid::nil());
        });
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn build_events_client_returns_none_when_disable_metrics_is_true() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config = test_config(
            &tempdir,
            tempdir.path().join("data"),
            /* disable_metrics */ true,
        );

        let client = build_events_client(&config, Uuid::new_v4(), None);
        assert!(client.is_none(), "disable_metrics must take priority");
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn build_events_client_returns_none_when_uuid_unreadable() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let data_dir = tempdir.path().join("data");
        std::fs::create_dir_all(&data_dir).expect("data dir");
        // No metrics-uuid file written: read_metrics_uuid errors.
        let config = test_config(&tempdir, data_dir, /* disable_metrics */ false);

        let client = build_events_client(&config, Uuid::new_v4(), None);
        assert!(client.is_none(), "missing uuid must short-circuit");
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn build_events_client_returns_some_by_default() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let uuid = Uuid::new_v4();
        let config = test_config_with_uuid(&tempdir, uuid);

        let client = build_events_client(&config, Uuid::new_v4(), None);
        assert!(client.is_some(), "v2 is enabled by default");
        assert_eq!(client.unwrap().device_id, uuid);
    }

    /// The wrapper stamps whatever subject the caller resolved — which
    /// subjects resolve for which auth states is pinned where that logic
    /// lives (`AuthContext::user_subject` / `FloxhubToken::sub`).
    #[test]
    #[serial(v2_events_wrapper_env)]
    fn build_events_client_stamps_provided_auth_subject() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config = test_config_with_uuid(&tempdir, Uuid::new_v4());

        let client =
            build_events_client(&config, Uuid::new_v4(), Some("github|3670948".to_string()))
                .expect("client installs");
        assert_eq!(client.auth_subject.as_deref(), Some("github|3670948"));

        let client = build_events_client(&config, Uuid::new_v4(), None).expect("client installs");
        assert_eq!(client.auth_subject, None, "anonymous use stays anonymous");
    }

    /// End-to-end: install a hub-owned client backed by a
    /// [`MockEventsConnection`], record run + completed for one invocation,
    /// and assert exactly one of each lands sharing one `invocation_id`.
    #[test]
    #[serial(global_events_client)]
    fn one_run_one_completed_end_to_end() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let invocation_id = Uuid::new_v4();

        let template = SharedMetadataTemplate {
            flox_version: "0.0.0-test".to_string(),
            os_family: Some("Linux".to_string()),
            os_family_release: None,
            os: None,
            os_version: None,
            empty_flags: vec![],
            invocation_sources: vec!["shell".to_string()],
        };
        let client = EventsClient::new_with_connection(
            Uuid::new_v4(),
            tempdir.path(),
            invocation_id,
            None,
            template,
            connection,
        );

        let previous = EventsHub::global().set_client(client);

        EventsHub::global()
            .record_command_run("install".to_string())
            .expect("record run");
        EventsHub::global()
            .record_command_completed("install".to_string(), LifecycleFields {
                exit_code: 0,
                duration_ms: Some(1),
                error_kind: None,
            })
            .expect("record completed");
        EventsHub::global().flush(true).expect("flush");

        // Confirm only one buffer file was written and now drained.
        assert_eq!(
            std::fs::read_to_string(tempdir.path().join(EVENTS_BUFFER_FILE_NAME))
                .expect("read buffer"),
            ""
        );

        let batches = sent_batches.lock().unwrap().clone();
        let events: Vec<_> = batches.into_iter().flatten().collect();
        assert_eq!(events.len(), 2, "exactly one run + one completed");
        let invocations: Vec<_> = events.iter().map(|e| e.invocation_id).collect();
        assert!(
            invocations.iter().all(|id| *id == invocation_id),
            "events must share one invocation_id"
        );

        // Restore the previous client (which was None unless another test
        // installed one before us).
        EventsHub::global().clear_client();
        if let Some(previous) = previous {
            EventsHub::global().set_client(previous);
        }
    }
}
