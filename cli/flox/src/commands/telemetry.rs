//! Token telemetry for the Flox Agent prototype.
//!
//! Emits structured events to a local JSONL file and, when FloxHub is
//! configured, also POSTs them directly to the BFF telemetry endpoint
//! (`<floxhub_url>/api/agent/telemetry`).  The HTTP POST is best-effort:
//! failures fall back to the local JSONL file so the demo works even without
//! a running FloxHub.
//!
//! File location: $FLOX_AGENT_TELEMETRY_FILE or ~/.cache/flox/agent-telemetry.jsonl
#![allow(dead_code)]
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use url::Url;
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

/// Emit a telemetry event to the local JSONL buffer.
/// Fire-and-forget: failures are logged but not propagated.
///
/// For HTTP posting to FloxHub, call [`post_to_floxhub`] separately from
/// an async context and `.await` the returned `JoinHandle` before returning
/// from the command handler.
pub fn emit(cache_dir: &Path, event: TelemetryEvent) {
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

/// POST a telemetry event to the FloxHub BFF asynchronously.
///
/// Returns a `tokio::task::JoinHandle` so the caller can await it before
/// returning, ensuring the HTTP call completes before the process exits.
/// A 3-second timeout ensures we never meaningfully delay activation.
/// All errors are swallowed — this must never fail activation.
///
/// BFF path convention: `<floxhub_url>/web-bff/api/agent/telemetry`
/// (matches the nginx routing in the FloxHub stack).
pub fn post_to_floxhub(
    base_url: &Url,
    event: &TelemetryEvent,
    auth_header: Option<&str>,
) -> tokio::task::JoinHandle<()> {
    // Derive API URL: replace leading "hub." in hostname with "api.".
    // For local dev: https://hub.local.flox.dev:8000
    //             → https://api.local.flox.dev:8000/web-bff/api/agent/telemetry
    let mut url = base_url.clone();
    if let Some(host) = base_url.host_str() {
        let api_host = if let Some(stripped) = host.strip_prefix("hub.") {
            format!("api.{stripped}")
        } else {
            host.to_string()
        };
        let _ = url.set_host(Some(&api_host));
    }
    url.set_path(&format!(
        "{}/web-bff/api/agent/telemetry",
        url.path().trim_end_matches('/')
    ));

    // Build a minimal JSON body that matches BFF endpoint expectations.
    let body = serde_json::json!({
        "session_id": event.session_id,
        "env_id": event.env_id.as_deref().unwrap_or(""),
        "event_type": event.event_type,
        "timestamp": event.timestamp.to_rfc3339(),
        "payload": event.payload,
    });

    let auth = auth_header.map(|h| h.to_string());
    let url_str = url.to_string();

    tokio::task::spawn(async move {
        // Accept self-signed TLS certs for local dev prototype.
        // Disable redirects so we don't silently lose the Authorization header
        // if nginx returns a 30x (it shouldn't, but this makes debugging easier).
        let client = match reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(3))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                debug!("Could not build reqwest client for telemetry POST: {e}");
                return;
            },
        };
        let mut req = client.post(&url_str).json(&body);
        if let Some(header_val) = &auth {
            req = req.header("Authorization", header_val);
        }
        match req.send().await {
            Ok(resp) => debug!("Telemetry POST {} -> {}", url_str, resp.status()),
            Err(e) => debug!("Telemetry POST failed (non-fatal): {e}"),
        }
    })
}

fn telemetry_log_path(cache_dir: &Path) -> PathBuf {
    std::env::var("FLOX_AGENT_TELEMETRY_FILE")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| cache_dir.join("agent-telemetry.jsonl"))
}

fn append_to_local_log(path: &Path, json: &str) -> anyhow::Result<()> {
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
    user_id: Option<&str>,
) -> TelemetryEvent {
    let mut payload = serde_json::json!({ "command": command });
    if let Some(uid) = user_id {
        payload["user_id"] = serde_json::Value::String(uid.to_string());
        payload["anonymous"] = serde_json::Value::Bool(false);
    } else {
        payload["anonymous"] = serde_json::Value::Bool(true);
    }
    TelemetryEvent {
        session_id: session_id.to_string(),
        env_id,
        event_type: TelemetryEventType::CommandStarted,
        timestamp: Utc::now(),
        payload,
    }
}

/// Build a CommandFinished event.
pub fn command_finished_event(
    session_id: &str,
    env_id: Option<String>,
    command: &str,
    success: bool,
) -> TelemetryEvent {
    TelemetryEvent {
        session_id: session_id.to_string(),
        env_id,
        event_type: TelemetryEventType::CommandFinished,
        timestamp: Utc::now(),
        payload: serde_json::json!({ "command": command, "success": success }),
    }
}
