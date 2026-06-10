//! Canonical telemetry events emitted by the Flox CLI.
//!
//! This crate contains the canonical event envelope and the self-contained
//! pipeline for buffering and sending `cli.*` events. The global hub is dormant
//! until a client is installed by the CLI.

mod buffer;
mod client;
mod connection;
mod guard;
mod hub;

pub use buffer::{EVENTS_BUFFER_FILE_NAME, EventsBuffer};
pub use client::{BATCH_SIZE, EventsClient};
pub use connection::{CanonicalEventsConnection, EventsConnection, TRAILING_NETWORK_CALL_TIMEOUT};
pub use guard::EventsGuard;
pub use hub::EventsHub;
use serde::{Deserialize, Serialize, de};
use serde_with::{TimestampMilliSeconds, serde_as};
use time::OffsetDateTime;
use uuid::Uuid;

#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    pub use crate::connection::MockEventsConnection;
}

const CLI_SOURCE: &str = "cli";

/// A single telemetry event in the canonical envelope shape.
///
/// `source` is always `"cli"`. `kind` carries the variant tag and its
/// typed payload and is flattened into the envelope, so the wire shape
/// is `{event_id, event_timestamp, source, invocation_id, device_id,
/// auth_subject?, event_type, payload}`.
///
/// The CLI serializes events for transport and deserializes the same shape to
/// reload its local buffer.
#[serde_as]
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Event {
    /// Unique id for this event (used downstream for de-duplication).
    pub event_id: Uuid,
    /// When the event occurred. Serialized as an integer millisecond
    /// count since the Unix epoch — matches the downstream
    /// `DateTime64(3, 'UTC')` storage granularity, avoids the
    /// `f64`-mantissa precision loss that bites nanosecond timestamps
    /// when consumers parse JSON numbers as floats, and avoids the
    /// timezone-ambiguity class entirely (no offset, no DST gaps).
    #[serde_as(as = "TimestampMilliSeconds<i64>")]
    pub event_timestamp: OffsetDateTime,
    /// The producer. Always `"cli"`.
    pub source: &'static str,
    /// Correlates every event emitted during one CLI invocation.
    pub invocation_id: Uuid,
    /// Stable per-installation id.
    pub device_id: Uuid,
    /// Pseudonymous authenticated-subject identifier — the OIDC/JWT
    /// `sub` claim (sourced from the auth token) when known. Must not
    /// contain email addresses, raw user handles, or token bytes — those
    /// are PII and a different category from this field's pseudonymous-
    /// identifier contract.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_subject: Option<String>,
    /// The event variant and its typed payload. Flattened into the
    /// envelope: the variant's `#[serde(rename)]` becomes `event_type`
    /// and the variant's payload struct becomes `payload`.
    #[serde(flatten)]
    pub kind: EventKind,
}

#[serde_as]
#[derive(Debug, Deserialize)]
struct EventWire {
    event_id: Uuid,
    #[serde_as(as = "TimestampMilliSeconds<i64>")]
    event_timestamp: OffsetDateTime,
    source: String,
    invocation_id: Uuid,
    device_id: Uuid,
    auth_subject: Option<String>,
    #[serde(flatten)]
    kind: EventKind,
}

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = EventWire::deserialize(deserializer)?;
        if wire.source != CLI_SOURCE {
            return Err(de::Error::custom(format!(
                "expected canonical event source {CLI_SOURCE:?}, got {:?}",
                wire.source
            )));
        }

        Ok(Self {
            event_id: wire.event_id,
            event_timestamp: wire.event_timestamp,
            source: CLI_SOURCE,
            invocation_id: wire.invocation_id,
            device_id: wire.device_id,
            auth_subject: wire.auth_subject,
            kind: wire.kind,
        })
    }
}

