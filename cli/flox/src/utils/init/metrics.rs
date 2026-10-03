use std::fs;
use std::path::Path;

use anyhow::Result;
use fslock::LockFile;
use indoc::indoc;
use tracing::debug;

use crate::utils::message;
use crate::utils::metrics::{METRICS_LOCK_FILE_NAME, METRICS_UUID_FILE_NAME};

/// The notice that Flox prints when it creates the telemetry device ID.
///
/// Keep this list in step with `disable_metrics` in `flox-config(1)` and
/// with https://flox.dev/docs/concepts/data-collection.
const TELEMETRY_NOTICE: &str = indoc! {"
    Flox collects telemetry and sends it to Flox and to Sentry (sentry.io).
    Telemetry contains:

      - a random device ID, a random ID for each run, and the ID in .flox/env.json
      - your FloxHub account ID, when you are signed in or use a FloxHub token
      - whether you use a FloxHub login, a FloxHub token or neither
      - each command's name, time, exit code, duration and error category
      - the time of each prompt in an activated shell, from Flox's prompt hook
      - the Flox version, your OS and kernel versions, shell and CPU architecture
      - whether Flox runs in CI, VS Code or Imageless Kubernetes, and which AI
        coding tool runs it
      - the value of FLOX_INVOCATION_SOURCE, when it is set
      - the search terms you type, and the command names you search for
      - environment names, with the owner of each FloxHub environment
      - each environment's type, package count, generation and schema versions
      - for each activation: its mode, shell and invocation type, whether it
        starts services, and whether the environment includes other environments
      - the packages you install, upgrade or uninstall, as written in your command
        or manifest, and whether each one succeeded
      - the old and new version of each package you upgrade
      - each build's type, outcome, duration, error category and lockfile hash
      - error reports, with your hostname, file paths, recent Flox log messages
        and the output of your environment's hook.on-deactivate script
      - performance traces, with your hostname, file paths, FloxHub URL, search
        terms, package names and environment names

    Flox links the device ID to every FloxHub account that signs in on this device.
    Flox and Sentry receive your IP address when Flox sends telemetry.

    To turn telemetry off, do one of the following:

      add 'export FLOX_DISABLE_METRICS=true' to your shell profile
      run 'flox config --set disable_metrics true'

    For every field, how long Flox keeps it, and what turning telemetry off
    does not stop, see https://flox.dev/docs/concepts/data-collection

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

    message::plain(TELEMETRY_NOTICE);

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
        let first_line = TELEMETRY_NOTICE.lines().next().unwrap_or_default();
        assert!(first_line.contains("to Flox"), "{first_line:?}");
        assert!(first_line.contains("Sentry"), "{first_line:?}");
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
