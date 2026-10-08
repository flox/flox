use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use anyhow::Result;
use time::{Duration, OffsetDateTime};
use tracing::debug;
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

/// How long the oldest buffered event waits before the buffer is due.
const DEFAULT_BUFFER_EXPIRY: Duration = Duration::minutes(2);
/// The same wait in CI, where the runner is typically destroyed after the
/// job, so anything still buffered when the job ends is lost.
const CI_BUFFER_EXPIRY: Duration = Duration::seconds(10);
/// Marker in the data dir, created by the first successful send. The buffer
/// is due immediately until it exists, so a first run that ends before the
/// expiry (such as a CI job on an ephemeral runner) still delivers.
const FIRST_SEND_MARKER_FILE_NAME: &str = "events-v2-first-send";
pub const BATCH_SIZE: usize = 100;

/// Client that stamps v2 event metadata, buffers events, and flushes
/// them through an [`EventsConnection`].
///
/// The connection owns the endpoint URL and credential; the client itself
/// holds the per-invocation identity (`device_id`, `invocation_id`,
/// `auth_subject`) and the static shared metadata template stamped onto
/// every command event payload.
///
/// `auth_subject` is an opaque, pseudonymous subject identifier
/// (e.g. `github|3670948`). It is never the user's email, handle, or
/// display name.
/// Setting this is the caller's responsibility. In practice we use the
/// OIDC `sub` field directly for JWTs and the accounts `/me` identity for
/// PATs and SATs. Kerberos and logged-out invocations pass `None`, causing
/// every emitted [`Event`] to omit the field.
///
/// Like `device_id` and `invocation_id`, the value is a per-process
/// snapshot: a token change mid-invocation does not re-stamp events.
///
/// The buffer is due once its oldest event is older than `max_age`: two
/// minutes, or 10 seconds when the shared metadata's `invocation_sources`
/// include `ci` (or a `ci.*` sub-token). The buffer is also due regardless
/// of age until the first successful send from this data dir.
#[derive(Debug)]
pub struct EventsClient {
    pub device_id: Uuid,
    pub data_dir: PathBuf,
    pub invocation_id: Uuid,
    pub auth_subject: Option<String>,
    pub max_age: Duration,
    pub connection: Box<dyn EventsConnection>,
    shared_metadata: SharedMetadataTemplate,
    /// Snapshot from the last append, so checking expiry at command exit
    /// does not need to read the buffer again. Flushers still check on disk.
    oldest_buffered_timestamp: Option<OffsetDateTime>,
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
        let is_ci = shared_metadata.invocation_sources.iter().any(|source| {
            let source = source.to_ascii_lowercase();
            source == "ci" || source.starts_with("ci.")
        });
        Self {
            device_id,
            data_dir: data_dir.as_ref().to_path_buf(),
            invocation_id,
            auth_subject,
            max_age: if is_ci {
                CI_BUFFER_EXPIRY
            } else {
                DEFAULT_BUFFER_EXPIRY
            },
            connection: connection.boxed(),
            shared_metadata,
            oldest_buffered_timestamp: None,
        }
    }

    /// Record a `cli.command_run` event for `subcommand` — the one event
    /// per invocation carrying the full command context built from the
    /// client's shared metadata.
    pub fn record_command_run(&mut self, subcommand: String) -> Result<()> {
        let payload = CliCommandRunPayload::new(self.shared_metadata.into_payload(subcommand));
        self.record_event(EventKind::CliCommandRun(payload))
    }

    /// Record a `cli.command_completed` event carrying the dispatch
    /// lifecycle fields. The full command context stays on `cli.command_run`;
    /// this payload keeps only the subcommand plus the lifecycle.
    pub fn record_command_completed(
        &mut self,
        subcommand: String,
        lifecycle: LifecycleFields,
    ) -> Result<()> {
        self.record_event(EventKind::CliCommandCompleted(
            CliCommandCompletedPayload::new(subcommand, lifecycle),
        ))
    }

    pub fn record_event(&mut self, kind: EventKind) -> Result<()> {
        let auth_subject = self.auth_subject.clone();
        self.record_event_with_auth_subject(kind, auth_subject.as_deref())
    }

    pub(crate) fn record_event_with_auth_subject(
        &mut self,
        kind: EventKind,
        auth_subject: Option<&str>,
    ) -> Result<()> {
        let event = Event {
            event_id: Uuid::new_v4(),
            event_timestamp: OffsetDateTime::now_utc(),
            source: "cli",
            invocation_id: self.invocation_id,
            device_id: self.device_id,
            auth_subject: auth_subject.map(str::to_owned),
            producer_version: Some(self.shared_metadata.flox_version.clone()),
            kind,
        };

        self.oldest_buffered_timestamp = None;
        let mut events_buffer = EventsBuffer::read(&self.data_dir)?;
        events_buffer.push(event)?;
        self.oldest_buffered_timestamp = events_buffer.oldest_timestamp();
        Ok(())
    }

    /// The age past which the oldest buffered event makes the buffer due.
    /// `Duration::MIN` makes any buffered event due: used until the first
    /// successful send, since an ephemeral runner may never get a second
    /// chance.
    fn expiry(&self) -> Duration {
        if !self.data_dir.join(FIRST_SEND_MARKER_FILE_NAME).exists() {
            Duration::MIN
        } else {
            self.max_age
        }
    }

    /// Record that a send succeeded. Best-effort: a missing marker only
    /// means the next invocation sends again immediately.
    fn mark_sent(&self) {
        if let Err(err) = std::fs::write(self.data_dir.join(FIRST_SEND_MARKER_FILE_NAME), b"") {
            debug!(error = %err, "Could not write v2 events first-send marker");
        }
    }

    /// Advisory expiry check for deciding whether to start a background sender.
    /// Uses the last append's timestamp, or a non-blocking read if no timestamp
    /// is cached. A busy buffer is left for a later invocation. The sender must
    /// recheck: another process may have drained or re-buffered events since
    /// this snapshot was taken.
    pub fn is_flush_due(&self) -> Result<bool> {
        if let Some(oldest) = self.oldest_buffered_timestamp {
            return Ok(OffsetDateTime::now_utc() - oldest > self.expiry());
        }
        Ok(EventsBuffer::try_read(&self.data_dir)?
            .is_some_and(|events| events.is_expired(self.expiry())))
    }

    pub fn flush(&mut self, force: bool) -> Result<()> {
        self.oldest_buffered_timestamp = None;
        let mut events = EventsBuffer::read(&self.data_dir)?;
        if !events.is_expired(self.expiry()) && !force {
            return Ok(());
        }

        while !events.is_empty() {
            let batch_size = events.batch_size(BATCH_SIZE);
            {
                let batch: Vec<&Event> = events.iter().take(batch_size).collect();
                self.connection.send(batch)?;
            }
            self.mark_sent();

            events.drain_sent(batch_size);
            events.overwrite_file()?;
        }

        Ok(())
    }

    /// Flush using a non-blocking try-lock on the buffer file, never holding
    /// the lock across a network send.
    ///
    /// The buffer lock also gates the append path (`record_event`), so holding
    /// it across the send would block the next `flox` invocation's event
    /// recording for the full network timeout — the exact stall this pipeline's
    /// detached-flush design exists to avoid. So this drains the sendable
    /// entries to memory and truncates the file under the lock, releases the
    /// lock, then sends. A failed send re-buffers the unsent entries by
    /// re-reading and prepending, so nothing is lost on a send failure. (A
    /// crash between truncate and re-buffer can still lose the snapshot; see
    /// `EventsBuffer::take_sendable`.)
    ///
    /// Returns `Ok(false)` when another flusher holds the lock — the buffer
    /// was not drained but that is not an error. Returns `Ok(true)` when the
    /// flush ran (whether or not the expiry had elapsed).
    pub fn try_flush(&mut self, force: bool) -> Result<bool> {
        self.oldest_buffered_timestamp = None;
        let drained = {
            let Some(mut events) = EventsBuffer::try_read(&self.data_dir)? else {
                debug!("v2 events buffer lock held by another process; skipping flush");
                return Ok(false);
            };
            if !events.is_expired(self.expiry()) && !force {
                return Ok(true);
            }
            // Snapshot all sendable entries and remove them from the file while
            // the lock is held, then drop `events` to release the lock before
            // any network I/O.
            events.take_sendable()?
        };

        self.send_drained(drained)?;
        Ok(true)
    }

    /// Send entries already drained from the buffer, re-buffering any that a
    /// failed send leaves unsent. The lock is acquired only to re-buffer, never
    /// across the network call.
    fn send_drained(&mut self, mut drained: VecDeque<Event>) -> Result<()> {
        while !drained.is_empty() {
            let batch_size = std::cmp::min(drained.len(), BATCH_SIZE);
            let batch: Vec<&Event> = drained.iter().take(batch_size).collect();
            if let Err(err) = self.connection.send(batch) {
                // The send failed, so the whole remaining snapshot is unsent.
                // Re-buffer it at the front (oldest-first) so a later flush
                // retries it ahead of anything appended in the meantime.
                EventsBuffer::read(&self.data_dir)?.prepend(drained)?;
                return Err(err);
            }
            self.mark_sent();
            drained.drain(..batch_size);
        }
        Ok(())
    }
}
