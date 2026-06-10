//! Periodic background pruning of managed-environment generation GC-root links.
//!
//! The activation executive is a long-running per-user process (users activate
//! their default environment on login), so it is a natural place to drive the
//! opportunistic generation-link prune on a timer (flox#4332). The worker
//! self-locks, so multiple executives firing it concurrently is harmless.
//!
//! design-debt: the executive has to shell out to the `flox` CLI's hidden
//! `prune-generation-links` worker because the executive is a separate binary
//! that cannot link `flox-rust-sdk` to run the prune directly. Were the
//! executive the same binary as the CLI, this would be a direct function call.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::{sleep, spawn};
use std::time::{Duration, SystemTime};

use anyhow::Result;
use tracing::error;

/// How often the executive fires a generation-link prune.
const GENERATION_PRUNE_INTERVAL: Duration = Duration::from_secs(3600);

/// Starts a background thread that periodically prunes aged generation GC-root
/// links by spawning `<flox_bin> prune-generation-links`. Best-effort: failures
/// are logged and the thread keeps looping until the executive exits.
pub(super) fn spawn_generation_prune_thread(flox_bin: String, log_dir: PathBuf) {
    spawn(move || {
        loop {
            // Wait first so the prune never competes with activation startup.
            sleep(GENERATION_PRUNE_INTERVAL);
            if let Err(err) = spawn_prune_process(&flox_bin, &log_dir) {
                error!(%err, "failed to spawn generation-link prune");
            }
        }
    });
}

/// Fire-and-forget a `flox prune-generation-links` subprocess, redirecting its
/// output to a per-process log file rather than the executive's std streams.
fn spawn_prune_process(flox_bin: &str, log_dir: &Path) -> Result<()> {
    let timestamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    fs::create_dir_all(log_dir)?;
    let log_file = File::create(log_dir.join(format!("generation-prune.{timestamp}.log")))?;

    Command::new(flox_bin)
        .arg("prune-generation-links")
        .arg("-vv") // enable debug logging into the log file
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log_file)
        .spawn()?;

    Ok(())
}
