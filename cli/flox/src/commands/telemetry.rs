/// Token telemetry for the Flox Agent prototype.
///
/// Emits structured events to a local JSONL file that a background process
/// (or the FloxHub agent) can tail and forward to FloxHub.  Direct HTTP
/// posting is left as a future enhancement; the local file lets the demo
/// work end-to-end even without a running FloxHub.
///
/// File location: $FLOX_AGENT_TELEMETRY_FILE or ~/.cache/flox/agent-telemetry.jsonl
use std::io::Write;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use uuid::Uuid;

/// Structured telemetry event emitted from the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelemetryEvent {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_id: Option<String>,
    pub event_type: TelemetryEventType,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryEventType {
    CommandStarted,
    CommandFinished,
    TokensConsumed,
    FileChanged,
    NetworkAccessed,
    CostGuardrailBreached,
    SessionStart,
    SessionEnd,
}

/// Emit a telemetry event.  Fire-and-forget: failures are logged but not propagated.
pub fn emit(cache_dir: &PathBuf, event: TelemetryEvent) {
    let log_path = telemetry_log_path(cache_dir);

    let json = match serde_json::to_string(&event) {
        Ok(j) => j,
        Err(e) => {
            warn!("Failed to serialize telemetry event: {e}");
            return;
        },
    };

    match append_to_local_log(&log_path, &json) {
        Ok(()) => debug!("Telemetry event buffered to {}", log_path.display()),
        Err(e) => warn!("Could not write telemetry to log: {e}"),
    }
}

fn telemetry_log_path(cache_dir: &PathBuf) -> PathBuf {
    std::env::var("FLOX_AGENT_TELEMETRY_FILE")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| cache_dir.join("agent-telemetry.jsonl"))
}

fn append_to_local_log(path: &PathBuf, json: &str) -> anyhow::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{json}")?;
    Ok(())
}

/// Generate a new session ID.
pub fn new_session_id() -> String {
    Uuid::new_v4().to_string()
}

/// Build a CommandStarted event.
pub fn command_started_event(
    session_id: &str,
    env_id: Option<String>,
    command: &str,
) -> TelemetryEvent {
    TelemetryEvent {
        session_id: session_id.to_string(),
        env_id,
        event_type: TelemetryEventType::CommandStarted,
        timestamp: Utc::now(),
        payload: serde_json::json!({ "command": command }),
    }
}
