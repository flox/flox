//! Shared Sentry initialization for flox binaries.

use std::borrow::Cow;
use std::sync::Arc;

use anyhow::anyhow;
use sentry::{ClientInitGuard, ClientOptions, Integration, IntoDsn};
use tracing::{debug, warn};
use uuid::Uuid;

use crate::vars::{FLOX_SENTRY_ENV, FLOX_VERSION_STRING};

/// Initialize Sentry with the given release name and metrics UUID.
///
/// - `release_name`: The name of the binary (e.g., "flox-cli", "flox-activations::executive")
/// - `metrics_uuid`: The user ID for trace correlation
///
/// Returns None if FLOX_SENTRY_DSN is not set or invalid.
pub fn init_sentry(release_name: &str, metrics_uuid: Uuid) -> Option<ClientInitGuard> {
    let Ok(sentry_dsn) = std::env::var("FLOX_SENTRY_DSN") else {
        debug!("No Sentry DSN set, skipping Sentry initialization");
        return None;
    };
    let sentry_dsn = match sentry_dsn.into_dsn() {
        Ok(Some(dsn)) => {
            debug!("Initializing Sentry with DSN: {dsn}");
            dsn
        },
        Ok(None) => {
            warn!("Sentry DSN is empty, skipping Sentry initialization");
            return None;
        },
        Err(err) => {
            warn!("Invalid Sentry DSN: {}", anyhow!(err));
            return None;
        },
    };

    let sentry_env = (*FLOX_SENTRY_ENV)
        .clone()
        .unwrap_or_else(|| "development".to_string());

    let sentry = sentry::init(ClientOptions {
        dsn: Some(sentry_dsn),
        // Don't set `integrations` here: it would drop `NoServerName`.
        ..client_options(release_name, sentry_env)
    });

    // Configure user for trace correlation
    // https://docs.sentry.io/platforms/rust/enriching-events/identify-user/
    debug!("Configuring Sentry user with metrics UUID");
    sentry::configure_scope(|scope| {
        scope.set_user(Some(sentry::User {
            id: Some(metrics_uuid.to_string()),
            ..Default::default()
        }));
    });

    Some(sentry)
}

/// Client options for every flox binary that reports to Sentry, minus the DSN.
fn client_options(release_name: &str, sentry_env: String) -> ClientOptions {
    ClientOptions {
        // https://docs.sentry.io/platforms/rust/configuration/releases/
        release: Some(Cow::Owned(format!(
            "{}@{}",
            release_name, &*FLOX_VERSION_STRING
        ))),

        // https://docs.sentry.io/platforms/rust/configuration/environments/
        environment: Some(sentry_env.into()),

        // certain personally identifiable information (PII) are added
        // TODO: enable based on environment (e.g. nightly only)
        // https://docs.sentry.io/platforms/rust/configuration/options/#send-default-pii
        send_default_pii: false,

        // Enable debug mode when needed
        debug: false,

        // To set a uniform sample rate
        // https://docs.sentry.io/platforms/rust/performance/
        traces_sample_rate: 1.0,

        integrations: vec![Arc::new(NoServerName)],

        ..Default::default()
    }
}

/// Keeps the machine's hostname out of events and transactions.
///
/// The default `contexts` integration sets `server_name` to the hostname
/// during setup, and the client copies it onto every event and transaction.
/// Leaving the option unset doesn't help because that integration fills it
/// in, and `before_send` never sees transactions.
/// `sentry::init` runs default integrations before custom ones,
/// so clearing the option here undoes the default.
struct NoServerName;

impl Integration for NoServerName {
    fn name(&self) -> &'static str {
        "no-server-name"
    }

    fn setup(&self, options: &mut ClientOptions) {
        options.server_name = None;
    }
}

#[cfg(test)]
mod tests {
    use sentry::protocol::EnvelopeItem;

    use super::*;

    /// Report one error event and one transaction with `options`, and return
    /// the `server_name` of every event and every transaction captured.
    fn reported_server_names(options: ClientOptions) -> (Vec<Option<String>>, Vec<Option<String>>) {
        let envelopes = sentry::test::with_captured_envelopes_options(
            || {
                sentry::capture_message("error", sentry::Level::Error);
                sentry::start_transaction(sentry::TransactionContext::new("transaction", "op"))
                    .finish();
            },
            // The test helper skips the default integrations that
            // `sentry::init` adds, including the one that sets the hostname.
            sentry::apply_defaults(options),
        );

        let mut events = Vec::new();
        let mut transactions = Vec::new();
        for item in envelopes.iter().flat_map(|envelope| envelope.items()) {
            match item {
                EnvelopeItem::Event(event) => {
                    events.push(event.server_name.as_deref().map(String::from))
                },
                EnvelopeItem::Transaction(transaction) => {
                    transactions.push(transaction.server_name.as_deref().map(String::from))
                },
                _ => {},
            }
        }
        (events, transactions)
    }

    #[test]
    fn events_and_transactions_omit_hostname() {
        let options = client_options("flox-test", "test".to_string());

        // Without NoServerName both reports carry the hostname,
        // so the assertion below can't pass vacuously.
        // If this fails after a `sentry` upgrade, check whether the SDK
        // still sets the hostname and whether NoServerName is still needed.
        let (events, transactions) = reported_server_names(ClientOptions {
            integrations: Vec::new(),
            ..options.clone()
        });
        assert!(
            matches!((&events[..], &transactions[..]), ([Some(_)], [Some(_)])),
            "expected one event and one transaction with a hostname, got {events:?} and {transactions:?}"
        );

        assert_eq!(reported_server_names(options), (vec![None], vec![None]));
    }
}
