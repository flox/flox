use std::collections::VecDeque;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use flox_core::open_lock_file;
use time::{Duration, OffsetDateTime};
use tracing::debug;

use crate::{Cli, Producer, ProducerEvent};

/// On-disk JSONL file used by the v2 event pipeline.
pub const EVENTS_BUFFER_FILE_NAME: &str = "events-v2.json";
const EVENTS_LOCK_FILE_NAME: &str = "events-v2.lock";
const MAX_BUFFER_SIZE: usize = 1000;
/// How long [`EventsBuffer::read_at`] waits for another process to release
/// the buffer lock. Every holder only reads or writes the file, so a wait
/// this long means a stuck holder.
pub const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(1);
/// How often a held lock is tried again.
const LOCK_RETRY: std::time::Duration = std::time::Duration::from_millis(10);

/// Where an event buffer lives: a directory, the buffer file in it, and the
/// lock file that guards the buffer file.
///
/// Two producers that share a directory keep apart by giving their buffers
/// different names; each one's decoder refuses the other's events anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BufferLocation {
    dir: PathBuf,
    file_name: String,
    lock_name: String,
}

impl BufferLocation {
    /// The CLI's buffer in `dir`: [`EVENTS_BUFFER_FILE_NAME`] and its lock.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self::with_names(dir, EVENTS_BUFFER_FILE_NAME, EVENTS_LOCK_FILE_NAME)
    }

    /// A buffer in `dir` named `file_name`, guarded by the lock file
    /// `lock_name` beside it.
    pub fn with_names(
        dir: impl Into<PathBuf>,
        file_name: impl Into<String>,
        lock_name: impl Into<String>,
    ) -> Self {
        Self {
            dir: dir.into(),
            file_name: file_name.into(),
            lock_name: lock_name.into(),
        }
    }

    /// The directory that holds the buffer and its lock.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The buffer file.
    pub fn file(&self) -> PathBuf {
        self.dir.join(&self.file_name)
    }

    /// The lock file.
    pub fn lock(&self) -> PathBuf {
        self.dir.join(&self.lock_name)
    }
}

/// Locked on-disk event buffer.
///
/// An instance holds the process lock for as long as it is alive. Keep values
/// short-lived so concurrent `flox` processes do not wait longer than needed.
#[derive(Debug)]
pub struct EventsBuffer<P: Producer = Cli> {
    storage: File,
    _file_lock: File,
    buffer: VecDeque<ProducerEvent<P>>,
    /// Lines this binary could not parse — almost always an event type
    /// written by a newer flox sharing the data dir. Kept verbatim so a
    /// binary that understands them can still send them.
    unknown: VecDeque<String>,
}

impl EventsBuffer<Cli> {
    /// Read the CLI's buffer from `data_dir`, creating the file if needed.
    ///
    /// Waits for the file lock at most [`LOCK_WAIT`].
    pub fn read(data_dir: &Path) -> Result<Self> {
        Self::read_at(&BufferLocation::new(data_dir))
    }

    /// Attempts to acquire the CLI's buffer lock in `data_dir` without
    /// blocking. See [`EventsBuffer::try_read_at`].
    pub fn try_read(data_dir: &Path) -> Result<Option<Self>> {
        Self::try_read_at(&BufferLocation::new(data_dir))
    }

    /// Delete the CLI's buffer file and its lock file from `data_dir`. See
    /// [`EventsBuffer::delete_at`].
    pub fn delete(data_dir: &Path) -> Result<()> {
        Self::delete_at(&BufferLocation::new(data_dir))
    }
}

