use tracing::debug;

use crate::hub::EventsHub;

/// Flushes the configured events client when dropped.
#[derive(Debug)]
pub struct EventsGuard {
    hub: EventsHub,
}

impl EventsGuard {
    pub fn new() -> Self {
        Self {
            hub: EventsHub::global().clone(),
        }
    }
}

impl Default for EventsGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for EventsGuard {
    fn drop(&mut self) {
        if let Err(err) = self.hub.flush(true) {
            debug!(error = %err, "Failed to flush canonical events on guard drop");
        }
    }
}
