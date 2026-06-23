use std::fmt::{Debug, Formatter};
use std::time::Duration as TimeoutDuration;

use anyhow::{Context, Result};
use tracing::debug;

use crate::Event;

/// Timeout used for network operations that run after the main command has
/// completed.
pub const TRAILING_NETWORK_CALL_TIMEOUT: TimeoutDuration = TimeoutDuration::from_secs(2);

/// A connection to a v2 events backend.
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

/// Blocking HTTP connection for v2 CLI events.
#[derive(Clone)]
pub struct EventsConnectionV2 {
    pub timeout: TimeoutDuration,
    pub(crate) endpoint_url: String,
    pub(crate) api_key: String,
}

impl Debug for EventsConnectionV2 {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventsConnectionV2")
            .field("timeout", &self.timeout)
            .field("endpoint_url", &self.endpoint_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl EventsConnectionV2 {
    pub fn new(endpoint_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            timeout: TRAILING_NETWORK_CALL_TIMEOUT,
            endpoint_url: endpoint_url.into(),
            api_key: api_key.into(),
        }
    }

    /// Serialize a batch of events as the request body for the ingest
    /// endpoint. The wire contract is **NDJSON** — one JSON object per line.
    ///
    /// The ingest is API Gateway -> Firehose -> S3 -> ClickHouse
    /// `JSONEachRow`: the API Gateway request-mapping template (see
    /// `flox-analytics/terraform/units/ingest/api_gateway.tf`) takes the raw
    /// `$input.body`, appends a single trailing newline, and writes the whole
    /// thing as one Firehose Record. So:
    ///
    /// - 1 event  -> body `{...}`              -> S3 line `{...}\n`        (1 NDJSON line)
    /// - N events -> body `{...}\n{...}\n...`  -> S3 record has N NDJSON lines
    ///
    /// An array body (e.g. `[{...},{...}]`) is the poison shape that stalls
    /// the entire S3Queue behind it — ClickHouse cannot parse a body starting
    /// with `[`. This is the parallel fix to the same bug on the FloxHub side
    /// (floxhub@128dce329).
    pub fn serialize_events(events: &[&Event]) -> Result<String> {
        let mut lines = Vec::with_capacity(events.len());
        for event in events {
            lines.push(serde_json::to_string(event).context("Could not serialize event")?);
        }
        Ok(lines.join("\n"))
    }
}

impl EventsConnection for EventsConnectionV2 {
    fn send(&mut self, events: Vec<&Event>) -> Result<()> {
        let event_count = events.len();
        let body = Self::serialize_events(&events)?;
        debug!(
            event_count,
            endpoint_url = %self.endpoint_url,
            "Sending v2 events"
        );

        let thread_timeout = self.timeout;
        let thread_endpoint_url = self.endpoint_url.clone();
        let thread_api_key = self.api_key.clone();

        // Run the blocking request on a separate thread so the channel
        // `recv_timeout` below — not the reqwest client — bounds how long the
        // send can take. reqwest's own timeout does not cover the `getaddrinfo`
        // call in the system libc, which can block far past the configured
        // timeout when DNS does not respond (for example a local resolver
        // forwarding to the internet while Wi-Fi is disabled). See
        // https://github.com/flox/flox/pull/1769#issuecomment-2260675622. On
        // timeout the thread is left to finish on its own; a short-lived CLI
        // invocation exits and the OS reaps it.
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = (|| {
                let client = reqwest::blocking::ClientBuilder::new()
                    .timeout(thread_timeout)
                    .build()
                    .context("Could not build v2 events HTTP client")?;
                client
                    .put(thread_endpoint_url)
                    .header("content-type", "application/json")
                    .header("x-api-key", thread_api_key)
                    .header("user-agent", "flox-cli")
                    .body(body)
                    .send()
                    .context("Could not send v2 events")
            })();
            let _ = sender.send(result);
        });

        let response = match receiver.recv_timeout(self.timeout) {
            Ok(Ok(response)) => response,
            Ok(Err(err)) => {
                debug!(error = %err, "V2 events request failed");
                return Err(err);
            },
            Err(err) => {
                let err = anyhow::anyhow!(err).context("v2 events api request");
                debug!(error = %err, "V2 events request timed out");
                return Err(err);
            },
        };

        let status = response.status();
        match response.error_for_status() {
            Ok(_) => {
                debug!(status = %status, event_count, "V2 events sent");
                Ok(())
            },
            // Non-2xx: the endpoint rejected the request (e.g. a malformed body
            // or a 4xx/5xx from the ingest gateway). Surface it as an error so
            // the caller leaves the events buffered for a later retry, instead
            // of treating the rejection as a successful send and dropping them.
            Err(err) => {
                debug!(error = %err, %status, "V2 events rejected by endpoint");
                Err(anyhow::Error::new(err)
                    .context("v2 events endpoint returned a non-success status"))
            },
        }
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
