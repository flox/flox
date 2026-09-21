use sentry::configure_scope;

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

/// Record operation context independently of UI visibility, without creating an error event.
pub(crate) fn command_started(command: &str) {
    tracing::info!(command, "Command started");
}

pub(crate) fn command_finished(
    command: &str,
    exit_code: u8,
    error_kind: Option<&str>,
    error: Option<&anyhow::Error>,
) {
    let error = error.map(|error| format!("{error:#}"));
    tracing::info!(command, exit_code, error_kind, error, "Command finished");
}
