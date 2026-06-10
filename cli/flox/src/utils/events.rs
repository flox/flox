//! Integration wrapper between the flox binary and the [`flox_events`] crate.
//!
//! This module is the **single integration surface** through which the flox
//! binary decides whether to install an [`EventsClient`] and how to populate
//! the shared metadata stamped onto every v2 event. The
//! [`flox_events`] crate itself is a clean leaf — no `flox-rust-sdk` edge, no
//! `env!` macros, no `Config` access — and this wrapper holds all of the
//! integration concerns that would otherwise have to live in the crate.
//!
//! # Runtime stack selection
//!
//! The CLI ships two telemetry stacks side-by-side: the legacy
//! `subcommand_metric!` pipeline (`cli/flox/src/utils/metrics.rs`) and the
//! new canonical-events pipeline (this module + the `flox-events` crate).
//! Exactly one stack installs its `Client` per process; the other stack's
//! `Client` is left `None` and its `record_*` calls short-circuit.
//!
//! [`selected_metrics_stack`] reads the [`FLOX_METRICS_STACK_VAR`] env var
//! once during startup and returns which stack to install:
//!
//! - unset / `new` (default) → [`MetricsStack::New`] — canonical envelopes
//!   to the new ingest endpoint; the legacy stack's `Client` is not
//!   installed.
//! - `legacy` → [`MetricsStack::Legacy`] — PostHog-shape payloads to the
//!   legacy ingest endpoint exactly as the prior release did; the new
//!   pipeline's `Client` is not installed.
//! - any other value → [`MetricsStack::New`] with a single `tracing::warn!`
//!   so the misconfiguration surfaces.
//!
//! Net result: exactly one emission stream per process, picked by the flag.
//! The legacy stack stays code-present and runtime-reachable until a future
//! cleanup removes it and this flag together.

use std::env;
use std::str::FromStr;
use std::sync::OnceLock;

use flox_events::{EnvDetail, EventsClient, EventsGuard, EventsHub, SharedMetadataTemplate};
use flox_rust_sdk::flox::FLOX_VERSION;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment};
use flox_rust_sdk::utils::INVOCATION_SOURCES;
use tracing::debug;
use uuid::Uuid;

use crate::config::Config;
use crate::utils::metrics::read_metrics_uuid;

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

/// User-facing env var that selects which telemetry stack the CLI's
/// startup chokepoint installs a `Client` on. Accepted values are
/// `new` (also the default, also unset) and `legacy`; any other value
/// behaves as `new` and logs a single `warn!`.
///
/// See [`selected_metrics_stack`] for the resolution rules and the
/// module rustdoc for the "Runtime stack selection" overview.
pub(crate) const FLOX_METRICS_STACK_VAR: &str = "FLOX_METRICS_STACK";

/// Build-time URL for the new canonical-events ingest endpoint. Injected
/// by the Nix wrapper (`pkgs/flox-cli/default.nix`) at the same site as
/// the legacy `METRICS_EVENTS_URL`.
const METRICS_EVENTS_URL_V2: &str = env!("METRICS_EVENTS_URL_V2");

/// Build-time API key for the new canonical-events ingest endpoint.
/// The new endpoint uses its own key (the original D1 assumption that
/// both stacks could share the legacy `METRICS_EVENTS_API_KEY` was
/// superseded once the new endpoint was stood up). The legacy stack
/// continues to read [`METRICS_EVENTS_API_KEY`] unchanged from
/// `cli/flox/src/utils/metrics.rs`.
const METRICS_EVENTS_API_KEY_V2: &str = env!("METRICS_EVENTS_API_KEY_V2");

/// Which telemetry stack the CLI installs a `Client` on for this
/// process — see [`selected_metrics_stack`] and the module rustdoc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MetricsStack {
    /// New canonical-events pipeline (default). Installs an
    /// [`EventsClient`] pointing at the new ingest endpoint; the legacy
    /// stack's `Client` is left uninstalled and its `record_metric`
    /// short-circuits.
    New,
    /// Legacy `subcommand_metric!` pipeline. Installs the legacy
    /// `Client` pointing at the legacy ingest endpoint; the new
    /// pipeline's `Client` is left uninstalled and its `record_event`
    /// short-circuits. This is the in-field rollback handle —
    /// `FLOX_METRICS_STACK=legacy` reverts a deployed binary to its
    /// prior-release behavior without a rebuild.
    Legacy,
}