/// The set of event variants the CLI emits.
///
/// The dotted wire name on `#[serde(rename)]` is the single source of
/// truth for each variant; call sites use the enum, never a string
/// literal. `derive_more::From` is derived so a call site can pass a
/// payload value directly to anything accepting `impl Into<EventKind>`.
#[derive(Debug, Clone, Serialize, Deserialize, derive_more::From, PartialEq, Eq)]
#[serde(tag = "event_type", content = "payload")]
pub enum EventKind {
    #[serde(rename = "cli.command_run")]
    CliCommandRun(CliCommandRunPayload),
    #[serde(rename = "cli.command_completed")]
    CliCommandCompleted(CliCommandCompletedPayload),
    #[serde(rename = "cli.environment.activate")]
    CliEnvironmentActivate(CliEnvironmentActivatePayload),
    #[serde(rename = "cli.environment.push")]
    CliEnvironmentPush(CliEnvironmentPushPayload),
    #[serde(rename = "cli.environment.pull")]
    CliEnvironmentPull(CliEnvironmentPullPayload),
}

/// Shared metadata fields stamped onto every `cli.*` command event payload.
///
/// These fields drive existing `cli.telemetry` reporting downstream, so the
/// new pipeline carries them on its payloads to preserve continuity once the
/// cutover flips production traffic. The shape mirrors the columns the legacy
/// `MetricEntry` carries today (with `extras` deferred to per-domain payloads
/// in later PRs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandPayload {
    /// Subcommand name derived from the parsed clap command (e.g. `install`,
    /// `activate`, or nested `services::start` under PR 5's encoding).
    pub subcommand: String,
    /// Flox CLI version string.
    pub flox_version: String,
    /// Coarse operating system family (e.g. `Mac OS`, `Linux`).
    pub os_family: Option<String>,
    /// OS family release version.
    pub os_family_release: Option<String>,
    /// Linux distribution id (e.g. `ubuntu`); `None` outside Linux.
    pub os: Option<String>,
    /// Linux distribution version (e.g. `22.04`); `None` outside Linux.
    pub os_version: Option<String>,
    /// CLI flags that were observed empty on this invocation. Reserved for
    /// the per-command instrumentation PRs.
    pub empty_flags: Vec<String>,
    /// Tokens describing how this CLI invocation was launched (shell, prompt,
    /// service runner, etc.). Mirrors the legacy `INVOCATION_SOURCES`.
    pub invocation_sources: Vec<String>,
}

/// Static slice of [`CommandPayload`] that is constant for the duration of
/// one CLI invocation.
///
/// Pass into [`EventsClient::new`] so the client can stamp every command
/// event it emits without the call site rebuilding the same fields each
/// time. The `subcommand` field is supplied per-emission and merged in by
/// [`SharedMetadataTemplate::into_payload`].
#[derive(Debug, Clone)]
pub struct SharedMetadataTemplate {
    pub flox_version: String,
    pub os_family: Option<String>,
    pub os_family_release: Option<String>,
    pub os: Option<String>,
    pub os_version: Option<String>,
    pub empty_flags: Vec<String>,
    pub invocation_sources: Vec<String>,
}

impl SharedMetadataTemplate {
    /// Merge the stored static fields with the supplied subcommand to produce
    /// a complete [`CommandPayload`] ready for serialization.
    pub fn into_payload(&self, subcommand: String) -> CommandPayload {
        CommandPayload {
            subcommand,
            flox_version: self.flox_version.clone(),
            os_family: self.os_family.clone(),
            os_family_release: self.os_family_release.clone(),
            os: self.os.clone(),
            os_version: self.os_version.clone(),
            empty_flags: self.empty_flags.clone(),
            invocation_sources: self.invocation_sources.clone(),
        }
    }
}

/// Payload for [`EventKind::CliCommandRun`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliCommandRunPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
}

impl CliCommandRunPayload {
    pub fn new(command: CommandPayload) -> Self {
        Self { command }
    }
}

/// Payload for [`EventKind::CliCommandCompleted`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliCommandCompletedPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
}

impl CliCommandCompletedPayload {
    pub fn new(command: CommandPayload) -> Self {
        Self { command }
    }
}