impl<P: Producer> EventsBuffer<P> {
    /// Read the buffer at `location`, creating the file if needed.
    ///
    /// Waits for the file lock at most [`LOCK_WAIT`], then fails: a process
    /// that holds it longer is stuck, and every later invocation would wait
    /// on it.
    pub fn read_at(location: &BufferLocation) -> Result<Self> {
        let data_dir = location.dir();
        std::fs::create_dir_all(data_dir).with_context(|| {
            format!(
                "Could not create v2 events buffer directory at {}",
                data_dir.display()
            )
        })?;

        let events_lock =
            open_lock_file(location.lock()).context("Could not open v2 events lock file")?;
        lock_within(&events_lock, LOCK_WAIT)?;

        let buffer_file_path = location.file();
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

        // Per line, not one `StreamDeserializer`: that stops at its first
        // error and reports the rest of the input as consumed, so one
        // unreadable entry would take every later entry with it.
        let mut buffer = VecDeque::new();
        let mut unknown = VecDeque::new();
        for line in buffer_json.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ProducerEvent<P>>(line) {
                Ok(event) => buffer.push_back(event),
                Err(err) => {
                    debug!(error = %err, "Retaining unreadable v2 event buffer entry");
                    unknown.push_back(line.to_string());
                },
            }
        }
        // Capped separately from the parsed entries, so the file holds up to
        // 2 * MAX_BUFFER_SIZE lines. Anything unparseable lands here, not
        // only newer-variant events: a line torn by a crash mid-append is
        // also retained, and no binary will ever drain it.
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

    /// Attempts to acquire the lock of the buffer at `location` without
    /// blocking.
    ///
    /// Returns `None` when another process holds the lock, so the caller can
    /// exit early rather than piling up concurrent flushers.
    pub fn try_read_at(location: &BufferLocation) -> Result<Option<Self>> {
        let data_dir = location.dir();
        std::fs::create_dir_all(data_dir).with_context(|| {
            format!(
                "Could not create v2 events buffer directory at {}",
                data_dir.display()
            )
        })?;

        let events_lock =
            open_lock_file(location.lock()).context("Could not open v2 events lock file")?;
        match events_lock.try_lock() {
            Ok(()) => {},
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Error(err)) => {
                return Err(err).context("Could not try-lock v2 events buffer");
            },
        }

        let buffer_file_path = location.file();
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

