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
    /// Lines this binary could not parse, kept verbatim.
    ///
    /// Almost always an event type written by a newer flox sharing the same
    /// data dir. They are carried through [`Self::overwrite_file`] unchanged
    /// so that a binary which understands them can still send them, and so
    /// that reading the buffer never destroys telemetry.
    unknown: VecDeque<String>,
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

        // Parse per line rather than as one stream. A `StreamDeserializer`
        // stops at its first error and reports the rest of the input as
        // consumed, so a single unreadable entry would silently take every
        // later entry with it — and `overwrite_file` would then persist that
        // loss.
        let mut buffer = VecDeque::new();
        let mut unknown = VecDeque::new();
        for line in buffer_json.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Event>(line) {
                Ok(event) => buffer.push_back(event),
                Err(err) => {
                    debug!(error = %err, "Retaining unreadable v2 event buffer entry");
                    unknown.push_back(line.to_string());
                },
            }
        }
        // Retained lines are bounded like the parsed ones: a machine that
        // permanently runs an older flox must not grow this file forever.
        while unknown.len() >= MAX_BUFFER_SIZE {
            unknown.pop_front();
        }

        Ok(Self {
            storage: events_buffer_file,
            _file_lock: events_lock,
            buffer,
            unknown,
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

        // Entries this binary could not read are written back untouched, so a
        // rewrite never drops what it could not interpret.
        for line in &self.unknown {
            self.storage
                .write_all(line.as_bytes())
                .context("Could not write retained v2 event to buffer file")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A serialized envelope of `event_type`, as an older binary would find
    /// it in a buffer written by a newer one.
    fn envelope(event_type: &str) -> String {
        format!(
            r#"{{"event_id":"00000000-0000-0000-0000-000000000000","event_timestamp":0,"source":"cli","invocation_id":"00000000-0000-0000-0000-000000000000","device_id":"00000000-0000-0000-0000-000000000000","event_type":"{event_type}","payload":{{"env_kind":"path","env_ref_or_name":"myenv"}}}}"#
        )
    }

    /// An entry this binary cannot parse — an event type added by a newer
    /// flox — must not take the entries after it down with it.
    #[test]
    fn unreadable_entry_does_not_discard_later_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(EVENTS_BUFFER_FILE_NAME),
            format!(
                "{}\n{}\n{}\n",
                envelope("cli.environment.delete"),
                envelope("cli.environment.from.the.future"),
                envelope("cli.environment.list"),
            ),
        )
        .unwrap();

        let buffer = EventsBuffer::read(dir.path()).unwrap();

        assert_eq!(buffer.buffer.len(), 2, "both readable entries survive");
        assert_eq!(buffer.unknown.len(), 1, "the unreadable entry is retained");
    }

    /// Reading a buffer and rewriting it must never destroy an entry this
    /// binary could not interpret: a newer flox has to still be able to send
    /// it.
    #[test]
    fn unreadable_entry_survives_a_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let future = envelope("cli.environment.from.the.future");
        std::fs::write(
            dir.path().join(EVENTS_BUFFER_FILE_NAME),
            format!("{}\n{}\n", envelope("cli.environment.delete"), future),
        )
        .unwrap();

        {
            let mut buffer = EventsBuffer::read(dir.path()).unwrap();
            let sent = buffer.batch_size(usize::MAX);
            buffer.drain_sent(sent);
            buffer.overwrite_file().unwrap();
        }

        let contents = std::fs::read_to_string(dir.path().join(EVENTS_BUFFER_FILE_NAME)).unwrap();
        assert_eq!(
            contents.lines().collect::<Vec<_>>(),
            vec![future.as_str()],
            "the sent entry is drained and the unreadable one is written back"
        );
    }
}
