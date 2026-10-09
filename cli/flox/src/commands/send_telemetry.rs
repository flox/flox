//! Hidden internal subcommand that flushes both telemetry pipelines.
//!
//! Invoked as a detached background child when telemetry is due at command
//! exit (`main.rs`) or just before `activate` and `develop` replace the
//! process with a shell. Because the parent never waits for this child,
//! network I/O here does not block the user's shell prompt.
//!
//! The handler intentionally does NOT record `cli.command_run` or
//! `cli.command_completed` events — doing so would re-arm the buffer it
//! just drained, causing every prompt to re-flush forever. The exclusion
//! is enforced at the recording sites via `is_telemetry_flush_command`.

use std::path::PathBuf;

use anyhow::Result;
use bpaf::Bpaf;
use flox_config::Config;
use flox_events::{EventsHub, LifecycleFields};
use flox_rust_sdk::flox::Flox;
use tracing::debug;

use super::is_detached_side_effect_command;
use crate::utils::detached::{self, DetachedCommand, LogFile};
use crate::utils::metrics::Hub;

#[derive(Bpaf, Clone, Debug)]
pub struct SendTelemetry {
    /// Flush even if the buffer has not yet expired (for testing)
    #[bpaf(long, hide)]
    pub force: bool,
}

/// Outcome of a single pipeline flush attempt.
#[derive(Debug, PartialEq)]
pub enum FlushOutcome {
    /// Lock was already held by another flusher; buffer not touched.
    LockTaken,
    /// Flush ran (buffer may or may not have been non-empty).
    Flushed,
}

impl SendTelemetry {
    pub async fn handle(self, config: flox_config::Config, _flox: Flox) -> Result<()> {
        // Defense-in-depth: the parent skips spawning when metrics are
        // disabled, but re-check here in case the child is invoked directly
        // or the config changed between parent and child startup.
        if config.flox.disable_metrics {
            debug!("send-telemetry: disable_metrics is true; nothing to flush");
            return Ok(());
        }

        let force = self.force
            || std::env::var("_FLOX_FORCE_FLUSH_METRICS")
                .unwrap_or_default()
                .parse()
                .unwrap_or(false);

        // Flush both pipelines independently. A down endpoint on one must not
        // starve the other, so each is attempted and its result captured before
        // any error is surfaced. Both clients were installed by
        // `FloxArgs::handle`. `try_flush` takes a non-blocking lock, so a
        // concurrent child (from a rapid `cd`) exits early rather than queuing a
        // second network call.
        let v2_result = EventsHub::global().try_flush(force);
        let legacy_result = Hub::global().try_flush_metrics(force);

        debug!(
            v2_outcome = ?flush_outcome(&v2_result),
            legacy_outcome = ?flush_outcome(&legacy_result),
            "send-telemetry flush complete"
        );

        // Surface the errors only after both pipelines have been attempted. If
        // both failed, the legacy error is chained as context on the v2 error so
        // neither is lost.
        match (v2_result, legacy_result) {
            (Ok(_), Ok(_)) => Ok(()),
            (Err(v2_err), Ok(_)) => Err(v2_err),
            (Ok(_), Err(legacy_err)) => Err(legacy_err),
            (Err(v2_err), Err(legacy_err)) => {
                Err(v2_err.context(format!("legacy metrics flush also failed: {legacy_err:#}")))
            },
        }
    }
}

/// Whether either telemetry pipeline has buffered events due for sending, or
/// `_FLOX_FORCE_FLUSH_METRICS` asks for a send regardless.
fn telemetry_flush_due() -> bool {
    if flox_events::force_flush_requested() {
        return true;
    }

    // If the advisory check fails, let the background sender try. Checking one
    // pipeline must not prevent the other from delivering its buffered events.
    EventsHub::global().is_flush_due().unwrap_or_else(|err| {
        debug!(error = %err, "Failed to check v2 events expiry");
        true
    }) || Hub::global().is_flush_due().unwrap_or_else(|err| {
        debug!(error = %err, "Failed to check metrics expiry");
        true
    })
}

