use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result};
use flox_config::Config;
use fslock::LockFile;
use indoc::formatdoc;
use tracing::debug;
use uuid::Uuid;

use crate::utils::message;

/// Buffer of the legacy telemetry stream, in the cache dir.
pub const METRICS_EVENTS_FILE_NAME: &str = "metrics-events-v2.json";
/// Device ID of this installation, in the data dir.
pub const METRICS_UUID_FILE_NAME: &str = "metrics-uuid";
/// Lock guarding the device ID and the legacy buffer, in the cache dir.
pub const METRICS_LOCK_FILE_NAME: &str = "metrics-lock";

/// Initializes the telemetry for the current installation by creating a new metrics uuid
///
/// If a metrics-uuid file is present, assume telemetry is already set up.
pub fn init_telemetry_uuid(data_dir: impl AsRef<Path>, cache_dir: impl AsRef<Path>) -> Result<()> {
    fs::create_dir_all(&data_dir)?;
    fs::create_dir_all(&cache_dir)?;

    // set a lock to avoid initializing telemetry multiple times from concurrent processes
    // the lock is released when the `metrics_lock` is dropped.
    let mut metrics_lock = LockFile::open(&cache_dir.as_ref().join(METRICS_LOCK_FILE_NAME))?;
    metrics_lock.lock()?;

    let uuid_path = data_dir.as_ref().join(METRICS_UUID_FILE_NAME);

    // we already have a uuid, so lets use that
    if uuid_path.exists() {
        return Ok(());
    }

    debug!("Metrics UUID not found, creating new user");

    // Create new user uuid
    let telemetry_uuid = uuid::Uuid::new_v4();

    debug!("Created new telemetry UUID: {}", telemetry_uuid);

    let notice = formatdoc! {"
        Flox collects basic usage metrics in order to improve the user experience.

        Flox includes a record of the subcommand invoked along with a unique token.
        It does not collect any personal information.

        The collection of metrics can be disabled in the following ways:

          environment: FLOX_DISABLE_METRICS=true
            user-wide: flox config --set disable_metrics true
          system-wide: update /etc/flox.toml as described in flox-config(1)

        This is a one-time notice.

        "};

    message::plain(notice);

    fs::write(uuid_path, telemetry_uuid.to_string())?;
    Ok(())
}

/// Delete the buffer of the retired legacy telemetry stream, if any.
///
/// Best-effort and non-blocking: an older `flox` on the same machine can
/// still hold the lock while it writes or sends that buffer, so contention
/// leaves the file for a later run instead of stalling this one. Creates
/// nothing when there is no buffer.
pub fn remove_legacy_metrics_buffer(cache_dir: &Path) {
    // An empty or relative cache dir resolves against the current directory.
    if !cache_dir.is_absolute() {
        return;
    }
    let buffer_path = cache_dir.join(METRICS_EVENTS_FILE_NAME);
    if fs::symlink_metadata(&buffer_path).is_err() {
        return;
    }

    let mut metrics_lock = match LockFile::open(&cache_dir.join(METRICS_LOCK_FILE_NAME)) {
        Ok(lock) => lock,
        Err(err) => {
            debug!(error = %err, "Could not open the metrics lock; keeping the legacy buffer");
            return;
        },
    };
    match metrics_lock.try_lock() {
        Ok(true) => {},
        Ok(false) => {
            debug!("Metrics lock held by another process; keeping the legacy buffer");
            return;
        },
        Err(err) => {
            debug!(error = %err, "Could not lock the metrics lock; keeping the legacy buffer");
            return;
        },
    }

    match fs::remove_file(&buffer_path) {
        Ok(()) => debug!(path = %buffer_path.display(), "Removed the legacy metrics buffer"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {},
        Err(err) => debug!(error = %err, "Could not remove the legacy metrics buffer"),
    }
}

/// Read the device ID that [`init_telemetry_uuid`] created.
pub(crate) fn read_metrics_uuid(config: &Config) -> Result<Uuid> {
    let uuid_path = config.flox.data_dir.join(METRICS_UUID_FILE_NAME);

    let mut uuid_str = String::new();
    File::open(&uuid_path)
        .and_then(|mut f| f.read_to_string(&mut uuid_str))
        .with_context(|| {
            format!(
                "Could not read the metrics UUID of this installation in {}",
                uuid_path.display()
            )
        })?;
    Uuid::try_parse(uuid_str.trim()).with_context(|| {
        format!(
            "Could not parse the metrics UUID of this installation in {}",
            uuid_path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use flox_config::FloxConfig;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_init_telemetry() {
        let tempdir = TempDir::new().unwrap();
        let uuid_file_path = tempdir.path().join("data").join(METRICS_UUID_FILE_NAME);
        init_telemetry_uuid(tempdir.path().join("data"), tempdir.path().join("cache")).unwrap();
        assert!(uuid_file_path.exists());

        let uuid_str = std::fs::read_to_string(uuid_file_path).unwrap();
        eprintln!("uuid: {uuid_str}");
        uuid::Uuid::try_parse(&uuid_str).expect("parses uuid");
    }

    fn config_with_data_dir(data_dir: &Path) -> Config {
        Config {
            flox: FloxConfig {
                data_dir: data_dir.to_path_buf(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn read_metrics_uuid_reads_the_initialized_uuid() {
        let tempdir = TempDir::new().unwrap();
        let data_dir = tempdir.path().join("data");
        init_telemetry_uuid(&data_dir, tempdir.path().join("cache")).unwrap();
        let written = fs::read_to_string(data_dir.join(METRICS_UUID_FILE_NAME)).unwrap();

        let uuid = read_metrics_uuid(&config_with_data_dir(&data_dir)).unwrap();

        assert_eq!(uuid.to_string(), written);
    }

    /// The buffer goes; the device ID and the lock stay.
    #[test]
    fn remove_legacy_metrics_buffer_keeps_device_id_and_lock() {
        let tempdir = TempDir::new().unwrap();
        let data_dir = tempdir.path().join("data");
        let cache_dir = tempdir.path().join("cache");
        init_telemetry_uuid(&data_dir, &cache_dir).unwrap();
        fs::write(cache_dir.join(METRICS_EVENTS_FILE_NAME), "{}\n").unwrap();

        remove_legacy_metrics_buffer(&cache_dir);

        assert!(!cache_dir.join(METRICS_EVENTS_FILE_NAME).exists());
        assert!(cache_dir.join(METRICS_LOCK_FILE_NAME).exists());
        assert!(data_dir.join(METRICS_UUID_FILE_NAME).exists());
    }

    /// Opted-out installations have no metrics files; don't create any.
    #[test]
    fn remove_legacy_metrics_buffer_without_buffer_creates_nothing() {
        let tempdir = TempDir::new().unwrap();

        remove_legacy_metrics_buffer(tempdir.path());

        assert_eq!(fs::read_dir(tempdir.path()).unwrap().count(), 0);
    }

    /// An older `flox` holding the lock must not stall this invocation.
    #[test]
    fn remove_legacy_metrics_buffer_skips_while_lock_is_held() {
        let tempdir = TempDir::new().unwrap();
        let buffer_path = tempdir.path().join(METRICS_EVENTS_FILE_NAME);
        fs::write(&buffer_path, "{}\n").unwrap();
        let mut held = LockFile::open(&tempdir.path().join(METRICS_LOCK_FILE_NAME)).unwrap();
        held.lock().unwrap();

        remove_legacy_metrics_buffer(tempdir.path());

        assert!(buffer_path.exists());
    }

    /// The error names the file, so a user can find and remove it.
    #[test]
    fn read_metrics_uuid_error_names_the_unparseable_file() {
        let tempdir = TempDir::new().unwrap();
        let uuid_path = tempdir.path().join(METRICS_UUID_FILE_NAME);
        fs::write(&uuid_path, "").unwrap();

        let err = read_metrics_uuid(&config_with_data_dir(tempdir.path())).unwrap_err();

        assert_eq!(
            err.to_string(),
            format!(
                "Could not parse the metrics UUID of this installation in {}",
                uuid_path.display()
            )
        );
    }
}
