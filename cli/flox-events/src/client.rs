use std::path::{Path, PathBuf};

use anyhow::Result;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::buffer::EventsBuffer;
use crate::connection::{EventsConnection, EventsConnectionV2};
use crate::{Event, EventKind};

const DEFAULT_BUFFER_EXPIRY: Duration = Duration::minutes(2);
pub const BATCH_SIZE: usize = 100;

/// Client that stamps v2 event metadata, buffers events, and flushes
/// them through an [`EventsConnection`].
///
/// The connection owns the endpoint URL and credential; the client itself
/// holds only the per-invocation metadata stamped onto each [`Event`].
#[derive(Debug)]
pub struct EventsClient {
    pub device_id: Uuid,
    pub data_dir: PathBuf,
    pub invocation_id: Uuid,
    pub max_age: Duration,
    pub connection: Box<dyn EventsConnection>,
}

impl EventsClient {
    pub fn new(
        device_id: Uuid,
        data_dir: impl AsRef<Path>,
        endpoint_url: impl Into<String>,
        api_key: impl Into<String>,
        invocation_id: Uuid,
    ) -> Self {
        let connection = EventsConnectionV2::new(endpoint_url, api_key);
        Self::new_with_connection(device_id, data_dir, invocation_id, connection)
    }

    pub fn new_with_connection(
        device_id: Uuid,
        data_dir: impl AsRef<Path>,
        invocation_id: Uuid,
        connection: impl EventsConnection + 'static,
    ) -> Self {
        Self {
            device_id,
            data_dir: data_dir.as_ref().to_path_buf(),
            invocation_id,
            max_age: DEFAULT_BUFFER_EXPIRY,
            connection: connection.boxed(),
        }
    }

    pub fn record_event(&self, kind: impl Into<EventKind>) -> Result<()> {
        let event = Event {
            event_id: Uuid::new_v4(),
            event_timestamp: OffsetDateTime::now_utc(),
            source: "cli",
            invocation_id: self.invocation_id,
            device_id: self.device_id,
            auth_subject: None,
            kind: kind.into(),
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
