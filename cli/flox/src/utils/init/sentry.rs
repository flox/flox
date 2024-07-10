use std::sync::Arc;

use anyhow::anyhow;
use flox_rust_sdk::flox::FLOX_SENTRY_ENV;
use log::{debug, warn};
use sentry::transports::ReqwestHttpTransport;
use sentry::{ClientInitGuard, ClientOptions, IntoDsn, Transport, TransportFactory};

use crate::utils::metrics::build_trailing_reqwest_client;

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

    // TODO: configure user
    // https://docs.sentry.io/platforms/rust/enriching-events/identify-user/
    // sentry::configure_scope(|scope| {
    //     scope.set_user(Some(sentry::User {
    //     ..
    //    }));
    // });

    let sentry = sentry::init(sentry::ClientOptions {
        dsn: Some(sentry_dsn),

        // https://docs.sentry.io/platforms/rust/configuration/releases/
        // TODO: should we maybe just use commit hash
        release: sentry::release_name!(),

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

        transport: Some(Arc::new(TrailingReqwestTransportFactory)),

        ..Default::default()
    });

    Some(sentry)
}

#[derive(Clone)]
struct TrailingReqwestTransportFactory;

impl TransportFactory for TrailingReqwestTransportFactory {
    fn create_transport(&self, options: &ClientOptions) -> Arc<dyn Transport> {
        Arc::new(ReqwestHttpTransport::with_client(
            options,
            build_trailing_reqwest_client().unwrap(),
        ))
    }
}
