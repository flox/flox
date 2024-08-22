use std::borrow::Cow;

use anyhow::anyhow;
use flox_rust_sdk::flox::{FLOX_SENTRY_ENV, FLOX_VERSION};
use log::{debug, warn};
use sentry::{ClientInitGuard, IntoDsn};

pub fn init_sentry() -> Option<ClientInitGuard> {
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
        release: Some(Cow::Owned(format!("flox-cli@{}", &*FLOX_VERSION))),

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

    Some(sentry)
}