/// Read the [`FLOX_METRICS_STACK_VAR`] env var and return which stack
/// to install a `Client` on for this process.
///
/// Resolution rules:
///
/// - unset, empty, or `new` → [`MetricsStack::New`]
/// - `legacy` → [`MetricsStack::Legacy`]
/// - any other value → [`MetricsStack::New`] with a single
///   `tracing::warn!` so the misconfiguration is loud. Fail-closed to
///   the production default — never emit nothing because the flag was
///   misspelled.
///
/// The resolution is cached process-wide in a `OnceLock` so both
/// `Client` install sites (the legacy install at
/// `cli/flox/src/commands/mod.rs` and the new install via
/// [`build_events_client`]) see the same value with exactly **one**
/// warning per process — calling this function N times from a
/// process that set `FLOX_METRICS_STACK=banana` produces exactly one
/// warn, not N. Tests bypass the cache via a `#[cfg(test)]`
/// alternate definition so each test can exercise a different env
/// value via `temp_env::with_var`.
#[cfg(not(test))]
pub(crate) fn selected_metrics_stack() -> MetricsStack {
    static CACHED: OnceLock<MetricsStack> = OnceLock::new();
    *CACHED.get_or_init(read_metrics_stack_from_env)
}

#[cfg(test)]
pub(crate) fn selected_metrics_stack() -> MetricsStack {
    read_metrics_stack_from_env()
}

fn read_metrics_stack_from_env() -> MetricsStack {
    match env::var(FLOX_METRICS_STACK_VAR) {
        Ok(raw) => match raw.as_str() {
            "" | "new" => MetricsStack::New,
            "legacy" => MetricsStack::Legacy,
            other => {
                tracing::warn!(
                    value = %other,
                    "{FLOX_METRICS_STACK_VAR} has an unrecognized value; defaulting to `new`"
                );
                MetricsStack::New
            },
        },
        Err(_) => MetricsStack::New,
    }
}

