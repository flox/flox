use std::path::{Path, PathBuf};

use anyhow::Result;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::buffer::EventsBuffer;
use crate::connection::{EventsConnection, EventsConnectionV2};
use crate::{
    CliCommandCompletedPayload,
    CliCommandRunPayload,
    Event,
    EventKind,
    LifecycleFields,
    SharedMetadataTemplate,
};

const DEFAULT_BUFFER_EXPIRY: Duration = Duration::minutes(2);
pub const BATCH_SIZE: usize = 100;

/// Client that stamps v2 event metadata, buffers events, and flushes
/// them through an [`EventsConnection`].
///
/// The connection owns the endpoint URL and credential; the client itself
/// holds the per-invocation identity (`device_id`, `invocation_id`,
/// `auth_subject`) and the static shared metadata template stamped onto
/// every command event payload.
///
/// `auth_subject` is the OIDC `sub` claim of the FloxHub auth token when
/// one was present at client construction time — an opaque, pseudonymous
/// subject identifier (e.g. `github|3670948`). It is never the user's
/// email, handle, or display name; the caller is responsible for passing
/// only the `sub` claim. Anonymous invocations pass `None` and every
/// emitted [`Event`] then omits the field. Like `device_id` and
/// `invocation_id`, the value is a per-process snapshot: a token change
/// mid-invocation does not re-stamp events.
#[derive(Debug)]
pub struct EventsClient {
    pub device_id: Uuid,
    pub data_dir: PathBuf,
    pub invocation_id: Uuid,
    pub auth_subject: Option<String>,
    pub max_age: Duration,
    pub connection: Box<dyn EventsConnection>,
    shared_metadata: SharedMetadataTemplate,
}

impl EventsClient {
    pub fn new(
        device_id: Uuid,
        data_dir: impl AsRef<Path>,
        endpoint_url: impl Into<String>,
        api_key: impl Into<String>,
        invocation_id: Uuid,
        auth_subject: Option<String>,
        shared_metadata: SharedMetadataTemplate,
    ) -> Self {
        let connection = EventsConnectionV2::new(endpoint_url, api_key);
        Self::new_with_connection(
            device_id,
            data_dir,
            invocation_id,
            auth_subject,
            shared_metadata,
            connection,
        )
    }

    pub fn new_with_connection(
        device_id: Uuid,
        data_dir: impl AsRef<Path>,
        invocation_id: Uuid,
        auth_subject: Option<String>,
        shared_metadata: SharedMetadataTemplate,
        connection: impl EventsConnection + 'static,
    ) -> Self {
        Self {
            device_id,
            data_dir: data_dir.as_ref().to_path_buf(),
            invocation_id,
            auth_subject,
            max_age: DEFAULT_BUFFER_EXPIRY,
            connection: connection.boxed(),
            shared_metadata,
        }
    }

    /// Record a `cli.command_run` event for `subcommand` — the one event
    /// per invocation carrying the full command context built from the
    /// client's shared metadata.
    pub fn record_command_run(&self, subcommand: String) -> Result<()> {
        let payload = CliCommandRunPayload::new(self.shared_metadata.into_payload(subcommand));
        self.record_event(EventKind::CliCommandRun(payload))
    }

    /// Record a `cli.command_completed` event carrying the dispatch
    /// lifecycle fields. The full command context stays on `cli.command_run`;
    /// this payload keeps only the subcommand plus the lifecycle.
    pub fn record_command_completed(
        &self,
        subcommand: String,
        lifecycle: LifecycleFields,
    ) -> Result<()> {
        self.record_event(EventKind::CliCommandCompleted(
            CliCommandCompletedPayload::new(subcommand, lifecycle),
        ))
    }

    pub fn record_event(&self, kind: EventKind) -> Result<()> {
        let event = Event {
            event_id: Uuid::new_v4(),
            event_timestamp: OffsetDateTime::now_utc(),
            source: "cli",
            invocation_id: self.invocation_id,
            device_id: self.device_id,
            auth_subject: self.auth_subject.clone(),
            kind,
        };

        let mut events_buffer = EventsBuffer::read(&self.data_dir)?;
        events_buffer.push(event)?;
        Ok(())
    }

    pub fn flush(&mut self, force: bool) -> Result<()> {
        let mut events = EventsBuffer::read(&self.data_dir)?;
        if !events.is_expired(self.max_age) && !force {
            return Ok(());
        }

        while !events.is_empty() {
            let batch_size = events.batch_size(BATCH_SIZE);
            {
                let batch: Vec<&Event> = events.iter().take(batch_size).collect();
                self.connection.send(batch)?;
            }

            events.drain_sent(batch_size);
            events.overwrite_file()?;
        }

        Ok(())
    }
}
