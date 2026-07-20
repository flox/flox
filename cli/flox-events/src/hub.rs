use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::{Result, bail};
use tracing::{debug, trace};

use crate::client::EventsClient;
use crate::guard::EventsGuard;
use crate::{EventKind, LifecycleFields};

static EVENTS_HUB: LazyLock<EventsHub> = LazyLock::new(EventsHub::new);

/// Shared event client holder used by CLI call sites.
#[derive(Debug, Clone)]
pub struct EventsHub {
    client: Arc<Mutex<Option<EventsClient>>>,
    /// Sticky flag: set the first time `record_command_completed` runs
    /// against this hub, so a second call (e.g. when `activate.rs` emits
    /// before `command.exec()` and the dispatcher would otherwise emit
    /// again after `exec` returns an error) is a no-op. Reset whenever the
    /// installed client is replaced or cleared.
    completed_recorded: Arc<AtomicBool>,
}

impl EventsHub {
    pub fn global() -> &'static Self {
        &EVENTS_HUB
    }

    pub fn new() -> Self {
        Self {
            client: Arc::new(Mutex::new(None)),
            completed_recorded: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_client(&self, new_client: EventsClient) -> Option<EventsClient> {
        self.completed_recorded.store(false, Ordering::SeqCst);
        self.with_client(|client| client.replace(new_client))
    }

    pub fn clear_client(&self) -> Option<EventsClient> {
        self.completed_recorded.store(false, Ordering::SeqCst);
        self.with_client(Option::take)
    }

    pub fn flush(&self, force: bool) -> Result<()> {
        self.with_client(|client| {
            if let Some(client) = client {
                client.flush(force)
            } else {
                trace!("No v2 events client configured, skipping flush");
                Ok(())
            }
        })
    }

    pub fn record_event(&self, kind: EventKind) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping record");
                return Ok(());
            };

            client.record_event(kind)
        })
    }

    /// Record a `cli.command_run` event for `subcommand`. No-op when no
    /// client is installed.
    pub fn record_command_run(&self, subcommand: String) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping command_run record");
                return Ok(());
            };
            client.record_command_run(subcommand)
        })
    }

    /// Record a `cli.command_completed` event carrying the dispatch lifecycle
    /// fields. No-op when no client is installed. The first record per client
    /// install wins, so the dispatcher, the `activate.rs` pre-exec path, and
    /// the interrupt handler cannot double-emit for one invocation.
    pub fn record_command_completed(
        &self,
        subcommand: String,
        lifecycle: LifecycleFields,
    ) -> Result<()> {
        if self.completed_recorded.swap(true, Ordering::SeqCst) {
            debug!("command_completed already recorded for this client install, skipping");
            return Ok(());
        }
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping command_completed record");
                return Ok(());
            };
            client.record_command_completed(subcommand, lifecycle)
        })
    }

    /// Return an [`EventsGuard`] that flushes this hub's client on drop —
    /// the counterpart of the legacy `Hub::try_guard`. Errors if a guard is
    /// already active for this hub, so at most one guard flushes per process.
    ///
    /// The `strong_count` probe is a faithful live-guard count only because
    /// `try_guard` is the sole site that clones an [`EventsHub`] (hence its
    /// `client` `Arc`). Keep it that way if you add hub clones elsewhere, or
    /// the check will report spurious "guard already active" errors.
    pub fn try_guard(&self) -> Result<EventsGuard> {
        if Arc::strong_count(&self.client) > 1 {
            bail!("A guard is already active, there can only be one guard at a time")
        }
        Ok(EventsGuard::from_hub(self.clone()))
    }

    fn with_client<T>(&self, f: impl FnOnce(&mut Option<EventsClient>) -> T) -> T {
        let mut client = self
            .client
            .lock()
            .expect("v2 events client mutex panicked on another thread");
        f(&mut client)
    }
}

impl Default for EventsHub {
    fn default() -> Self {
        Self::new()
    }
}
