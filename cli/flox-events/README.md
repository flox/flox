# flox-events: v2 CLI telemetry events

This crate defines the v2 telemetry events the Flox CLI emits and the
self-contained pipeline that buffers and sends them. It is the single
place where the event wire format is defined; everything else in the
repo either constructs events through this crate's types or stays out
of the way.

This document is the contract reference for anyone adding or changing
an event ("metrics message") or a field on one ("metric value"). The
step-by-step procedure lives in the repo skill
`.claude/skills/adding-metrics-events/SKILL.md`; this file explains
the rules that procedure enforces and why they exist.

Telemetry can be disabled entirely with
`flox config --set disable_metrics true` (or `FLOX_DISABLE_METRICS=true`).
When disabled, no client is installed, no `Event` envelope is created,
and nothing is sent — but payload construction at a call site still
happens, since the payload is built as the `record_event` argument.
That is why expensive payload-only work must go behind
`when_client_set` (see Emission mechanics). Telemetry is on by
default with a first-run notice; see `flox config --help` for the
user-facing description of what is collected.

## The two streams

The CLI currently emits two telemetry streams in parallel:

- **Legacy (v1):** the `subcommand_metric!` macros in
  `cli/flox/src/utils/metrics.rs`. Do not add new instrumentation
  here.
- **v2 (this crate):** typed events recorded through
  `EventsHub::global().record_event(...)`, integrated via
  `cli/flox/src/utils/events.rs`. All new instrumentation goes here.

The two stacks share no code and write separate on-disk buffers.
`disable_metrics` silences both.

## The wire contract

Events are serialized as JSON objects and sent as **NDJSON — one JSON
object per line, never a JSON array**. Each object has this envelope
shape:

```jsonc
{
  "event_id":         "b6e2…",         // UUID, unique per event
  "event_timestamp":  1733600000000,   // integer ms since Unix epoch
  "source":           "cli",           // constant for this producer
  "invocation_id":    "aa10…",         // UUID, one per CLI invocation
  "device_id":        "9f42…",         // UUID, stable per installation
  "auth_subject":     "github|123",    // optional; pseudonymous only
  "producer_version": "1.16.0",        // optional; the CLI version
  "event_type":       "cli.environment.activate",
  "payload":          { }              // object, typed per event_type
}
```

The Rust source of truth is the `Event` struct and `EventKind` enum in
[`src/lib.rs`](src/lib.rs). `EventKind` is tagged with
`#[serde(tag = "event_type", content = "payload")]`, and the dotted
wire name on each variant's `#[serde(rename)]` attribute is the single
source of truth for that event's name. Call sites construct the
variant explicitly and never pass a string literal.

Rules the envelope encodes:

- `event_timestamp` is an **integer millisecond count** since the Unix
  epoch. Not RFC 3339, not seconds, not nanoseconds, not a float.
- `event_id` is fresh per event; downstream consumers de-duplicate on
  it. `invocation_id` correlates every event from one CLI invocation
  and is inherited by child processes via `FLOX_INVOCATION_ID`.
- Optional fields are **omitted when absent** — never serialized as
  `null` or `""`. Downstream, an explicit empty value is
  indistinguishable from a missing one, so sending it destroys the
  absent/empty distinction. Use
  `#[serde(skip_serializing_if = "Option::is_none")]` — that alone is
  enough; a missing key already deserializes to `None` without
  `#[serde(default)]`. (The legacy machine-context fields on
  `CommandPayload` — `os_family`, `os_family_release`, `os`,
  `os_version` — predate this rule and serialize as `null` when
  absent. They are frozen as shipped: don't imitate them, and don't
  "fix" them, since removing a shipped key is itself a breaking
  change.)
- The set of top-level envelope keys is **closed**. Consumers drop
  unknown envelope keys silently, so adding one is a coordinated
  change (see "Coordinating changes" below). New data goes in
  `payload`, where additions are safe.
- `payload` is always a JSON object. Payload-less events serialize as
  `"payload": {}`.

