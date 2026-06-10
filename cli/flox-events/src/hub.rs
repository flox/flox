use std::sync::{Arc, LazyLock, Mutex};

use anyhow::Result;
use tracing::debug;

use crate::EventKind;
use crate::client::EventsClient;

static EVENTS_HUB: LazyLock<EventsHub> = LazyLock::new(EventsHub::new);

/// Shared event client holder used by CLI call sites.
#[derive(Debug, Clone)]
pub struct EventsHub {
    client: Arc<Mutex<Option<EventsClient>>>,
}

impl EventsHub {
    pub fn global() -> &'static Self {
        &EVENTS_HUB
    }

    pub fn new() -> Self {
        Self {
            client: Arc::new(Mutex::new(None)),
        }
    }

    pub fn set_client(&self, new_client: EventsClient) -> Option<EventsClient> {
        self.with_client(|client| client.replace(new_client))
    }

    pub fn clear_client(&self) -> Option<EventsClient> {
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
