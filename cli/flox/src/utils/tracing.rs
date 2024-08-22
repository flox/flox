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
