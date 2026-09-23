//! Hidden internal subcommand that flushes both telemetry pipelines.
//!
//! Invoked as a detached background child by `main.rs` on every normal
//! command exit. Because the parent exits immediately after spawning this
//! child, network I/O here does not block the user's shell prompt.
//!
//! The handler intentionally does NOT record `cli.command_run` or
//! `cli.command_completed` events — doing so would re-arm the buffer it
//! just drained, causing every prompt to re-flush forever. The exclusion
//! is enforced at the recording sites via `is_telemetry_flush_command`.

use anyhow::Result;
use bpaf::Bpaf;
use flox_events::EventsHub;
use flox_rust_sdk::flox::Flox;
use tracing::debug;

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

        let client = EventsClient::new_with_connection(
            Uuid::new_v4(),
            tempdir.path(),
            invocation_id,
            None,
            make_template(),
            connection,
        );

        // Seed one real event.
        client
            .record_command_run("list".to_string())
            .expect("record");

        EventsHub::global().set_client(client);

        // Flush with force=true.
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
    // Buffer-race safety: second flusher should get LockTaken, not block.
    // -------------------------------------------------------------------------

    /// When another process holds the v2 events buffer lock, try_flush returns
    /// false so the caller exits early without blocking.
    #[test]
    fn v2_events_try_flush_returns_false_when_locked() {
        use flox_events::EventsBuffer;

        let tempdir = tempfile::tempdir().expect("tempdir");
        let data_dir = tempdir.path();

        // Seed one event so there is something to flush.
        let connection = MockEventsConnection::default();
        let client = EventsClient::new_with_connection(
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
