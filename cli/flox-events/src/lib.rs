//! Canonical telemetry event types emitted by the flox CLI.
//!
//! Types only. There is no sink, no context, and no emission machinery
//! in this crate.

use serde::Serialize;
use serde_with::{TimestampMilliSeconds, serde_as};
use time::OffsetDateTime;
use uuid::Uuid;

/// A single telemetry event in the canonical envelope shape.
///
/// `source` is always `"cli"`. `kind` carries the variant tag and its
/// typed payload and is flattened into the envelope, so the wire shape
/// is `{event_id, event_timestamp, source, invocation_id, device_id,
/// auth_subject?, event_type, payload}`.
///
/// Serialize-only: the CLI produces events, it never reads them back.
#[serde_as]
#[derive(Debug, Clone, Serialize)]
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

/// The set of event variants the CLI emits.
///
/// The dotted wire name on `#[serde(rename)]` is the single source of
/// truth for each variant; call sites use the enum, never a string
/// literal. `derive_more::From` is derived so a call site can pass a
/// payload value directly to anything accepting `impl Into<EventKind>`.
#[derive(Debug, Clone, Serialize, derive_more::From)]
#[serde(tag = "event_type", content = "payload")]
pub enum EventKind {
    #[serde(rename = "cli.command_run")]
    CliCommandRun(CliCommandRunPayload),
    #[serde(rename = "cli.command_completed")]
    CliCommandCompleted(CliCommandCompletedPayload),
}

/// Payload for [`EventKind::CliCommandRun`]. Serializes to `{}`.
#[derive(Debug, Clone, Serialize)]
pub struct CliCommandRunPayload {}

/// Payload for [`EventKind::CliCommandCompleted`]. Serializes to `{}`.
#[derive(Debug, Clone, Serialize)]
pub struct CliCommandCompletedPayload {}

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

    #[test]
    fn command_run_serializes_to_canonical_envelope() {
        let value = serde_json::to_value(fixed_event(EventKind::CliCommandRun(
            CliCommandRunPayload {},
        )))
        .expect("event serializes");
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.command_run",
            "payload": {},
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn command_completed_serializes_to_canonical_envelope() {
        let value = serde_json::to_value(fixed_event(EventKind::CliCommandCompleted(
            CliCommandCompletedPayload {},
        )))
        .expect("event serializes");
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.command_completed",
            "payload": {},
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn auth_subject_serializes_when_present() {
        let mut event = fixed_event(EventKind::CliCommandRun(CliCommandRunPayload {}));
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
            "payload": {},
        });
        assert_eq!(value, expected);
    }
}
