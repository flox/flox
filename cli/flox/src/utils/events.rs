//! Integration wrapper between the flox binary and the [`flox_events`] crate.
//!
//! This module is the **single integration surface** through which the flox
//! binary decides whether to install an [`EventsClient`] and how to populate
//! the shared metadata stamped onto every canonical event. The
//! [`flox_events`] crate itself is a clean leaf — no `flox-rust-sdk` edge, no
//! `env!` macros, no `Config` access — and this wrapper holds all of the
//! integration concerns that would otherwise have to live in the crate.
//!
//! The cutover PR replaces the production branch of [`build_events_client`]
//! to install a real client; until then the wrapper installs a client only
//! when the dev/test override `_FLOX_METRICS_URL_OVERRIDE` is set, so
//! production builds remain byte-identical to `main` on every code path.

use std::env;
use std::str::FromStr;
use std::sync::OnceLock;

use flox_events::{EventsClient, EventsGuard, EventsHub, SharedMetadataTemplate};
use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::utils::INVOCATION_SOURCES;
use tracing::debug;
use uuid::Uuid;

use crate::config::Config;
use crate::utils::metrics::{METRICS_EVENTS_API_KEY, read_metrics_uuid};

/// Stores the invocation_id resolved by [`resolve_invocation_id`] so detached
/// subprocess spawn sites can propagate it via [`FLOX_INVOCATION_ID_VAR`]
/// without threading it through every call layer.
///
/// Set exactly once per process by [`resolve_invocation_id`]; never read by
/// the [`flox_events`] crate (the value flows there through [`EventsClient`]
/// at install time). The value lives in this `OnceLock` rather than in the
/// process environment because we do not want the activated user shell to
/// inherit it — subsequent `flox` commands run from inside an activate'd
/// shell should be treated as fresh top-level invocations.
static RESOLVED_INVOCATION_ID: OnceLock<Uuid> = OnceLock::new();

/// Env var carrying the parent flox process's invocation id across a
/// detached subprocess boundary. Internal — never documented as a
/// user-facing knob.
pub const FLOX_INVOCATION_ID_VAR: &str = "FLOX_INVOCATION_ID";

/// Resolve the invocation id for the current process.
///
/// If [`FLOX_INVOCATION_ID_VAR`] is set in the environment and parses as a
/// UUID, the process inherits it from a parent flox invocation — so its
/// canonical events join the parent's stream downstream. Otherwise a fresh
/// v4 UUID is minted, marking this as a top-level user invocation.
pub fn resolve_invocation_id() -> Uuid {
    let resolved = match env::var(FLOX_INVOCATION_ID_VAR) {
        Ok(raw) => match Uuid::from_str(&raw) {
            Ok(uuid) => {
                debug!(invocation_id = %uuid, "inherited canonical invocation_id from FLOX_INVOCATION_ID");
                uuid
            },
            Err(err) => {
                // Deliberately do not log the raw value — it is an env var
                // a parent flox process set, so it is generally not
                // sensitive, but matching `resolve_endpoint_url`'s rule
                // keeps the policy uniform for tracing subscribers.
                debug!(error = %err, "FLOX_INVOCATION_ID set but unparseable; minting fresh id");
                Uuid::new_v4()
            },
        },
        Err(_) => Uuid::new_v4(),
    };
    // Best-effort: store in the once-cell so subprocess spawn sites can
    // propagate it. If `resolve_invocation_id` was already called (e.g.
    // from a test), keep the first value.
    let _ = RESOLVED_INVOCATION_ID.set(resolved);
    resolved
}

/// Return the invocation_id resolved by [`resolve_invocation_id`] earlier in
/// this process, if any. Detached subprocess spawn sites use this to set
/// [`FLOX_INVOCATION_ID_VAR`] on the child's `Command` so the child joins
/// the parent's canonical event stream rather than minting a fresh top-level
/// invocation downstream.
pub fn current_invocation_id() -> Option<Uuid> {
    RESOLVED_INVOCATION_ID.get().copied()
}

/// Build the [`SharedMetadataTemplate`] stamped onto every canonical event
/// emitted by this process.
///
/// The fields here mirror what the legacy [`crate::utils::metrics::MetricEntry`]
/// carries today, so downstream consumers can reconstruct the existing
/// `cli.telemetry` columns once the cutover flips traffic to the new
/// pipeline.
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

