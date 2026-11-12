use std::fs;
use std::path::Path;

use anyhow::Result;
use flox_core::open_lock_file;
use indoc::indoc;
use tracing::debug;

use crate::utils::message;
use crate::utils::metrics::{METRICS_LOCK_FILE_NAME, METRICS_UUID_FILE_NAME};

/// The notice that Flox prints when it creates the telemetry device ID.
///
/// Keep this text in step with the Summary of
/// https://flox.dev/docs/concepts/data-collection and with `disable_metrics`
/// in `flox-config(1)`.
const TELEMETRY_NOTICE: &str = indoc! {"
    Flox collects telemetry to improve Flox. It sends usage events and error
    reports (to Flox and to Sentry) with a random device ID, which Flox links to
    your FloxHub account when you sign in. FloxHub logs requests the CLI sends it
    with your IP address. Extensions can collect and send their own telemetry.

    To turn telemetry off, run 'flox config --set disable_metrics true'
    or add 'export FLOX_DISABLE_METRICS=true' to your shell profile.

    What Flox collects: https://flox.dev/docs/concepts/data-collection

    This is a one-time notice.

"};

/// Initializes the telemetry for the current installation by creating a new metrics uuid
///
/// If a metrics-uuid file is present, assume telemetry is already set up.
pub fn init_telemetry_uuid(data_dir: impl AsRef<Path>, cache_dir: impl AsRef<Path>) -> Result<()> {
    fs::create_dir_all(&data_dir)?;
    fs::create_dir_all(&cache_dir)?;

    // set a lock to avoid initializing telemetry multiple times from concurrent processes
    // the lock is released when the `metrics_lock` is dropped.
    let metrics_lock = open_lock_file(cache_dir.as_ref().join(METRICS_LOCK_FILE_NAME))?;
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

    // The UUID records that this notice was shown, so verbosity and logging
    // filters must not suppress it.
    message::unconditional(TELEMETRY_NOTICE);

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

    #[test]
    fn telemetry_notice_links_docs_and_names_opt_outs() {
        assert!(TELEMETRY_NOTICE.contains("https://flox.dev/docs/concepts/data-collection"));
        assert!(TELEMETRY_NOTICE.contains("flox config --set disable_metrics true"));
        // A bare assignment in a shell profile is not exported to `flox`.
        assert!(TELEMETRY_NOTICE.contains("'export FLOX_DISABLE_METRICS=true'"));
    }

    #[test]
    fn telemetry_notice_names_both_destinations() {
        assert!(TELEMETRY_NOTICE.contains("to Flox and to Sentry"));
    }

    #[test]
    fn telemetry_notice_fits_80_columns() {
        for line in TELEMETRY_NOTICE.lines() {
            assert!(
                line.chars().count() <= 80,
                "notice line is longer than 80 columns: {line:?}"
            );
        }
    }

    #[test]
    fn telemetry_notice_avoids_retracted_claims() {
        let notice = TELEMETRY_NOTICE.to_lowercase();
        // Only `true` and `false` parse as FLOX_DISABLE_METRICS values.
        for phrase in ["personal information", "basic", "anonym", "=1"] {
            assert!(
                !notice.contains(phrase),
                "notice contains {phrase:?}: {TELEMETRY_NOTICE}"
            );
        }
    }
}
