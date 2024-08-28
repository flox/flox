use std::time::Duration;

use sentry::{configure_scope, Hub};
use tracing::debug;
use tracing::span::EnteredSpan;

const SHUTDOWN_TIMEOUT: Option<Duration> = Some(Duration::from_secs(1));

/// Sets a tracing tag for the current scope.
///
/// In practice these appear to always be rolled up to the root transaction/span
/// but that shouldn't make a difference for searching.
///
/// We use this in place of the following because they aren't searchable in
/// Sentry:
///
/// - `#instrument(fields(foo = "bar"))`
/// - `Span::current().record("foo", "bar")`
///
/// They may support converting fields to tags in future:
///
/// - https://github.com/getsentry/sentry-rust/issues/653
pub fn sentry_set_tag<V: ToString>(key: &str, value: V) {
    configure_scope(|scope| {
        scope.set_tag(key, value);
    });
}

/// Guard against any existing spans being started so that we can ensure that
/// a single span is closed by `sentry_shutdown()`.
pub fn sentry_guard_no_existing_span() {
    if configure_scope(|scope| scope.get_span()).is_some() {
        unreachable!("no existing span should be started");
    }
}

/// Shuts down the Sentry client and flushes data immediately. This should only
/// be used before a function calls `exec()`, which prevents the normal
/// destructors from flushing spans.
///
/// `span` must be the only span active. `sentry_guard_no_existing_span()`
/// should be used before starting/entering `span` to ensure that is true.
///
/// If any other spans are active, which we have no way of checking for, then no
/// data will be flushed to Sentry.
pub fn sentry_shutdown(current_span: EnteredSpan) {
    debug!("closing span");
    current_span.exit();

    if let Some(client) = Hub::main().client() {
        debug!("closing sentry client");
        // close appears to always return `false`, even when data is sent.
        client.close(SHUTDOWN_TIMEOUT);
    }
}
