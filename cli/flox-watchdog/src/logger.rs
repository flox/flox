use std::fs::{remove_file, OpenOptions};
use std::path::{Path, PathBuf};
use std::thread::{sleep, spawn};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use glob::glob;
use tracing::{debug, error};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(3600);
const KEEP_WATCHDOG_DAYS: u64 = 3;
const KEEP_SERVICES_LAST: usize = 5;

/// Initializes a logger that persists logs to an optional file in addition to `stderr`
pub(crate) fn init_logger(file_path: &Option<PathBuf>) -> Result<(), anyhow::Error> {
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(EnvFilter::from_default_env());
    let file_layer = if let Some(path) = file_path {
        let path = if path.is_relative() {
            std::env::current_dir()?.join(path)
        } else {
            path.clone()
        };
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("failed to open log file {}", path.display()))?;
        Some(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(file)
                .with_filter(EnvFilter::from_env("_FLOX_WATCHDOG_LOG_LEVEL")),
        )
    } else {
        None
    };
    let sentry_layer = sentry::integrations::tracing::layer().enable_span_attributes();
    tracing_subscriber::registry()
        .with(file_layer)
        .with(sentry_layer)
        .with(stderr_layer)
        .init();
    Ok(())
}

/// Starts a background thread which emits a log entry at an interval. This is
/// used as an indication of whether a watchdog's log file can be garbage
/// collected. The thread will run until the watchdog exits.
pub(crate) fn spawn_heartbeat_log() {
    /// Assert that HEARTBEAT_INTERVAL falls in the range of KEEP_WATCHDOG_DAYS at compile time.
    const _: () = assert!(
        HEARTBEAT_INTERVAL.as_secs() < duration_from_days(KEEP_WATCHDOG_DAYS).as_secs(),
        "`HEARTBEAT_INTERVAL` must be less than `KEEP_WATCHDOG_DAYS` days"
    );

    spawn(|| loop {
        debug!("still watching, woof woof");
        sleep(HEARTBEAT_INTERVAL);
    });
}

/// Starts a background thread which garbage collects known log files. This is
/// done on a best effort basis; errors are traced rather than being bubbled up
/// and the thread will run until the watchdog exits.
pub(crate) fn spawn_gc_logs(dir: impl AsRef<Path>) {
    let dir = dir.as_ref().to_path_buf();
    std::thread::spawn(move || {
        gc_logs_watchdog(&dir, KEEP_WATCHDOG_DAYS)
            .unwrap_or_else(|err| error!(%err, "failed to delete watchdog logs"));
        gc_logs_services(&dir, KEEP_SERVICES_LAST)
            .unwrap_or_else(|err| error!(%err, "failed to delete services logs"));
    });
}

/// Garbage collects watchdog log files, keeping the last N days by modified
/// time. This relies on the watchdog emitting a heartbeat log file from
/// `log_heartbeat`.
fn gc_logs_watchdog(dir: impl AsRef<Path>, keep_days: u64) -> Result<()> {
    let mut files = glob_log_files(dir, "watchdog.*.log")?;
    let threshold = duration_from_days(keep_days);
    let now = SystemTime::now();

    // Defaults to keeping if mtime is not supported by platform or filesystem.
    files.retain(|file| file_older_than(file, now, threshold).unwrap_or(false));

    for file in files {
        try_delete_log(file);
    }

    Ok(())
}

/// Garbage collects services log files, keeping the last N files by filename.
/// This relies on the log files having a timestamp in their filename.
fn gc_logs_services(dir: impl AsRef<Path>, keep_last: usize) -> Result<()> {
    let mut files = glob_log_files(dir, "services.*.log")?;
    if files.len() <= keep_last {
        return Ok(());
    }

    files.sort_unstable();
    files.truncate(files.len() - keep_last);

    for file in files {
        try_delete_log(file);
    }

    Ok(())
}

/// Construct a Duration from `days`.
const fn duration_from_days(days: u64) -> Duration {
    let seconds_in_day = 24 * 60 * 60;
    Duration::from_secs(days * seconds_in_day)
}

/// Returns whether a file is older than `threshold`.
fn file_older_than(file: impl AsRef<Path>, now: SystemTime, threshold: Duration) -> Result<bool> {
    let metadata = file
        .as_ref()
        .metadata()
        .context("failed to get file metadata")?;
    let modified = metadata.modified()?;
    let elapsed = now.duration_since(modified)?;
    let older = elapsed > threshold;

    Ok(older)
}

/// Glob files matching a pattern, ignoring any unreadable paths.
fn glob_log_files(dir: impl AsRef<Path>, name: &str) -> Result<Vec<PathBuf>> {
    let pattern = format!("{}/{name}", dir.as_ref().to_string_lossy());
    let paths = glob(&pattern).context("failed to glob logs")?;
    let files = paths
        .filter_map(Result::ok)
        .filter(|path| Path::is_file(path))
        .collect();
    Ok(files)
}