/// Environment kind a `flox activate` / `push` / `pull` invocation touched,
/// matching the three legacy `environment_subcommand_metric!` arms
/// (`remote_environment` / `managed_environment` / `path_environment`).
///
/// Carried on every `cli.environment.*` event so downstream classifiers can
/// reconstruct the legacy `*_environment` columns without re-parsing the
/// `event_type` tag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvDetail {
    /// One of `"remote"`, `"managed"`, `"path"` — the
    /// [`flox_rust_sdk::models::environment::ConcreteEnvironment`] variant
    /// the command operated on. `"managed"` is also used for `flox pull`'s
    /// `NewAbbreviated` branch, where only the remote `RemoteRef` is known
    /// at emission time (no materialized `ConcreteEnvironment` yet).
    pub env_kind: String,
    /// The environment's identifier — the result of `env_ref().to_string()`
    /// for remote and managed environments, and `Environment::name(...)`
    /// for path environments. Matches the value the legacy macros emit.
    pub env_ref_or_name: String,
}

/// Payload for [`EventKind::CliEnvironmentActivate`].
///
/// Carries the env detail plus the extras the legacy
/// `environment_subcommand_metric!("activate", ...)` and
/// `subcommand_metric!("activate", ...)` call sites in
/// `cli/flox/src/commands/activate.rs` recorded. Each call site emits one
/// event with only the fields it knows populated; the downstream consumer
/// correlates via `invocation_id`. `lockfile_version` lands here instead of
/// on a (dropped) `cli.environment.activate#version` pseudo-subcommand.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentActivatePayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_services: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_includes: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lockfile_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
}

impl CliEnvironmentActivatePayload {
    /// Construct an empty-extras payload — every Optional field defaulted
    /// to `None`. Call sites fill in the fields they know via the builder
    /// methods below or struct-literal field overrides.
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
            start_services: None,
            mode: None,
            has_includes: None,
            lockfile_version: None,
            shell: None,
        }
    }

    pub fn with_start_services(mut self, value: bool) -> Self {
        self.start_services = Some(value);
        self
    }

    pub fn with_mode(mut self, value: impl Into<String>) -> Self {
        self.mode = Some(value.into());
        self
    }

    pub fn with_has_includes(mut self, value: bool) -> Self {
        self.has_includes = Some(value);
        self
    }

    pub fn with_lockfile_version(mut self, value: impl Into<String>) -> Self {
        self.lockfile_version = Some(value.into());
        self
    }

    pub fn with_shell(mut self, value: impl Into<String>) -> Self {
        self.shell = Some(value.into());
        self
    }
}

