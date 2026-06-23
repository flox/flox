use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
use fslock::LockFile;
use time::{Duration, OffsetDateTime};
use tracing::debug;

use crate::Event;

/// On-disk JSONL file used by the v2 event pipeline.
pub const EVENTS_BUFFER_FILE_NAME: &str = "events-v2.json";
const EVENTS_LOCK_FILE_NAME: &str = "events-v2.lock";
const MAX_BUFFER_SIZE: usize = 1000;

/// Locked on-disk event buffer.
///
/// An instance holds the process lock for as long as it is alive. Keep values
/// short-lived so concurrent `flox` processes do not wait longer than needed.
#[derive(Debug)]
pub struct EventsBuffer {
    storage: File,
    _file_lock: LockFile,
    buffer: VecDeque<Event>,
}

impl EventsBuffer {
    /// Read the buffer from `data_dir`, creating the file if needed.
    pub fn read(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir).with_context(|| {
            format!(
                "Could not create v2 events buffer directory at {}",
                data_dir.display()
            )
        })?;

        let mut events_lock = LockFile::open(&data_dir.join(EVENTS_LOCK_FILE_NAME))
            .context("Could not open v2 events lock file")?;
        events_lock
            .lock()
            .context("Could not lock v2 events buffer")?;

        let buffer_file_path = data_dir.join(EVENTS_BUFFER_FILE_NAME);
        let mut events_buffer_file_options = OpenOptions::new();
        events_buffer_file_options
            .read(true)
            .append(true)
            .create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            events_buffer_file_options.mode(0o600);
        }
        let mut events_buffer_file = events_buffer_file_options
            .open(&buffer_file_path)
            .with_context(|| {
                format!(
                    "Could not open v2 events buffer file at {}",
                    buffer_file_path.display()
                )
            })?;

        let mut buffer_json = String::new();
        events_buffer_file
            .read_to_string(&mut buffer_json)
            .context("Could not read v2 events buffer file")?;

        let buffer = serde_json::Deserializer::from_str(&buffer_json)
            .into_iter::<Event>()
            .filter_map(|event| match event {
                Ok(event) => Some(event),
                Err(err) => {
                    debug!(error = %err, "Skipping unreadable v2 event buffer entry");
                    None
                },
            })
            .collect();

        Ok(Self {
            storage: events_buffer_file,
            _file_lock: events_lock,
            buffer,
        })
    }

    pub(crate) fn is_expired(&self, expiry: Duration) -> bool {
        let now = OffsetDateTime::now_utc();
        self.oldest_timestamp()
            .map(|oldest| now - oldest > expiry)
            .unwrap_or(false)
    }

    pub(crate) fn oldest_timestamp(&self) -> Option<OffsetDateTime> {
        self.buffer.front().map(|event| event.event_timestamp)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub(crate) fn batch_size(&self, max_batch_size: usize) -> usize {
        std::cmp::min(self.buffer.len(), max_batch_size)
    }

    pub(crate) fn drain_sent(&mut self, count: usize) {
        self.buffer.drain(..count);
    }

    /// Persist the current in-memory buffer, replacing the file contents.
    pub fn overwrite_file(&mut self) -> Result<()> {
        self.storage
            .set_len(0)
            .context("Could not truncate v2 events buffer file")?;

        for event in &self.buffer {
            self.storage
                .write_all(serde_json::to_string(event)?.as_bytes())
                .context("Could not write v2 event to buffer file")?;
            self.storage
                .write_all(b"\n")
                .context("Could not write v2 event buffer newline")?;
        }

        self.storage
            .flush()
            .context("Could not flush v2 events buffer file")?;
        self.storage
            .sync_data()
            .context("Could not sync v2 events buffer file")?;
        Ok(())
    }

    /// Push a new event and sync it to disk before returning.
    pub fn push(&mut self, event: Event) -> Result<()> {
        debug!(event_id = %event.event_id, "Pushing event to v2 events buffer");

        self.buffer.push_back(event);

        if self.buffer.len() >= MAX_BUFFER_SIZE {
            self.pop_front_to_max_size();
            self.overwrite_file()?;
        } else {
            self.append_new_event()?;
        };

        Ok(())
    }

    fn append_new_event(&mut self) -> Result<()> {
        let event = self
            .buffer
            .back()
            .expect("buffer has a just-pushed v2 event");
        self.storage
            .write_all(serde_json::to_string(event)?.as_bytes())
            .context("Could not write new v2 event to buffer file")?;
        self.storage
            .write_all(b"\n")
            .context("Could not write v2 event buffer newline")?;
        self.storage
            .flush()
            .context("Could not flush v2 events buffer file")?;
        self.storage
            .sync_data()
            .context("Could not sync v2 events buffer file")?;
        Ok(())
    }

    /// Iterate over buffered events from oldest to newest.
    pub fn iter(&self) -> impl Iterator<Item = &Event> {
        self.buffer.iter()
    }

    fn pop_front_to_max_size(&mut self) {
        while self.buffer.len() >= MAX_BUFFER_SIZE {
            self.buffer.pop_front();
        }
    }
}
