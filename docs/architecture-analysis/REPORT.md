# Flox Architecture Analysis — Overall Report

**Date:** 2026-06-11
**Status:** Analysis only. This report interprets the six workstream outputs
in this directory (`A-command-audit.md` … `F-risk-map.md`); it is the verdict,
they are the evidence. Read this instead of them; follow the pointers when you
need proof.

---

## 1. Executive summary

The four goals — faster subcommand development, a gh-style plugin system, a
CLI/API split reusable by floxhub and floxdash, and review intensity legible
from the diff path — are all achievable, and **none requires a rewrite or a
new architecture**. The analysis found that the architecture flox wants
mostly already exists; it is undermined by one dependency edge, one missing
design decision, and a large amount of business logic stranded one layer too
high.

The five findings that matter:

1. **flox-rust-sdk can be the shared API layer directly — no facade crate
   needed** (B, verdict section). It is print-clean, TTY-free, returns typed
   results for every operation, and takes context through an injectable
   `Flox` struct. Its gaps are *missing logic*, not wrong shape.
2. **One Cargo.toml line contaminates 8 of 15 crates.** `flox-core` depends
   on `crossterm` to color two ~10-line formatting helpers
   (`cli/flox-core/src/util/message.rs`, 29 lines total). Because flox-core
   sits at the bottom of the graph, every crate above it — including the SDK
   floxhub would link — carries a terminal-event library (D, violation #1).
   Cutting this single edge is the cheapest, highest-leverage fix in the
   entire analysis.
3. **The one hard design decision is mid-operation interactivity, and it
   reduces to one contract.** Eight mid-operation prompts exist; all reduce
   to three shapes, and the codebase has already invented four bespoke
   solutions to the problem (C). Recommendation: hoist decisions to
   parameters by default; return typed, serializable "needs input" outcomes
   for the rest; standardize progress on the tracing-span convention the SDK
   already uses at 25 sites with zero terminal dependencies.
4. **Plugins are small and independent**: PATH dispatch for `flox-*`
   binaries is 1–2 days of work in `main.rs`, plus a documented contract
   (E). The only real decisions are token access (recommended: explicit
   `flox auth token`, never auto-exported) and a minimum `--json` set.
5. **The hard floxhub constraint is not fixable by refactoring** — every
   build-touching operation mutates the host-global `/nix/store` via
   subprocess (B, side-effect profile). The honest answer is operation
   subsetting: search/show/reads/resolve-only locking/metadata sync run
   in-process; builds go behind a worker boundary. That is a deployment
   decision, and no crate shape changes it.

Cost shape: Phase 0 hygiene is days; the operations-layer migration is
mechanical and incremental (ranked backlog in §5, driven by A's matrix);
the single biggest item is init's ~3.5k-LOC language-detection subsystem.
What it unblocks: floxhub/floxdash reuse, plugins that consume structured
output, new subcommands as thin glue, and path-based review tiers.

---

## 2. Current vs. target architecture

```
CURRENT                                      TARGET
┌────────────────────────────────┐           ┌────────────────────────────────┐
│ flox (CLI binary)        L3    │           │ flox (CLI binary)        L3    │
│  commands/: logic ⊕ prompts    │           │  commands/: parse → render     │
│  ⊕ rendering ⊕ subprocesses    │           │  (tier-3 glue, light review)   │
│  init: 4,476 LOC of detection  │           │  + PATH dispatch: flox-* ──────┼──▶ plugins
│  install: onboarding + RC edits│           │  prompts answer typed outcomes │
└───────────────┬────────────────┘           └───────────────┬────────────────┘
                │                                            │ structured results,
                ▼                                            ▼ progress spans
┌────────────────────────────────┐           ┌────────────────────────────────┐
│ flox-rust-sdk            L2    │           │ flox-rust-sdk = API layer L2   │
│  clean ops, typed results,     │           │  + stranded logic moved in     │
│  BUT missing the logic above   │           │  (pull recovery, install       │
│  (carries crossterm transitively)          │   interpretation, init detect) │
└───────────────┬────────────────┘           │  consumed by CLI, floxdash,    │
                │                            │  floxhub (in-process subset)   │
                ▼                            └───────────────┬────────────────┘
┌────────────────────────────────┐           ┌────────────────────────────────┐
│ flox-catalog / flox-manifest   │           │ L1 domain crates (unchanged,   │
│ / nef-lock-catalog       L1    │           │  reqwest removed from manifest)│
└───────────────┬────────────────┘           └───────────────┬────────────────┘
                ▼                                            ▼
┌────────────────────────────────┐           ┌────────────────────────────────┐
│ flox-core                L0    │           │ flox-core (terminal-free) L0   │
│ ⚠ crossterm + supports-color  │           │  enforced by cargo-deny +      │
│   → contaminates 8/15 crates   │           │  xtask lint-layers in CI       │
└────────────────────────────────┘           └────────────────────────────────┘
```

The delta, in prose: nothing moves *between* layers except stranded logic
descending from the binary into the SDK. The layer structure already exists
(D found dependencies already point strictly downward); the target makes it
**enforced** rather than incidental, removes the one upward-facing leak at
the bottom, and adds two consumers (floxhub, floxdash) and one extension
point (plugin dispatch) at the top.

---

## 3. The six outputs, interpreted

### 3.1 Workstream A — Command Audit Matrix (`A-command-audit.md`)

**Context.** To migrate 22 commands to `parse → call operation → render`,
you need to know which are already there, which are fat, and which contain
the genuinely hard problem (prompts in the middle of an operation).

**What the data says.** The distribution is extremely skewed. Seven commands
are already at or near the target shape (`include upgrade` at 108 LOC,
`delete` at 89, `uninstall`, `upgrade`, `push`, `show`, `search`); a middle
band needs only logic extraction; and three commands hold most of the
problem: `init` (4,476 LOC — an entire language-detection subsystem in the
command layer), `activate` (906 non-test LOC of context assembly ending in
process replacement), and `install` (onboarding flow that creates remote
environments and edits the user's shell RC files). Eight mid-operation
prompts exist, all cataloged with file:line. The audit also corrected a
baseline error: `--json` exists on five commands today (`envs`, `search`,
`services status`, `generations list/history`), not one — there is just no
systemic pattern.

**Impact.** The migration is a ranked backlog, not a leap of faith: start
where the pattern already holds (push is the model), end at init. The
"stranded logic" column became Workstream B's add-list, and the
mid-operation column scoped Workstream C to exactly eight cases. Ignoring
this ordering means burning the hardest 20% of effort first for the least
reusable 20% of value.

**Example.** The two ends of the spectrum: `include.rs:80-104` is the whole
of `include upgrade` — one SDK call returning `UpgradeResult`, then a render
loop. `init/node.rs` alone is 1,747 lines that resolve Node versions
against the catalog, interleaved with `Select` dialogs (`node.rs:935`) —
logic floxdash could not reuse today without linking the CLI binary's
internals.

**Diagram.** The matrix is inherently tabular; the structural picture it
feeds is the architecture diagram in §2 (commands box shrinking). A
spectrum sketch of where the 22 commands sit:

```
easy ◀──────────────────────────────────────────────────────────▶ hard
include  delete uninstall push show search … gc build auth install pull edit init activate
└── already parse→call→render ──┘  └─ extract logic ─┘ └─ + mid-op prompts ─┘ └ structural ┘
```

### 3.2 Workstream B — SDK Fitness Review (`B-sdk-fitness.md`)

**Context.** The reuse question: is flox-rust-sdk the shared API layer for
CLI + floxhub + floxdash, and what must be added or removed?

**What the data says.** Of ~30 major public items, ~24 are clean operations.
Verified independently: zero non-test prints, zero TTY probes, one ambient
cwd read in the whole SDK (`remote_environment.rs:469`). Four leaky items
(telemetry env-var fingerprinting, a display-string method, a UI-flavored
trait method name, and CLI command suggestions embedded in error `Display`
impls). The add-list is 24 items, prioritized: P1 (blocks floxhub) is pull
orchestration/recovery, install-result interpretation, and publish git
pre-flight; the long tail is floxdash parity dominated by init. The
side-effect profile is the structural result: **every floxhub "no" shares
one root cause — host-global `/nix/store` mutation via nix subprocess** —
which no crate refactor changes.

**Impact.** The verdict ("SDK directly; no `flox-ops` facade") avoids
building and maintaining a whole crate that would re-export the same types
while adding no information. The conditions attached (fix the flox-core
leak, land P1 adds, relocate R6–R10) are the actual scope definition of the
refactor. The side-effect profile answers the expensive-to-discover-late
question now: floxhub gets an in-process subset plus a build-worker
boundary, decided at deployment design time, not discovered in production.

**Example.** The subtlest finding: SDK error types embed CLI copy —
`ResolutionFailures`' `Display` renders multi-paragraph advice including
"run 'flox edit'" (`lock_manifest.rs:1266-1330`). That is *intended* per
AGENTS.md's error architecture, but it means a floxhub web page rendering
that error would tell a browser user to run a terminal command. The fix is
not flattening the enums (the typed variants are correct); it is moving the
suggestion copy to the renderer — a decision recorded in §4.

**Diagram.** The floxhub boundary that falls out of the side-effect profile:

```
                 in-process (per-tenant Flox, spawn_blocking)
   floxhub ────▶ search/show · generations read · lock (resolve-only)
                 push/pull metadata sync · publish metadata checks
                       │
                       │ anything that builds
                       ▼
                 worker boundary (queue / separate host)
                 install · upgrade · build · publish upload · store gc
                 root cause: nix subprocess writes host-global /nix/store
```

### 3.3 Workstream C — Interactivity & Side-Effects Inventory (`C-interactivity-inventory.md`)

**Context.** CLI/API separations fail at interactivity; everything else is
mechanical. This workstream makes the hard decision once, on the eight real
cases.

**What the data says.** Classification: 3 of 8 mid-operation prompts are
hoistable by decomposition (install onboarding, RC-file, init hooks — in
each, nothing the answer depends on has been mutated yet), 4 are modelable
(pull's two recovery dialogs, activate's include-trust, auth's device flow,
edit's retry loop — the subject of the prompt is only discoverable
mid-flight), and a short structural list can never be API calls (activate's
`exec()`, `$EDITOR`, RC-file writes, browser spawning). Two assets were
found, not designed: **progress is already a headless event contract**
(spans with a `progress` field; the SDK emits 25 with no terminal deps, the
CLI's `IndicatifLayer` merely renders them), and **pull already contains
the modelable pattern** as the injected `QueryFunctions` seam
(`pull.rs:86-89`), where `None` means "non-interactive" and forces
conservative defaults.

**Impact.** The recommended contract — hoist by default; typed serializable
`NeedsX` outcomes for the rest; progress on the span channel — costs almost
nothing new because both halves already exist in embryo. The alternative
(injected callbacks) was examined and rejected with a reason that matters:
callbacks cannot cross an HTTP boundary and block a transaction while a
human thinks. Not deciding this is also a measured cost: the codebase
already contains four bespoke solutions to this exact problem (pull's
callbacks, edit's error classifier, init's prompt/detect split, activate's
policy chain).

**Example.** Pull, worked end to end (case b in the memo): today, answering
"No" to the add-your-system dialog **deletes the clone** (`pull.rs:458`).
Under the contract, `pull` returns
`PullOutcome::IncompatibleSystem { system }` with the environment kept in a
pending state, and the caller follows up with `amend_system`, `accept_broken`,
or an explicit `discard`. The CLI renders that as the same dialog; floxhub
pre-answers it with request policy or receives a typed 409-class error. The
contract change (abort becomes explicit, not a side effect of saying no) is
the one behavioral decision buried here, surfaced in §4.

**Diagram.**

```
 caller (CLI / floxdash / floxhub)            operation (SDK)
 ──────────────────────────────────           ─────────────────────────
 call op(args, policy)  ────────────────────▶ runs; emits progress spans
                                              │
        ◀── Complete(result) ─────────────────┤ done
        ◀── NeedsX { subject, options } ──────┤ world left in NAMED state
 CLI: prompt → follow-up call                 │
 floxdash: modal → follow-up call             │
 floxhub: policy param pre-answers, or        │
          typed error names the decision      ▼
```

### 3.4 Workstream D — Dependency & Layering Analysis (`D-dependency-layering.md`)

**Context.** Is the implied layering real, where does terminal code leak,
and what rule would be enforceable rather than aspirational?

**What the data says.** The graph is already correctly layered — every
workspace dependency points downward — with exactly one structural leak:
`flox-core → crossterm` (+`supports-color`), declared at
`cli/flox-core/Cargo.toml:12,24`, used in exactly one 29-line file, reaching
8 of 15 crates transitively (the SDK three separate ways). Everything else
checked clean: inquire/indicatif/tracing-indicatif/minus live only in the
`flox` binary; all SDK print-macro hits are inside `#[cfg(test)]`. Bonus
findings: `flox-manifest` links all of reqwest for one `Url` re-export,
`nef-lock-catalog` inherits crossterm/sentry/sysinfo for one `Version`
type, and `flox-events` (152 LOC) has zero consumers.

**Impact.** "The SDK is the API layer" is false today in the only sense a
web service cares about — its dependency closure — and becomes true with
one small change. The proposed enforcement (a cargo-deny `[bans]` list with
`wrappers = ["flox"]`, plus an `xtask lint-layers` for layer-direction
rules cargo-deny can't express) is what converts the architecture from
documented to checkable; the draft config *fails on today's tree by
design*, so adopting it in the same change as the fix makes regression
impossible.

**Example.** Two ~10-line helpers (`format_error` colors a ✘ red,
`format_updated` colors a ✔ green) were pushed to the bottom of the graph
so two binaries could share them. The price: every intermediate crate
carries crossterm with its `event-stream` feature (mio, signal machinery)
enabled workspace-wide.

**Diagram.** The contamination, and what one cut removes:

```
            flox-core ──crossterm (29-line message.rs)        ← cut here
           ▲    ▲    ▲     ▲
           │    │    │     └ nef-lock-catalog ▲
  flox-activations  flox-manifest ◀───────────┤
                          ▲                   │
                          └─── flox-rust-sdk ◀┴── beta, xtask
                               (3 parallel paths)
  8 of 15 crates carry crossterm today; 1 (the flox binary) has a reason to.
```

### 3.5 Workstream E — Plugin Feasibility (`E-plugin-feasibility.md`)

**Context.** Goal 2: what does a gh-style plugin system cost, and what
contract must flox offer?

**What the data says.** No dispatch exists; unknown subcommands die in
bpaf's generic error at `main.rs:139-141`. The right hook is **not**
intercepting that failure (bpaf returns the same variant for bad flags on
valid commands) but peeking at the first positional token before the parse,
cargo-style: unknown name + `flox-<name>` on PATH → exec it; built-ins
always win. The environment contract is largely free (activated envs
already export `FLOX_ENV*`); the version string is machine-parseable; the
auth token is reachable via config file or `FLOX_FLOXHUB_TOKEN`.

**Impact.** Dispatch is 1–2 days; the full primitive including a
`flox auth token` built-in and a contract doc is under a week. Plugin
*usefulness* scales with `--json` coverage, which ties plugins to the same
structured-results work everything else needs — every datum a plugin needs
is by definition something requiring structured output. The conservative
token policy (explicit `flox auth token`, gh's model, never auto-exported)
turns credential access into an auditable opt-in act.

**Example.** A hypothetical `flox-doctor` plugin: dropped into PATH, invoked
as `flox doctor`, it calls `flox envs --json` and `flox --version`, inspects
the active `FLOX_ENV`, and prints a health report — no flox changes needed
beyond the dispatcher, and no scraping of human output.

**Diagram.**

```
 $ flox doctor --verbose
     │
     ▼ main.rs: peek argv[1] BEFORE bpaf parse
 "doctor" ∈ built-ins? ──yes──▶ normal bpaf parse → Commands::…
     │ no
     ▼ search PATH for "flox-doctor"
 found? ──no──▶ fall through to bpaf → its normal unknown-command error
     │ yes
     ▼ exec flox-doctor --verbose   (env passed through; exit code passes through;
                                     auth only via explicit `flox auth token`)
```

### 3.6 Workstream F — Risk Map & Review Labeling (`F-risk-map.md`)

**Context.** Goal 4: make "review carefully" vs. "thin glue" a clean-cut,
path-based decision.

**What the data says.** Ten tier-1 regions identified by three sufficient
signals (blast radius; an AGENTS.md caution — each one a scar; sustained
fix density above the 25% repo baseline combined with stateful behavior).
Highest fix density: `cli/flox-activations/` at 16/55 (29%) — layered
deactivation and zsh state are the bug magnet. Highest ratio:
`publish.rs` at 34%, deliberately kept tier 2 because its failures are loud
and non-corrupting. Near-zero churn but tier 1 on consequence: `ld-floxlib/`
(LD_AUDIT library in every process, hand-maintained `.symver` GLIBC
bindings) and `nix-plugins/`. Current CODEOWNERS covers ~1 of 10 tier-1
regions. Measurement caveat: the clone is shallow (457 commits, ~3 months),
so densities are short-window.

**Impact.** The proposed CODEOWNERS scheme can be adopted **now** and
survives the migration unchanged — activation, auth, schema, and native
code stay where they are; only `commands/` flips from tier 2 to tier 3 when
the backlog completes. After the split plus D's enforced layering, a
`commands/`-only diff is *provably* glue, because the dependencies that
would let logic hide there are banned. The `// TRICKY:` convention is
endorsed narrowly (SAFETY-style: invariant + what breaks + where enforced)
for the line-level granularity paths can't give — `activate.rs` is 1,240
lines of which perhaps 40 are dangerous.

**Example.** The sharpest gap found: the one existing CODEOWNERS rule
protects `cli/schemas/` (the published JSON schemas), but the Rust source
that *defines* those shapes — `cli/flox-manifest/src/parsed/v*.rs` — is
uncovered. A schema-breaking change can merge without ever touching the
protected path, bypassing the rule's entire intent.

**Diagram.**

```
 diff path                                   tier   review question
 ──────────────────────────────────────────  ────  ─────────────────────────
 cli/flox/src/commands/* (post-migration)     3    "does the output look right?"
 cli/flox-rust-sdk/** (operations)            2    "is the logic sound?"
 cli/flox-manifest/**, cli/schemas/           1    "is the contract preserved?"
 flox-activations/, environment-interpreter/,
 shell_gen/, ld-floxlib/, nix-plugins/, auth  1    "is the invariant preserved?"
```

---

## 4. Decision list

Each decision: the question, the recommendation, the evidence, and the cost
of deciding otherwise.

| # | Decision | Recommendation | Evidence | If decided otherwise |
|---|---|---|---|---|
| 1 | Is flox-rust-sdk the API layer, or build a `flox-ops` facade? | **SDK directly.** | B verdict §: 24/30 items clean; typed results everywhere; `Flox` struct is the DI seam. | A facade re-exports the same types, adds a crate to maintain, and renames the gap problem without closing it. B lists four concrete conditions that would flip this (e.g. an async-native requirement) — revisit only if one occurs. |
| 2 | Fix `flox-core → crossterm` and adopt enforcement first? | **Yes — Phase 0.** Move the two helpers into the binaries; adopt cargo-deny bans + `xtask lint-layers` in the same change. | D violations #1–#2; the draft config fails on today's tree by design. | Every floxhub/floxdash build links a terminal-event library, and the layering remains an honor system that the next convenient helper erodes. |
| 3 | What is the interactivity contract? | **Hoist by default; typed serializable `NeedsX` outcomes for the rest; reject injected callbacks.** Progress = the existing `progress` span-field convention, promoted to a documented contract. | C design memo + 4 worked cases; the SDK already emits 25 progress spans; `QueryFunctions` (`pull.rs:86-89`) proves the outcome pattern works. | A fifth bespoke solution joins the existing four, and each future command re-litigates the question; callbacks dead-end at the first HTTP boundary. |
| 4 | May "No" keep destroying state (pull deletes the clone, `pull.rs:458,519`)? | **No — returned outcomes leave the world in a named pending state; abort becomes an explicit `discard` call.** | C worked case (b). | Outcomes can't be answered across process lifetimes, which silently re-couples operations to an interactive caller. |
| 5 | Plugin dispatch mechanism? | **argv[1] peek before the bpaf parse (cargo-style); built-ins always win.** Phase-2 managed extensions deferred. | E; `ParseFailure::Stderr` interception would shadow real usage errors on valid commands. | Hooking the parse failure breaks error messages for typos in flags; gh-style managed extensions first is ~2 weeks of infra before the first plugin can exist. |
| 6 | Do plugins get the auth token automatically? | **No — explicit `flox auth token` built-in; never auto-export `FLOX_FLOXHUB_TOKEN` to spawned plugins.** | E security §; gh's model. | Every plugin (and everything *it* spawns) silently inherits credentials; a compromised PATH entry exfiltrates tokens with no audit trail. |
| 7 | What happens to CLI copy in SDK error `Display` impls ("run 'flox edit'")? | **Keep the typed variants and their data; move suggestion copy to the renderer.** | B item R9 (`lock_manifest.rs:128-142,1266-1330`); AGENTS.md forbids flattening the enums. | floxhub renders terminal instructions in a web UI; or worse, someone "fixes" it by string-matching errors downstream, which AGENTS.md explicitly bans. |
| 8 | How does floxhub consume the SDK? | **In-process subset (search/show, reads, resolve-only lock, metadata sync) per-tenant via injected `Flox` dirs + `spawn_blocking`; builds behind a worker boundary.** | B side-effect profile: every "no" traces to host-global `/nix/store` mutation. | Either floxhub runs user-triggered nix builds inside a multi-tenant web process (arbitrary code execution, host-global state), or the team discovers this boundary after integration work has assumed otherwise. |
| 9 | Adopt CODEOWNERS / tiers now or after the refactor? | **Now.** Tier-1 set survives the migration unchanged; tier-2 routing via labeler action if branch protection would make entries gating. | F §(c),(e). | The riskiest code (activation, auth, schema sources) stays review-optional during exactly the period of heaviest churn. |
| 10 | Where does init's ~3.5k-LOC detection subsystem go? | **Into the SDK (`providers/init_detection/` or a sibling crate)** — largest single add item, but mechanical: each hook already separates detect from prompt. | A fattest-commands §; B item A24. | floxdash reimplements Node/Python/Go detection, and the two copies drift. |

---

## 5. Phased migration backlog (analysis deliverable — execution out of scope)

Ordering is driven by A's difficulty ranking and B's priorities. Estimates
assume one engineer familiar with the codebase.

**Phase 0 — hygiene and guardrails (~1 week).** Fix D #1–#2 (move
`format_error`/`format_updated` into the binaries); adopt cargo-deny bans +
`xtask lint-layers` in the same change; fix D #3–#5 (`Version` relocation,
`reqwest`→`url` in flox-manifest, sentry init to binaries); decide
`flox-events`' fate; adopt F's tier-1 CODEOWNERS; fix the stale
`assets/activation-scripts/` path in AGENTS.md.
*Unblocks: the SDK's dependency closure becomes web-clean; architecture
becomes regression-proof; riskiest paths gain gating review.*

**Phase 1 — contracts (~1–2 weeks, mostly decisions + docs).** Ratify the
C contract (progress spans + `NeedsX` outcomes) and the §4 decisions; add
`flox auth token`; implement E's PATH dispatch + plugin contract doc;
define the `--json` envelope convention (the five existing flags are the
prior art).
*Unblocks: plugins ship; every subsequent migration has a pattern to follow
instead of a debate.*

**Phase 2 — P1 logic descent + pilot (~3–4 weeks).** Move B's A1–A7 into
the SDK (pull orchestration/recovery as `PullOutcome`, install
interpretation + retry, publish git pre-flight) and execute R6–R10 (SDK
leaks incl. the cwd read and error-copy relocation); migrate `pull` and
`install` to the new shape as the pilot (they exercise both the modelable
and hoistable cases); add `--json` to `list` and `show`.
*Unblocks: the assumed floxhub surface exists; the two hardest
non-structural commands prove the contract.*

**Phase 3 — parity long tail (ongoing, parallelizable).** Remaining adds
(A8–A23: detect/trust policy, auth device flow, services state, activate
context assembly, gc) command by command down A's ranking; finish with
A24 (init detection, ~3.5k LOC, mechanical); flip `commands/` to tier 3 in
the risk map when done.
*Unblocks: floxdash full parity; new subcommands are ~20 lines of wiring +
a render function over a tested operation.*

**Phase 4 — consumer integration (scheduled by product, not by this
analysis).** floxhub in-process subset + build-worker boundary per §4
decision 8; consider the async facade only if B's verdict-flip condition
(async-native requirement) materializes; consider gh-style managed plugin
extensions only if PATH-dispatch adoption warrants it.

---

## 6. Assumptions & open inputs

The conclusions above hold under these assumptions; each names what would
change if the assumption is wrong.

1. **floxhub/floxdash capabilities were assumed, not gathered** (GOAL.md
   defaults: floxdash = CLI parity minus activate; floxhub = reads +
   push/pull/publish-adjacent mutations, in-process multi-tenant). *A real
   capability sketch is the single most valuable input the team can
   provide.* If floxhub needs builds synchronously in-request, decision 8
   becomes a service-architecture project. If floxdash drops init parity,
   A24 (the largest add) leaves the backlog entirely. If a
   stability-versioned external API is required, B's verdict flips toward a
   facade — by its own stated criteria.
2. **Shallow git history.** F's fix densities cover ~3 months (457
   commits), not the requested window; low-churn conclusions are
   consequence-based by design. Re-measure on a full clone before treating
   densities as long-run rates.
3. **No compile-based verification.** `cargo`/`nix` were unavailable in the
   analysis container; everything rests on manifest reading and grep. Two
   checks should run once a dev shell is available: `cargo tree -i
   crossterm --workspace` (confirm D's reach claims) and the
   tighten-visibility experiment from B's verdict-flip condition #1.
4. **Line numbers drift.** All citations are against the working tree of
   2026-06-11; each workstream doc carries a "How to reproduce" section to
   re-derive its evidence.
5. **Biz-logic percentages are judgment estimates** (A's stated ±10-point
   band); the migration *ranking* is robust to that band even where
   individual figures are debatable.

## How to reproduce

This report introduces no new measurements. Every claim traces to one of
the six workstream documents in this directory; reproduce any of them via
the "How to reproduce" section at the end of the respective file.
