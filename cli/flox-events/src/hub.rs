use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::Result;
use tracing::debug;

use crate::client::EventsClient;
use crate::{CliEnvironmentActivatePayload, EnvDetail, EventKind};

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
                debug!("No canonical events client configured, skipping flush");
                Ok(())
            }
        })
    }

    pub fn record_event(&self, kind: EventKind) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                debug!("No canonical events client configured, skipping record");
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
                debug!("No canonical events client configured, skipping command_run record");
                return Ok(());
            };
            client.record_command_run(subcommand)
        })
    }

    /// Record a `cli.command_completed` event for `subcommand`. No-op when
    /// no client is installed. Subsequent calls against the same client
    /// install are no-ops so the dispatcher and the `activate.rs` pre-exec
    /// path cannot race-emit twice for one invocation.
    pub fn record_command_completed(&self, subcommand: String) -> Result<()> {
        if self.completed_recorded.swap(true, Ordering::SeqCst) {
            debug!("command_completed already recorded for this client install, skipping");
            return Ok(());
        }
        self.with_client(|client| {
            let Some(client) = client else {
                debug!("No canonical events client configured, skipping command_completed record");
                return Ok(());
            };
            client.record_command_completed(subcommand)
        })
    }

    /// Record a `cli.environment.activate` event. The caller builds the
    /// payload via [`EventsClient::build_environment_activate_payload`]
    /// (typically through [`EventsHub::build_environment_activate_payload`])
    /// so the call site can populate only the activate extras it knows.
    /// No-op when no client is installed.
    pub fn record_environment_activate(
        &self,
        payload: CliEnvironmentActivatePayload,
    ) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                debug!(
                    "No canonical events client configured, skipping environment.activate record"
                );
                return Ok(());
            };
            client.record_environment_activate(payload)
        })
    }

    /// Build an empty-extras `cli.environment.activate` payload using the
    /// installed client's shared metadata. Returns `None` when no client is
    /// installed (production-dormant branch).
    pub fn build_environment_activate_payload(
        &self,
        env_detail: EnvDetail,
    ) -> Option<CliEnvironmentActivatePayload> {
        self.with_client(|client| {
            client
                .as_ref()
                .map(|c| c.build_environment_activate_payload(env_detail))
        })
    }

    /// Record a `cli.environment.push` event with the supplied env detail.
    /// No-op when no client is installed.
    pub fn record_environment_push(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                debug!("No canonical events client configured, skipping environment.push record");
                return Ok(());
            };
            client.record_environment_push(env_detail)
        })
    }

    /// Record a `cli.environment.pull` event with the supplied env detail.
    /// No-op when no client is installed.
    pub fn record_environment_pull(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                debug!("No canonical events client configured, skipping environment.pull record");
                return Ok(());
            };
            client.record_environment_pull(env_detail)
        })
    }

    fn with_client<T>(&self, f: impl FnOnce(&mut Option<EventsClient>) -> T) -> T {
        let mut client = self
            .client
            .lock()
            .expect("canonical events client mutex panicked on another thread");
        f(&mut client)
    }
}

impl Default for EventsHub {
    fn default() -> Self {
        Self::new()
    }
}
