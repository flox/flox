//! Hidden internal subcommand that flushes the telemetry buffer.
//!
//! Invoked as a detached background child by `main.rs` when telemetry is due
//! at command exit. Because the parent exits immediately after spawning this
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

        // The client was installed by `FloxArgs::handle`. `try_flush` takes a
        // non-blocking lock, so a concurrent child (from a rapid `cd`) exits
        // early rather than queuing a second network call.
        let v2_result = EventsHub::global().try_flush(force);

        debug!(
            v2_outcome = ?flush_outcome(&v2_result),
            "send-telemetry flush complete"
        );

        v2_result.map(|_| ())
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
}
