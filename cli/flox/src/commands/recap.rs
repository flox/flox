/// Audit/recap surface for Flox Agent sessions.
///
/// Tracks what the agent changed during a sandboxed session and presents a
/// human-readable summary.  The underlying audit log is written by the launcher
/// process (outside the sandbox) so the agent cannot tamper with it.
///
/// Structured log format matches the FloxHub "Recap" tab schema.
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bpaf::Bpaf;
use chrono::{DateTime, Utc};
use flox_rust_sdk::flox::Flox;
use serde::{Deserialize, Serialize};

use crate::utils::message;

/// Default location for the agent audit log, outside any sandbox writable view.
pub fn default_audit_log_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("agent-sessions.jsonl")
}

/// One change event recorded during a sandboxed session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub session_id: String,
    pub env_id: Option<String>,
    pub event_type: AuditEventType,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEventType {
    ManifestDiff,
    SkillInstall,
    FileWrite,
    NetworkAccess,
    CommandRun,
    SessionStart,
    SessionEnd,
}

#[derive(Bpaf, Clone, Debug)]
pub struct Recap {
    /// Session ID to recap (defaults to the most recent session)
    #[bpaf(long("session"), argument("id"), optional)]
    pub session: Option<String>,

    /// Output as JSON instead of human-readable text
    #[bpaf(long)]
    pub json: bool,
}

impl Recap {
    pub fn handle(self, flox: Flox) -> Result<()> {
        let log_path = default_audit_log_path(&flox.cache_dir);

        if !log_path.exists() {
            message::plain(
                "No agent sessions recorded yet.\n  Start a sandboxed session with 'flox activate --sandbox' to begin auditing.",
            );
            return Ok(());
        }

        let events = read_events(&log_path, self.session.as_deref())
            .with_context(|| format!("Could not read audit log at {}", log_path.display()))?;

        if events.is_empty() {
            message::plain("No events found for this session.");
            return Ok(());
        }

        if self.json {
            println!("{}", serde_json::to_string_pretty(&events)?);
            return Ok(());
        }

        print_recap(&events);
        Ok(())
    }
}

fn read_events(log_path: &std::path::Path, session_id: Option<&str>) -> Result<Vec<AuditEvent>> {
    let contents = std::fs::read_to_string(log_path)?;
    let mut events: Vec<AuditEvent> = Vec::new();

    for line in contents.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<AuditEvent>(line) {
            events.push(event);
        }
    }

    // Filter to the requested session or pick the most recent.
    if let Some(sid) = session_id {
        events.retain(|e| e.session_id == sid);
    } else if let Some(last_session) = events.last().map(|e| e.session_id.clone()) {
        events.retain(|e| e.session_id == last_session);
    }

    Ok(events)
}

fn print_recap(events: &[AuditEvent]) {
    let session_id = events
        .first()
        .map(|e| e.session_id.as_str())
        .unwrap_or("unknown");
    let env_id = events
        .first()
        .and_then(|e| e.env_id.as_deref())
        .unwrap_or("unknown");

    println!("Session Recap");
    println!("─────────────────────────────────────────");
    println!("  Session: {session_id}");
    println!("  Env:     {env_id}");
    println!();

    let mut manifest_changes = 0u32;
    let mut file_writes = 0u32;
    let mut network_hosts: Vec<String> = Vec::new();
    let mut skill_installs: Vec<String> = Vec::new();

    for event in events {
        match event.event_type {
            AuditEventType::ManifestDiff => {
                manifest_changes += 1;
                println!("  📦  Manifest change: {}", event.detail);
                if let Some(ref before) = event.before {
                    println!("      before: {before}");
                }
                if let Some(ref after) = event.after {
                    println!("      after:  {after}");
                }
            },
            AuditEventType::SkillInstall => {
                skill_installs.push(event.detail.clone());
                println!("  🧩  Skill installed: {}", event.detail);
            },
            AuditEventType::FileWrite => {
                file_writes += 1;
                println!("  📝  File written: {}", event.detail);
            },
            AuditEventType::NetworkAccess => {
                network_hosts.push(event.detail.clone());
            },
            AuditEventType::CommandRun => {
                println!("  ▶   Command: {}", event.detail);
            },
            AuditEventType::SessionStart => {
                println!(
                    "  🚀  Session started: {}",
                    event.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
                );
            },
            AuditEventType::SessionEnd => {
                println!(
                    "  🏁  Session ended: {}",
                    event.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
                );
            },
        }
    }

    println!();
    println!("Summary");
    println!("─────────────────────────────────────────");
    println!("  Manifest changes: {manifest_changes}");
    println!("  Files written:    {file_writes}");
    println!("  Skills installed: {}", skill_installs.len());
    if !network_hosts.is_empty() {
        println!("  Network hosts accessed:");
        for host in &network_hosts {
            println!("    • {host}");
        }
    }
}

/// Stable path for the persistent-activation marker for an environment.
///
/// Written by `flox activate --persistent` and read by `flox envs` to
/// display the `[persistent]` tag.  The marker lives in `cache_dir` rather
/// than in the runtime activation-state directory so it survives shell exit
/// (the activation-state dir is cleaned up when all PIDs detach).
///
/// Path: `{cache_dir}/agent/persistent-markers/{hash(dot_flox_path)}`
pub fn persistent_marker_path(cache_dir: &Path, dot_flox_path: &Path) -> PathBuf {
    cache_dir
        .join("agent")
        .join("persistent-markers")
        .join(flox_core::path_hash(dot_flox_path))
}

/// Append an audit event to the session log.
/// Called by commands that track agent actions (install, uninstall, edit).
// Prototype public API — not yet wired into commands.
#[allow(dead_code)]
pub fn append_audit_event(cache_dir: &Path, event: AuditEvent) -> Result<()> {
    use std::io::Write;
    let log_path = default_audit_log_path(cache_dir);
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("Could not open audit log at {}", log_path.display()))?;
    let line = serde_json::to_string(&event)?;
    writeln!(file, "{line}")?;
    Ok(())
}