/// Payload for [`EventKind::CliEnvironmentPush`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentPushPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentPushPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentPull`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentPullPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentPullPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The wire form of `OffsetDateTime::from_unix_timestamp(0)` under
    /// `TimestampMilliSeconds<i64>` — milliseconds since the Unix
    /// epoch, where 1970-01-01T00:00:00Z is exactly 0.
    const EPOCH_UNIX_MS: i64 = 0;

    fn fixed_event(kind: EventKind) -> Event {
        Event {
            event_id: Uuid::nil(),
            event_timestamp: OffsetDateTime::from_unix_timestamp(0)
                .expect("0 is a valid unix timestamp"),
            source: "cli",
            invocation_id: Uuid::nil(),
            device_id: Uuid::nil(),
            auth_subject: None,
            kind,
        }
    }

    fn command_payload(subcommand: &str) -> CommandPayload {
        CommandPayload {
            subcommand: subcommand.to_string(),
            flox_version: "0.0.0-test".to_string(),
            os_family: Some("Linux".to_string()),
            os_family_release: Some("6.10.0".to_string()),
            os: Some("ubuntu".to_string()),
            os_version: Some("24.04".to_string()),
            empty_flags: vec![],
            invocation_sources: vec!["shell".to_string()],
        }
    }

    fn expected_payload_json(subcommand: &str) -> serde_json::Value {
        json!({
            "subcommand": subcommand,
            "flox_version": "0.0.0-test",
            "os_family": "Linux",
            "os_family_release": "6.10.0",
            "os": "ubuntu",
            "os_version": "24.04",
            "empty_flags": [],
            "invocation_sources": ["shell"],
        })
    }

    #[test]
    fn command_run_serializes_to_canonical_envelope() {
        let value = serde_json::to_value(fixed_event(EventKind::CliCommandRun(
            CliCommandRunPayload::new(command_payload("install")),
        )))
        .expect("event serializes");
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.command_run",
            "payload": expected_payload_json("install"),
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn command_completed_serializes_to_canonical_envelope() {
        let value = serde_json::to_value(fixed_event(EventKind::CliCommandCompleted(
            CliCommandCompletedPayload::new(command_payload("install")),
        )))
        .expect("event serializes");
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.command_completed",
            "payload": expected_payload_json("install"),
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn auth_subject_serializes_when_present() {
        let mut event = fixed_event(EventKind::CliCommandRun(CliCommandRunPayload::new(
            command_payload("install"),
        )));
        event.auth_subject = Some("test-subject-7f3a".to_string());
        let value = serde_json::to_value(event).expect("event serializes");
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "auth_subject": "test-subject-7f3a",
            "event_type": "cli.command_run",
            "payload": expected_payload_json("install"),
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn shared_metadata_template_merges_subcommand_into_payload() {
        let template = SharedMetadataTemplate {
            flox_version: "0.0.0-test".to_string(),
            os_family: Some("Linux".to_string()),
            os_family_release: Some("6.10.0".to_string()),
            os: Some("ubuntu".to_string()),
            os_version: Some("24.04".to_string()),
            empty_flags: vec![],
            invocation_sources: vec!["shell".to_string()],
        };
        let payload = template.into_payload("activate".to_string());
        assert_eq!(payload, command_payload("activate"));
    }

    fn env_detail(kind: &str, ref_or_name: &str) -> EnvDetail {
        EnvDetail {
            env_kind: kind.to_string(),
            env_ref_or_name: ref_or_name.to_string(),
        }
    }

    fn activate_envelope_json(payload_extras: serde_json::Value) -> serde_json::Value {
        let mut payload = expected_payload_json("activate");
        let payload_obj = payload.as_object_mut().expect("payload object");
        for (key, value) in payload_extras.as_object().expect("extras object") {
            payload_obj.insert(key.clone(), value.clone());
        }
        json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.environment.activate",
            "payload": payload,
        })
    }

    #[test]
    fn cli_environment_activate_remote_envelope_golden() {
        let payload = CliEnvironmentActivatePayload::new(
            command_payload("activate"),
            env_detail("remote", "alice/myenv"),
        )
        .with_start_services(false)
        .with_mode("dev");
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentActivate(payload)))
            .expect("event serializes");
        let expected = activate_envelope_json(json!({
            "env_kind": "remote",
            "env_ref_or_name": "alice/myenv",
            "start_services": false,
            "mode": "dev",
        }));
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_environment_activate_managed_envelope_golden() {
        let payload = CliEnvironmentActivatePayload::new(
            command_payload("activate"),
            env_detail("managed", "alice/myenv"),
        )
        .with_has_includes(true);
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentActivate(payload)))
            .expect("event serializes");
        let expected = activate_envelope_json(json!({
            "env_kind": "managed",
            "env_ref_or_name": "alice/myenv",
            "has_includes": true,
        }));
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_environment_activate_path_envelope_golden() {
        let payload = CliEnvironmentActivatePayload::new(
            command_payload("activate"),
            env_detail("path", "myenv"),
        )
        .with_lockfile_version("1")
        .with_shell("bash");
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentActivate(payload)))
            .expect("event serializes");
        let expected = activate_envelope_json(json!({
            "env_kind": "path",
            "env_ref_or_name": "myenv",
            "lockfile_version": "1",
            "shell": "bash",
        }));
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_environment_activate_omits_unset_extras() {
        // No extras populated: every Optional field is `None` and the
        // wire shape omits them entirely (skip_serializing_if).
        let payload = CliEnvironmentActivatePayload::new(
            command_payload("activate"),
            env_detail("path", "myenv"),
        );
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentActivate(payload)))
            .expect("event serializes");
        let expected = activate_envelope_json(json!({
            "env_kind": "path",
            "env_ref_or_name": "myenv",
        }));
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_environment_push_envelope_golden() {
        let payload = CliEnvironmentPushPayload::new(
            command_payload("push"),
            env_detail("managed", "alice/myenv"),
        );
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentPush(payload)))
            .expect("event serializes");
        let mut payload_json = expected_payload_json("push");
        let payload_obj = payload_json.as_object_mut().expect("payload object");
        payload_obj.insert("env_kind".to_string(), json!("managed"));
        payload_obj.insert("env_ref_or_name".to_string(), json!("alice/myenv"));
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.environment.push",
            "payload": payload_json,
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_environment_pull_envelope_golden() {
        // The `NewAbbreviated` branch in `pull.rs:103` constructs the
        // detail directly with `env_kind = "managed"`; assert that
        // shape on the wire so a future drift in the wrapper trips
        // this test.
        let payload = CliEnvironmentPullPayload::new(
            command_payload("pull"),
            env_detail("managed", "alice/myenv"),
        );
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentPull(payload)))
            .expect("event serializes");
        let mut payload_json = expected_payload_json("pull");
        let payload_obj = payload_json.as_object_mut().expect("payload object");
        payload_obj.insert("env_kind".to_string(), json!("managed"));
        payload_obj.insert("env_ref_or_name".to_string(), json!("alice/myenv"));
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.environment.pull",
            "payload": payload_json,
        });
        assert_eq!(value, expected);
    }
}

