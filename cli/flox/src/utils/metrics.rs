use std::collections::{HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};

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

pub static METRICS_EVENTS_URL: Lazy<String> = Lazy::new(|| {
    std::env::var("_FLOX_METRICS_URL_OVERRIDE").unwrap_or(env!("METRICS_EVENTS_URL").to_string())
});
pub const METRICS_EVENTS_API_KEY: &str = env!("METRICS_EVENTS_API_KEY");

/// Creates a trace event for the given subcommand.
///
/// We set the target to `flox_command` so that we can filter for these exact events.
#[macro_export]
macro_rules! subcommand_metric {
    ($arg:tt $(, $key:ident = $value:expr)*) => {{
        tracing::trace!(target: "flox_command", subcommand = $arg $(, $key = $value)*);
    }};
}

struct PosthogVisitor<'a>(&'a mut Option<String>, &'a mut HashMap<String, String>);

impl<'a> tracing::field::Visit for PosthogVisitor<'a> {
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

pub struct PosthogEvent {
    pub subcommand: Option<String>,
    pub extras: HashMap<String, String>,
}

pub struct PosthogLayer {}

impl PosthogLayer {
    pub fn new() -> Self {
        PosthogLayer {}
    }
}

impl<S> tracing_subscriber::Layer<S> for PosthogLayer
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
        let mut visitor = PosthogVisitor(&mut subcommand, &mut extras);
        event.record(&mut visitor);

        if let Err(err) = add_metric(PosthogEvent { subcommand, extras }) {
            debug!("Error adding metric: {err}");
        }
    }
}

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
        PosthogEvent { subcommand, extras }: PosthogEvent,
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

fn push_metrics(mut metrics: MetricsBuffer, uuid: Uuid) -> Result<()> {
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
    let req = reqwest::Client::new()
        .put(&*METRICS_EVENTS_URL)
        .header("content-type", "application/json")
        .header("x-api-key", METRICS_EVENTS_API_KEY)
        .header("user-agent", format!("flox-cli/{}", version))
        .json(&events)
        .send();

    let handle = tokio::runtime::Handle::current();
    let _guard = handle.enter();
    futures::executor::block_on(req)?;

    metrics.clear();
    Ok(())
}

#[derive(Default, Debug)]
struct MetricsBuffer(Option<File>, VecDeque<MetricEntry>);
impl MetricsBuffer {
    /// Reads the metrics buffer from the given file
    fn read_from_file(mut file: File) -> Result<Self> {
        let mut buffer_json = String::new();
        file.read_to_string(&mut buffer_json)?;

        let buffer_iter = serde_json::Deserializer::from_str(&buffer_json)
            .into_iter::<MetricEntry>()
            .filter_map(|x| x.ok())
            .collect();

        Ok(MetricsBuffer(Some(file), buffer_iter))
    }

    /// Reads the metrics buffer from the cache directory
    fn read(config: &Config) -> Result<Self> {
        // dont create a metrics buffer if metrics are disabled anyway
        if config.flox.disable_metrics {
            return Ok(Default::default());
        }

        let cache_dir = &config.flox.cache_dir;

        let mut metrics_lock = LockFile::open(&cache_dir.join(METRICS_LOCK_FILE_NAME))?;
        metrics_lock.lock()?;

        let buffer_file_path = cache_dir.join(METRICS_EVENTS_FILE_NAME);
        let events_buffer_file = OpenOptions::new()
            .write(true)
            .read(true)
            .create(true)
            .open(buffer_file_path)?;

        Self::read_from_file(events_buffer_file)
    }

    /// Returns the oldest timestamp in the buffer
    fn oldest_timestamp(&self) -> Option<OffsetDateTime> {
        self.1.front().map(|x| x.timestamp)
    }

    /// Pushes a new metric entry to the buffer and syncs it to the buffer file
    fn push(&mut self, entry: MetricEntry) -> Result<()> {
        if let Some(ref mut file) = self.0 {
            let mut buffer_json = String::new();
            buffer_json.push_str(&serde_json::to_string(&entry)?);
            buffer_json.push('\n');
            file.write_all(buffer_json.as_bytes())?;
            file.flush()?;
        }

        self.1.push_back(entry);

        Ok(())
    }

    /// Clears the buffer and the buffer file
    ///
    /// This is used when the buffer is pushed to the server
    /// and we start collecting metrics in a new buffer.
    fn clear(&mut self) {
        if let Some(ref mut file) = self.0 {
            if let Err(e) = file.set_len(0) {
                debug!("Could not truncate metrics buffer file: {e}")
            }
            if let Err(e) = file.flush() {
                dbg!(&e);
                debug!("Could not truncate metrics buffer file: {e}")
            }
        }
        self.1.clear();
    }

    fn iter(&self) -> impl Iterator<Item = &MetricEntry> {
        self.1.iter()
    }
}

fn read_metrics_uuid(config: &Config) -> Result<Uuid> {
    let data_dir = &config.flox.data_dir;
    let uuid_path = data_dir.join(METRICS_UUID_FILE_NAME);

    std::fs::File::open(uuid_path)
        .or_else(|e| Err(e).context("Could not read metrics UUID file"))
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

fn add_metric(event: PosthogEvent) -> Result<()> {
    let config = Config::parse()?;

    if config.flox.disable_metrics {
        return Ok(());
    }

    let uuid = read_metrics_uuid(&config)?;
    let mut metrics_buffer = MetricsBuffer::read(&config)?;

    let now = OffsetDateTime::now_utc();

    let new_entry = MetricEntry::new(event, now);
    metrics_buffer.push(new_entry)?;

    // Note: assumes the oldest metric entry must come first
    let buffer_time_passed = metrics_buffer
        .oldest_timestamp()
        .map(|oldest| now - oldest > Duration::hours(2))
        .unwrap_or(false);

    let force_flush_buffer = std::env::var("_FLOX_FORCE_FLUSH_METRICS").unwrap() != "";

    if buffer_time_passed || force_flush_buffer {
        debug!("Pushing buffered metrics");
        push_metrics(metrics_buffer, uuid)?;
    }

    Ok(())
}
