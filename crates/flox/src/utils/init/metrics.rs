use std::path::Path;

use anyhow::{Context, Result};
use fslock::LockFile;
use indoc::formatdoc;
use log::{debug, info};
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::utils::metrics::{MetricEntry, METRICS_LOCK_FILE_NAME, METRICS_UUID_FILE_NAME};

/// Check whether the current uuid file is empty
///
/// An empty metrics uuid file used to signal telemtry opt-out.
/// We are moving this responsibility to the user configuration file.
/// This detects whether a migration is necessary.
pub async fn telemetry_denial_need_migration(data_dir: &Path, cache_dir: &Path) -> Result<bool> {
    tokio::fs::create_dir_all(data_dir).await?;

    let mut metrics_lock = LockFile::open(&cache_dir.join(METRICS_LOCK_FILE_NAME))?;
    tokio::task::spawn_blocking(move || metrics_lock.lock()).await??;

    let uuid_path = data_dir.join(METRICS_UUID_FILE_NAME);

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
pub async fn init_telemetry_consent(data_dir: &Path, cache_dir: &Path) -> Result<()> {
    tokio::fs::create_dir_all(data_dir).await?;

    let mut metrics_lock = LockFile::open(&cache_dir.join(METRICS_LOCK_FILE_NAME))?;
    tokio::task::spawn_blocking(move || metrics_lock.lock()).await??;
    let uuid_path = data_dir.join(METRICS_UUID_FILE_NAME);

    // we already have a uuid, so lets use that
    if uuid_path.exists() {
        return Ok(());
    }

    debug!("Metrics UUID not found, creating new user");

    // Generate a real metric to use as an example so they can see the field contents are non-threatening
    let now = OffsetDateTime::now_utc();
    let example_metric_entry = MetricEntry::new(Some("[subcommand]".to_string()), now);

    // Convert it to JSON so we can inject extra bits for the purpose of demonstration,
    // and can print it without `Some()` noising up the output
    let mut example_json = serde_json::to_value(example_metric_entry)
        .context("Failed to JSON-ify example metric entry")?;

    // This isn't actually in the struct (gets added later),
    // so we put a placeholder in there to be more fair.
    example_json["uuid"] = "[uuid generated]".into();
    // The default encoding is disturbing
    example_json["timestamp"] = now.to_string().into();

    // Turn it into a pretty string, if this is too noisy we can make it the normal string
    let example = serde_json::to_string_pretty(&example_json)
        .context("Failed to stringify example metric entry")?;

    let notice = formatdoc! {"
        flox collects basic usage metrics in order to improve the user experience.

        flox includes a record of the subcommand invoked along with a unique token.
        It does not collect any personal information.

        An example of one of these metrics looks like this:

        {example}

        The collection of metrics can be disabled by setting

            FLOX_DISABLE_METRICS=true

        or any of the other methods described in flox(1).
        "};
    info!("{notice}");

    let mut file = tokio::fs::File::create(&uuid_path).await?;
    let uuid = uuid::Uuid::new_v4();
    file.write_all(uuid.to_string().as_bytes()).await?;
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