/// Spawn a detached `send-telemetry` child when telemetry is due, so that no
/// network I/O happens in the calling process.
///
/// This is the only way telemetry gets sent: the caller never sends in-process,
/// whether it is `main` at command exit or `activate` and `develop` just before
/// they `exec` into a shell. The child is its own session leader, so it outlives
/// an `exec` of the caller just as it outlives the caller exiting.
///
/// Skipped when:
/// - metrics are disabled (no data to send);
/// - neither buffer is due and flushing was not explicitly forced;
/// - `subcommand` is itself a detached side-effect command (prevents a
///   fork-bomb and stops `send-telemetry` from re-spawning itself);
/// - `_FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS=1` (CI escape hatch).
///
/// The child logs to a single rolling file; see [`LogFile::Rolling`].
pub fn spawn_if_due(config: &Config, subcommand: &str) {
    spawn_if_due_with(config, subcommand, None);
}

/// [`spawn_if_due`] with the child's executable overridable, because
/// [`DetachedCommand::spawn`] refuses to guess it in tests. Returns whether a
/// spawn was attempted.
fn spawn_if_due_with(config: &Config, subcommand: &str, self_executable: Option<PathBuf>) -> bool {
    if config.flox.disable_metrics
        || is_detached_side_effect_command(subcommand)
        || detached::bg_side_effects_disabled()
        || !telemetry_flush_due()
    {
        return false;
    }

    let log_dir = config.flox.cache_dir.join("log");
    let args = [String::from("send-telemetry"), String::from("-vv")];
    let spawn_result = DetachedCommand {
        args: &args,
        log_file: LogFile::Rolling(detached::SEND_TELEMETRY_LOG_NAME.to_string()),
        log_dir: &log_dir,
    }
    .spawn(self_executable);
    if let Err(err) = spawn_result {
        debug!(error = %err, "Failed to spawn detached send-telemetry process");
    }
    true
}

/// Record the v2 `cli.command_completed` for a command that is about to
/// `exec` away, then hand any due sending to a detached child.
///
/// `exec` replaces the process, so `main`'s end-of-run emit never runs; the
/// completion is recorded here with `exit_code = 0` for the successful handoff
/// and no duration, since the process becomes the shell rather than completing.
/// `exec` returns only on failure, and by then this record has set the sticky
/// flag, so `main`'s lifecycle emit is a no-op: that rare failure is recorded
/// optimistically as this success.
pub fn record_completed_and_spawn_if_due(config: &Config, subcommand: &'static str) {
    record_completed_and_spawn_if_due_with(config, subcommand, None);
}

fn record_completed_and_spawn_if_due_with(
    config: &Config,
    subcommand: &'static str,
    self_executable: Option<PathBuf>,
) -> bool {
    if let Err(err) =
        EventsHub::global().record_command_completed(subcommand.to_string(), LifecycleFields {
            exit_code: 0,
            duration_ms: None,
            error_kind: None,
        })
    {
        debug!(
            error = %err,
            "Failed to record v2 cli.command_completed event before exec"
        );
    }
    spawn_if_due_with(config, subcommand, self_executable)
}