## Naming

- Event names are dotted, lowercase, `snake_case` within a segment:
  `cli.command_run`, `cli.environment.activate`,
  `cli.environment.generations.switch`. No camelCase, no hyphens.
- **Every CLI event starts with `cli.`** — consumers select this
  producer's events by that prefix, so an event named outside it is
  invisible to them.
- The template is `cli.<entity>.<verb>` when there is an entity, and
  `cli.<verb>` for CLI-level actions with none (`cli.build`,
  `cli.search`, `cli.authenticated`, `cli.update_prompted`). Group by
  the **entity**, not the caller: `cli.environment.install` (the
  environment is what changed), so a whole domain can be selected by
  prefix.
- `cli.environment.install` and `cli.package.install` are different
  **grains**, not competing names: the environment event is one row
  per invocation, the package events one row per package. Pick the
  grain that matches what a consumer counts.
- Payload field names are **bare**, discriminated by `event_type`:
  `outcome`, `duration_ms`, `error_kind` — not `build_outcome`,
  `build_duration_ms`. Shared meanings keep shared names across
  events; the event type is the namespace.
- Use one vocabulary for shared concepts. Outcomes are `success` /
  `failure` (the `Outcome` enum), not `succeeded` / `failed`.
- Keep names neutral and technical, matching the vocabulary already in
  this crate. When unsure, choose the most boring, descriptive name.

## Stability: what is safe and what is breaking

The consumers of these events live outside this repository and have
**no compile-time link back to this crate**. Nothing here breaks
locally when the contract moves — which is exactly why these rules
exist. Statements here about downstream behavior (unknown envelope
keys dropped, de-duplication on `event_id`, no backfill of data never
sent) are the ingestion side's contract, asserted for producers rather
than verifiable from this repo.

Safe, no coordination needed:

- Adding a new `cli.*` event type.
- Adding a new **optional** payload field to an existing event.

Breaking — never do these without coordination (see below):

- Renaming a shipped event type or payload field. A rename does not
  fail loudly anywhere; it silently produces empty data downstream
  from the release forward.
- Changing the JSON type of an existing field, including
  optional↔required, or a value's unit or encoding.
- Removing a field or event.
- Adding, renaming, or retyping a **top-level envelope** field.
- Changing the meaning of an existing field while keeping its name.

When an event's shape needs to change incompatibly, mint a **new
event type** rather than editing the old one in place — the
`event_type` string is the version discriminator.

Enum-like string values are part of the contract too. `error_kind`
slugs are derived from Rust error type names via `strum`, so renaming
a Rust error variant silently renames a wire value — and only a few
sample slugs are pinned by tests (the `error_kind_tests` module in
`cli/flox/src/main.rs` and the build-slug test in `flox-rust-sdk`), so
most renames trip nothing. Treat any rename of a variant on a
slug-bearing error enum as a wire change, even when every test stays
green.

## How the contract is enforced: golden tests

The golden tests in [`src/lib.rs`](src/lib.rs) — most named
`*_envelope_golden` — pin the serialized wire shape: the whole
envelope, as JSON, compared with `assert_eq!`. They are **the only
machine-checkable signal** that the contract moved:

> A failing golden means the contract moved and the consumer needs a
> migration. Update the expected JSON once that is arranged, not to
> make the test pass.

Test discipline:

- Every new event gets **one canonical test** asserting the full
  envelope, exercising the `#[serde(rename)]` through a real value.
  Most are named `*_envelope_golden`, but not all
  (`command_run_serializes_to_v2_envelope`,
  `authenticated_serializes_with_empty_payload`), and shape-identical
  variants sharing a payload struct may share coverage — so a
  name-filtered test run is not the whole contract.
- Optional fields additionally get one "absent key omits the field"
  pin (e.g. `cli_environment_activate_without_manifest_version_envelope_golden`).
- Do not add per-variant string assertions that only prove serde obeys
  its own rename attribute — that tests the library, and it duplicates
  the rename table. One canonical shape test per contract.

