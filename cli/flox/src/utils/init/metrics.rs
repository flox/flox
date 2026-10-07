use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use fslock::LockFile;
use indoc::indoc;
use tracing::debug;

use crate::commands::is_detached_side_effect_command;
use crate::utils::message;
use crate::utils::metrics::{METRICS_LOCK_FILE_NAME, METRICS_UUID_FILE_NAME};

/// Set when [init_telemetry_uuid] deferred telemetry for this process.
static TELEMETRY_DEFERRED: AtomicBool = AtomicBool::new(false);

/// Whether this process found no metrics uuid and could not show the
/// telemetry notice, so it creates no uuid and records no telemetry.
pub fn telemetry_deferred() -> bool {
    TELEMETRY_DEFERRED.load(Ordering::Relaxed)
}

/// Whether the invocation with command line `args` (including `argv[0]`) can
/// show the one-time telemetry notice.
///
/// It cannot when user-facing messages are hidden (`-q` or `RUST_LOG`), in a
/// detached background child (its stderr is a log file), or while answering
/// a shell completion request (stderr output would garble the completion).
pub fn telemetry_notice_visible(args: &[OsString], user_messages_visible: bool) -> bool {
    let detached_child = args
        .get(1)
        .and_then(|arg| arg.to_str())
        .is_some_and(is_detached_side_effect_command);
    // Like bpaf, look for the completion flag only before `--`, and treat it
    // as a completion request only with a numeric revision.
    let completion = args
        .iter()
        .skip(1)
        .take_while(|arg| *arg != "--")
        .filter_map(|arg| arg.to_str()?.strip_prefix("--bpaf-complete-rev="))
        .any(|revision| revision.parse::<usize>().is_ok());
    user_messages_visible && !detached_child && !completion
}

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
      - error reports, with file paths, recent Flox log messages and the output
        of your environment's hook.on-deactivate script
      - performance traces, with file paths, FloxHub URL, search terms, package
        names and environment names

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
///
/// The uuid is created together with the one-time notice, so that nothing is
/// recorded before the user has seen the notice.
/// `notice_visible` is asked only when no uuid exists.
/// If it returns `false`, this creates nothing, marks telemetry as deferred
/// (see [telemetry_deferred]) and returns `false`.
/// Otherwise it returns `true`.
pub fn init_telemetry_uuid(
    data_dir: impl AsRef<Path>,
    cache_dir: impl AsRef<Path>,
    notice_visible: impl Fn() -> bool,
) -> Result<bool> {
    let uuid_path = data_dir.as_ref().join(METRICS_UUID_FILE_NAME);

    // Check before creating any directory or the lock file,
    // so that a deferred invocation leaves nothing behind.
    if !uuid_path.exists() && !notice_visible() {
        return Ok(defer_telemetry());
    }

    fs::create_dir_all(&data_dir)?;
    fs::create_dir_all(&cache_dir)?;

    // set a lock to avoid initializing telemetry multiple times from concurrent processes
    // the lock is released when the `metrics_lock` is dropped.
    let mut metrics_lock = LockFile::open(&cache_dir.as_ref().join(METRICS_LOCK_FILE_NAME))?;
    metrics_lock.lock()?;

    // we already have a uuid, so lets use that
    if uuid_path.exists() {
        return Ok(true);
    }

    // The uuid was removed since the check above, e.g. by 'flox reset-metrics'.
    if !notice_visible() {
        return Ok(defer_telemetry());
    }

    debug!("Metrics UUID not found, creating new user");

    // Create new user uuid
    let telemetry_uuid = uuid::Uuid::new_v4();

    debug!("Created new telemetry UUID: {}", telemetry_uuid);

    message::plain(TELEMETRY_NOTICE);

    fs::write(uuid_path, telemetry_uuid.to_string())?;
    Ok(true)
}

fn defer_telemetry() -> bool {
    debug!("Metrics UUID not found and the telemetry notice is not visible, deferring telemetry");
    TELEMETRY_DEFERRED.store(true, Ordering::Relaxed);
    false
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_init_telemetry() {
        let tempdir = TempDir::new().unwrap();
        let uuid_file_path = tempdir.path().join("data").join(METRICS_UUID_FILE_NAME);
        init_telemetry_uuid(
            tempdir.path().join("data"),
            tempdir.path().join("cache"),
            || true,
        )
        .unwrap();
        assert!(uuid_file_path.exists());

        let uuid_str = std::fs::read_to_string(uuid_file_path).unwrap();
        eprintln!("uuid: {uuid_str}");
        uuid::Uuid::try_parse(&uuid_str).expect("parses uuid");
    }

    /// Without a uuid and without a visible notice, nothing is created:
    /// no data or cache dir, no lock file, no uuid.
    #[test]
    fn hidden_notice_defers_telemetry_and_creates_nothing() {
        let tempdir = TempDir::new().unwrap();
        let data_dir = tempdir.path().join("data");
        let cache_dir = tempdir.path().join("cache");

        let initialized = init_telemetry_uuid(&data_dir, &cache_dir, || false).unwrap();

        assert!(!initialized);
        assert!(telemetry_deferred());
        assert!(!data_dir.exists());
        assert!(!cache_dir.exists());
    }

    /// Once the notice was shown, a quiet invocation keeps using the uuid.
    #[test]
    fn hidden_notice_keeps_existing_uuid() {
        let tempdir = TempDir::new().unwrap();
        let data_dir = tempdir.path().join("data");
        let cache_dir = tempdir.path().join("cache");
        init_telemetry_uuid(&data_dir, &cache_dir, || true).unwrap();
        let uuid_before = fs::read_to_string(data_dir.join(METRICS_UUID_FILE_NAME)).unwrap();

        let initialized = init_telemetry_uuid(&data_dir, &cache_dir, || false).unwrap();

        assert!(initialized);
        let uuid_after = fs::read_to_string(data_dir.join(METRICS_UUID_FILE_NAME)).unwrap();
        assert_eq!(uuid_before, uuid_after);
    }

    fn args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn notice_visible_only_where_user_messages_are_printed() {
        let cases = [
            (args(&["flox", "envs"]), true, true),
            (args(&["flox", "-q", "envs"]), false, false),
            // Detached children write stderr into a log file.
            (args(&["flox", "send-telemetry", "-vv"]), true, false),
            (
                args(&["flox", "check-for-upgrades", "{}", "-vv"]),
                true,
                false,
            ),
            // Only the subcommand position identifies a detached child.
            (args(&["flox", "install", "send-telemetry"]), true, true),
            // Shell completion, as requested by the bash, zsh and fish scripts.
            (args(&["flox", "--bpaf-complete-rev=8", "ac"]), true, false),
            (
                args(&["flox", "--bpaf-complete-rev=7", "flox", "ac"]),
                true,
                false,
            ),
            // After `--` the flag is an argument of the command, not a request.
            (
                args(&["flox", "activate", "--", "echo", "--bpaf-complete-rev=8"]),
                true,
                true,
            ),
            // bpaf ignores a revision that is not a number.
            (
                args(&["flox", "--bpaf-complete-rev=abc", "envs"]),
                true,
                true,
            ),
        ];
        for (args, user_messages_visible, expected) in cases {
            assert_eq!(
                telemetry_notice_visible(&args, user_messages_visible),
                expected,
                "{args:?} with user_messages_visible={user_messages_visible}"
            );
        }
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
