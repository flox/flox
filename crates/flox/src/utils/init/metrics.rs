use std::path::Path;

use anyhow::{Context, Result};
use crossterm::tty::IsTty;
use fslock::LockFile;
use indoc::indoc;
use log::{debug, info, trace};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::utils::dialog::{Confirm, Dialog};
use crate::utils::metrics::{METRICS_LOCK_FILE_NAME, METRICS_UUID_FILE_NAME};

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

    if !std::io::stderr().is_tty() || !std::io::stdin().is_tty() {
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

    debug!("Metrics consent not recorded");

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

    trace!("Prompting user for metrics consent");

    let dialog = Dialog {
        message: "Do you consent to the collection of basic usage metrics?",
        help_message: Some(indoc! {"
            flox collects basic usage metrics in order to improve the user experience,
            including a record of the subcommand invoked along with a unique token.
            It does not collect any personal information."}),
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
            debug!("Reading uuid from file");
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