#[cfg(test)]
mod pipeline_tests {
    use pretty_assertions::assert_eq;
    use serial_test::serial;
    use tempfile::TempDir;

    use super::*;
    use crate::test_helpers::MockEventsConnection;

    const DEVICE_ID: Uuid = Uuid::from_u128(0xaaaaaaaa_aaaa_aaaa_aaaa_aaaaaaaaaaaa);
    const INVOCATION_ID: Uuid = Uuid::from_u128(0xbbbbbbbb_bbbb_bbbb_bbbb_bbbbbbbbbbbb);

    fn fixed_event(kind: EventKind) -> Event {
        Event {
            event_id: Uuid::from_u128(0x11111111_1111_1111_1111_111111111111),
            event_timestamp: OffsetDateTime::from_unix_timestamp(1_700_000_000)
                .expect("fixture timestamp is valid"),
            source: "cli",
            invocation_id: INVOCATION_ID,
            device_id: DEVICE_ID,
            auth_subject: None,
            kind,
        }
    }

    fn shared_metadata() -> SharedMetadataTemplate {
        SharedMetadataTemplate {
            flox_version: "0.0.0-test".to_string(),
            os_family: Some("Linux".to_string()),
            os_family_release: Some("6.10.0".to_string()),
            os: Some("ubuntu".to_string()),
            os_version: Some("24.04".to_string()),
            empty_flags: vec![],
            invocation_sources: vec!["shell".to_string()],
        }
    }

    fn command_run_kind() -> EventKind {
        EventKind::CliCommandRun(CliCommandRunPayload::new(
            shared_metadata().into_payload("install".to_string()),
        ))
    }

    fn command_completed_kind() -> EventKind {
        EventKind::CliCommandCompleted(CliCommandCompletedPayload::new(
            shared_metadata().into_payload("install".to_string()),
        ))
    }

    fn unix_timestamp_millis(time: OffsetDateTime) -> i128 {
        time.unix_timestamp_nanos() / 1_000_000
    }

    fn client_with_connection(tempdir: &TempDir, connection: MockEventsConnection) -> EventsClient {
        EventsClient::new_with_connection(
            DEVICE_ID,
            tempdir.path(),
            INVOCATION_ID,
            shared_metadata(),
            connection,
        )
    }

