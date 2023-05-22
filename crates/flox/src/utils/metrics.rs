use std::sync::mpsc;

use anyhow::{Context, Result};
use fslock::LockFile;
use indoc::indoc;
use log::{debug, error};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::format_description::well_known::Iso8601;
use time::{Duration, OffsetDateTime};
use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

use crate::config::Config;

pub const FLOX_VERSION: &str = env!("FLOX_VERSION");

pub const METRICS_EVENTS_FILE_NAME: &str = "metrics-events-v2.json";
pub const METRICS_UUID_FILE_NAME: &str = "metrics-uuid";
pub const METRICS_LOCK_FILE_NAME: &str = "metrics-lock";

pub const METRICS_EVENTS_URL: &str = env!("METRICS_EVENTS_URL");
pub const METRICS_EVENTS_API_KEY: &str = env!("METRICS_EVENTS_API_KEY");

#[macro_export]
macro_rules! subcommand_metric {
    ($arg:tt) => {{
        tracing::trace!(target: "flox_command", subcommand = $arg, "Handling subcommand: {}", $arg);
    }};
}

struct PosthogVisitor<'a>(&'a mut Option<String>);

impl<'a> tracing::field::Visit for PosthogVisitor<'a> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "subcommand" {
            *self.0 = Some(value.to_string());
        }
    }

    fn record_debug(&mut self, _field: &tracing::field::Field, _value: &dyn std::fmt::Debug) {}
}

struct PosthogEvent {
    subcommand: Option<String>,
}

pub struct PosthogLayer {
    tx: std::sync::Mutex<mpsc::Sender<PosthogEvent>>,
}

impl PosthogLayer {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<PosthogEvent>();

        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            handle.block_on(async {
                while let Ok(event) = rx.recv() {
                    if let Err(err) = add_metric(event.subcommand).await {
                        error!("Error adding metric: {err}");
                    }
                }
            })
        });

        PosthogLayer {
            tx: std::sync::Mutex::new(tx),
        }
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
        let mut visitor = PosthogVisitor(&mut subcommand);
        event.record(&mut visitor);

        if let Ok(tx) = self.tx.lock() {
            if let Err(err) = tx.send(PosthogEvent { subcommand }) {
                error!("Error adding metric: {err}");
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MetricEntry {
    subcommand: Option<String>,
    timestamp: OffsetDateTime,
    flox_version: String,
    os_family: Option<String>,
    os_family_release: Option<String>,
    os: Option<String>,
    os_version: Option<String>,
    empty_flags: Vec<String>,
}

impl MetricEntry {
    pub fn new(subcommand: Option<String>, now: OffsetDateTime) -> MetricEntry {
        let linux_release = sys_info::linux_os_release().ok();

        MetricEntry {
            subcommand,
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

async fn push_metrics(metrics: Vec<MetricEntry>, uuid: Uuid) -> Result<()> {
    let events = metrics
        .into_iter()
        .map(|entry| {
            Ok(json!({
                "event": "cli-invocation",
                "properties": {
                    "distinct_id": uuid,
                    "$device_id": uuid,

                    "$current_url": entry.subcommand.as_ref().map(|x| format!("flox://{x}")),
                    "$pathname": entry.subcommand,
                    "subcommand": entry.subcommand,

                    "empty_flags": entry.empty_flags,

                    "$lib": "flox-cli",

                    "rust_preview": true,

                    "os": entry.os,
                    "os_version": entry.os_version,
                    "os_family": entry.os_family,
                    "os_family_release": entry.os_family_release,

                    // compat
                    "$os": entry.os_family,
                    "kernel_version": entry.os_family_release,

                    "$set_once": {
                        "initial_flox_version": FLOX_VERSION,

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

                        "flox_version": FLOX_VERSION,

                        "os": entry.os,
                        "os_version": entry.os_version,
                        "os_family": entry.os_family,
                        "os_family_release": entry.os_family_release,

                        // compat
                        "$os": entry.os_family,
                        "kernel_version": entry.os_family_release,

                        // to be deprecated
                        "flox-cli-uuid": uuid,
                    },
                },

                // Event ID used for deduplication
                "uuid": Uuid::new_v4(),

                "timestamp": entry.timestamp.format(&Iso8601::DEFAULT)?,
            }))
        })
        .collect::<Result<Vec<serde_json::Value>>>()?;

    reqwest::Client::new()
        .post(METRICS_EVENTS_URL)
        .json(&json!({
            "api_key": METRICS_EVENTS_API_KEY,
            "batch": events,
        }))
        .send()
        .await?;

    Ok(())
}

pub async fn add_metric(subcommand: Option<String>) -> Result<()> {
    let config = Config::parse()?;

    if config.flox.disable_metrics {
        return Ok(());
    }

    let cache_dir = config.flox.cache_dir;
    let data_dir = config.flox.data_dir;

    let mut metrics_lock = LockFile::open(&cache_dir.join(METRICS_LOCK_FILE_NAME))?;
    tokio::task::spawn_blocking(move || metrics_lock.lock()).await??;

    let uuid_path = data_dir.join(METRICS_UUID_FILE_NAME);

    let uuid = match tokio::fs::File::open(&uuid_path).await {
        Ok(mut f) => {
            let mut uuid_str = String::new();
            f.read_to_string(&mut uuid_str).await?;
            let uuid_str_trimmed = uuid_str.trim();
            Uuid::try_parse(uuid_str_trimmed).with_context(|| {
                indoc! {"
                Could not parse the metrics UUID of this installation in {uuid_path}
            "}
            })?
        },

        Err(err) => Err(err)?,
    };

    let buffer_file_path = cache_dir.join(METRICS_EVENTS_FILE_NAME);
    let mut events_buffer_file = OpenOptions::new()
        .write(true)
        .read(true)
        .create(true)
        .open(buffer_file_path)
        .await?;

    let mut buffer_json = String::new();
    events_buffer_file.read_to_string(&mut buffer_json).await?;
    let mut buffer_iter = serde_json::Deserializer::from_str(&buffer_json)
        .into_iter::<MetricEntry>()
        .filter_map(|x| x.ok())
        .peekable();

    let now = OffsetDateTime::now_utc();

    let new_entry = MetricEntry::new(subcommand, now);

    // Note: assumes the oldest metric entry must come first
    if buffer_iter
        .peek()
        .map(|e| (now - e.timestamp) > Duration::hours(2))
        .unwrap_or(false)
    {
        debug!("Pushing buffered metrics");
        let mut buffer: Vec<MetricEntry> = buffer_iter.collect();
        buffer.push(new_entry);
        push_metrics(buffer, uuid).await?;
        events_buffer_file.set_len(0).await?;
    } else {
        debug!("Writing new metrics buffer entry");
        events_buffer_file
            .write_all(format!("\n{}", serde_json::to_string(&new_entry)?).as_bytes())
            .await?;
    }

    Ok(())
}