/// Delete a log file. Errors are traced and not bubbled up, so that we don't
/// get stuck on individual files.
fn try_delete_log(file: impl AsRef<Path>) {
    remove_file(file.as_ref()).unwrap_or_else(|err| error!(%err, "failed to delete log"));
    debug!(path = %file.as_ref().display(), "deleted log");
}

#[cfg(test)]
mod tests {
    use std::fs::{create_dir, File};

    use tempfile::tempdir;

    use super::*;

    /// Create a test log file. Optionally set a modified time in days.
    fn create_log_file(dir: &Path, name: &str, days: Option<u64>) -> PathBuf {
        let path = dir.join(name);
        File::create(&path).unwrap();

        if let Some(days) = days {
            let mtime = SystemTime::now() - duration_from_days(days);
            File::open(&path).unwrap().set_modified(mtime).unwrap();
        }

        path
    }

    #[test]
    fn test_gc_logs_watchdog_removes_old_files() {
        let keep_last = 2;
        let dir = tempdir().unwrap();
        let file_now = create_log_file(&dir.path(), "watchdog.now.log", None);
        let file_one = create_log_file(&dir.path(), "watchdog.one.log", Some(keep_last - 1));
        let file_two = create_log_file(&dir.path(), "watchdog.two.log", Some(keep_last));
        let file_three = create_log_file(&dir.path(), "watchdog.three.log", Some(keep_last + 1));

        gc_logs_watchdog(dir.path(), keep_last).unwrap();
        assert!(file_now.exists());
        assert!(file_one.exists());
        assert!(!file_two.exists());
        assert!(!file_three.exists());

        // Keeps the same files on second run.
        gc_logs_watchdog(dir.path(), keep_last).unwrap();
        assert!(file_now.exists());
        assert!(file_one.exists());
    }

    #[test]
    fn test_gc_logs_watchdog_ignores_other_files() {
        let keep_last = 2;
        let dir = tempdir().unwrap();
        let filenames = vec![
            "watchdog",
            "watchdog.log",
            "services.log",
            "services.123.log",
        ];
        let files: Vec<PathBuf> = filenames
            .clone()
            .into_iter()
            .map(|filename| create_log_file(&dir.path(), filename, Some(keep_last - 1)))
            .collect();
        assert_eq!(files.len(), filenames.len());

        gc_logs_watchdog(dir.path(), keep_last).unwrap();
        for file in files {
            assert!(file.exists(), "file should exist: {}", file.display());
        }
    }

    #[test]
    fn test_gc_logs_services_removes_old_files() {
        let keep_days = 2;
        let dir = tempdir().unwrap();
        let files: Vec<PathBuf> = (1..=keep_days * 2)
            .map(|i| create_log_file(&dir.path(), &format!("services.{}.log", i), None))
            .collect();

        gc_logs_services(dir.path(), keep_days).unwrap();
        assert!(!files[0].exists());
        assert!(!files[1].exists());
        assert!(files[2].exists());
        assert!(files[3].exists());

        // Keeps the same files on second run.
        gc_logs_services(dir.path(), keep_days).unwrap();
        assert!(files[2].exists());
        assert!(files[3].exists());
    }

    #[test]
    fn test_gc_logs_services_ignores_other_files() {
        let keep_days: usize = 2;
        let dir = tempdir().unwrap();
        let filenames = vec![
            "services",
            "services.log",
            "watchdog.log",
            "watchdog.123.log",
        ];
        let files: Vec<PathBuf> = filenames
            .clone()
            .into_iter()
            .map(|filename| create_log_file(&dir.path(), filename, None))
            .collect();
        assert_eq!(files.len(), filenames.len());

        gc_logs_services(dir.path(), keep_days).unwrap();
        for file in files {
            assert!(file.exists(), "file should exist: {}", file.display());
        }
    }

    #[test]
    fn test_glob_files() {
        let dir = tempdir().unwrap();
        let expected = dir.path().join("file.log");
        File::create(&expected).unwrap();

        // The following are ignored.
        File::create(dir.path().join("file.other")).unwrap();
        let sub_dir = dir.path().join("sub");
        create_dir(&sub_dir).unwrap();
        File::create(sub_dir.join("sub.log")).unwrap();
        File::create(sub_dir.join("sub.other")).unwrap();

        let files = glob_log_files(dir.path(), "*.log").unwrap();
        assert_eq!(files, vec![expected]);
    }

    #[test]
    fn test_file_older_than() {
        let threshold_days = 3;
        let threshold_dur = duration_from_days(threshold_days);
        let dir = tempdir().unwrap();
        let now = SystemTime::now();

        let old_file = create_log_file(&dir.path(), "old.log", Some(threshold_days + 1));
        assert!(file_older_than(&old_file, now, threshold_dur).unwrap());

        let new_file = create_log_file(&dir.path(), "new.log", Some(threshold_days - 1));
        assert!(!file_older_than(&new_file, now, threshold_dur).unwrap());
    }
}