/// Resolve the invocation id for the current process.
///
/// If [`FLOX_INVOCATION_ID_VAR`] is set in the environment and parses as a
/// UUID, the process inherits it from a parent flox invocation — so its
/// v2 events join the parent's stream downstream. Otherwise a fresh
/// v4 UUID is minted, marking this as a top-level user invocation.
pub fn resolve_invocation_id() -> Uuid {
    let resolved = match env::var(FLOX_INVOCATION_ID_VAR) {
        Ok(raw) => match Uuid::from_str(&raw) {
            Ok(uuid) => {
                debug!(invocation_id = %uuid, "inherited v2 invocation_id from FLOX_INVOCATION_ID");
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
/// the parent's v2 event stream rather than minting a fresh top-level
/// invocation downstream.
pub fn current_invocation_id() -> Option<Uuid> {
    RESOLVED_INVOCATION_ID.get().copied()
}

/// Build the [`SharedMetadataTemplate`] stamped onto every v2 event
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
/// Returns `None` (no client installed, [`flox_events::EventsHub::record_event`]
/// short-circuits) when any of the following holds:
///
/// - [`Config::flox::disable_metrics`] is `true` (the same gate the legacy
///   metrics pipeline honors at `cli/flox/src/main.rs` and
///   `cli/flox/src/commands/mod.rs`). Honoring the gate is consent-affecting:
///   silent telemetry-after-opt-out would be a privacy violation in a
///   public OSS repo.
/// - [`selected_metrics_stack`] returns [`MetricsStack::Legacy`] — the user
///   has opted into the legacy stack as the in-field rollback handle. The
///   legacy `Client` is installed instead at the
///   `cli/flox/src/commands/mod.rs` chokepoint.
/// - [`read_metrics_uuid`] returns `Err` (missing or unparseable
///   per-installation uuid file). The CLI runs to completion normally;
///   only the v2 event stream is silenced for the run.
/// - The dev/test override `_FLOX_METRICS_URL_OVERRIDE` is set but not
///   parseable as a URL. Unparseable user override is surfaced via the
///   existing `debug!` log so the typo is visible under `RUST_LOG=debug`.
///
/// Otherwise — the production path — returns `Some(EventsClient)` pointing
/// at [`_FLOX_METRICS_URL_OVERRIDE`] when set to a parseable URL, falling
/// back to the build-injected [`METRICS_EVENTS_URL_V2`]. The client
/// authenticates with the build-injected
/// [`METRICS_EVENTS_API_KEY_V2`] — the new endpoint's own API key,
/// distinct from the legacy stack's
/// [`crate::utils::metrics::METRICS_EVENTS_API_KEY`].
pub fn build_events_client(config: &Config, invocation_id: Uuid) -> Option<EventsClient> {
    if config.flox.disable_metrics {
        debug!("v2 events: disable_metrics is true; not installing client");
        return None;
    }

    if selected_metrics_stack() == MetricsStack::Legacy {
        debug!(
            "Canonical events: {FLOX_METRICS_STACK_VAR}=legacy; new pipeline inert this process"
        );
        return None;
    }

    let device_id = match read_metrics_uuid(config) {
        Ok(uuid) => uuid,
        Err(err) => {
            debug!(error = %err, "v2 events: could not read metrics uuid; not installing client");
            return None;
        },
    };

    let endpoint_url = match resolve_endpoint_url() {
        Ok(url) => url,
        Err(EndpointUrlError::OverrideUnparseable) => {
            // Unparseable override is user error worth surfacing under
            // `RUST_LOG=debug` — do not silently fall back to the
            // build-injected URL because the user's intent was
            // explicitly to redirect.
            return None;
        },
    };

    Some(EventsClient::new(
        device_id,
        &config.flox.data_dir,
        endpoint_url,
        METRICS_EVENTS_API_KEY_V2,
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
/// When [`build_events_client`] returns `None` (disable_metrics, the
/// legacy stack is selected via `FLOX_METRICS_STACK=legacy`, or one
/// of the other short-circuit cases), the guard is still returned
/// but its flush is a no-op — `EventsHub::global()` has no client
/// installed and `record_event` short-circuits.
pub fn install_events_client_for_main(config: &Config, invocation_id: Uuid) -> EventsGuard {
    if let Some(client) = build_events_client(config, invocation_id) {
        EventsHub::global().set_client(client);
    }
    EventsGuard::new()
}

/// Build an [`EnvDetail`] for the supplied [`ConcreteEnvironment`], using
/// the same env-kind / env-ref mapping the legacy
/// `environment_subcommand_metric!` macro at
/// `cli/flox/src/utils/metrics.rs:57-71` applies.
///
/// This is the **single shared helper** the spec mandates for the new path
/// — the per-kind match must not be duplicated across `activate`, `push`,
/// and `pull` call sites. (PR 2's `pull.rs:103` `NewAbbreviated` branch is
/// the lone exception: it runs before a `ConcreteEnvironment` is
/// materialized, so it constructs the `EnvDetail` directly from the
/// `RemoteRef` available there — see the call-site comment.)
pub fn env_detail_from_concrete(env: &ConcreteEnvironment) -> EnvDetail {
    match env {
        ConcreteEnvironment::Remote(environment) => EnvDetail {
            env_kind: "remote".to_string(),
            env_ref_or_name: environment.env_ref().to_string(),
        },
        ConcreteEnvironment::Managed(environment) => EnvDetail {
            env_kind: "managed".to_string(),
            env_ref_or_name: environment.env_ref().to_string(),
        },
        ConcreteEnvironment::Path(environment) => EnvDetail {
            env_kind: "path".to_string(),
            env_ref_or_name: Environment::name(environment).to_string(),
        },
    }
}

/// Error returned by [`resolve_endpoint_url`] when the user-supplied
/// `_FLOX_METRICS_URL_OVERRIDE` is set but doesn't parse as a URL. The
/// caller surfaces this as "no client installed for this run" rather
/// than silently falling back to the build-injected default — the
/// override is an explicit user intent to redirect, so a typo there
/// should fail loudly under `RUST_LOG=debug`, not be papered over.
#[derive(Debug, Clone, Copy)]
enum EndpointUrlError {
    OverrideUnparseable,
}

/// Resolve the endpoint URL for the v2 events client.
///
/// Returns:
/// - the parsed value of `_FLOX_METRICS_URL_OVERRIDE` when set to a
///   parseable URL — the dev/test capture hatch the legacy pipeline
///   already honors, so with it set both stacks emit to the same local
///   collector and the payloads can be diffed.
/// - the build-injected [`METRICS_EVENTS_URL_V2`] when the override is
///   unset or empty — the production path.
/// - [`EndpointUrlError::OverrideUnparseable`] when the override is set
///   but the value is not a parseable URL.
fn resolve_endpoint_url() -> Result<String, EndpointUrlError> {
    let raw = match env::var("_FLOX_METRICS_URL_OVERRIDE") {
        Ok(value) if !value.is_empty() => value,
        _ => return Ok(METRICS_EVENTS_URL_V2.to_string()),
    };
    match url::Url::parse(&raw) {
        Ok(parsed) => Ok(parsed.to_string()),
        Err(err) => {
            // Deliberately do not log `raw` — a dev who experiments with
            // putting credentials in the override URL should not have them
            // captured by a `RUST_LOG=debug` tracing subscriber.
            debug!(
                error = %err,
                "v2 events: _FLOX_METRICS_URL_OVERRIDE is unparseable; not installing client"
            );
            Err(EndpointUrlError::OverrideUnparseable)
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

    /// Pin the parent process's `FLOX_METRICS_STACK` to unset for tests
    /// that exercise the default-stack branch — otherwise a CI runner
    /// that pre-set the var would silently flip the test's stack and
    /// invert the assertions.
    fn with_default_stack<F: FnOnce() -> R, R>(f: F) -> R {
        with_var(FLOX_METRICS_STACK_VAR, None::<&str>, f)
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

        with_default_stack(|| {
            with_var(
                "_FLOX_METRICS_URL_OVERRIDE",
                Some("http://127.0.0.1:9999"),
                || {
                    let client = build_events_client(&config, Uuid::new_v4());
                    assert!(client.is_none(), "disable_metrics must take priority");
                },
            );
        });
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn build_events_client_returns_none_when_uuid_unreadable() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let data_dir = tempdir.path().join("data");
        std::fs::create_dir_all(&data_dir).expect("data dir");
        // No metrics-uuid file written: read_metrics_uuid errors.
        let config = test_config(&tempdir, data_dir, /* disable_metrics */ false);

        with_default_stack(|| {
            with_var(
                "_FLOX_METRICS_URL_OVERRIDE",
                Some("http://127.0.0.1:9999"),
                || {
                    let client = build_events_client(&config, Uuid::new_v4());
                    assert!(client.is_none(), "missing uuid must short-circuit");
                },
            );
        });
    }

    /// With `FLOX_METRICS_STACK` unset (default `new`) and
    /// `_FLOX_METRICS_URL_OVERRIDE` unset, the production fallback to
    /// `METRICS_EVENTS_URL_V2` installs a real client. Pin both the
    /// success of the default-stack install and the use of the
    /// build-injected URL — without this assertion a future refactor
    /// that re-introduced the no-override `None` branch would silently
    /// disable production telemetry.
    #[test]
    #[serial(v2_events_wrapper_env)]
    fn build_events_client_returns_some_on_default_stack_with_v2_url_fallback() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let uuid = Uuid::new_v4();
        let config = test_config_with_uuid(&tempdir, uuid);

        with_default_stack(|| {
            with_var("_FLOX_METRICS_URL_OVERRIDE", None::<&str>, || {
                let client = build_events_client(&config, Uuid::new_v4());
                assert!(
                    client.is_some(),
                    "default stack must install a client using the build-injected URL"
                );
                assert_eq!(client.unwrap().device_id, uuid);
            });
        });
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn build_events_client_returns_none_when_override_unparseable() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let uuid = Uuid::new_v4();
        let config = test_config_with_uuid(&tempdir, uuid);

        with_default_stack(|| {
            with_var("_FLOX_METRICS_URL_OVERRIDE", Some("not a url"), || {
                let client = build_events_client(&config, Uuid::new_v4());
                assert!(client.is_none(), "unparseable override must short-circuit");
            });
        });
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn build_events_client_returns_some_when_override_is_parseable() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let uuid = Uuid::new_v4();
        let config = test_config_with_uuid(&tempdir, uuid);

        with_default_stack(|| {
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
        });
    }

    /// `FLOX_METRICS_STACK=legacy` is the rollback handle: the new
    /// pipeline's `Client` must NOT install, even when the override is
    /// set and the metrics uuid is readable. The legacy `Client` is
    /// installed instead at the `commands/mod.rs` chokepoint.
    #[test]
    #[serial(canonical_events_wrapper_env)]
    fn build_events_client_returns_none_on_stack_legacy_even_with_override() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let uuid = Uuid::new_v4();
        let config = test_config_with_uuid(&tempdir, uuid);

        with_var(FLOX_METRICS_STACK_VAR, Some("legacy"), || {
            with_var(
                "_FLOX_METRICS_URL_OVERRIDE",
                Some("http://127.0.0.1:9999/"),
                || {
                    let client = build_events_client(&config, Uuid::new_v4());
                    assert!(
                        client.is_none(),
                        "stack=legacy must leave the new pipeline's Client uninstalled"
                    );
                },
            );
        });
    }

    #[test]
    #[serial(canonical_events_wrapper_env)]
    fn selected_metrics_stack_defaults_to_new_when_unset() {
        with_var(FLOX_METRICS_STACK_VAR, None::<&str>, || {
            assert_eq!(selected_metrics_stack(), MetricsStack::New);
        });
    }

    #[test]
    #[serial(canonical_events_wrapper_env)]
    fn selected_metrics_stack_defaults_to_new_when_empty() {
        with_var(FLOX_METRICS_STACK_VAR, Some(""), || {
            assert_eq!(selected_metrics_stack(), MetricsStack::New);
        });
    }

    #[test]
    #[serial(canonical_events_wrapper_env)]
    fn selected_metrics_stack_returns_new_for_explicit_new() {
        with_var(FLOX_METRICS_STACK_VAR, Some("new"), || {
            assert_eq!(selected_metrics_stack(), MetricsStack::New);
        });
    }

    #[test]
    #[serial(canonical_events_wrapper_env)]
    fn selected_metrics_stack_returns_legacy_for_legacy() {
        with_var(FLOX_METRICS_STACK_VAR, Some("legacy"), || {
            assert_eq!(selected_metrics_stack(), MetricsStack::Legacy);
        });
    }

    /// Unknown values fail-closed to the production default `new`
    /// rather than emitting nothing — a misspelled flag value must not
    /// silently disable telemetry. The single `warn!` makes the misconfig
    /// visible under any tracing subscriber.
    #[test]
    #[serial(canonical_events_wrapper_env)]
    fn selected_metrics_stack_falls_back_to_new_for_unknown_value() {
        with_var(FLOX_METRICS_STACK_VAR, Some("banana"), || {
            assert_eq!(selected_metrics_stack(), MetricsStack::New);
        });
    }

    /// Sentinel: the build-injected [`METRICS_EVENTS_URL_V2`] must not
    /// be the placeholder. Pairs with the companion `lib.warnIf` in
    /// `pkgs/flox-cli/default.nix` so a release built with the
    /// placeholder fails both `nix build` (warning) and `cargo test`
    /// (this assertion). Survives rebases / merge conflicts that a
    /// comment alone wouldn't.
    #[test]
    fn metrics_events_url_v2_is_not_placeholder() {
        assert!(
            !METRICS_EVENTS_URL_V2.contains("example.invalid"),
            "METRICS_EVENTS_URL_V2 still has the .invalid placeholder; \
             replace it in pkgs/flox-cli/default.nix before merging."
        );
        assert!(
            !METRICS_EVENTS_URL_V2.contains("REPLACE-BEFORE-MERGE"),
            "METRICS_EVENTS_URL_V2 still has the REPLACE-BEFORE-MERGE \
             sentinel; replace it in pkgs/flox-cli/default.nix before \
             merging."
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
