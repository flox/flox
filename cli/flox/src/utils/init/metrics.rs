use std::fs;
use std::path::Path;

use anyhow::Result;
use fslock::LockFile;
use indoc::formatdoc;
use tracing::debug;

use crate::utils::message;
use crate::utils::metrics::{METRICS_LOCK_FILE_NAME, METRICS_UUID_FILE_NAME};

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

#[cfg(test)]
mod tests {
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
}
