use tracing::debug;

use crate::hub::EventsHub;

/// Flushes the configured events client when dropped. Like the legacy
/// `MetricGuard`, the flush only sends once the buffer has expired, unless
/// `_FLOX_FORCE_FLUSH_METRICS` forces an immediate send.
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

/// Whether `_FLOX_FORCE_FLUSH_METRICS` requests an immediate send regardless
/// of buffer expiry — the same test/dev hook the legacy `MetricGuard` honors.
pub fn force_flush_requested() -> bool {
    std::env::var("_FLOX_FORCE_FLUSH_METRICS")
        .unwrap_or_default()
        .parse()
        .unwrap_or(false)
}

impl Drop for EventsGuard {
    fn drop(&mut self) {
        if let Err(err) = self.hub.flush(force_flush_requested()) {
            debug!(error = %err, "Failed to flush v2 events on guard drop");
        }
    }
}
