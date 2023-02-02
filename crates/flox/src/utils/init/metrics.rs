use std::path::Path;

use anyhow::{Context, Result};
use fslock::LockFile;
use indoc::formatdoc;
use log::{debug, info, trace};
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::utils::dialog::{Confirm, Dialog};
use crate::utils::metrics::{MetricEntry, METRICS_LOCK_FILE_NAME, METRICS_UUID_FILE_NAME};

async fn write_metrics_uuid(uuid_path: &Path, consent: bool) -> Result<()> {
    let mut file = tokio::fs::File::create(&uuid_path).await?;
    if consent {
        let uuid = uuid::Uuid::new_v4();
        file.write_all(uuid.to_string().as_bytes()).await?;
    }
    Ok(())
}

pub async fn init_telemetry_consent(data_dir: &Path, cache_dir: &Path) -> Result<()> {
    tokio::fs::create_dir_all(data_dir).await?;

    if !Dialog::can_prompt() {
        // Can't prompt user now, do it another time
        return Ok(());
    }

    let mut metrics_lock = LockFile::open(&cache_dir.join(METRICS_LOCK_FILE_NAME))?;
    tokio::task::spawn_blocking(move || metrics_lock.lock()).await??;

    let uuid_path = data_dir.join(METRICS_UUID_FILE_NAME);

    match tokio::fs::File::open(&uuid_path).await {
        Ok(_) => return Ok(()),
        Err(err) => match err.kind() {
            std::io::ErrorKind::NotFound => {},
            _ => return Err(err.into()),
        },
    }

    debug!("Metrics UUID not found, determining consent");

    let bash_flox_dirs =
        xdg::BaseDirectories::with_prefix("flox").context("Unable to find config dir")?;
    let bash_user_meta_path = bash_flox_dirs.get_config_home().join("floxUserMeta.json");

    if let Ok(mut file) = tokio::fs::File::open(&bash_user_meta_path).await {
        trace!("Attempting to extract metrics consent value from bash flox");

        let mut bash_user_meta_json = String::new();
        file.read_to_string(&mut bash_user_meta_json).await?;

        let json: serde_json::Value = serde_json::from_str(&bash_user_meta_json)?;

        if let Some(x) = json["floxMetricsConsent"].as_u64() {
            debug!("Using metrics consent value from bash flox");
            write_metrics_uuid(&uuid_path, x == 1).await?;
            return Ok(());
        }
    }

    debug!("Metrics consent not determined, prompting for consent");

    // Generate a real metric to use as an example so they can see the field contents are non-threatening
    let now = OffsetDateTime::now_utc();
    let example_metric_entry = MetricEntry::new(Some("[subcommand]".to_string()), now);

    // Convert it to JSON so we can inject extra bits for the purpose of demonstration,
    // and can print it without `Some()` noising up the output
    let mut example_json = serde_json::to_value(example_metric_entry)
        .context("Failed to JSON-ify example metric entry")?;

    // This isn't actually in the struct (gets added later),
    // and doesn't actually exist unless they say "yes",
    // so we put a placeholder in there to be more fair.
    example_json["uuid"] = "[uuid generated upon consent]".into();
    // The default encoding is disturbing
    example_json["timestamp"] = now.to_string().into();

    // Turn it into a pretty string, if this is too noisy we can make it the normal string
    let example = serde_json::to_string_pretty(&example_json)
        .context("Failed to stringify example metric entry")?;

    let help = formatdoc! {"
        flox collects basic usage metrics in order to improve the user experience,
        including a record of the subcommand invoked along with a unique token.
        It does not collect any personal information.

        An example of one of these metrics looks like this: {example}"};

    let dialog = Dialog {
        message: "Do you consent to the collection of basic usage metrics?",
        help_message: Some(&help),
        typed: Confirm {
            default: Some(false),
        },
    };

    let consent = dialog.prompt().await?;

    if consent {
        write_metrics_uuid(&uuid_path, true).await?;
        info!("\nThank you for helping to improve flox!\n");
    } else {
        let dialog = Dialog {
            message: "Can we log your refusal?",
            help_message: Some("Doing this helps us keep track of our user count, it would just be a single anonymous request"),
            typed: Confirm {
                default: Some(true),
            },
        };

        let _consent_refusal = dialog.prompt().await?;

        // TODO log if Refuse

        write_metrics_uuid(&uuid_path, false).await?;
        info!("\nUnderstood. If you change your mind you can change your election\nat any time with the following command: flox reset-metrics\n");
    }

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