        let mut buffer = VecDeque::new();
        let mut unknown = VecDeque::new();
        for line in buffer_json.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<ProducerEvent<P>>(line) {
                Ok(event) => buffer.push_back(event),
                Err(err) => {
                    debug!(error = %err, "Retaining unreadable v2 event buffer entry");
                    unknown.push_back(line.to_string());
                },
            }
        }
        while unknown.len() >= MAX_BUFFER_SIZE {
            unknown.pop_front();
        }

        Ok(Some(Self {
            storage: events_buffer_file,
            _file_lock: events_lock,
            buffer,
            unknown,
        }))
    }

    /// Delete the buffer file and its lock file at `location`, discarding
    /// every buffered event, including entries this binary cannot parse.
    /// Succeeds without doing anything when its directory does not exist.
    ///
    /// Blocks until the file lock is acquired, so no other process is
    /// mid-write. The lock file is deleted while still held. A process that
    /// was already waiting on it then locks the deleted file, while a process
    /// that starts later creates and locks a new one, so those two processes
    /// do not exclude each other until the first one releases its lock.
    pub fn delete_at(location: &BufferLocation) -> Result<()> {
        if !location
            .dir()
            .try_exists()
            .context("Could not check the v2 events buffer directory")?
        {
            return Ok(());
        }

        let lock_file_path = location.lock();
        let events_lock =
            open_lock_file(&lock_file_path).context("Could not open v2 events lock file")?;
        events_lock
            .lock()
            .context("Could not lock v2 events buffer")?;

        remove_file_if_present(&location.file())
            .context("Could not delete v2 events buffer file")?;
        remove_file_if_present(&lock_file_path).context("Could not delete v2 events lock file")
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

    /// Remove all parsed entries from the in-memory buffer and the file,
    /// returning them for sending after the lock is released. Unparsed
    /// (`unknown`) lines stay in the file — no binary here can send them.
    ///
    /// Truncate-before-send is deliberate: the caller cannot hold the buffer
    /// lock across the network send without re-blocking the append path (the
    /// stall this pipeline's detached flush exists to remove). This drains the
    /// whole buffer and truncates the file, so a SIGKILL or panic of the
    /// detached child between here and the failure re-buffer in `prepend` loses
    /// the entire drained snapshot — up to MAX_BUFFER_SIZE events, not a single
    /// batch. Accepted: telemetry is best-effort, not durable.
    pub fn take_sendable(&mut self) -> Result<VecDeque<ProducerEvent<P>>> {
        let taken = std::mem::take(&mut self.buffer);
        self.overwrite_file()?;
        Ok(taken)
    }

    /// Re-buffer entries a failed send left unsent, placing them ahead of
    /// anything appended since [`take_sendable`], so a retry sends
    /// oldest-first. Enforces the same cap as [`push`].
    pub fn prepend(&mut self, mut unsent: VecDeque<ProducerEvent<P>>) -> Result<()> {
        while let Some(event) = unsent.pop_back() {
            self.buffer.push_front(event);
        }
        self.pop_front_to_max_size();
        self.overwrite_file()
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

        // Written back untouched, so a rewrite never drops what this binary
        // could not interpret.
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
    pub fn push(&mut self, event: ProducerEvent<P>) -> Result<()> {
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

    /// Push several events and sync them to disk once, before returning.
    ///
    /// The same as calling [`push`](Self::push) for each event in turn, with
    /// one write and one sync for all of them instead of one each; past the
    /// cap, one rewrite of the file instead of one for every event.
    pub fn push_all(&mut self, events: impl IntoIterator<Item = ProducerEvent<P>>) -> Result<()> {
        let before = self.buffer.len();
        self.buffer.extend(events);
        let added = self.buffer.len() - before;
        if added == 0 {
            return Ok(());
        }
        debug!(count = added, "Pushing events to v2 events buffer");

        if self.buffer.len() >= MAX_BUFFER_SIZE {
            self.pop_front_to_max_size();
            return self.overwrite_file();
        }

        let mut lines = String::new();
        for event in self.buffer.iter().skip(before) {
            lines.push_str(&serde_json::to_string(event)?);
            lines.push('\n');
        }
        self.storage
            .write_all(lines.as_bytes())
            .context("Could not write new v2 events to buffer file")?;
        self.storage
            .flush()
            .context("Could not flush v2 events buffer file")?;
        self.storage
            .sync_data()
            .context("Could not sync v2 events buffer file")?;
        Ok(())
    }

    /// Iterate over buffered events from oldest to newest.
    pub fn iter(&self) -> impl Iterator<Item = &ProducerEvent<P>> {
        self.buffer.iter()
    }

    fn pop_front_to_max_size(&mut self) {
        while self.buffer.len() >= MAX_BUFFER_SIZE {
            self.buffer.pop_front();
        }
    }
}

/// Takes the exclusive lock on `file`, giving up after `wait`.
fn lock_within(file: &File, wait: std::time::Duration) -> Result<()> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(LOCK_RETRY);
            },
            Err(TryLockError::WouldBlock) => {
                anyhow::bail!("Timed out after {wait:?} waiting for the v2 events buffer lock");
            },
            Err(TryLockError::Error(err)) => {
                return Err(err).context("Could not lock v2 events buffer");
            },
        }
    }
}