## Privacy: data is structurally absent, not scrubbed

The standing rule: **prefer designs where user data cannot reach the
wire, rather than designs that filter it out.** A denylist or regex
scrub is safe only against the leak shapes someone thought to
enumerate; a closed set of compile-time values is safe by
construction.

Three buckets govern what a field may carry:

- **Never in a new field:** email addresses, tokens or token bytes,
  filesystem paths, hostnames, and free-form user text (error
  messages, descriptions, command lines).
- **Allowed, and often the point:** low-cardinality identifiers the
  user chose that *are* the usage signal — package coordinates,
  environment names, manifest install ids, subcommand and flag names.
  These are identifiers, not prose: a field in this bucket must not
  be able to carry arbitrary sentences, paths, or secrets.
- **Pseudonymous, handled carefully:** `auth_subject` (the opaque
  OIDC/JWT `sub` claim) and `device_id`. Never substitute an email or
  handle for either.

Some shipped fields carry user-controlled text that a new field would
not be allowed. They are frozen as-is and are **not precedents**:

| Field | Events | What it carries |
|---|---|---|
| `search_term` | `cli.search` | The search text verbatim (legacy parity). |
| `env_ref_or_name` | `cli.environment.*` | `owner/name` for managed/remote environments — the owner segment is a FloxHub handle; the environment name for path environments. |
| `package` | `cli.package.*` | The coordinate as written: a catalog path, a store path (canonicalized under `/nix/store`), or a flake URL — which can embed a hostname or a local path. |
| `install_id` | `cli.package.upgrade` | The user-chosen manifest install id. |
| `invocation_sources` | `cli.command_run` | Launcher tokens, including any supplied via `FLOX_INVOCATION_SOURCE` — an open set by design. |

Patterns that make this structural:

- `error_kind` carries a slug derived from the error's **type**, never
  its rendered message — user data enters an error only through
  interpolated parameters, and the type name contains none of them.
  There is deliberately no `error_message` field.
- `CliBuildPayload::with_error_kind` takes `&'static str` instead of
  `impl Into<String>`, so only compile-time slugs can populate it; a
  runtime-rendered string is a compile error.
- `invocation_type` reports a four-value projection of the invocation,
  dropping the command the user passed to `-c` / `--`.
- `auth_subject` carries only the opaque OIDC/JWT `sub` claim —
  pseudonymous by contract, never an email or handle.

When you add a string field, its doc comment must state the value
domain and why it cannot carry personal data. If the honest answer is
"it can", redesign the field before shipping it.

Instrumentation must also stay within its scope: report what the
command already computes, and don't reach into data the code path
didn't already touch.

## Designing a new event or field

Get the shape right before implementing — shape mistakes are cheap to
fix before a release ships them and nearly impossible after. Questions
to answer (and to include in the PR description):

1. **Identity.** For each entity in the payload, is the identifying
   field a durable coordinate (stays meaningful as the system grows)
   or a convenience label? Prefer the coordinate.
2. **What does this emission destroy?** Emission is the last chance to
   keep structure. Prefer emitting structure and letting consumers
   flatten; a producer that pre-aggregates throws away the signal.
3. **The 12-month consumer.** Name the questions someone will ask of
   this data in a year. A producer change requires a release users
   must adopt; a downstream query change does not. Bias toward shapes
   that push future change downstream.
4. **Can it be derived?** If a consumer can compute the value from
   fields already on the same event — or from the same invocation's
   `cli.command_run` row via `invocation_id` — don't emit it. A
   derived field at the producer is a drift liability. (Domain events
   deliberately carry no `subcommand` and no machine context for this
   reason: consumers join to `cli.command_run`.)
5. **Caps and truncation.** If you bound a list, emit the true count
   alongside it, so truncation is distinguishable from genuinely
   short.
6. **Faithful values at every emit site.** Enumerate every site that
   constructs this payload and confirm each populates the real value.
   If the real value lives in a type the emit site doesn't read, the
   fix is plumbing it through — never a hardcoded stand-in.

