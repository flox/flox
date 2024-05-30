use std::any::Any;
use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration as TimeoutDuration;

use anyhow::{bail, Context, Result};
use flox_rust_sdk::flox::FLOX_VERSION;
use fslock::LockFile;
use indoc::indoc;
use log::debug;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::format_description::well_known::Iso8601;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use super::TRAILING_NETWORK_CALL_TIMEOUT;
use crate::config::Config;

pub const METRICS_EVENTS_FILE_NAME: &str = "metrics-events-v2.json";
pub const METRICS_UUID_FILE_NAME: &str = "metrics-uuid";
pub const METRICS_LOCK_FILE_NAME: &str = "metrics-lock";
const DEFAULT_BUFFER_EXPIRY: Duration = Duration::minutes(2);

pub static METRICS_EVENTS_URL: Lazy<String> = Lazy::new(|| {
    std::env::var("_FLOX_METRICS_URL_OVERRIDE").unwrap_or(env!("METRICS_EVENTS_URL").to_string())
});
pub const METRICS_EVENTS_API_KEY: &str = env!("METRICS_EVENTS_API_KEY");

/// Creates a trace event for the given subcommand.
///
/// We set the target to `flox_command` so that we can filter for these exact events.
#[macro_export]
macro_rules! subcommand_metric {
    ($arg:tt $(, $key:tt = $value:expr)*) => {{
        tracing::trace!(target: "flox_command", subcommand = $arg $(, $key = $value)*);
    }};
}

/// Extracts [MetricEvent] data from a raw [tracing] event
struct MetricVisitor<'a>(&'a mut Option<String>, &'a mut HashMap<String, String>);

impl<'a> tracing::field::Visit for MetricVisitor<'a> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "subcommand" {
            *self.0 = Some(value.to_string());
            return;
        }

        self.1.insert(field.name().to_string(), value.to_string());
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.record_str(field, &format!("{:?}", value))
    }
}

/// A [tracing] event that represents a metric of a run command
/// with additional ad-hoc metadata
///
/// Produced by [subcommand_metric!] and processed with [MetricsLayer].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricEvent {
    pub subcommand: Option<String>,
    pub extras: HashMap<String, String>,
}

/// A [tracing_subscriber::Layer] that stores metrics events in a buffer
/// and pushes them to the server when the buffer is expired.
///
/// Listens for [tracing] events with the target `flox_command`.
pub struct MetricsLayer {}

impl MetricsLayer {
    pub fn new() -> Self {
        MetricsLayer {}
    }
}

impl<S> tracing_subscriber::Layer<S> for MetricsLayer
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if event.metadata().target() != "flox_command" {
            return;
        }

        let mut subcommand = None;
        let mut extras = HashMap::new();
        let mut visitor = MetricVisitor(&mut subcommand, &mut extras);
        event.record(&mut visitor);

        // Catch any errors that occurred while writing/pushing the metric.
        // We do want to _know_ about errors
        // but they should not block flox commands from running.
        if let Err(err) = Hub::global().record_metric(MetricEvent { subcommand, extras }) {
            debug!("Error adding metric: {err}");
        }
    }
}

/// A single metric entry
/// This is the a metric event with additional static metadata
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MetricEntry {
    subcommand: Option<String>,
    #[serde(flatten)]
    extras: HashMap<String, String>,
    timestamp: OffsetDateTime,
    uuid: Uuid,
    flox_version: String,
    os_family: Option<String>,
    os_family_release: Option<String>,
    os: Option<String>,
    os_version: Option<String>,
    empty_flags: Vec<String>,
}

impl MetricEntry {
    pub fn new(
        MetricEvent { subcommand, extras }: MetricEvent,
        timestamp: OffsetDateTime,
        uuid: Uuid,
    ) -> MetricEntry {
        let linux_release = sys_info::linux_os_release().ok();

        MetricEntry {
            subcommand,
            extras,
            uuid,
            timestamp,
            flox_version: FLOX_VERSION.to_string(),
            os_family: sys_info::os_type()
                .ok()
                .map(|x| x.replace("Darwin", "Mac OS")),
            os_family_release: sys_info::os_release().ok(),
            os: linux_release.as_ref().and_then(|r| r.id.clone()),
            os_version: linux_release.and_then(|r| r.version_id),
            empty_flags: vec![],
        }
    }
}

