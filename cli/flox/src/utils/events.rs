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
use flox_events::{EnvDetail, EventsClient, EventsHub, SharedMetadataTemplate};
use flox_rust_sdk::flox::{FLOX_VERSION, Flox};
use flox_rust_sdk::models::environment::generations::GenerationsExt;
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

/// The identity fields for the environment the current invocation operates
/// on, read once at the first environment event and reused by later emits of
/// the same command so the generation and lockfile reads don't repeat on
/// multi-emit paths like `activate`.
#[derive(Debug, Clone, Copy)]
struct EnvIdentityFields {
    environment_id: Option<Uuid>,
    generation_number: Option<u64>,
    package_count: Option<u64>,
}

/// Process-global; never resets within a test binary, so tests that install
/// a client must use distinct environments.
static ENV_IDENTITY_FIELDS: OnceLock<(String, EnvIdentityFields)> = OnceLock::new();

/// Read the identity fields for `env`. Every read is best-effort: a failure
/// leaves the field absent, never fails the command.
fn read_env_identity_fields(flox: &Flox, env: &ConcreteEnvironment) -> EnvIdentityFields {
    let environment_id = match env {
        ConcreteEnvironment::Path(environment) => environment.pointer.id,
        ConcreteEnvironment::Managed(environment) => environment.pointer().id,
        // The cached pointer of a remote environment is rewritten on every
        // invocation, so it carries no stable id.
        ConcreteEnvironment::Remote(_) => None,
    };
    // A command opened at a pinned generation acts on that generation;
    // only unpinned managed/remote environments read the branch tip.
    let generation_number = match env {
        ConcreteEnvironment::Managed(environment) => environment
            .generation()
            .map(|generation| *generation as u64)
            .or_else(|| current_generation_number(environment)),
        ConcreteEnvironment::Remote(environment) => environment
            .generation()
            .map(|generation| *generation as u64)
            .or_else(|| current_generation_number(environment)),
        // Path environments have no generations.
        ConcreteEnvironment::Path(_) => None,
    };

    EnvIdentityFields {
        environment_id,
        generation_number,
        package_count: package_count(flox, env),
    }
}

fn current_generation_number(env: &impl GenerationsExt) -> Option<u64> {
    env.generations_metadata()
        .map_err(|err| debug!(error = %err, "could not read generations metadata for event"))
        .ok()
        .and_then(|metadata| metadata.current_gen())
        .map(|generation| *generation as u64)
}

/// Packages locked for the invoking system (the `flox list` count), read
/// without locking, building, materializing a checkout, or touching the
/// network.
fn package_count(flox: &Flox, env: &ConcreteEnvironment) -> Option<u64> {
    let lockfile = match env {
        ConcreteEnvironment::Path(environment) => environment.existing_lockfile(flox),
        // `Environment::existing_lockfile` may materialize the local
        // checkout for managed/remote environments; these reads must not.
        ConcreteEnvironment::Managed(environment) => {
            environment.existing_lockfile_without_checkout()
        },
        ConcreteEnvironment::Remote(environment) => {
            environment.existing_lockfile_without_checkout()
        },
    };
    lockfile
        .map_err(|err| debug!(error = %err, "could not read lockfile for event"))
        .ok()
        .flatten()
        .and_then(|lockfile| {
            lockfile
                .list_packages(&flox.system)
                .map_err(|err| debug!(error = %err, "could not list packages for event"))
                .ok()
        })
        .map(|packages| packages.len() as u64)
}

/// Identity fields for `env`, computed once per invocation (keyed by the
/// environment's kind and `.flox` path) and skipped entirely when no events
/// client is installed.
fn env_identity_fields(
    flox: &Flox,
    env: &ConcreteEnvironment,
    key: &str,
) -> Option<EnvIdentityFields> {
    if !EventsHub::global().has_client() {
        return None;
    }
    match ENV_IDENTITY_FIELDS.get() {
        Some((cached_key, fields)) if cached_key == key => Some(*fields),
        // A second environment in the same process is computed fresh but
        // not cached; commands operate on one environment.
        Some(_) => Some(read_env_identity_fields(flox, env)),
        None => {
            let fields = read_env_identity_fields(flox, env);
            let _ = ENV_IDENTITY_FIELDS.set((key.to_string(), fields));
            Some(fields)
        },
    }
}