/// Decide whether to install an [`EventsClient`] on the global
/// [`flox_events::EventsHub`] for this invocation.
///
/// Returns `None` (production dormant — no client installed, `record_event`
/// short-circuits) when any of the following holds:
///
/// - [`Config::flox::disable_metrics`] is `true` (the same gate the legacy
///   metrics pipeline honors at `cli/flox/src/main.rs` and
///   `cli/flox/src/commands/mod.rs`). Honoring the gate is consent-affecting:
///   silent telemetry-after-opt-out would be a privacy violation in a
///   public OSS repo.
/// - [`read_metrics_uuid`] returns `Err` (missing or unparseable
///   per-installation uuid file). The CLI runs to completion normally;
///   only the canonical event stream is silenced for the run.
/// - The dev/test override `_FLOX_METRICS_URL_OVERRIDE` is unset (this is
///   the **production-dormant** branch). The cutover PR replaces this `None`
///   with a Client pointing at the new pipeline's build-injected URL.
/// - `_FLOX_METRICS_URL_OVERRIDE` is set but not parseable as a URL.
///
/// When the override is set to a parseable URL, the returned client points
/// at it and authenticates with the existing build-injected
/// [`METRICS_EVENTS_API_KEY`].
pub fn build_events_client(config: &Config, invocation_id: Uuid) -> Option<EventsClient> {
    if config.flox.disable_metrics {
        debug!("Canonical events: disable_metrics is true; not installing client");
        return None;
    }

    let device_id = match read_metrics_uuid(config) {
        Ok(uuid) => uuid,
        Err(err) => {
            debug!(error = %err, "Canonical events: could not read metrics uuid; not installing client");
            return None;
        },
    };

    let endpoint_url = match resolve_endpoint_url() {
        Some(url) => url,
        None => {
            debug!("Canonical events: no override URL set; production dormant");
            return None;
        },
    };

    Some(EventsClient::new(
        device_id,
        &config.flox.data_dir,
        endpoint_url,
        METRICS_EVENTS_API_KEY,
        invocation_id,
        shared_metadata_template(),
    ))
}

/// Install (or skip-install) an [`EventsClient`] on the global
/// [`EventsHub`] for the lifetime of `main` and return an [`EventsGuard`]
/// the caller holds until end of process.
///
/// The guard's `Drop` flushes any buffered events through the connection,
/// so a normal-flow command's `command_run` (emitted by the chokepoint),
/// any per-domain events emitted during the dispatch (PRs 3–5), and the
/// `command_completed` emitted by the dispatcher or by `activate.rs` are
/// all delivered before the process exits.
///
/// When [`build_events_client`] returns `None` (production-dormant case),
/// the guard is still returned but its flush is a no-op — `EventsHub::global()`
/// has no client installed and `record_event` short-circuits.
pub fn install_events_client_for_main(config: &Config, invocation_id: Uuid) -> EventsGuard {
    if let Some(client) = build_events_client(config, invocation_id) {
        EventsHub::global().set_client(client);
    }
    EventsGuard::new()
}

/// Best-effort emission of a `command_run` + `command_completed` pair for an
/// early-exit branch in `main.rs` (e.g. `--version`, `--prefix`,
/// `--bpaf-complete-style-bash`).
///
/// These branches return their `ExitCode` *before* the normal chokepoint
/// runs in `cli/flox/src/commands/mod.rs`, so they install the client and
/// emit the pair themselves. Every step is fallible-and-silent: a missing
/// or unparseable config, a missing `metrics-uuid` file, or an unset
/// `_FLOX_METRICS_URL_OVERRIDE` simply skips the emission — the user-facing
/// command completes regardless. After emission the global hub's client is
/// cleared so a subsequent test-mode invocation does not see leaked state.
/// No previous client is restored here because the early-exit branches
/// always run *before* [`install_events_client_for_main`] — the hub holds
/// no client at this point.
pub fn emit_early_exit_command_pair(subcommand: &str, invocation_id: Uuid) {
    let Ok(config) = Config::parse() else {
        debug!("Canonical events early-exit: could not parse config; skipping emit");
        return;
    };
    let Some(client) = build_events_client(&config, invocation_id) else {
        return;
    };
    let hub = EventsHub::global();
    hub.set_client(client);
    if let Err(err) = hub.record_command_run(subcommand.to_string()) {
        debug!(error = %err, "Canonical events early-exit: command_run record failed");
    }
    if let Err(err) = hub.record_command_completed(subcommand.to_string()) {
        debug!(error = %err, "Canonical events early-exit: command_completed record failed");
    }
    if let Err(err) = hub.flush(true) {
        debug!(error = %err, "Canonical events early-exit: flush failed");
    }
    hub.clear_client();
}

