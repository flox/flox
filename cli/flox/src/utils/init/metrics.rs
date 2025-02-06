use std::fs;
use std::path::Path;

use anyhow::Result;
use fslock::LockFile;
use indoc::formatdoc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::debug;

use crate::utils::message;
use crate::utils::metrics::{METRICS_LOCK_FILE_NAME, METRICS_UUID_FILE_NAME};

/// Determine whether the user has previously opted-out of metrics
/// through the legacy consent dialog.
///
/// Check whether the current uuid file is empty.
///
/// An empty metrics uuid file used to signal telemetry opt-out.
/// We are moving this responsibility to the user configuration file.
/// This detects whether a migration is necessary.
pub async fn telemetry_opt_out_needs_migration(
    data_dir: impl AsRef<Path>,
    cache_dir: impl AsRef<Path>,
) -> Result<bool> {
    tokio::fs::create_dir_all(&data_dir).await?;
    tokio::fs::create_dir_all(&cache_dir).await?;

    let mut metrics_lock = LockFile::open(&cache_dir.as_ref().join(METRICS_LOCK_FILE_NAME))?;
    tokio::task::spawn_blocking(move || metrics_lock.lock()).await??;

    let uuid_path = data_dir.as_ref().join(METRICS_UUID_FILE_NAME);

    match tokio::fs::File::open(&uuid_path).await {
        Ok(mut file) => {
            let mut content = String::new();
            file.read_to_string(&mut content).await?;
            if content.trim().is_empty() {
                return Ok(true);
            }
            Ok(false)
        },
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err.into()),
    }
}

/// Initializes the telemetry for the current installation by creating a new metrics uuid
///
/// If a metrics-uuid file is present, assume telemetry is already set up.
/// Any migration concerning user opt-out should be handled before using [telemetry_denial_need_migration].
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
            user-wide: flox config --set-bool disable_metrics true
          system-wide: update /etc/flox.toml as described in flox-config(1)

        This is a one-time notice.

        "};

    message::plain(notice);

    fs::write(uuid_path, telemetry_uuid.to_string())?;
    Ok(())
}

pub async fn init_uuid(data_dir: &Path) -> Result<uuid::Uuid> {
    tokio::fs::create_dir_all(data_dir).await?;

    let uuid_file_path = data_dir.join("uuid");

    match tokio::fs::File::open(&uuid_file_path).await {
        Ok(mut uuid_file) => {
            debug!("Attempting to read own UUID from file");
            let mut uuid_str = String::new();
            uuid_file.read_to_string(&mut uuid_str).await?;
            Ok(uuid::Uuid::try_parse(&uuid_str)?)
        },
        Err(err) => match err.kind() {
            std::io::ErrorKind::NotFound => {
                debug!("Creating new uuid");
                let uuid = uuid::Uuid::new_v4();
                let mut file = tokio::fs::File::create(&uuid_file_path).await?;
                file.write_all(uuid.to_string().as_bytes()).await?;

                Ok(uuid)
            },
            _ => Err(err.into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    /// An empty metrics-uuid file needs migration
    #[allow(clippy::bool_assert_comparison)]
    #[tokio::test]
    async fn test_telemetry_denial_need_migration_empty_uuid() {
        let tempdir = TempDir::new().unwrap();
        let data_dir = tempdir.path().join("data");

        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::File::create(data_dir.join(METRICS_UUID_FILE_NAME)).unwrap();

        let need_migration =
            telemetry_opt_out_needs_migration(data_dir, tempdir.path().join("cache"))
                .await
                .unwrap();

        assert_eq!(need_migration, true);
    }

    /// An empty data dir (without metrics-uuid file) does not need migration
    #[allow(clippy::bool_assert_comparison)]
    #[tokio::test]
    async fn test_telemetry_denial_need_migration_empty_data() {
        let tempdir = TempDir::new().unwrap();
        let need_migration = telemetry_opt_out_needs_migration(
            tempdir.path().join("data"),
            tempdir.path().join("cache"),
        )
        .await
        .unwrap();

        assert_eq!(need_migration, false);
    }

    /// A non-empty metrics-uuid file does not need migration
    #[allow(clippy::bool_assert_comparison)]
    #[tokio::test]
    async fn test_telemetry_denial_need_migration_filled_uuid() {
        let tempdir = TempDir::new().unwrap();
        let data_dir = tempdir.path().join("data");

        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(
            data_dir.join(METRICS_UUID_FILE_NAME),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();

        let need_migration =
            telemetry_opt_out_needs_migration(data_dir, tempdir.path().join("cache"))
                .await
                .unwrap();

        assert_eq!(need_migration, false);
    }

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