"An existing spec said so" and "the legacy stream did it this way" are
not answers to these questions. Parity with a legacy mistake is a
chance to fix the mistake, not a reason to carry it forward.

## Emission mechanics

Call sites record events through the global hub — with one sanctioned
exception noted below — and never construct transport-level shapes
themselves:

```rust
use flox_events::{CliEnvironmentPayload, EventKind, EventsHub};

if let Err(err) = EventsHub::global().record_event(EventKind::CliEnvironmentDelete(
    CliEnvironmentPayload::new(env_detail_from_concrete(&flox, &concrete_environment)),
)) {
    debug!(error = %err, "Failed to record v2 event");
}
```

- **Telemetry never fails a command.** Every `record_event` call is
  wrapped in `if let Err(err) = … { debug!(…) }`.
- The hub is dormant until `cli/flox/src/commands/mod.rs` installs a
  client, and no client is installed when metrics are disabled — so
  the opt-out is honored at every call site automatically.
- Wrap expensive payload-only work (git or lockfile reads that exist
  only to fill a payload) in `EventsHub::global().when_client_set(…)`
  so opted-out users don't pay for it.
- Events are buffered on disk (`events-v2.json` in the data dir, one
  JSON object per line) and sent in batches of 100 once the buffer is
  older than two minutes, from a flush-on-drop guard in `main`. A
  failed send keeps events buffered for a later retry. Two buffer
  consequences shape the contract: the buffer caps at 1000 events with
  oldest-first eviction, so a domain or completion row can outlive the
  `cli.command_run` row it would join to — the join consumers are told
  to design around is best-effort, and they must tolerate its absence.
  And lines the running binary cannot parse (events from a newer flox
  sharing the data dir) are kept verbatim and sent only once a binary
  that understands them runs — so on a machine with several flox
  versions, an event can land later than the release that minted it.
- Lifecycle placement matters. `flox activate` replaces the process
  with `exec()`: anything recorded after that line is dead code in the
  parent, which is why `activate.rs` records completion and requests a
  flush before exec'ing. (An unforced flush still waits for buffer
  expiry — events not yet due are delivered by a later invocation.)
  When instrumenting a new path, confirm the emission is reached on
  every branch — including early exits and hand-off paths.
- `record_command_completed` is single-shot per client install: the
  first recorder wins and later calls are silently dropped, so the
  pre-exec emit and the dispatcher cannot double-count. Over-emission
  is as real a hazard as under-emission — the first writer must carry
  the richest payload it can.
- One sanctioned exception to "always through the hub": the
  no-subcommand welcome path in `cli/flox/src/commands/mod.rs` builds
  a client directly with `build_events_client`, because it returns
  before dispatch installs the global client. Events recorded that way
  are buffered and drained by a later invocation. Don't copy this
  shape onto paths the dispatcher reaches.
- `record_event_with_auth_subject` exists for the login flow, where
  the authenticated subject becomes known mid-invocation and the
  client's snapshot is stale by construction.
- Propagating `invocation_id` to child flox processes is explicit,
  per spawn site: set `FLOX_INVOCATION_ID` on the child's `Command`
  from `current_invocation_id()` (`cli/flox/src/utils/events.rs`), so
  one user action doesn't appear as several uncorrelated invocations.
  The id is deliberately kept out of the process environment so that
  commands run inside an activated shell count as fresh invocations —
  never export it; set it on the specific spawn.

The integration layer in `cli/flox/src/utils/events.rs` provides
`env_detail_from_concrete` / `env_detail_from_concrete_without_lineage`
for building the shared `EnvDetail` payload half, and
`build_events_client` for client construction. `EnvDetail` — flattened
into every `cli.environment.*` payload — serializes as `env_kind`
(`path`, `managed`, or `remote`) plus `env_ref_or_name`, with
`local_environment_id` for path environments and `generation_number`
for managed/remote ones (path environments structurally cannot carry a
generation number), plus an optional `package_count`.

