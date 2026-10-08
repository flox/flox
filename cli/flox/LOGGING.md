# CLI output and diagnostics

Use `tracing::{trace, debug, info, warn, error}` for diagnostics, with structured
fields and spans. The console layer is off by default; `FLOX_LOG` overrides
`-v` / `-q` and only changes diagnostic stderr output. `RUST_LOG` does not
configure this CLI logger. Other binaries retain their own logging settings.

Sentry has a separate filter: ERROR creates events, WARN/INFO add breadcrumbs,
and DEBUG/TRACE events are ignored. Existing telemetry opt-out and client/span
configuration remain in effect. Printing a user message does not report it to
Sentry. Explicit command-start and command-outcome INFO diagnostics retain operation
and propagated-error context as breadcrumbs, without adding ERROR events. UI
wording, icons, success banners, and repeated presentation are deliberately not
exported as breadcrumbs. Handled service failures, interactive edit/build recovery,
Factory controlled exits, and invalid activation state record their cause where
propagation ends. Existing SDK operation diagnostics remain in place.

Use `message::` only in the CLI for explicit user communication. Each call
writes independently of diagnostic filtering. A separate UI quiet policy suppresses
routine notices (including advisory warnings); errors and command results remain
visible. `FLOX_LOG` never changes that policy. Keep notices and errors on
stderr, and command results on stdout. Preserve existing presentation when
migrating a call that previously relied on a visible diagnostic; add an explicit
message alongside the diagnostic when both are needed.

CLI startup (`utils/init/output.rs`) constructs stdout and the shared progress-aware
stderr writer, installs the tracing subscriber, and registers message output once.
All production callers, including callbacks and spawned work, use those writers
and their quiet policy. Raw stdout/stderr are used only before initialization.
`logger.rs` only configures diagnostic layers and filters; it neither constructs
nor registers message output. Configuration errors return to startup for reporting.

Unit tests use a `#[cfg(test)]` tracing writer at the final output boundary.
Formatting, quiet handling, stream selection, and newline behavior are shared
with production. `message::test_helpers::test_subscriber_message_only` captures
stderr using the existing `.with_subscriber(...)` or `with_default(...)` pattern;
`capture_output` provides a layer with separate stdout/stderr buffers. Capture
events use a dedicated test-only TRACE target, excluded from console diagnostics
and ignored by Sentry. This level is transport metadata, not message severity.
The subscriber records exact message bytes without adding tracing formatting.
Tests capture spawned futures with `.with_current_subscriber()` and install a
subscriber inside test-owned threads when needed; tracing does not propagate
subscribers automatically. No custom output scopes or SDK output dependencies
are required. Integration tests and isolated terminal probes exercise the real
writers, bypassing the test adapter.

## Errors

Propagate typed errors with context using `Result` / `?`. Do not log an error
and immediately return the same failure. Report once where propagation ends:
the command boundary, task supervisor, or recovery handler. A detached task
reports its own failures if no caller receives them.

Expected validation failures and cancellation do not automatically warrant
ERROR. Use DEBUG/TRACE for retry attempts, WARN for meaningful degradation, and
ERROR for final operational failures. Print the user explanation separately.
Avoid duplicate reports from `#[instrument(err)]` or direct Sentry capture.

This draft preserves existing error text and diagnostic reporting sites. A
broader error-classification and report-once audit remains follow-up work.

## Compatibility audit

- UI output retains formatting, colors/NO_COLOR, multiline messages, stderr
  routing, and best-effort message writes. Command-result writes retain their
  error handling. Help/completion remain stdout; exit codes are unchanged.
- The shared IndicatifWriter suspends progress for a complete formatted write.
  A real-PTY regression covers active and exited-but-undropped progress spans,
  threads, and concurrent UI/diagnostic output. Spawned command work and
  credential/resolve callbacks share the production output without wrappers. Dialog
  blocking workers use their existing terminal renderer; GC and metrics workers
  emit no `message::` output.
- SDK buildenv warnings describe retries or errors already propagated to CLI
  rendering. Registry pruning and missing-upstream metadata warnings are internal
  diagnostics; publish validates repository state and presents its failures.
  Missing Kerberos tickets retain a lazy, once-per-context CLI notice with the
  recovery command. SDK crates gain no UI dependency.
- Default-off diagnostics and FLOX_LOG precedence are intentional changes.
  Quiet mode suppresses routine UI again, but retains essential errors instead
  of reproducing the old INFO-filtering artifact that hid `message::error`.
  Sentry filtering/opt-out and progress-layer policy remain independent.
