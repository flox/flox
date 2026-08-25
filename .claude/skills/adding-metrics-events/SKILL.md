---
name: adding-metrics-events
description: Use when adding or changing CLI telemetry/metrics - a new metric value (payload field) on an existing v2 event, a new metric message (event type), a new emit site, or any change to what the flox CLI reports. Also use when a *_envelope_golden test fails, or when tempted to rename an event or payload field.
---

# Adding a v2 metrics event or field

All new telemetry goes through the v2 events pipeline: the
`cli/flox-events` crate (types, buffering, sending) plus the
integration layer in `cli/flox/src/utils/events.rs`. The contract
rules — wire format, naming, stability, privacy — live in
`cli/flox-events/README.md`. **Read it before starting**; this skill
is the procedure, that file is the law.

The events are consumed by ingestion outside this repository with no
compile-time link back here. Nothing breaks locally when the contract
moves, so the discipline below is the only protection.

## When not to use this skill

- Adding to the legacy `subcommand_metric!` stream
  (`cli/flox/src/utils/metrics.rs`) — don't add new instrumentation
  there. Touch it only when an emit site you're riding already uses
  it.
- Server-side or non-CLI telemetry — out of scope for this repo.

## Step 0: decide the shape before writing code

Answer these, and put the answers in the PR description:

1. **New field or new event?** A new fact about an existing action is
   a field on that action's payload, preferably riding an existing
   emit so fields cross-tabulate without a join (see the
   `invocation_type` example below). A new *action* is a new event
   type. An incompatible change to an existing shape is a **new event
   type** — never an in-place edit.
2. **The README's design questions**, answered one line each
   (`cli/flox-events/README.md` § "Designing a new event or field"):
   durable identity, structure preserved, the 12-month consumer,
   derivability (if a consumer can compute it from the same event or
   the invocation's `cli.command_run` row, don't emit it), caps with
   true counts, faithful values at every emit site.