/// Build an [`EnvDetail`] for the supplied [`ConcreteEnvironment`], using the
/// same env-kind / env-ref mapping as the legacy
/// `environment_subcommand_metric!` macro. Shared across call sites so the
/// per-kind match is not duplicated.
pub fn env_detail_from_concrete(flox: &Flox, env: &ConcreteEnvironment) -> EnvDetail {
    let (env_kind, env_ref_or_name) = match env {
        ConcreteEnvironment::Remote(environment) => ("remote", environment.env_ref().to_string()),
        ConcreteEnvironment::Managed(environment) => ("managed", environment.env_ref().to_string()),
        ConcreteEnvironment::Path(environment) => {
            ("path", Environment::name(environment).to_string())
        },
    };
    let cache_key = format!("{env_kind}:{}", env.dot_flox_path().display());
    let mut detail = EnvDetail::new(env_kind, env_ref_or_name);
    if let Some(fields) = env_identity_fields(flox, env, &cache_key) {
        if let Some(environment_id) = fields.environment_id {
            detail = detail.with_environment_id(environment_id);
        }
        if let Some(generation_number) = fields.generation_number {
            detail = detail.with_generation_number(generation_number);
        }
        if let Some(package_count) = fields.package_count {
            detail = detail.with_package_count(package_count);
        }
    }
    detail
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use flox_config::FloxConfig;
    use flox_core::data::environment_ref::EnvironmentOwner;
    use flox_events::test_helpers::MockEventsConnection;
    use flox_events::{EVENTS_BUFFER_FILE_NAME, EventsHub, LifecycleFields};
    use flox_rust_sdk::flox::test_helpers::{flox_instance, flox_instance_with_optional_floxhub};
    use flox_rust_sdk::models::environment::generations::GenerationId;
    use flox_rust_sdk::models::environment::managed_environment::test_helpers::{
        mock_managed_environment_in,
        mock_managed_environment_unlocked,
        with_pinned_generation,
        with_pointer_id,
    };
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment;
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

    /// Install a mock client on the global hub; returns whatever client
    /// was installed before so callers can restore it.
    fn install_mock_client(tempdir: &TempDir) -> Option<EventsClient> {
        let template = SharedMetadataTemplate {
            flox_version: "0.0.0-test".to_string(),
            os_family: None,
            os_family_release: None,
            os: None,
            os_version: None,
            empty_flags: vec![],
            invocation_sources: vec![],
        };
        let client = EventsClient::new_with_connection(
            Uuid::new_v4(),
            tempdir.path(),
            Uuid::new_v4(),
            None,
            template,
            MockEventsConnection::default(),
        );
        EventsHub::global().set_client(client)
    }

    fn restore_client(previous: Option<EventsClient>) {
        EventsHub::global().clear_client();
        if let Some(previous) = previous {
            EventsHub::global().set_client(previous);
        }
    }

    /// The env-detail identity fields ride for a path environment with a
    /// pointer id — and the sourcing is skipped entirely when no events
    /// client is installed.
    #[test]
    #[serial(global_events_client)]
    fn env_detail_carries_identity_fields_for_path_env() {
        let (flox, _temp_dir_handle) = flox_instance();
        let mut env = new_path_environment(&flox, "version = 1\n");
        let environment_id = Uuid::new_v4();
        env.pointer.id = Some(environment_id);
        env.lockfile(&flox).unwrap();
        let name = Environment::name(&env).to_string();
        let concrete = ConcreteEnvironment::Path(env);

        // No client installed: recording is a no-op, so nothing is sourced.
        let previous = EventsHub::global().clear_client();
        let detail = serde_json::to_value(env_detail_from_concrete(&flox, &concrete))
            .expect("detail serializes");
        assert_eq!(
            detail,
            serde_json::json!({
                "env_kind": "path",
                "env_ref_or_name": name,
            })
        );

        let tempdir = tempfile::tempdir().expect("tempdir");
        install_mock_client(&tempdir);

        let detail = serde_json::to_value(env_detail_from_concrete(&flox, &concrete))
            .expect("detail serializes");
        assert_eq!(
            detail,
            serde_json::json!({
                "env_kind": "path",
                "env_ref_or_name": name,
                "environment_id": environment_id.to_string(),
                "package_count": 0,
            })
        );

        restore_client(previous);
    }

    /// The managed arm sources the current generation when unpinned and
    /// prefers the pin when set — without materializing the checkout.
    #[test]
    #[serial(global_events_client)]
    fn env_detail_carries_identity_fields_for_managed_env() {
        let owner: EnvironmentOwner = "owner".parse().unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));
        let environment_id = Uuid::new_v4();
        let env = with_pointer_id(
            mock_managed_environment_unlocked(&flox, "version = 1\n", owner.clone()),
            environment_id,
        );
        let dot_flox_path = env.dot_flox_path().to_path_buf();

        let tempdir = tempfile::tempdir().expect("tempdir");
        let previous = EventsHub::global().clear_client();
        install_mock_client(&tempdir);

        let concrete = ConcreteEnvironment::Managed(env);
        let detail = serde_json::to_value(env_detail_from_concrete(&flox, &concrete))
            .expect("detail serializes");
        // The mock generation carries no lockfile, so package_count is
        // absent; the current generation is 1.
        assert_eq!(
            detail,
            serde_json::json!({
                "env_kind": "managed",
                "env_ref_or_name": "owner/name",
                "environment_id": environment_id.to_string(),
                "generation_number": 1,
            })
        );
        assert!(
            !dot_flox_path.join("env").exists(),
            "sourcing must not materialize the checkout"
        );

        // A fresh environment (fresh dot-flox path, so a fresh cache key):
        // the pin must win over the current generation.
        let pinned_env_dir = tempfile::tempdir().expect("tempdir");
        let pinned_env = with_pinned_generation(
            mock_managed_environment_in(
                &flox,
                "version = 1\n",
                owner,
                pinned_env_dir.path(),
                Some("name2"),
            ),
            GenerationId::from(5_usize),
        );
        let concrete = ConcreteEnvironment::Managed(pinned_env);
        let detail = serde_json::to_value(env_detail_from_concrete(&flox, &concrete))
            .expect("detail serializes");
        assert_eq!(
            detail.get("generation_number"),
            Some(&serde_json::json!(5)),
            "the pin wins over the current generation"
        );

        restore_client(previous);
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