/// A representation of the metrics buffer
///
/// The metrics buffer is a file that contains a list of metrics entries.
/// It is used to store metrics for a period of time
/// and then push them to the server.
///
/// An instance of this struct represents the metrics buffer file and its contents.
/// While the metrics buffer is being used, it is locked to avoid data corruption.
/// Thus, a [MetricsBuffer] instance should be short-lived
/// to avoid blocking other processes.
#[derive(Debug)]
struct MetricsBuffer {
    /// The file where the metrics buffer is stored
    storage: File,
    /// The lock file for the metrics buffer.
    /// Used to avoid concurrent writes to the metrics buffer file.
    _file_lock: LockFile,
    buffer: VecDeque<MetricEntry>,
}
impl MetricsBuffer {
    /// Reads the metrics buffer from the cache directory
    fn read(cache_dir: &Path) -> Result<Self> {
        // Create a file lock to avoid concurrent access to the metrics.
        // The lock is released once the object is dropped.
        // We store the lock in the instance of [MetricsBuffer],
        // thus the lifetime of the lock is extended until the buffer is dropped.
        let mut metrics_lock = LockFile::open(&cache_dir.join(METRICS_LOCK_FILE_NAME))?;
        metrics_lock.lock()?;

        let buffer_file_path = cache_dir.join(METRICS_EVENTS_FILE_NAME);
        let mut events_buffer_file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .open(buffer_file_path)?;

        let mut buffer_json = String::new();

        events_buffer_file.read_to_string(&mut buffer_json)?;

        let buffer_iter = serde_json::Deserializer::from_str(&buffer_json)
            .into_iter::<MetricEntry>()
            .filter_map(|x| x.ok())
            .collect();

        Ok(MetricsBuffer {
            storage: events_buffer_file,
            _file_lock: metrics_lock,
            buffer: buffer_iter,
        })
    }

    /// Returns the oldest timestamp in the buffer
    fn oldest_timestamp(&self) -> Option<OffsetDateTime> {
        self.buffer.front().map(|x| x.timestamp)
    }

    /// Returns whether the buffer is expired,
    /// i.e. needs to be pushed to the server.
    ///
    /// The buffer is expired if it contains >= 1 entry
    /// and the oldest entry is older than `expiry`.
    fn is_expired(&self, expiry: Duration) -> bool {
        let now = OffsetDateTime::now_utc();
        self.oldest_timestamp()
            .map(|oldest| now - oldest > expiry)
            .unwrap_or(false)
    }

    /// Pushes a new metric entry to the buffer and syncs it to the buffer file
    fn push(&mut self, entry: MetricEntry) -> Result<()> {
        debug!("pushing entry to metrics buffer: {entry:?}");

        // update file with new entry
        // [MetricsBuffer::read] ensures that the file is opened with write permissions
        // and append mode.
        let mut buffer_json = String::new();
        buffer_json.push_str(&serde_json::to_string(&entry)?);
        buffer_json.push('\n');
        self.storage
            .write_all(buffer_json.as_bytes())
            .context("could not write new metrics entry to buffer file")?;
        self.storage.flush()?;

        // update the buffer in memory
        self.buffer.push_back(entry);

        Ok(())
    }

    /// Clears the buffer and the buffer file
    ///
    /// This is used when the buffer is pushed to the server
    /// and we start collecting metrics in a new buffer.
    fn clear(&mut self) -> Result<()> {
        self.storage
            .set_len(0)
            .context("Could not truncate metrics buffer file")?;
        self.buffer.clear();
        Ok(())
    }

    /// Returns an iterator over the entries in the buffer
    fn iter(&self) -> impl Iterator<Item = &MetricEntry> {
        self.buffer.iter()
    }
}

