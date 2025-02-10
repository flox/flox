use std::fs::remove_file;
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
const WATCHDOG_GC_INTERVAL: Duration = Duration::from_secs(3600);
const KEEP_WATCHDOG_DAYS: u64 = 3;
const KEEP_LAST_N_PROCESSES: usize = 5;

/// Initializes a logger that persists logs to an optional file in addition to `stderr`
pub(crate) fn init_logger(
    logs_dir: &Option<PathBuf>,
    log_file_prefix: &str,
) -> Result<(), anyhow::Error> {
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(EnvFilter::from_default_env());
    let file_layer = if let Some(dir_path) = logs_dir {
        let appender = tracing_appender::rolling::daily(dir_path, log_file_prefix);
        Some(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(appender)
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
/// and the thread will loop until the watchdog exits.
///
/// All of the functions called here must be deterministic because there may be
/// multiple watchdogs running for the same environment log dir.
pub(crate) fn spawn_logs_gc_threads(dir: impl AsRef<Path>) {
    let dir = dir.as_ref().to_path_buf();
    spawn(move || loop {
        gc_logs_watchdog(&dir, KEEP_WATCHDOG_DAYS)
            .unwrap_or_else(|err| error!(%err, "failed to delete watchdog logs"));
        gc_logs_per_process(&dir, "services.*.log", KEEP_LAST_N_PROCESSES)
            .unwrap_or_else(|err| error!(%err, "failed to delete services logs"));
        gc_logs_per_process(&dir, "upgrade-check.*.log", KEEP_LAST_N_PROCESSES)
            .unwrap_or_else(|err| error!(%err, "failed to delete upgrade-check logs"));

        std::thread::sleep(WATCHDOG_GC_INTERVAL);
    });
}

/// Garbage collects watchdog log files. There may be multiple watchdog
/// processes running, each performing its own log rotation, so we keep the last
/// N days by modified time. This relies on the watchdog emitting a heartbeat
/// log file from `log_heartbeat`.
fn gc_logs_watchdog(dir: impl AsRef<Path>, keep_days: u64) -> Result<()> {
    let dir = dir.as_ref().to_path_buf();
    let files = watchdog_logs_to_gc(&dir, keep_days)?;

    for file in files {
        try_delete_log(file);
    }

    Ok(())
}

/// Returns a list of watchdog logs ready to be garbage collected
fn watchdog_logs_to_gc(dir: impl AsRef<Path>, keep_days: u64) -> Result<Vec<PathBuf>> {
    let mut files = glob_log_files(&dir, "watchdog.*.log.*")?;
    // Clean up old <= v1.3.4 pattern.
    files.extend(glob_log_files(&dir, "watchdog.*.log")?);
    let threshold = duration_from_days(keep_days);
    let now = SystemTime::now();

    // Defaults to keeping if mtime is not supported by platform or filesystem.
    files.retain(|file| file_older_than(file, now, threshold).unwrap_or(false));
    Ok(files)
}

/// Garbage collects log files from processes that create one log file per
/// invocation, keeping the last (most recent) N files by filename. This relies
/// on the log files having a sortable timestamp in their filename.
fn gc_logs_per_process(dir: impl AsRef<Path>, glob: &str, keep_last: usize) -> Result<()> {
    let mut files = glob_log_files(dir, glob)?;
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
    fn create_log_file(dir: &Path, name: &str, days_old: Option<u64>) -> PathBuf {
        let path = dir.join(name);
        File::create(&path).unwrap();

        if let Some(days) = days_old {
            let mtime = SystemTime::now() - duration_from_days(days);
            File::open(&path).unwrap().set_modified(mtime).unwrap();
        }

        path
    }

    #[test]
    fn identifies_watchdog_logs_to_gc() {
        let days_old_to_keep = 2;
        let dir = tempdir().unwrap();
        let _file_now = create_log_file(dir.path(), "watchdog.now.log.1234-12-12", None);
        let _file_one = create_log_file(
            dir.path(),
            "watchdog.one.log.1234-12-12",
            Some(days_old_to_keep - 1),
        );
        let file_two = create_log_file(
            dir.path(),
            "watchdog.two.log.1234-12-12",
            Some(days_old_to_keep),
        );
        let file_three = create_log_file(
            dir.path(),
            "watchdog.three.log.1234-12-12",
            Some(days_old_to_keep + 1),
        );
        // Old <= v1.3.4 pattern.
        let file_old = create_log_file(dir.path(), "watchdog.old.log", Some(days_old_to_keep + 1));
        let should_be_gced = {
            let mut paths = vec![file_two.clone(), file_three.clone(), file_old.clone()];
            paths.sort();
            paths
        };

        // Ensure that the old files are selected for GC
        let mut to_gc = watchdog_logs_to_gc(dir.path(), days_old_to_keep).unwrap();
        to_gc.sort();
        assert_eq!(to_gc, should_be_gced);

        // Simulate the old files being GCed
        std::fs::remove_file(file_two).unwrap();
        std::fs::remove_file(file_three).unwrap();
        std::fs::remove_file(file_old).unwrap();

        // Ensure that the younger files aren't selected for GC after the old
        // files are deleted.
        let to_gc = watchdog_logs_to_gc(dir.path(), days_old_to_keep).unwrap();
        assert_eq!(to_gc, vec![] as Vec<PathBuf>);
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
            .map(|filename| create_log_file(dir.path(), filename, Some(keep_last - 1)))
            .collect();
        assert_eq!(files.len(), filenames.len());

        assert!(watchdog_logs_to_gc(dir.path(), keep_last)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn test_gc_logs_per_process_removes_old_files() {
        let glob = "services.*.log";
        let keep_days = 2;
        let dir = tempdir().unwrap();
        let files: Vec<PathBuf> = (1..=keep_days * 2)
            .map(|i| create_log_file(dir.path(), &format!("services.{i}.log"), None))
            .collect();

        gc_logs_per_process(dir.path(), glob, keep_days).unwrap();
        assert!(!files[0].exists());
        assert!(!files[1].exists());
        assert!(files[2].exists());
        assert!(files[3].exists());

        // Keeps the same files on second run.
        gc_logs_per_process(dir.path(), glob, keep_days).unwrap();
        assert!(files[2].exists());
        assert!(files[3].exists());
    }

    #[test]
    fn test_gc_logs_per_process_ignores_other_files() {
        let glob = "services.*.log";
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
            .map(|filename| create_log_file(dir.path(), filename, None))
            .collect();
        assert_eq!(files.len(), filenames.len());

        gc_logs_per_process(dir.path(), glob, keep_days).unwrap();
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

        let old_file = create_log_file(dir.path(), "old.log", Some(threshold_days + 1));
        assert!(file_older_than(&old_file, now, threshold_dur).unwrap());

        let new_file = create_log_file(dir.path(), "new.log", Some(threshold_days - 1));
        assert!(!file_older_than(&new_file, now, threshold_dur).unwrap());
    }
}
