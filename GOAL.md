# GOAL.md — Flox Architecture Analysis Plan

This document is an **analysis-only** plan. Executing it produces documents,
matrices, and diagrams — never changes to production code, build files, or
CI configuration. It is designed to be executed incrementally (workstream by
workstream) via the `/goal` slash command, with all outputs written to
`docs/architecture-analysis/`.

## Why this analysis exists

Four goals drive it:

1. **Faster subcommand development** — adding a subcommand should be thin
   wiring plus a render function, not 1,000+ lines of interleaved logic and
   terminal I/O.
2. **A gh-style plugin system** — external `flox-*` executables discovered on
   `PATH`, with a documented contract (env vars, JSON output, exit codes).
3. **Clean CLI / API separation** — the same operations layer must be
   consumable by the CLI, floxhub (web portal), and floxdash (TUI). The
   working hypothesis is that `flox-rust-sdk` becomes that layer, but
   **whether it should — and what must be added to or removed from it — is a
   question this analysis answers, not an assumption it makes.**
4. **Review-labelable structure** — code organized so that "this is tricky,
   review carefully" vs. "this is thin glue" is legible from the diff path
   alone, enforceable via CODEOWNERS.

Goals 1, 3, and 4 are served by the same structural insight (commands become
`parse → call operation → render`); goal 2 is mostly independent and cheap.

## Baseline findings (verified, with sources)

These facts anchor the workstreams. Re-verify any that look stale before
relying on them.

- The CLI uses **bpaf**; all 26 commands are statically wired in
  `cli/flox/src/commands/mod.rs` (~1,500 lines). Adding a command costs
  ~20–40 lines of wiring plus the command file.
- **Entanglement is concentrated in the command layer**:
  `cli/flox/src/commands/install.rs` (~1,090 lines, ~30% UI concerns),
  `activate.rs` (~1,240 lines, ~35%), `push.rs` (~520 lines, ~15%, the
  cleanest model).
- `flox-rust-sdk` (~37k LOC) has **no direct dependency** on crossterm,
  inquire, or indicatif, and returns typed results (e.g.
  `ManagedEnvironment::push` → `PushResult`).
- **Known layering leak**: `flox-core` depends on `crossterm` for color in
  its message module, so "pure" layers transitively carry terminal code.
- **No plugin dispatch exists** (no `flox-*` PATH fallback, no hooks).
- **JSON output exists on exactly one command** (`envs`); there is no
  systemic `--json` pattern.
- **CODEOWNERS covers only `/cli/schemas/`**; ownership is otherwise
  implicit in commit history.

## Ground rules

- Analysis only. No changes outside `docs/architecture-analysis/`, `GOAL.md`,
  and `.claude/commands/goal.md`.
- Every claim in an output must cite a file path (and line where useful) or a
  command that reproduces the measurement.
- Where the analysis needs input only humans can provide (notably
  floxhub/floxdash capability requirements, Workstream B), proceed on
  **explicitly labeled assumptions** and record them in the output's
  "Assumptions" section rather than blocking.
- Each output ends with a "How to reproduce" section listing the commands or
  greps used, so the analysis can be re-run as the codebase moves.

## Output directory layout

```
docs/architecture-analysis/
├── REPORT.md                      # Overall interpretive report (see spec below)
├── A-command-audit.md             # Workstream A
├── B-sdk-fitness.md               # Workstream B
├── C-interactivity-inventory.md   # Workstream C
├── D-dependency-layering.md       # Workstream D
├── E-plugin-feasibility.md        # Workstream E
└── F-risk-map.md                  # Workstream F
```

---

## Workstream A — Command Audit Matrix

**Question answered:** Which commands are already thin, which are fat, and
which contain the hard problem (mid-operation interactivity)?

**Method:** For every command in `cli/flox/src/commands/`, measure:

| Metric | How |
|---|---|
| Total LOC | `wc -l` |
| Business-logic LOC in command file vs. delegated to SDK | targeted reading |
| `message::*` call count | grep |
| `Dialog` / `Select` / `Confirm` usage count | grep |
| Prompts mid-operation vs. only up front | reading |
| SDK call returns a structured result type? | reading |
| `--json` feasible today? | derived from above |

**Output:** `A-command-audit.md` — one row per command, sorted by migration
difficulty, plus a one-page summary naming the 3 thinnest and 3 fattest
commands with a paragraph of evidence each.

**Benefit:** Converts "separate CLI from API" into a **ranked backlog with
per-command effort estimates**. The "logic stranded in command file" column is
the candidate list for *additions* to `flox-rust-sdk` (feeds B). The
mid-operation-prompt column scopes the hard design work (feeds C).

---

## Workstream B — flox-rust-sdk Fitness Review (the reuse question)