/// Resolve the endpoint URL for the canonical events client.
///
/// Until the cutover PR repoints the production endpoint, this only returns
/// `Some` when `_FLOX_METRICS_URL_OVERRIDE` is set to a parseable URL. The
/// override is the dev/test capture hatch — the legacy pipeline already
/// honors it, so with it set both pipelines emit to the same local
/// collector and the payloads can be diffed.
fn resolve_endpoint_url() -> Option<String> {
    let raw = env::var("_FLOX_METRICS_URL_OVERRIDE").ok()?;
    match url::Url::parse(&raw) {
        Ok(parsed) => Some(parsed.to_string()),
        Err(err) => {
            // Deliberately do not log `raw` — a dev who experiments with
            // putting credentials in the override URL should not have them
            // captured by a `RUST_LOG=debug` tracing subscriber.
            debug!(
                error = %err,
                "Canonical events: _FLOX_METRICS_URL_OVERRIDE is unparseable; not installing client"
            );
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use flox_events::test_helpers::MockEventsConnection;
    use flox_events::{EVENTS_BUFFER_FILE_NAME, EventsHub};
    use serial_test::serial;
    use temp_env::with_var;
    use tempfile::TempDir;

    use super::*;
    use crate::config::{Config, FloxConfig};

    /// A `Config` value pointing at a fresh tempdir, with metrics enabled
    /// and a pre-written metrics uuid so the wrapper has everything it
    /// needs to install a client (subject to the override gate).
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
    #[serial(canonical_events_wrapper_env)]
    fn resolve_invocation_id_returns_parent_id_when_env_set() {
        let parent = Uuid::new_v4();
        with_var(FLOX_INVOCATION_ID_VAR, Some(parent.to_string()), || {
            assert_eq!(resolve_invocation_id(), parent);
        });
    }

    #[test]
    #[serial(canonical_events_wrapper_env)]
    fn resolve_invocation_id_mints_fresh_when_env_unset() {
        with_var(FLOX_INVOCATION_ID_VAR, None::<&str>, || {
            let a = resolve_invocation_id();
            let b = resolve_invocation_id();
            assert_ne!(a, Uuid::nil());
            assert_ne!(a, b, "consecutive mints should not collide");
        });
    }

    #[test]
    #[serial(canonical_events_wrapper_env)]
    fn resolve_invocation_id_mints_fresh_when_env_unparseable() {
        with_var(FLOX_INVOCATION_ID_VAR, Some("not-a-uuid"), || {
            let id = resolve_invocation_id();
            assert_ne!(id, Uuid::nil());
        });
    }

    #[test]
    #[serial(canonical_events_wrapper_env)]
    fn build_events_client_returns_none_when_disable_metrics_is_true() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config = test_config(
            &tempdir,
            tempdir.path().join("data"),
            /* disable_metrics */ true,
        );

        with_var(
            "_FLOX_METRICS_URL_OVERRIDE",
            Some("http://127.0.0.1:9999"),
            || {
                let client = build_events_client(&config, Uuid::new_v4());
                assert!(client.is_none(), "disable_metrics must take priority");
            },
        );
    }

    #[test]
    #[serial(canonical_events_wrapper_env)]
    fn build_events_client_returns_none_when_uuid_unreadable() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let data_dir = tempdir.path().join("data");
        std::fs::create_dir_all(&data_dir).expect("data dir");
        // No metrics-uuid file written: read_metrics_uuid errors.
        let config = test_config(&tempdir, data_dir, /* disable_metrics */ false);

        with_var(
            "_FLOX_METRICS_URL_OVERRIDE",
            Some("http://127.0.0.1:9999"),
            || {
                let client = build_events_client(&config, Uuid::new_v4());
                assert!(client.is_none(), "missing uuid must short-circuit");
            },
        );
    }

    #[test]
    #[serial(canonical_events_wrapper_env)]
    fn build_events_client_returns_none_when_override_unset() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let uuid = Uuid::new_v4();
        let config = test_config_with_uuid(&tempdir, uuid);

        with_var("_FLOX_METRICS_URL_OVERRIDE", None::<&str>, || {
            let client = build_events_client(&config, Uuid::new_v4());
            assert!(client.is_none(), "production must be dormant pre-cutover");
        });
    }

    #[test]
    #[serial(canonical_events_wrapper_env)]
    fn build_events_client_returns_none_when_override_unparseable() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let uuid = Uuid::new_v4();
        let config = test_config_with_uuid(&tempdir, uuid);

        with_var("_FLOX_METRICS_URL_OVERRIDE", Some("not a url"), || {
            let client = build_events_client(&config, Uuid::new_v4());
            assert!(client.is_none(), "unparseable override must short-circuit");
        });
    }

    #[test]
    #[serial(canonical_events_wrapper_env)]
    fn build_events_client_returns_some_when_override_is_parseable() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let uuid = Uuid::new_v4();
        let config = test_config_with_uuid(&tempdir, uuid);

        with_var(
            "_FLOX_METRICS_URL_OVERRIDE",
            Some("http://127.0.0.1:9999/"),
            || {
                let client = build_events_client(&config, Uuid::new_v4());
                assert!(client.is_some(), "parseable override must yield a client");
                let client = client.unwrap();
                assert_eq!(client.device_id, uuid);
            },
        );
    }

    /// End-to-end test mirroring the spec's "one-run-one-completed" AC:
    /// install a hub-owned client backed by a [`MockEventsConnection`],
    /// record run + completed for one invocation, and assert exactly one
    /// of each lands on the connection sharing one `invocation_id`.
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
            template,
            connection,
        );

        let previous = EventsHub::global().set_client(client);

        EventsHub::global()
            .record_command_run("install".to_string())
            .expect("record run");
        EventsHub::global()
            .record_command_completed("install".to_string())
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