3. **Name and privacy**, per the README's "Naming" and "Privacy"
   sections: `cli.<entity>.<verb>`, bare snake_case payload fields
   (`event_type` is the namespace), `success`/`failure` outcomes; and
   a value domain structurally unable to carry user data — plan the
   projection now (an enum's `Display`, a `strum` slug), not a scrub.

For anything beyond a straightforward additive field, share the
proposed shape (fields, sources, deliberate exclusions — ~10 lines)
with the maintainers **before** implementing. A reviewer reading a
finished diff anchors on "is this implemented correctly", not "is
this the right shape".

## Path A: add a field to an existing event

Model: commit `be060afd5` (`feat(metrics): report activation
invocation type`) — two files, ~28 lines.

1. **Add the field** to the payload struct in
   `cli/flox-events/src/lib.rs`:
   - `#[serde(skip_serializing_if = "Option::is_none")]` for optional
     fields. That alone is correct: it keeps the key off the wire when
     absent, and a missing key already deserializes to `None`. (Some
     existing fields also carry `default`, which is redundant on
     `Option` fields — don't add it to new ones, and don't churn-fix
     the old ones.)
   - Doc comment stating the value domain and why it cannot carry
     user data.
   - Initialize to `None` in `new()`; add a
     `with_<field>(mut self, value: …) -> Self` builder, matching the
     existing builders in the same struct. For fields that must only
     ever hold compile-time slugs, take `&'static str` (see
     `CliBuildPayload::with_error_kind`).
2. **Populate it at the emit site(s)** in `cli/flox/src/commands/`.
   Grep for every constructor of that payload
   (`grep -rn 'PayloadName::new' cli/flox/src/`) and decide per site:
   populate with the **real** value, or leave absent. Never a
   hardcoded stand-in — if the real value lives in a type the emit
   site doesn't read, plumb it through.
3. **Update the golden tests** in `cli/flox-events/src/lib.rs`:
   extend the event's canonical `*_envelope_golden` with the new
   field, and keep (or add) one golden pinning that the key is
   omitted when absent. Do not add per-variant serde string
   assertions — one canonical shape test per contract.

## Path B: add a new event type

Model: commit `0eec48c2a` (added `cli.authenticated` and
`cli.update_prompted`).

1. **Add the variant** to `EventKind` in
   `cli/flox-events/src/lib.rs` with the dotted name on
   `#[serde(rename = "cli.<entity>.<verb>")]`. That rename string is
   the single source of truth for the wire name — no `Display`, no
   `as_str`, no string literals at call sites.
2. **Pick the payload type.** Reuse an existing payload struct when
   the shape is identical (most environment events share
   `CliEnvironmentPayload`) — the variant is the discriminant, so
   don't clone a struct just to rename it. A payload-less event is a
   unit-braces variant (`CliAuthenticated {}` → `"payload": {}`).
   A new payload struct keeps fields private, takes required data in
   `new()`, and adds `with_*` builders for optionals.
3. **Emit it** where the action happens:

   ```rust
   if let Err(err) = EventsHub::global().record_event(EventKind::CliFoo(payload)) {
       debug!(error = %err, "Failed to record v2 event");
   }
   ```

   - Telemetry never fails a command: always the
     `if let Err … debug!` wrapper.
   - For environment events, build `EnvDetail` with
     `env_detail_from_concrete(&flox, &env)` (or
     `…_without_lineage(&env)` before the environment is locked or
     trusted) from `cli/flox/src/utils/events.rs`.
   - Wrap payload-only expensive work (git/lockfile reads) in
     `EventsHub::global().when_client_set(…)`.
   - **Check reachability:** the emit must run on every branch that
     should count — early exits, error paths, and before any
     `exec()` (code after `exec()` never runs in the parent; see the
     pre-exec emit + flush in `activate.rs`). An outcome event with
     no outcome emits nothing.
   - If the path spawns flox subprocesses, set `FLOX_INVOCATION_ID`
     on the child's `Command` from `current_invocation_id()`
     (`cli/flox/src/utils/events.rs`) — propagation is explicit per
     spawn site, deliberately never via the exported environment.
4. **Add the golden test:** one canonical `<event>_envelope_golden`
   asserting the **entire** envelope JSON with `assert_eq!` against a
   `json!({...})` literal, using the `fixed_event` fixture. Plus one
   absent-optional golden if the payload has optionals.
5. **Add a call-site test** if the emit logic has branches worth
   pinning: install the `MockHub` harness (see
   `cli/flox/src/commands/upgrade.rs`), run the handler, filter
   `sent_batches` by `EventKind`. Mark it
   `#[serial(global_events_client)]`.

## Verify

Run the tests (inside `nix develop`, or prefix with
`nix develop -c`):

```bash
just ut envelope_golden    # wire-shape goldens, skips the full build
just unit-tests            # full workspace, before the PR
```

The name filter catches most canonical tests but not all (the
`command_run` and `authenticated` pins use other names), so the full
run before the PR is not optional.

Then verify the real bytes with the local-collector recipe in
`cli/flox-events/README.md` § "Verifying what is actually sent" —
reading the code is not evidence, and that section lists the traps
that silently produce no output (wrong override var, buffering,
`flox --version`). Inspect the NDJSON for each branch that matters:
success, a typed failure, a non-1 exit code, and any exec/hand-off
path. Confirm the field carries the real value on each, and is absent
(not `null`, not `""`) where unknown.

## Red flags — stop and re-read the README

| Thought | Reality |
|---|---|
| "I'll just rename this field/event to something better" | Renames break nothing locally and silently produce empty data downstream forever. Shipped names are frozen; an incompatible shape is a new event type. |
| "The golden test fails — I'll update the expected JSON" | A failing golden means the contract moved. Update it only for the intentional addition you made; if it fails for a rename/retype, that's the signal working. |
| "The spec / the legacy stream did it this way" | Specs and legacy precedent are inputs to challenge. Parity with a legacy mistake is a chance to fix it, not a justification. |
| "I'll hardcode this value here; it's always 1 anyway" | A stand-in value at one emit site poisons the data for every consumer. Plumb the real value. |
| "The consumer could compute this, but emitting it is convenient" | Don't. Derived fields at the producer drift; consumers join to `cli.command_run` on `invocation_id`. |
| "I'll scrub the string before emitting" | Denylists are unsound by construction. Redesign so user data is structurally absent (type-derived slug, closed-set projection). |
| "I'll add the field now and populate it in a follow-up" | Every PR is a complete vertical slice: defined → populated → tested → verifiable in the same PR. No builders without call sites. |
| "It compiled and unit tests pass, so it works" | The consumer is not in the test loop. Run the local-collector check and look at the bytes. |
| "I'll note in the PR which internal system/dashboard needs this" | This is a public repo. PR text, commits, and comments describe the events and fields on their own technical terms — never name or link internal systems, tools, or trackers. |