pub fn read_metrics_uuid(config: &Config) -> Result<Uuid> {
    let data_dir = &config.flox.data_dir;
    let uuid_path = data_dir.join(METRICS_UUID_FILE_NAME);

    File::open(uuid_path)
        .context("Could not read metrics UUID file")
        .and_then(|mut f| {
            let mut uuid_str = String::new();
            f.read_to_string(&mut uuid_str)?;
            let uuid_str_trimmed = uuid_str.trim();
            Uuid::try_parse(uuid_str_trimmed).with_context(|| {
                indoc! {"
                Could not parse the metrics UUID of this installation in {uuid_path}
            "}
            })
        })
}

static METRICS_HUB: Lazy<Hub> = Lazy::new(|| Hub {
    client: Arc::new(Mutex::new(None)),
});

/// A sharable wrapper around a metrics [Client],
/// that allows for registering and flushing metric events.
///
/// A global [Hub] is used by the [MetricsLayer] to record metrics
/// from tracing events emitted by [subcommand_metric!].
#[derive(Debug)]
pub struct Hub {
    client: Arc<Mutex<Option<Client>>>,
}

impl Hub {
    pub fn global() -> &'static Self {
        &METRICS_HUB
    }

    /// Set the client for the hub, replacing the existing one.
    /// Returns the previous client, if any.
    ///
    ///
    /// In practice this should only be called once,
    /// when the metrics are setup.
    pub fn set_client(&self, new_client: Client) -> Option<Client> {
        self.with_client(|client| client.replace(new_client))
    }

    /// Get a guard for the client, that will automatically flush the metrics on drop
    ///
    /// The guard will return an error if another guard is already active.
    pub fn try_guard(&self) -> Result<MetricGuard> {
        if Arc::strong_count(&self.client) > 1 {
            bail!("A guard is already active, there can only be one guard at a time")
        }

        Ok(MetricGuard {
            hub: Self {
                client: self.client.clone(),
            },
        })
    }

    /// Run a function with the wrapped client, returning the result.
    /// A mutex is locked for the duration of the function.
    fn with_client<T>(&self, f: impl FnOnce(&mut Option<Client>) -> T) -> T {
        let mut client = self
            .client
            .lock()
            .expect("Metrics client mutex panicked on another thread");
        f(&mut client)
    }

    /// Flush the metrics to the telemetry backend
    ///
    /// If `force` is true, the metrics are flushed even if the buffer is not expired.
    /// This methods is jut a convience wrapper around [Client::flush],
    /// that will do nothing if no client is setup.
    fn flush_metrics(&self, force: bool) -> Result<()> {
        self.with_client(|client| {
            if let Some(client) = client {
                client.flush(force)
            } else {
                debug!("No metrics client setup, skipping flush");
                Ok(())
            }
        })
    }

    /// Record a metric event
    ///
    /// This is a convience wrapper around [Client::record_metric],
    /// that will do nothing if no client is setup.
    pub fn record_metric(&self, event: MetricEvent) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                debug!("No metrics client setup, skipping record");
                return Ok(());
            };

            client.record_metric(event)
        })
    }
}

/// A connection to a telemetry backend, i.e. a service that receives metrics
pub trait Connection: Debug + Any + Send + Sync {
    /// Send the metrics to the telemetry backend defined by the [Connection]
    fn send(&mut self, data: Vec<&MetricEntry>) -> Result<()>;

    /// A helper method to box a [Connection] trait object
    fn boxed(self) -> Box<dyn Connection>
    where
        Self: Sized + 'static,
    {
        Box::new(self)
    }