/// Classify a `try_flush` result for logging: whether the flush ran or the
/// buffer lock was held by another flusher. An `Err` is logged as its message.
fn flush_outcome(result: &Result<bool>) -> String {
    match result {
        Ok(true) => format!("{:?}", FlushOutcome::Flushed),
        Ok(false) => format!("{:?}", FlushOutcome::LockTaken),
        Err(err) => format!("error: {err:#}"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use flox_config::FloxConfig;
    use flox_events::test_helpers::MockEventsConnection;
    use flox_events::{CredentialType, EventsClient, EventsHub, SharedMetadataTemplate};
    use serial_test::serial;
    use uuid::Uuid;

    use super::*;
    use crate::utils::metrics::Connection as _;
    use crate::utils::metrics::tests::TestConnection;

    fn make_template() -> SharedMetadataTemplate {
        SharedMetadataTemplate {
            credential_type: CredentialType::None,
            flox_version: "0.0.0-test".to_string(),
            os_family: None,
            os_family_release: None,
            os: None,
            os_version: None,
            os_platform_version: None,
            shell: None,
            architecture: None,
            empty_flags: vec![],
            invocation_sources: vec![],
        }
    }

    // -------------------------------------------------------------------------
    // Recursion guard: send-telemetry must not record any events of its own.
    // -------------------------------------------------------------------------

    /// `send-telemetry` must not push a command_run event into the buffer it
    /// is about to drain — doing so would cause every prompt to re-flush forever.
    ///
    /// This test verifies that after a flush, the buffer is empty (no events
    /// were added by the flush itself).
    #[test]
    #[serial(global_events_client)]
    fn flush_drains_buffer_without_adding_events() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent = connection.sent_batches();
        let invocation_id = Uuid::new_v4();

        let mut client = EventsClient::new_with_connection(
            Uuid::new_v4(),
            tempdir.path(),
            invocation_id,
            None,
            make_template(),
            connection,
        );

        client
            .record_command_run("list".to_string())
            .expect("record");

        EventsHub::global().set_client(client);

        EventsHub::global()
            .try_flush(true)
            .expect("flush must succeed");

        // Buffer should now be empty — no new events were recorded during the
        // flush.
        let buffer_path = tempdir.path().join(flox_events::EVENTS_BUFFER_FILE_NAME);
        let contents = std::fs::read_to_string(&buffer_path).unwrap_or_default();
        assert!(
            contents.trim().is_empty(),
            "buffer must be empty after flush, but got: {contents}"
        );

        // Exactly one event was sent (the one we seeded, not a new one).
        let batches = sent.lock().unwrap().clone();
        let events: Vec<_> = batches.into_iter().flatten().collect();
        assert_eq!(events.len(), 1, "exactly one seeded event must be sent");

        EventsHub::global().clear_client();
    }

    // -------------------------------------------------------------------------
    // Scheduling: sending is always delegated to the detached child.
    // -------------------------------------------------------------------------

    /// An executable that does not exist: `spawn` opens the child's log file
    /// before launching, so the log's presence shows a spawn was attempted
    /// without running anything.
    const NO_SUCH_EXECUTABLE: &str = "/nonexistent/flox";

    /// Env that lets a spawn through: neither the CI escape hatch nor a forced
    /// flush, so only buffer age decides.
    const GATES_OPEN: [(&str, Option<&str>); 2] = [
        ("_FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS", None),
        ("_FLOX_FORCE_FLUSH_METRICS", None),
    ];

    #[allow(deprecated)]
    fn test_config(dir: &Path, disable_metrics: bool) -> Config {
        Config {
            flox: FloxConfig {
                cache_dir: dir.join("cache"),
                data_dir: dir.join("data"),
                disable_metrics,
                ..FloxConfig::default()
            },
            features: None,
        }
    }

    /// Install a global client over `dir/data` and return the batches its
    /// connection receives. Until a first send succeeds, the buffer is due as
    /// soon as it holds an event; `first_send_done` simulates a later run.
    fn install_client(
        dir: &Path,
        first_send_done: bool,
    ) -> Arc<Mutex<Vec<Vec<flox_events::Event>>>> {
        let data_dir = dir.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        if first_send_done {
            std::fs::write(data_dir.join("events-v2-first-send"), b"").unwrap();
        }
        let connection = MockEventsConnection::default();
        let sent = connection.sent_batches();
        let mut client = EventsClient::new_with_connection(
            Uuid::new_v4(),
            &data_dir,
            Uuid::new_v4(),
            None,
            make_template(),
            connection,
        );
        client
            .record_command_run("activate".to_string())
            .expect("record");
        EventsHub::global().set_client(client);
        sent
    }

    fn spawn_attempted(config: &Config, subcommand: &str) -> bool {
        spawn_if_due_with(config, subcommand, Some(NO_SUCH_EXECUTABLE.into()))
    }

    fn sender_log(config: &Config) -> std::path::PathBuf {
        config
            .flox
            .cache_dir
            .join("log")
            .join(detached::SEND_TELEMETRY_LOG_NAME)
    }

    /// The pre-exec path of `activate` and `develop` must not send in-process
    /// (that would block the shell's start on the network); it records the
    /// completion, leaves the events buffered, and hands them to the child.
    #[test]
    #[serial(global_events_client)]
    fn pre_exec_handoff_spawns_the_sender_instead_of_sending() {
        temp_env::with_vars(GATES_OPEN, || {
            let tempdir = tempfile::tempdir().unwrap();
            let config = test_config(tempdir.path(), false);
            let sent = install_client(tempdir.path(), false);

            let attempted = record_completed_and_spawn_if_due_with(
                &config,
                "activate",
                Some(NO_SUCH_EXECUTABLE.into()),
            );

            assert!(attempted, "a first send is due immediately");
            assert!(sender_log(&config).is_file(), "sender spawn was attempted");
            assert!(
                sent.lock().unwrap().is_empty(),
                "nothing may be sent in-process"
            );
            let buffered =
                std::fs::read_to_string(tempdir.path().join("data/events-v2.json")).unwrap();
            assert!(
                buffered.contains("cli.command_completed"),
                "completion must be buffered for the sender: {buffered}"
            );

            EventsHub::global().clear_client();
        });
    }

    #[test]
    #[serial(global_events_client)]
    fn spawn_waits_for_buffer_expiry_unless_forced() {
        temp_env::with_vars(GATES_OPEN, || {
            let tempdir = tempfile::tempdir().unwrap();
            let config = test_config(tempdir.path(), false);
            install_client(tempdir.path(), true);

            assert!(!spawn_attempted(&config, "activate"));
            assert!(!sender_log(&config).exists());

            temp_env::with_var("_FLOX_FORCE_FLUSH_METRICS", Some("true"), || {
                assert!(spawn_attempted(&config, "activate"));
            });
            assert!(sender_log(&config).is_file());

            EventsHub::global().clear_client();
        });
    }

    /// Each case would spawn (a first send is due) but for one closed gate.
    #[test]
    #[serial(global_events_client)]
    fn spawn_is_skipped_when_a_gate_is_closed() {
        for (disable_metrics, subcommand, bg_disabled) in [
            (true, "activate", None),
            (false, "send-telemetry", None),
            (false, "check-for-upgrades", None),
            (false, "activate", Some("1")),
        ] {
            temp_env::with_vars(
                [
                    ("_FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS", bg_disabled),
                    ("_FLOX_FORCE_FLUSH_METRICS", Some("true")),
                ],
                || {
                    let tempdir = tempfile::tempdir().unwrap();
                    let config = test_config(tempdir.path(), disable_metrics);
                    install_client(tempdir.path(), false);

                    assert!(
                        !spawn_attempted(&config, subcommand),
                        "disable_metrics={disable_metrics} subcommand={subcommand} \
                         bg_disabled={bg_disabled:?}"
                    );
                    assert!(!sender_log(&config).exists());

                    EventsHub::global().clear_client();
                },
            );
        }
    }

    // -------------------------------------------------------------------------
    // Buffer-race safety: second flusher should get LockTaken, not block.
    // -------------------------------------------------------------------------

    /// When another process holds the v2 events buffer lock, try_flush returns
    /// false so the caller exits early without blocking.
    #[test]
    fn v2_events_try_flush_returns_false_when_locked() {
        use flox_events::EventsBuffer;

        let tempdir = tempfile::tempdir().expect("tempdir");
        let data_dir = tempdir.path();

        let connection = MockEventsConnection::default();
        let mut client = EventsClient::new_with_connection(
            Uuid::new_v4(),
            data_dir,
            Uuid::new_v4(),
            None,
            make_template(),
            connection,
        );
        client
            .record_command_run("test".to_string())
            .expect("record");

        // Hold the lock with a blocking read so try_flush sees a contended
        // lock.
        let _held = EventsBuffer::read(data_dir).expect("hold lock");

        // A second client attempting try_flush must return false (lock taken).
        let connection2 = MockEventsConnection::default();
        let mut client2 = EventsClient::new_with_connection(
            Uuid::new_v4(),
            data_dir,
            Uuid::new_v4(),
            None,
            make_template(),
            connection2,
        );
        let flushed = client2.try_flush(true).expect("try_flush must not error");
        assert!(
            !flushed,
            "try_flush must return false when the buffer lock is held"
        );
    }

    /// When another process holds the legacy metrics buffer lock,
    /// try_flush_metrics returns false so the caller exits early.
    #[test]
    fn legacy_metrics_try_flush_returns_false_when_locked() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let cache_dir = tempdir.path();

        // Hold the lock by opening a blocking MetricsBuffer.
        let _held = crate::utils::metrics::MetricsBuffer::blocking_read_for_lock_test(cache_dir)
            .expect("hold lock");

        let connection = TestConnection::default();
        let client = crate::utils::metrics::Client {
            uuid: Uuid::new_v4(),
            metrics_dir: cache_dir.to_path_buf(),
            max_age: time::Duration::ZERO,
            connection: connection.boxed(),
            oldest_buffered_timestamp: None,
        };
        let hub = Hub {
            client: std::sync::Arc::new(std::sync::Mutex::new(Some(client))),
        };
        let result = hub
            .try_flush_metrics(true)
            .expect("try_flush_metrics must not error");
        assert!(!result, "must return false when the lock is held");
    }
}