fn remove_file_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
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

    #[test]
    fn delete_removes_the_buffer_with_unreadable_entries_and_the_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(EVENTS_BUFFER_FILE_NAME),
            format!(
                "{}\n{}\n",
                envelope("cli.environment.delete"),
                envelope("cli.environment.from.the.future"),
            ),
        )
        .unwrap();
        drop(EventsBuffer::read(dir.path()).unwrap());

        EventsBuffer::delete(dir.path()).unwrap();

        let remaining: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert!(remaining.is_empty(), "left behind: {remaining:?}");

        EventsBuffer::delete(dir.path()).expect("deleting a missing buffer succeeds");
        EventsBuffer::delete(&dir.path().join("missing"))
            .expect("deleting from a missing directory succeeds");
        assert!(!dir.path().join("missing").exists());
    }

    /// `n` events of the CLI, stamped one second apart.
    fn events(n: usize) -> Vec<crate::Event> {
        (0..n)
            .map(|i| {
                let line = envelope("cli.environment.delete");
                let mut event: crate::Event = serde_json::from_str(&line).unwrap();
                event.event_id = uuid::Uuid::from_u128(i as u128);
                event.event_timestamp =
                    time::OffsetDateTime::from_unix_timestamp(i as i64).unwrap();
                event
            })
            .collect()
    }

    #[test]
    fn push_all_leaves_what_push_one_at_a_time_leaves() {
        for (held, added) in [(0, 0), (0, 3), (10, 5), (990, 5), (990, 20), (998, 3)] {
            let one = tempfile::tempdir().unwrap();
            let all = tempfile::tempdir().unwrap();
            let (old, new) = {
                let mut e = events(held + added);
                let new = e.split_off(held);
                (e, new)
            };
            for dir in [one.path(), all.path()] {
                let mut buffer = EventsBuffer::read(dir).unwrap();
                buffer.push_all(old.clone()).unwrap();
            }
            {
                let mut buffer = EventsBuffer::read(one.path()).unwrap();
                for event in new.clone() {
                    buffer.push(event).unwrap();
                }
            }
            EventsBuffer::read(all.path())
                .unwrap()
                .push_all(new)
                .unwrap();
            let read =
                |dir: &Path| std::fs::read_to_string(dir.join(EVENTS_BUFFER_FILE_NAME)).unwrap();
            assert_eq!(
                read(all.path()),
                read(one.path()),
                "{held} held, {added} added"
            );
            let kept = EventsBuffer::read(all.path()).unwrap().buffer.len();
            assert_eq!(
                kept,
                (held + added).min(MAX_BUFFER_SIZE - 1),
                "{held} held, {added} added"
            );
        }
    }

    #[test]
    fn a_reader_gives_up_on_a_lock_held_past_the_wait() {
        let dir = tempfile::tempdir().unwrap();
        let location = BufferLocation::new(dir.path());
        let held = open_lock_file(location.lock()).unwrap();
        held.lock().unwrap();

        let start = std::time::Instant::now();
        let read = EventsBuffer::read(dir.path());
        let waited = start.elapsed();
        assert!(read.is_err(), "read the buffer while another held its lock");
        assert!(
            waited >= LOCK_WAIT && waited < 5 * LOCK_WAIT,
            "waited {waited:?}"
        );

        drop(held);
        EventsBuffer::read(dir.path()).expect("reads once the lock is free");
    }

    /// A producer other than the CLI, whose events have their own source.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct Other;

    impl Producer for Other {
        type Kind = crate::EventKind;
        type Metadata = ();

        const SOURCE: &'static str = "other";
    }

    #[test]
    fn a_buffer_with_names_of_its_own_is_apart_from_the_clis() {
        let dir = tempfile::tempdir().unwrap();
        let cli_line = envelope("cli.environment.delete");
        std::fs::write(
            dir.path().join(EVENTS_BUFFER_FILE_NAME),
            format!("{cli_line}\n"),
        )
        .unwrap();
        let location = BufferLocation::with_names(dir.path(), "other.json", "other.lock");

        {
            let buffer = EventsBuffer::<Other>::read_at(&location).unwrap();
            assert!(
                buffer.is_empty(),
                "read the CLI's buffer through other names"
            );
        }
        assert!(location.file().exists(), "the buffer file is created");
        assert!(location.lock().exists(), "the lock file is created");

        EventsBuffer::<Other>::delete_at(&location).unwrap();
        assert!(!location.file().exists() && !location.lock().exists());
        let cli = EventsBuffer::read(dir.path()).unwrap();
        assert_eq!(cli.buffer.len(), 1, "the CLI's buffer is untouched");
    }

    #[test]
    fn a_buffer_keeps_another_producers_lines_unsent() {
        let dir = tempfile::tempdir().unwrap();
        let location = BufferLocation::with_names(dir.path(), "other.json", "other.lock");
        let cli_line = envelope("cli.environment.delete");
        std::fs::write(location.file(), format!("{cli_line}\n")).unwrap();

        let mut buffer = EventsBuffer::<Other>::read_at(&location).unwrap();
        assert!(buffer.is_empty(), "the CLI's event is not this producer's");
        assert!(buffer.take_sendable().unwrap().is_empty());
        drop(buffer);
        assert_eq!(
            std::fs::read_to_string(location.file()).unwrap(),
            format!("{cli_line}\n"),
            "the line is kept as it was"
        );
    }
}