    /// A helper trampoline to downcast [Box<dyn Connection>] to a known type
    ///
    /// This is used in tests to retrieve [tests::TestConnection] from a [Client]
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

/// Connection to the AWS Datalake backend
#[derive(Debug)]
pub struct AWSDatalakeConnection {
    pub timeout: TimeoutDuration,
    pub endpoint_url: String,
    pub api_key: String,
}

impl Connection for AWSDatalakeConnection {
    fn send(&mut self, entries: Vec<&MetricEntry>) -> Result<()> {
        let events = entries
            .iter()
            .map(|entry| {
                Ok(json!({
                    "event": "cli-invocation",
                    "properties": {
                        "distinct_id": entry.uuid,
                        "subcommand": entry.subcommand,
                        "extras": entry.extras,

                        "$device_id": entry.uuid,

                        "$current_url": entry.subcommand.as_ref().map(|x| format!("flox://{x}")),
                        "$pathname": entry.subcommand,

                        "empty_flags": entry.empty_flags,

                        "$lib": "flox-cli",

                        "os": entry.os,
                        "os_version": entry.os_version,
                        "os_family": entry.os_family,
                        "os_family_release": entry.os_family_release,

                        // compat
                        "$os": entry.os_family,
                        "kernel_version": entry.os_family_release,

                        "$set_once": {
                            "initial_flox_version": entry.flox_version,

                            "initial_os": entry.os,
                            "initial_os_version": entry.os_version,
                            "initial_os_family": entry.os_family,
                            "initial_os_family_release": entry.os_family_release,

                            // compat
                            "$initial_os": entry.os_family,
                            "initial_kernel_version": entry.os_family_release,
                        },

                        "$set": {
                            "test": true,

                            "used_rust_preview": true,
                            "flox_cli_uuid": entry.uuid,

                            "flox_version": entry.flox_version,

                            "os": entry.os,
                            "os_version": entry.os_version,
                            "os_family": entry.os_family,
                            "os_family_release": entry.os_family_release,

                            // compat
                            "$os": entry.os_family,
                            "kernel_version": entry.os_family_release,
                        },
                    },

                    // Event ID used for deduplication
                    "uuid": Uuid::new_v4(),

                    "timestamp": entry.timestamp.format(&Iso8601::DEFAULT)?,
                }))
            })
            .collect::<Result<Vec<serde_json::Value>>>()?;

        let events = json!(events);

        debug!("Sending metrics to {}", &self.endpoint_url);
        debug!("Metrics: {events:#}");

        reqwest::blocking::Client::new()
            .put(&self.endpoint_url)
            .header("content-type", "application/json")
            .header("x-api-key", &self.api_key)
            .header("user-agent", format!("flox-cli/{}", &*FLOX_VERSION))
            .json(&events)
            .timeout(self.timeout)
            .send()?;
        Ok(())
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}

impl Default for AWSDatalakeConnection {
    fn default() -> Self {
        Self {
            timeout: TRAILING_NETWORK_CALL_TIMEOUT,
            endpoint_url: METRICS_EVENTS_URL.clone(),
            api_key: METRICS_EVENTS_API_KEY.to_string(),
        }
    }
}

#[derive(Debug)]
pub struct Client {
    pub uuid: Uuid,
    pub metrics_dir: PathBuf,
    pub max_age: Duration,
    pub connection: Box<dyn Connection>,
}

impl Client {
    /// Create a new client with defaults read from the config
    pub fn new_with_config(config: &Config, connection: impl Connection + 'static) -> Result<Self> {
        let uuid = read_metrics_uuid(config)?;
        let metrics_dir = config.flox.cache_dir.clone();
        Ok(Client {
            uuid,
            metrics_dir,
            max_age: DEFAULT_BUFFER_EXPIRY,
            connection: connection.boxed(),
        })
    }

    /// Send the metrics to the telemetry backend and clear the buffer file.
    ///
    /// Any connection errors will bubble up and be catched by the event handler.
    /// If the network request failed, the buffer file is _not_ cleared.
    fn flush(&mut self, force: bool) -> Result<()> {
        let mut metrics = MetricsBuffer::read(&self.metrics_dir)?;
        if metrics.is_expired(self.max_age) || force {
            self.connection.send(metrics.iter().collect())?;
            metrics.clear()?;
        }
        Ok(())
    }