**Question answered:** Can `flox-rust-sdk` serve as the shared API layer for
CLI + floxhub + floxdash, and exactly what must be **added** to or **removed**
from it?

**Method — three passes:**

1. **Surface inventory.** Catalog the SDK's public API (via `cargo
   public-api` if available in the dev shell, otherwise from `lib.rs` module
   exports). Classify each public item:
   - *clean operation* — structured in/out, no printing, no TTY assumption
   - *leaky* — prints, reads ambient state, assumes interactive flow
   - *internal* — should not be public at all
2. **Add candidates.** Cross-reference Workstream A: business logic stranded
   in command files that floxhub/floxdash would need (e.g. install's
   conflict resolution, activate's environment resolution). For each, name
   the source location and the SDK module it belongs in.
3. **Remove/relocate candidates.** SDK (and `flox-core`) contents that exist
   only for CLI presentation or that drag in dependencies a web service
   shouldn't carry (the `flox-core` → `crossterm` leak is the known first
   entry). Additionally, profile each major operation's **side effects**
   (filesystem writes, git subprocesses, Nix invocations, network) and flag
   which are incompatible with an in-process floxhub deployment vs. fine
   for floxdash.

**Inputs:** A capability sketch for floxhub and floxdash (what each needs to
*do*: read-only browsing? env mutation? builds?). If absent, assume:
floxdash = full CLI parity minus `activate`; floxhub = read operations plus
push/pull/publish-adjacent mutations; record this in "Assumptions".

**Output:** `B-sdk-fitness.md` with three explicit lists — **keep as-is**,
**add** (with source location), **remove/relocate** (with destination) — a
side-effect profile table, and a **verdict section**: "SDK is the API layer
directly" vs. "a thin facade crate (`flox-ops`) on top is warranted", with
the criteria that drove the verdict.

**Benefit:** The direct, evidence-backed answer to the reuse question. The
add/remove lists *are* the scope definition for the eventual refactor. The
side-effect profile answers the expensive-to-discover-late question of
whether floxhub can link the SDK in-process or needs a service boundary.

---

## Workstream C — Interactivity & Side-Effects Inventory

**Question answered:** What breaks headless use, and what is the one contract
for progress/input that all consumers share?

**Method:** Catalog every interaction point in the command layer: mid-
operation prompts (activate's service-start dialog, install's environment
selection, push's auth redirect), progress reporting (tracing-indicatif
spans), pager usage, exit-code mapping, ambient-state reads (env vars,
config, cwd). Classify each:

- **hoistable** — resolve before the operation runs (expected ~90%)
- **modelable** — operation returns a typed "needs input" outcome the caller
  loops on
- **structural** — can never be a pure API call (e.g. activate's shell exec)

**Output:** `C-interactivity-inventory.md` — the classified inventory plus a
short design memo recommending the progress/input contract, with the 3–4
hardest real cases worked through on paper (sequence sketches, no code).

**Benefit:** Interactivity is where CLI/API separations fail in practice;
everything else is mechanical. This makes the hardest design decision
**once, deliberately**, instead of improvising it per command. The
"structural" list sets honest expectations for what floxhub can never share.

---

## Workstream D — Dependency & Layering Analysis

**Question answered:** What is the actual crate graph, where does terminal
code leak below the UI layer, and what layering rule would be enforceable?

**Method:** Generate the workspace crate graph (`cargo tree` /
`cargo-depgraph` if available). Trace every path by which crossterm,
inquire, indicatif, and tracing-indicatif reach each crate. Document
violations of the implied layering. Draft — **as a proposal document, not a
committed config** — the layering policy and its enforcement mechanism
(cargo-deny ban list or an `xtask` lint).

**Output:** `D-dependency-layering.md` — ASCII dependency diagram, violations
list, and a one-page proposed layering policy including a draft enforcement
config (inline in the doc).

**Benefit:** Makes the target architecture **checkable rather than
aspirational** — documented architecture erodes; CI-enforced architecture
doesn't. The violations list is the cheapest first refactor when
implementation begins. The policy doubles as goal 4's structural definition
of "what kind of code lives where."

---

## Workstream E — Plugin System Feasibility Memo

**Question answered:** What would a gh-style plugin system cost, and what
contract must flox offer plugins?

**Method:** Comparative study of git/cargo/gh external-command dispatch
(PATH-based `name-*` lookup, precedence vs. built-ins, help integration).
Map onto flox: where the fallback hook lives in the bpaf parse, the plugin
environment contract (`FLOX_ENV` etc., auth token access policy, `--json`
availability so plugins never scrape human output), and security
considerations (PATH hijacking, built-in shadowing policy).

**Output:** `E-plugin-feasibility.md` (~2 pages): recommended dispatch model,
required plugin contract, dependencies on other workstreams (notably `--json`
coverage from B), and an effort estimate.

**Benefit:** Scopes goal 2 precisely — likely showing it is **small and
mostly independent**, sequencable as an early win. The contract list feeds
back into B: every datum a plugin needs is by definition something requiring
structured output.

---

## Workstream F — Risk Map & Review-Labeling Proposal

**Question answered:** Which code regions are high-consequence, and what
structure makes review intensity a clean-cut, path-based decision?

**Method:** Identify high-consequence regions (activation scripts in
`assets/`, `ld-floxlib/`, auth, git providers, manifest schema migration,
lockfile handling) using judgment, bug-fix commit density (`git log`
analysis), and the cautions already encoded in AGENTS.md. Compare against
current CODEOWNERS coverage. Propose a labeling taxonomy: which paths get
mandatory-review CODEOWNERS tiers, whether a `// TRICKY:` comment convention
(analogous to Rust's `// SAFETY:`) is worth adding, and how the target
directory structure from B/C makes "thin glue vs. load-bearing logic"
legible from the diff path alone.

**Output:** `F-risk-map.md` — a risk map (path × risk tier × rationale) and a
proposed CODEOWNERS/labeling scheme (proposal only).

**Benefit:** Delivers goal 4 directly: reviewers get clean-cut decisions
("touches tier-1 paths → mandatory careful review; touches only rendering →
light review") enforced by structure rather than memory.

---

## Final Output — REPORT.md (the overall interpretive report)

`REPORT.md` is **not a concatenation** of the workstream outputs. It is the
document a decision-maker reads *instead of* the outputs. It interprets every
output, and it must be extremely clear: every section grounded in a concrete
example from the codebase, every structural claim shown as a diagram.

**Required structure:**

1. **Executive summary** (≤1 page). What was found, what is recommended,
   what it costs, what it unblocks — in plain language for a reader who has
   never opened the codebase.

2. **Current vs. target architecture** — two ASCII diagrams side by side
   (crate/module level), with the delta narrated in prose. Example shape:

   ```
   CURRENT                                TARGET
   ┌──────────────────────────┐           ┌──────────────────────────┐
   │ flox (CLI binary)        │           │ flox (CLI binary)        │
   │  commands/* :            │           │  commands/* : parse+render│
   │   logic ⊕ prompts ⊕ I/O  │           │  (thin, low review tier) │
   └─────────────┬────────────┘           └───┬──────────┬───────────┘
                 │                            │          │ flox-* plugins
   ┌─────────────▼────────────┐           ┌───▼──────────▼───────────┐
   │ flox-rust-sdk            │           │ operations layer (SDK ±) │
   │  (mostly clean)          │           │  structured in/out, the  │
   └─────────────┬────────────┘           │  floxhub/floxdash surface│
   ┌─────────────▼────────────┐           └───┬──────────────────────┘
   │ flox-core (⚠ crossterm)  │           ┌───▼──────────────────────┐
   └──────────────────────────┘           │ flox-core (terminal-free)│
                                          └──────────────────────────┘
   ```

3. **One section per workstream output (A–F)**, each with exactly these
   subsections:
   - **Context** — what question this output answers and why it was asked.
   - **What the data says** — the findings, interpreted in prose; never a
     raw table dump. Tables only for short enumerable facts.
   - **Impact** — what becomes possible, cheaper, or safer because of this
     finding; what it would cost to ignore it.
   - **Example** — at least one concrete, named example from the codebase
     (file path, before/after sketch, or worked scenario).
   - **Diagram** — an ASCII diagram where the finding is structural (flows,
     layers, dependencies); omit only where a diagram would add nothing,
     and say why.

4. **Decision list** — every decision the analysis surfaces, each stated as
   a question, the recommendation, the evidence pointer (which output), and
   the consequence of deciding otherwise.

5. **Phased migration backlog** — analysis-only deliverable: ordered phases
   with effort estimates and what each unblocks, driven by the Workstream A
   ranking. (Executing the backlog is out of scope for this plan.)

6. **Assumptions & open inputs** — everything assumed (especially
   floxhub/floxdash capabilities) and what answer would change which
   conclusion.

**Benefit of REPORT.md:** the six outputs are evidence; this is the verdict.
It is the single artifact stakeholders debate and approve, after which
implementation is execution rather than design. The per-output
Context/Impact/Example/Diagram discipline guarantees no finding is presented
without its "so what".

---

## Sequencing

```
A (command audit) ──┬──> B (SDK fitness) ──┐
D (dependencies)  ──┘                      ├──> REPORT.md
                    ┌──> C (interactivity)─┤
E (plugins, anytime)───────────────────────┤
F (risk map, anytime)──────────────────────┘
```

- A and D are mechanical; run them first (parallelizable).
- B and C depend on A's matrix.
- E and F are independent and can run anytime.
- REPORT.md is written last and revised if any output changes.
