//! Shared Sentry initialization for flox binaries.

use std::borrow::Cow;

use anyhow::anyhow;
use sentry::{ClientInitGuard, IntoDsn};
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

    let sentry = sentry::init(sentry::ClientOptions {
        dsn: Some(sentry_dsn),

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

        ..Default::default()
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