    /// Record a metric event
    ///
    /// Takes a Metric event and adds additional shared metadata to it
    /// before pushing it to the metrics buffer.
    fn record_metric(&self, event: MetricEvent) -> Result<()> {
        let entry = MetricEntry::new(event, OffsetDateTime::now_utc(), self.uuid);
        let mut metrics_buffer = MetricsBuffer::read(&self.metrics_dir)?;
        metrics_buffer.push(entry)?;
        Ok(())
    }
}

pub struct MetricGuard {
    hub: Hub,
}
impl Drop for MetricGuard {
    fn drop(&mut self) {
        let force = std::env::var("_FLOX_FORCE_FLUSH_METRICS")
            .unwrap_or_default()
            .parse()
            .unwrap_or(false);
        if let Err(e) = self.hub.flush_metrics(force) {
            debug!("Failed to flush metrics on guard drop: {e}")
        };
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;
    use crate::config::FloxConfig;
    use crate::utils::init::{create_registry_and_filter_reload_handle, update_filters};

    #[derive(Debug, Default)]
    pub(super) struct TestConnection {
        pub sent: Vec<Vec<MetricEntry>>,
    }

    impl Connection for TestConnection {
        /// Store the sent metrics in memory
        fn send(&mut self, data: Vec<&MetricEntry>) -> Result<()> {
            self.sent.push(data.into_iter().cloned().collect());
            Ok(())
        }

        fn into_any(self: Box<Self>) -> Box<dyn Any> {
            self
        }
    }

    /// Create a new client with test defaults
    fn create_client() -> (Client, TempDir) {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_dir = tempdir.path().join("cache");

        fs::create_dir_all(&cache_dir).unwrap();

        let client = Client {
            uuid: Uuid::new_v4(),
            metrics_dir: cache_dir,
            max_age: Duration::hours(2),
            connection: TestConnection::default().boxed(),
        };

        (client, tempdir)
    }

    /// Make a test entry with known values
    fn make_entry(subcommand: &str) -> MetricEntry {
        MetricEntry {
            subcommand: Some(subcommand.to_string()),
            extras: HashMap::new(),
            timestamp: OffsetDateTime::now_utc(),
            uuid: Uuid::new_v4(),
            flox_version: "1.0.0".to_string(),
            os_family: Some("Linux".to_string()),
            os_family_release: Some("5.4.0-42-generic".to_string()),
            os: Some("ubuntu".to_string()),
            os_version: Some("20.04".to_string()),
            empty_flags: vec![],
        }
    }

    /// Make event with known values
    fn make_event(subcommand: &str) -> MetricEvent {
        MetricEvent {
            subcommand: Some(subcommand.to_string()),
            extras: HashMap::new(),
        }
    }

    /// Test that the [MetricsBuffer] writes entries to disk when dropped
    #[test]
    fn test_metrics_buffer_write() {
        let tempdir = tempfile::tempdir().unwrap();

        let mut buffer = MetricsBuffer::read(tempdir.as_ref()).unwrap();

        let entry_foo = make_entry("foo");
        let entry_bar = make_entry("bar");

        buffer.push(entry_foo.clone()).unwrap();
        buffer.push(entry_bar.clone()).unwrap();

        // entries are writtent to disk immediately,
        // but lets drop the buffer anyway to be closer to reality
        drop(buffer);

        let metrics_buffer_file =
            fs::read_to_string(tempdir.path().join(METRICS_EVENTS_FILE_NAME)).unwrap();
        assert_eq!(
            metrics_buffer_file,
            serde_json::to_string(&entry_foo).unwrap()
                + "\n"
                + &serde_json::to_string(&entry_bar).unwrap()
                + "\n"
        );
    }

    /// Test that the [MetricsBuffer] clears entries as expected
    #[test]
    fn test_metrics_buffer_clear() {
        let tempdir = tempfile::tempdir().unwrap();

        let mut buffer = MetricsBuffer::read(tempdir.as_ref()).unwrap();

        let entry_foo = make_entry("foo");
        let entry_bar = make_entry("bar");

        buffer.push(entry_foo.clone()).unwrap();
        buffer.push(entry_bar.clone()).unwrap();

        buffer.clear().unwrap();

        assert_eq!(buffer.buffer.len(), 0);
        assert_eq!(buffer.oldest_timestamp(), None);

        // entries are writtent to disk immediately,
        // but lets drop the buffer anyway to be closer to reality
        drop(buffer);

        let metrics_buffer_file =
            fs::read_to_string(tempdir.path().join(METRICS_EVENTS_FILE_NAME)).unwrap();

        assert_eq!(metrics_buffer_file, "");
    }

    /// Test that the [MetricsBuffer] reads entries as expected
    #[test]
    fn test_metrics_buffer_read() {
        let tempdir = tempfile::tempdir().unwrap();

        let mut buffer = MetricsBuffer::read(tempdir.as_ref()).unwrap();

        assert!(!buffer.is_expired(Duration::seconds(0)));
        assert_eq!(buffer.oldest_timestamp(), None);
        assert_eq!(buffer.buffer.len(), 0);

        let mut entry_foo = make_entry("foo");
        entry_foo.timestamp = OffsetDateTime::now_utc() - Duration::hours(3);

        let entry_bar = make_entry("bar");

        buffer.push(entry_foo.clone()).unwrap();
        buffer.push(entry_bar.clone()).unwrap();

        // Can't create another bufer object while one is currently locked
        drop(buffer);

        let buffer = MetricsBuffer::read(tempdir.as_ref()).unwrap();

        assert_eq!(buffer.buffer.len(), 2);
        assert_eq!(buffer.oldest_timestamp(), Some(entry_foo.timestamp));
        assert!(!buffer.is_expired(Duration::hours(4)));
        assert!(buffer.is_expired(Duration::hours(2)));

        {
            let mut iter = buffer.iter();
            assert_eq!(iter.next(), Some(&entry_foo));
            assert_eq!(iter.next(), Some(&entry_bar));
        }
    }

    /// Test that the client is constructed correctly given a config
    #[test]
    fn test_client_construction() {
        let uuid = Uuid::new_v4();

        let tempdir = tempfile::tempdir().unwrap();
        let data_dir = tempdir.path().join("data");
        let cache_dir = tempdir.path().join("cache");

        fs::create_dir_all(&data_dir).unwrap();
        fs::create_dir_all(&cache_dir).unwrap();

        fs::write(data_dir.join(METRICS_UUID_FILE_NAME), format!("{uuid}")).unwrap();

        let config = Config {
            flox: FloxConfig {
                data_dir,
                cache_dir: cache_dir.clone(),
                ..Default::default()
            },
            ..Default::default()
        };

        let client = Client::new_with_config(&config, TestConnection::default()).unwrap();

        assert_eq!(client.uuid, uuid);
        assert_eq!(client.metrics_dir, cache_dir);
    }

    /// Test that [Client::record_metric] records metrics as expected
    #[test]
    fn test_client_record_metric() {
        let (client, _tempdir) = create_client();

        let event_foo = make_event("foo");
        client.record_metric(event_foo.clone()).unwrap();
        {
            let buffer = MetricsBuffer::read(&client.metrics_dir).unwrap();
            let entry_foo = buffer.iter().next().unwrap();
            assert_eq!(entry_foo.subcommand, event_foo.subcommand);
        }

        let event_bar = make_event("bar");
        client.record_metric(event_bar.clone()).unwrap();

        {
            let buffer = MetricsBuffer::read(&client.metrics_dir).unwrap();
            let mut iter = buffer.iter();

            let entry_foo = iter.next().unwrap();
            let entry_bar = iter.next().unwrap();
            assert_eq!(entry_foo.subcommand, event_foo.subcommand);
            assert_eq!(entry_bar.subcommand, event_bar.subcommand);
        }
    }

    /// Test that [Client::flush] sends metrics as expected and clears the buffer
    #[test]
    fn test_client_flush() {
        let (mut client, _tempdir) = create_client();

        let event_foo = make_event("foo");
        let event_bar = make_event("bar");

        client.record_metric(event_foo.clone()).unwrap();
        client.record_metric(event_bar.clone()).unwrap();
        client.flush(true).unwrap();

        let buffer = MetricsBuffer::read(&client.metrics_dir).unwrap();
        assert_eq!(buffer.buffer.len(), 0);

        let x: Box<TestConnection> = client.connection.into_any().downcast().unwrap();

        let mut sent = x.sent.iter();
        let mut first_send = sent.next().unwrap().iter();
        let entry_foo = first_send.next().unwrap();
        let entry_bar = first_send.next().unwrap();

        assert!(first_send.next().is_none());
        assert!(sent.next().is_none());
        assert_eq!(entry_foo.subcommand, event_foo.subcommand);
        assert_eq!(entry_bar.subcommand, event_bar.subcommand);
    }

    /// Test that [Hub::try_guard] returns a guard as expected
    /// And that the guard flushes the metrics on drop when the buffer is expired.
    /// And that the guard does not flush the metrics when the buffer is not expired.
    #[test]
    fn test_hub_try_guard() {
        let (client, _tempdir) = create_client();

        let hub = Hub {
            client: Arc::new(Mutex::new(None)),
        };
        hub.set_client(client);

        let guard = hub.try_guard();

        // only one guard at a time
        assert!(hub.try_guard().is_err());

        let event_foo = make_event("foo");
        let event_bar = make_event("bar");

        hub.record_metric(event_foo.clone()).unwrap();
        hub.record_metric(event_bar.clone()).unwrap();

        let metrics_dir = hub.with_client(|c| c.as_ref().unwrap().metrics_dir.clone());

        // check that the events are in the buffer
        // buffer needs to be dropped first to release the lock
        {
            let buffer = MetricsBuffer::read(&metrics_dir).unwrap();

            let mut buffered_items = buffer.iter();
            assert_eq!(
                buffered_items.next().unwrap().subcommand,
                event_foo.subcommand
            );
            assert_eq!(
                buffered_items.next().unwrap().subcommand,
                event_bar.subcommand
            );
        }

        // dropping the guard should flush the metrics if expired
        // nothing to be flushed now as the default max_age is 2 hours
        drop(guard);

        // force all events to be expired
        hub.with_client(|c| c.as_mut().unwrap().max_age = Duration::seconds(0));

        // create and drop the guard immediately
        drop(hub.try_guard().unwrap());

        // buffer should be empty now
        let buffer = MetricsBuffer::read(&metrics_dir).unwrap();
        assert_eq!(buffer.buffer.len(), 0);

        // extract the client
        let client = hub.with_client(|c| c.take().unwrap());

        // check what has been sent
        let x: Box<TestConnection> = client.connection.into_any().downcast().unwrap();

        let mut sent = x.sent.iter();
        let mut first_send = sent.next().unwrap().iter();
        let entry_foo = first_send.next().unwrap();
        let entry_bar = first_send.next().unwrap();

        assert!(first_send.next().is_none());
        assert!(sent.next().is_none());
        assert_eq!(entry_foo.subcommand, event_foo.subcommand);
        assert_eq!(entry_bar.subcommand, event_bar.subcommand);
    }

    /// Test that [MetricsLayer] records metrics as expected
    ///
    /// This test uses the global client to record metrics
    /// and thus requires to run in serial with other tests that use the global client.
    #[test]
    #[serial_test::serial(global_metrics_client)]
    fn test_metrics_layer() {
        let (client, _tempdir) = create_client();

        let backup_client = Hub::global().with_client(|c| c.replace(client));

        let subscriber = tracing_subscriber::registry().with(MetricsLayer::new());

        tracing::subscriber::with_default(subscriber, || {
            subcommand_metric!("foo", bar = "baz");
        });

        let client = Hub::global().with_client(|c| {
            let test_client = c.take().unwrap();
            *c = backup_client;
            test_client
        });

        let buffer = MetricsBuffer::read(&client.metrics_dir).unwrap();

        let entry = buffer.iter().next().unwrap();
        assert_eq!(entry.subcommand, Some("foo".to_string()));
        assert_eq!(
            entry.extras,
            vec![("bar".to_string(), "baz".to_string())]
                .into_iter()
                .collect()
        );
    }

    /// Test that [MetricsLayer] records metrics as expected
    /// even if a filter is applied
    ///
    /// This test uses the global client to record metrics
    /// and thus requires to run in serial with other tests that use the global client.
    #[test]
    #[serial_test::serial(global_metrics_client)]
    fn test_metrics_layer_with_filter() {
        let (client, _tempdir) = create_client();

        let backup_client = Hub::global().with_client(|c| c.replace(client));

        let (subscriber, reload_handle) = create_registry_and_filter_reload_handle();

        tracing::subscriber::with_default(subscriber, || {
            subcommand_metric!("foo");
            update_filters(&reload_handle, "debug");
            subcommand_metric!("bar");
            update_filters(&reload_handle, "off");
            subcommand_metric!("baz");
        });

        let client = Hub::global().with_client(|c| {
            let test_client = c.take().unwrap();
            *c = backup_client;
            test_client
        });

        let buffer = MetricsBuffer::read(&client.metrics_dir).unwrap();
        let mut buffer_iter = buffer.iter();
        let entry_foo = buffer_iter.next().unwrap();
        let entry_bar = buffer_iter.next().unwrap();
        let entry_baz = buffer_iter.next().unwrap();

        assert_eq!(entry_foo.subcommand, Some("foo".to_string()));
        assert_eq!(entry_bar.subcommand, Some("bar".to_string()));
        assert_eq!(entry_baz.subcommand, Some("baz".to_string()));
    }
}
