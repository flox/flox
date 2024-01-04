use std::path::Path;

use anyhow::{Context, Result};
use fslock::LockFile;
use indoc::formatdoc;
use log::{debug, info};
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::utils::metrics::{
    MetricEntry,
    PosthogEvent,
    METRICS_LOCK_FILE_NAME,
    METRICS_UUID_FILE_NAME,
};

/// Determine whether the user has previously opted-out of metrics
/// through the legacy consent dialog.
///
/// Check whether the current uuid file is empty.
///
/// An empty metrics uuid file used to signal telemtry opt-out.
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
pub async fn init_telemetry(data_dir: impl AsRef<Path>, cache_dir: impl AsRef<Path>) -> Result<()> {
    tokio::fs::create_dir_all(&data_dir).await?;
    tokio::fs::create_dir_all(&cache_dir).await?;

    let mut metrics_lock = LockFile::open(&cache_dir.as_ref().join(METRICS_LOCK_FILE_NAME))?;
    tokio::task::spawn_blocking(move || metrics_lock.lock()).await??;
    let uuid_path = data_dir.as_ref().join(METRICS_UUID_FILE_NAME);

    // we already have a uuid, so lets use that
    if uuid_path.exists() {
        return Ok(());
    }

    debug!("Metrics UUID not found, creating new user");

    // Create new user uuid
    let telemetry_uuid = uuid::Uuid::new_v4();

    // Generate a real metric to use as an example so they can see the field contents are non-threatening
    let now = OffsetDateTime::now_utc();
    let example_metric_entry = MetricEntry::new(
        PosthogEvent {
            subcommand: "[subcommand]".to_string().into(),
            extras: Default::default(),
        },
        now,
    );

    // Convert it to JSON so we can inject extra bits for the purpose of demonstration,
    // and can print it without `Some()` noising up the output
    let mut example_json = serde_json::to_value(example_metric_entry)
        .context("Failed to JSON-ify example metric entry")?;

    // This isn't actually in the struct (gets added later),
    // so we put a placeholder in there to be more fair.
    example_json["uuid"] = telemetry_uuid.to_string().into();
    // The default encoding is disturbing
    example_json["timestamp"] = now.to_string().into();

    // Turn it into a pretty string, if this is too noisy we can make it the normal string
    let example = serde_json::to_string_pretty(&example_json)
        .context("Failed to stringify example metric entry")?;

    let notice = formatdoc! {"
        flox collects basic usage metrics in order to improve the user experience.

        flox includes a record of the subcommand invoked along with a unique token.
        It does not collect any personal information.

        Example metric for this invocation:

        {example}

        The collection of metrics can be disabled in the following ways:

          environment: FLOX_DISABLE_METRICS=true
            user-wide: flox config --set-bool disable_metrics true
          system-wide: update /etc/flox.toml as described in flox(1)

        "};
    info!("{notice}");

    let mut file = tokio::fs::File::create(&uuid_path).await?;
    file.write_all(telemetry_uuid.to_string().as_bytes())
        .await?;
    file.flush().await?;
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

    #[tokio::test]
    async fn test_init_telemetry() {
        let tempdir = TempDir::new().unwrap();
        let uuid_file_path = tempdir.path().join("data").join(METRICS_UUID_FILE_NAME);
        init_telemetry(tempdir.path().join("data"), tempdir.path().join("cache"))
            .await
            .unwrap();
        assert!(uuid_file_path.exists());

        let uuid_str = std::fs::read_to_string(uuid_file_path).unwrap();
        eprintln!("uuid: {uuid_str}");
        uuid::Uuid::try_parse(&uuid_str).expect("parses uuid");
    }
}