## Verifying what is actually sent

Reading the code is not enough — exercise the instrumented path and
inspect the emitted JSON. Build first (`just build-cli`, inside
`nix develop` — the endpoint and key defaults are compiled in and the
build tools are not on bare PATH). Four gotchas leave your collector
silent:

1. Use `_FLOX_METRICS_URL_V2_OVERRIDE` (and
   `_FLOX_METRICS_API_KEY_V2_OVERRIDE`) to point the v2 stream at a
   local collector. The unsuffixed `_FLOX_METRICS_URL_OVERRIDE`
   redirects only the **legacy** stream — with only it set, your
   collector stays quiet while your test events go to the production
   endpoint, which is worse than no output.
2. Set `_FLOX_FORCE_FLUSH_METRICS=true`. Without it a single command
   sends nothing — the buffer flushes on expiry and in batches.
3. Use a real dispatched subcommand. `flox --version` returns before
   the events client is installed and emits nothing by construction.
4. Check that metrics aren't disabled (`flox config --get
   disable_metrics`, `FLOX_DISABLE_METRICS`) — many developers have
   the opt-out set, and with it on no client is installed and nothing
   is sent, with no error. Also look in the right buffer: the v2
   stream buffers to `events-v2.json` in the data dir; the
   similarly-named `metrics-events-v2.json` in the cache dir belongs
   to the legacy stream.

```bash
# terminal 1: a local collector that acknowledges each send
while true; do printf 'HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n' | nc -l 8080; done

# terminal 2
_FLOX_METRICS_URL_V2_OVERRIDE=http://localhost:8080 \
_FLOX_METRICS_API_KEY_V2_OVERRIDE=dummy \
_FLOX_FORCE_FLUSH_METRICS=true \
  ./target/debug/flox envs
```

The collector must answer with a 2xx: a send that gets no response
fails after a short timeout and the events stay buffered for retry, so
a bare one-shot `nc -l` makes every rerun re-send all earlier events
first — and BSD `nc` (the macOS default) exits after one connection
anyway. The loop above acknowledges each request, so each run prints
only its own events.

Set the override variables per command, as above — never export them
from a shell profile, `.envrc`, or CI config. An exported override
silently redirects that machine's real telemetry to whatever the value
says, over plain HTTP if the URL does. And `nc -l` listens on all
interfaces, so stop the loop when you're done.

Check the branches that matter: a success, a typed failure, a non-1
exit code, and (for activate) the exec hand-off path. A test that
asserts a constant you hardcoded is a claim, not evidence.

In unit tests, install a mock-backed client on the global hub and
assert on the sent events — copy the `MockHub` pattern from
`cli/flox/src/commands/upgrade.rs` (it also appears in
`commands/init/mod.rs`; it is a private test struct, not an importable
helper). Any test touching the global hub must be marked
`#[serial(global_events_client)]`.

## Coordinating changes

The events emitted here are ingested by systems maintained outside
this repository. Before merging any change from the "breaking" list
above — or when a new field needs to show up in downstream reporting
rather than just being recorded — raise it with the Flox maintainers
on the PR first. Include:

- The exact event type string(s) affected.
- Each new or changed payload key: name, JSON type, optional or not,
  and whether absent-vs-empty is meaningful.
- Every emit site, and confirmation each carries a real value.
- For enumerated values, the full value set and whether it grows.
- The first CLI version that emits it. Data a producer never sent
  cannot be reconstructed later, so the field's history starts at
  that release.

Additive changes (new `cli.*` events, new optional payload fields)
don't need prior coordination to be safe on the wire — but flag them
in the PR description regardless, so the downstream side knows they
exist.

This is a public repository. Code, comments, commit messages, and PR
text describe the change on its own technical terms — the events and
fields themselves. Don't name or link internal systems, tools,
dashboards, or issue trackers, and don't explain which internal report
a field powers. (A few pre-existing comments in this crate name
specific downstream systems; they are grandfathered, not license for
new text.)