    #[test]
    fn events_buffer_round_trips_entries_from_disk() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let first = fixed_event(command_run_kind());
        let second = fixed_event(command_completed_kind());

        let mut buffer = EventsBuffer::read(tempdir.path()).expect("read empty buffer");
        buffer.push(first.clone()).expect("push first event");
        buffer.push(second.clone()).expect("push second event");
        drop(buffer);

        let buffer = EventsBuffer::read(tempdir.path()).expect("read persisted buffer");

        assert_eq!(buffer.iter().cloned().collect::<Vec<_>>(), vec![
            first, second
        ]);
    }

    #[test]
    fn events_hub_without_client_skips_recording() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let hub = EventsHub::new();

        hub.record_event(command_run_kind())
            .expect("record with no client");

        assert!(!tempdir.path().join(EVENTS_BUFFER_FILE_NAME).exists());
    }

    #[test]
    fn events_hub_records_and_flushes_when_client_is_set() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let hub = EventsHub::new();
        hub.set_client(client_with_connection(&tempdir, connection));

        hub.record_event(command_run_kind()).expect("record event");
        assert!(tempdir.path().join(EVENTS_BUFFER_FILE_NAME).exists());

        hub.flush(true).expect("flush events");

        let sent_batches = sent_batches.lock().expect("sent batches lock").clone();
        assert_eq!(sent_batches.len(), 1);
        assert_eq!(sent_batches[0].len(), 1);
        assert_eq!(sent_batches[0][0].kind, command_run_kind());
    }

    #[test]
    fn events_client_record_stamps_event_metadata() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let client = client_with_connection(&tempdir, MockEventsConnection::default());
        let before = OffsetDateTime::now_utc();

        client
            .record_event(command_completed_kind())
            .expect("record event");

        let after = OffsetDateTime::now_utc();
        let buffer = EventsBuffer::read(tempdir.path()).expect("read buffer");
        let event = buffer.iter().next().expect("one buffered event");

        assert_ne!(event.event_id, Uuid::nil());
        assert!(unix_timestamp_millis(event.event_timestamp) >= unix_timestamp_millis(before));
        assert!(unix_timestamp_millis(event.event_timestamp) <= unix_timestamp_millis(after));
        assert_eq!(event.source, "cli");
        assert_eq!(event.invocation_id, INVOCATION_ID);
        assert_eq!(event.device_id, DEVICE_ID);
        assert_eq!(event.auth_subject, None);
        assert_eq!(event.kind, command_completed_kind());
    }

    #[test]
    fn events_client_flush_batches_and_overwrites_buffer_file() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let mut client = client_with_connection(&tempdir, connection);

        for _ in 0..(BATCH_SIZE + 1) {
            client
                .record_event(command_run_kind())
                .expect("record event");
        }

        client.flush(true).expect("flush events");

        let sent_batches = sent_batches.lock().expect("sent batches lock").clone();
        assert_eq!(sent_batches.iter().map(Vec::len).collect::<Vec<_>>(), vec![
            BATCH_SIZE, 1
        ]);

        let buffer = EventsBuffer::read(tempdir.path()).expect("read buffer");
        assert_eq!(buffer.iter().count(), 0);
        assert_eq!(
            std::fs::read_to_string(tempdir.path().join(EVENTS_BUFFER_FILE_NAME))
                .expect("read buffer file"),
            ""
        );
    }

    #[test]
    fn events_client_flush_retains_buffer_when_connection_errors() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        connection.fail_next_send();
        let mut client = client_with_connection(&tempdir, connection);

        client
            .record_event(command_run_kind())
            .expect("record event");

        let err = client.flush(true).expect_err("flush should fail");
        assert!(err.to_string().contains("mock events send failed"));

        let buffer = EventsBuffer::read(tempdir.path()).expect("read buffer");
        let buffered = buffer.iter().cloned().collect::<Vec<_>>();
        assert_eq!(buffered.len(), 1);
        assert_eq!(buffered[0].kind, command_run_kind());
    }

    #[test]
    fn canonical_events_connection_serializes_canonical_json_body() {
        let event = fixed_event(command_run_kind());
        let body =
            CanonicalEventsConnection::serialize_events(&[&event]).expect("serialize events");

        assert_eq!(
            body,
            r#"[{"event_id":"11111111-1111-1111-1111-111111111111","event_timestamp":1700000000000,"source":"cli","invocation_id":"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb","device_id":"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa","event_type":"cli.command_run","payload":{"subcommand":"install","flox_version":"0.0.0-test","os_family":"Linux","os_family_release":"6.10.0","os":"ubuntu","os_version":"24.04","empty_flags":[],"invocation_sources":["shell"]}}]"#
        );
    }

    #[test]
    #[serial(global_events_client)]
    fn events_guard_drop_flushes_global_hub() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let previous_client =
            EventsHub::global().set_client(client_with_connection(&tempdir, connection));

        EventsHub::global()
            .record_event(command_run_kind())
            .expect("record event");
        drop(EventsGuard::new());

        EventsHub::global().clear_client();
        if let Some(previous_client) = previous_client {
            EventsHub::global().set_client(previous_client);
        }

        let sent_batches = sent_batches.lock().expect("sent batches lock").clone();
        assert_eq!(sent_batches.len(), 1);
        assert_eq!(sent_batches[0].len(), 1);
    }

    #[test]
    fn events_hub_record_command_run_stamps_subcommand_and_shared_metadata() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let hub = EventsHub::new();
        hub.set_client(client_with_connection(&tempdir, connection));

        hub.record_command_run("activate".to_string())
            .expect("record command_run");
        hub.flush(true).expect("flush events");

        let sent_batches = sent_batches.lock().expect("sent batches lock").clone();
        assert_eq!(sent_batches.len(), 1);
        assert_eq!(sent_batches[0].len(), 1);
        match &sent_batches[0][0].kind {
            EventKind::CliCommandRun(payload) => {
                assert_eq!(
                    payload.command,
                    shared_metadata().into_payload("activate".to_string())
                );
            },
            other => panic!("expected CliCommandRun, got {other:?}"),
        }
    }

    #[test]
    fn events_hub_record_command_completed_is_idempotent_per_install() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let hub = EventsHub::new();
        hub.set_client(client_with_connection(&tempdir, connection));

        hub.record_command_completed("install".to_string())
            .expect("first completed record succeeds");
        hub.record_command_completed("install".to_string())
            .expect("second completed record is a silent no-op");
        hub.flush(true).expect("flush events");

        let sent_batches = sent_batches.lock().expect("sent batches lock").clone();
        let total_events: usize = sent_batches.iter().map(Vec::len).sum();
        assert_eq!(
            total_events, 1,
            "second record_command_completed must be a no-op"
        );
    }

    #[test]
    fn events_hub_set_client_resets_completed_recorded_flag() {
        let first_dir = tempfile::tempdir().expect("first tempdir");
        let second_dir = tempfile::tempdir().expect("second tempdir");
        let first_conn = MockEventsConnection::default();
        let second_conn = MockEventsConnection::default();
        let first_batches = first_conn.sent_batches();
        let second_batches = second_conn.sent_batches();

        let hub = EventsHub::new();
        hub.set_client(client_with_connection(&first_dir, first_conn));
        hub.record_command_completed("install".to_string()).unwrap();
        hub.flush(true).unwrap();
        hub.set_client(client_with_connection(&second_dir, second_conn));
        hub.record_command_completed("upgrade".to_string())
            .expect("new install's completed record is allowed");
        hub.flush(true).unwrap();

        assert_eq!(
            first_batches
                .lock()
                .unwrap()
                .iter()
                .map(Vec::len)
                .sum::<usize>(),
            1
        );
        assert_eq!(
            second_batches
                .lock()
                .unwrap()
                .iter()
                .map(Vec::len)
                .sum::<usize>(),
            1
        );
    }
}
