use std::collections::{HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result};
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

use crate::config::Config;

pub const METRICS_EVENTS_FILE_NAME: &str = "metrics-events-v2.json";
pub const METRICS_UUID_FILE_NAME: &str = "metrics-uuid";
pub const METRICS_LOCK_FILE_NAME: &str = "metrics-lock";
const BUFFER_EXPIRY: Duration = Duration::hours(2);

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
        if let Err(err) = add_metric(MetricEvent { subcommand, extras }) {
            debug!("Error adding metric: {err}");
        }
    }
}

/// A single metric entry
/// This is the a metric event with additional static metadata
#[derive(Debug, Serialize, Deserialize)]
pub struct MetricEntry {
    subcommand: Option<String>,
    #[serde(flatten)]
    extras: HashMap<String, String>,
    timestamp: OffsetDateTime,
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
        now: OffsetDateTime,
    ) -> MetricEntry {
        let linux_release = sys_info::linux_os_release().ok();

        MetricEntry {
            subcommand,
            extras,
            timestamp: now,
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

/// Push metrics to the telemetry backend
///
/// Any network errors will bubble up and be catched by the event handler.
/// If the network request failed, the buffer file is _not_ cleared.
fn push_metrics(mut metrics: MetricsBuffer, uuid: Uuid) -> Result<()> {
    debug!("Pushing metrics to server");

    let version = FLOX_VERSION.to_string();
    let events = metrics
        .iter()
        .map(|entry| {
            Ok(json!({
                "event": "cli-invocation",
                "properties": {
                    "distinct_id": uuid,
                    "subcommand": entry.subcommand,
                    "extras": entry.extras,

                    "$device_id": uuid,

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
                        "initial_flox_version": version,

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
                        "flox_cli_uuid": uuid,

                        "flox_version": version,

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

    debug!("Sending metrics to {}", &*METRICS_EVENTS_URL);
    debug!("Metrics: {events:?}", events = events);

    let req = reqwest::Client::new()
        .put(&*METRICS_EVENTS_URL)
        .header("content-type", "application/json")
        .header("x-api-key", METRICS_EVENTS_API_KEY)
        .header("user-agent", format!("flox-cli/{}", version))
        .json(&events)
        .send();

    let handle = tokio::runtime::Handle::current();
    let _guard = handle.enter();
    futures::executor::block_on(req).context("could not send to telemetry backend")?;

    metrics.clear()?;
    Ok(())
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
            .write(true)
            .read(true)
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
    /// and the oldest entry is older than [BUFFER_EXPIRY].
    fn is_expired(&self) -> bool {
        let now = OffsetDateTime::now_utc();
        self.oldest_timestamp()
            .map(|oldest| now - oldest > BUFFER_EXPIRY)
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

fn read_metrics_uuid(config: &Config) -> Result<Uuid> {
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

fn add_metric(event: MetricEvent) -> Result<()> {
    let config = Config::parse()?;

    if config.flox.disable_metrics {
        return Ok(());
    }

    let uuid = read_metrics_uuid(&config)?;
    let mut metrics_buffer = MetricsBuffer::read(&config.flox.cache_dir)?;

    let new_entry = MetricEntry::new(event, OffsetDateTime::now_utc());
    metrics_buffer.push(new_entry)?;

    let force_flush_buffer = std::env::var("_FLOX_FORCE_FLUSH_METRICS").is_ok();

    if metrics_buffer.is_expired() || force_flush_buffer {
        push_metrics(metrics_buffer, uuid)?;
    }

    Ok(())
}
