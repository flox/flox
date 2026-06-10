use std::fmt::{Debug, Formatter};
use std::time::Duration as TimeoutDuration;

use anyhow::{Context, Result};
use tracing::debug;

use crate::Event;

/// Timeout used for network operations that run after the main command has
/// completed.
pub const TRAILING_NETWORK_CALL_TIMEOUT: TimeoutDuration = TimeoutDuration::from_secs(2);

/// A connection to a canonical events backend.
pub trait EventsConnection: Debug + Send + Sync {
    /// Send events to the backend defined by this connection.
    fn send(&mut self, events: Vec<&Event>) -> Result<()>;

    /// Box this connection as a trait object.
    fn boxed(self) -> Box<dyn EventsConnection>
    where
        Self: Sized + 'static,
    {
        Box::new(self)
    }
}

/// Blocking HTTP connection for canonical CLI events.
#[derive(Clone)]
pub struct CanonicalEventsConnection {
    pub timeout: TimeoutDuration,
    pub(crate) endpoint_url: String,
    pub(crate) api_key: String,
}

impl Debug for CanonicalEventsConnection {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanonicalEventsConnection")
            .field("timeout", &self.timeout)
            .field("endpoint_url", &self.endpoint_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl CanonicalEventsConnection {
    pub fn new(endpoint_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            timeout: TRAILING_NETWORK_CALL_TIMEOUT,
            endpoint_url: endpoint_url.into(),
            api_key: api_key.into(),
        }
    }

    pub fn serialize_events(events: &[&Event]) -> Result<String> {
        serde_json::to_string(events).context("Could not serialize canonical events")
    }
}

impl EventsConnection for CanonicalEventsConnection {
    fn send(&mut self, events: Vec<&Event>) -> Result<()> {
        let event_count = events.len();
        let body = Self::serialize_events(&events)?;
        debug!(
            event_count,
            endpoint_url = %self.endpoint_url,
            "Sending canonical events"
        );

        let thread_timeout = self.timeout;
        let thread_endpoint_url = self.endpoint_url.clone();
        let thread_api_key = self.api_key.clone();

        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = (|| {
                let client = reqwest::blocking::ClientBuilder::new()
                    .timeout(thread_timeout)
                    .build()
                    .context("Could not build canonical events HTTP client")?;
                client
                    .put(thread_endpoint_url)
                    .header("content-type", "application/json")
                    .header("x-api-key", thread_api_key)
                    .header("user-agent", "flox-cli")
                    .body(body)
                    .send()
                    .context("Could not send canonical events")
            })();
            let _ = sender.send(result);
        });

        let response = match receiver.recv_timeout(self.timeout) {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => {
                debug!(error = %err, "Canonical events request failed");
                return Err(err);
            },
            Err(err) => {
                let err = anyhow::anyhow!(err).context("canonical events api request");
                debug!(error = %err, "Canonical events request timed out");
                return Err(err);
            },
        };

        debug!(
            status = %response.status(),
            event_count,
            "Canonical events sent"
        );
        Ok(())
    }
}

#[cfg(any(test, feature = "tests"))]
mod mock {
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, bail};

    use super::{Debug, Event, EventsConnection};

    #[derive(Debug, Clone, Default)]
    pub struct MockEventsConnection {
        sent_batches: Arc<Mutex<Vec<Vec<Event>>>>,
        failures_remaining: Arc<Mutex<usize>>,
    }

    impl MockEventsConnection {
        pub fn sent_batches(&self) -> Arc<Mutex<Vec<Vec<Event>>>> {
            self.sent_batches.clone()
        }

        pub fn fail_next_send(&self) {
            let mut failures = self
                .failures_remaining
                .lock()
                .expect("mock failures lock poisoned");
            *failures += 1;
        }
    }

    impl EventsConnection for MockEventsConnection {
        fn send(&mut self, events: Vec<&Event>) -> Result<()> {
            let mut failures = self
                .failures_remaining
                .lock()
                .expect("mock failures lock poisoned");
            if *failures > 0 {
                *failures -= 1;
                bail!("mock events send failed");
            }
            drop(failures);

            self.sent_batches
                .lock()
                .expect("mock sent batches lock poisoned")
                .push(events.into_iter().cloned().collect());
            Ok(())
        }
    }
}

#[cfg(any(test, feature = "tests"))]
pub use mock::MockEventsConnection;
