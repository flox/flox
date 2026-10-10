# Workstream D — Dependency & Layering Analysis

**Date:** 2026-06-11
**Status:** Analysis only. Nothing in this document is a committed config; the
enforcement config appears inline as a proposal, per GOAL.md ground rules.

## Context

This workstream answers: *what is the actual crate graph, where does terminal
code leak below the UI layer, and what layering rule would be enforceable?*
The workspace root `Cargo.toml` (`/home/user/flox/Cargo.toml:2-18`) declares 15
member crates under `cli/`. Because `cargo` is unavailable in this analysis
environment, the graph below was built by reading every member `Cargo.toml`
directly (all 15, plus the workspace root for shared dependency declarations)
and grepping every `src/` tree for usage sites. Only **normal** (`[dependencies]`)
edges appear in the graph; dev-dependency edges are noted where relevant but do
not ship in any binary and are excluded from the layering judgments.

The headline finding confirms GOAL.md's baseline: the workspace has a sensible
implicit layering (binary → SDK → domain crates → core), and **exactly one
structural leak breaks it** — `flox-core` depends on `crossterm` (and
`supports-color`) for two color-formatting helpers in
`cli/flox-core/src/util/message.rs`. Because `flox-core` sits at the bottom of
the graph, that single edge transitively contaminates six downstream crates,
including `flox-rust-sdk`, the candidate CLI/API separation layer. Every other
terminal/UI crate (`inquire`, `indicatif`, `tracing-indicatif`, `minus`) is
already correctly confined to the `flox` binary (plus `indicatif` in the
`mk_data` dev tool, which is legitimate).

## Workspace dependency diagram

Arrows point from dependent to dependency. Normal dependencies only.
`⚠ TERM:` marks direct dependencies on terminal/UI crates.

```
L3  BINARIES / PRESENTATION
┌─────────────────────────────────┐ ┌──────────────────┐ ┌────────────┐ ┌──────────────┐
│ flox  (CLI binary)              │ │ flox-activations │ │ xtask      │ │ mk_data      │
│ ⚠ TERM: crossterm inquire      │ │ (activation bin) │ │ (dev tool) │ │ (dev tool)   │
│   indicatif tracing-indicatif   │ │                  │ │            │ │ ⚠ TERM:     │
│   minus supports-color          │ │                  │ │            │ │   indicatif  │
│   textwrap(terminal_size)       │ │                  │ │            │ │ (legitimate) │
└─┬──┬───┬────┬────┬────┬────┬────┘ └───────┬─────┬────┘ └──┬─────┬───┘ └──────────────┘
  │  │   │    │    │    │    └──────────────┘     │         │     │      (flox also depends
  │  │   │    │    │    │   (flox → flox-activations)       │     │       directly on every
  ▼  │   │    │    │    │                                   │     │       crate below; edges
┌──────┐ │    │    │    │                                   │     │       drawn once)
│ beta │─┼──┐ │    │    │                                   │     │
└──────┘ │  ▼ ▼    │    │                                   │     │
L2       │ ┌────────────────┐                               │     │
         │ │ flox-rust-sdk  │◀──────────────────────────────┘     │
         │ └─┬───┬───┬───┬──┘                                     │
         │   │   │   │   │                                        │
L1       │   ▼   │   │   ▼                                        ▼
  ┌────────────┐ │   │ ┌──────────────────┐              ┌───────────────┐
  │ flox-      │ │   │ │ nef-lock-catalog │              │ flox-manifest │
  │ catalog    │◀┼───┼─┤ (lib + bin)      │              └─┬─────┬─────┬─┘
  └─────┬──────┘ │   │ └────────┬─────────┘                │     │     │
        │        │   └──────────┼──────────────────────────┼─────┼─────┘
        ▼        │              │              ┌───────────┘     │  (sdk → flox-manifest,
  ┌────────────────┐            │              │                 │   sdk → systemd)
  │ catalog-api-v1 │◀───────────┼──────────────┼─────────────────┘
  └────────────────┘            │              │  (flox-manifest → catalog-api-v1)
L0                              ▼              ▼
              ┌──────────────────────────────────────────┐
              │ flox-core                                │
              │ ⚠ TERM: crossterm, supports-color       │   ◀── THE LEAK
              │   (only in src/util/message.rs)          │
              └────────────────────┬─────────────────────┘
                                   ▼
   ┌───────────┐  ┌─────────┐  ┌─────────────┐  ┌──────────────────┐
   │ shell_gen │  │ systemd │  │ flox-events │  │ flox-test-utils  │
   │           │  │         │  │ (no         │  │ (dev-deps only)  │
   │           │  │         │  │  consumers) │  │                  │
   └───────────┘  └─────────┘  └─────────────┘  └──────────────────┘
   (flox-core → shell_gen; flox-activations → shell_gen; flox → shell_gen;
    systemd ← flox-manifest, flox-rust-sdk)
```

Complete normal-dependency edge list (workspace-internal), each cited to the
declaring manifest line:

| From | To | Declaration |
|---|---|---|
| flox | beta | `cli/flox/Cargo.toml:11` |
| flox | flox-catalog | `cli/flox/Cargo.toml:19` |
| flox | flox-rust-sdk | `cli/flox/Cargo.toml:20` |
| flox | flox-core | `cli/flox/Cargo.toml:56` |
| flox | flox-activations | `cli/flox/Cargo.toml:57` |
| flox | shell_gen | `cli/flox/Cargo.toml:59` |
| flox | nef-lock-catalog | `cli/flox/Cargo.toml:61` |
| flox | flox-manifest | `cli/flox/Cargo.toml:62` |
| beta | flox-rust-sdk | `cli/beta/Cargo.toml:9` |
| xtask | flox-rust-sdk | `cli/xtask/Cargo.toml:9` |
| xtask | flox-manifest | `cli/xtask/Cargo.toml:10` |
| flox-rust-sdk | flox-catalog | `cli/flox-rust-sdk/Cargo.toml:10` |
| flox-rust-sdk | flox-core | `cli/flox-rust-sdk/Cargo.toml:16` |
| flox-rust-sdk | nef-lock-catalog | `cli/flox-rust-sdk/Cargo.toml:23` |
| flox-rust-sdk | systemd | `cli/flox-rust-sdk/Cargo.toml:34` |
| flox-rust-sdk | flox-manifest | `cli/flox-rust-sdk/Cargo.toml:54` |
| flox-manifest | systemd | `cli/flox-manifest/Cargo.toml:15` |
| flox-manifest | flox-core | `cli/flox-manifest/Cargo.toml:22` |
| flox-manifest | catalog-api-v1 | `cli/flox-manifest/Cargo.toml:24` |
| flox-manifest | flox-test-utils (optional, `tests` feature) | `cli/flox-manifest/Cargo.toml:30` |
| flox-catalog | catalog-api-v1 | `cli/flox-catalog/Cargo.toml:8` |
| nef-lock-catalog | flox-catalog | `cli/nef-lock-catalog/Cargo.toml:16` |
| nef-lock-catalog | flox-core | `cli/nef-lock-catalog/Cargo.toml:17` |
| flox-activations | flox-core | `cli/flox-activations/Cargo.toml:20` |
| flox-activations | shell_gen | `cli/flox-activations/Cargo.toml:33` |
| flox-core | shell_gen | `cli/flox-core/Cargo.toml:23` |

Crates with **no** workspace-internal normal dependencies: `catalog-api-v1`,
`flox-events`, `flox-test-utils`, `mk_data`, `shell_gen`, `systemd`.

## Per-crate table

LOC measured with `find <crate>/src -name '*.rs' | xargs wc -l` (totals include
`#[cfg(test)]` modules). "Terminal-crate deps" means a **direct** normal
dependency on crossterm / inquire / indicatif / tracing-indicatif / minus /
supports-color / textwrap(terminal_size).

| Crate | LOC | Direct workspace deps | Direct terminal-crate deps |
|---|---:|---|---|
| flox (binary) | 25,971 | beta, flox-catalog, flox-rust-sdk, flox-core, flox-activations, shell_gen, nef-lock-catalog, flox-manifest | crossterm (`cli/flox/Cargo.toml:16`), indicatif (`:24`), inquire (`:26`), supports-color (`:40`), textwrap+terminal_size (`:44`), tracing-indicatif (`:51`), minus (`:60`) |
| flox-rust-sdk | 37,468 | flox-catalog, flox-core, nef-lock-catalog, systemd, flox-manifest | **none** |
| flox-manifest | 11,067 | systemd, flox-core, catalog-api-v1 (+ flox-test-utils, tests-only) | none |
| flox-activations (binary) | 8,426 | flox-core, shell_gen | none (tracing-subscriber `ansi` feature, `cli/flox-activations/Cargo.toml:18` — log styling for its own stderr; legitimate, it is a binary) |
| catalog-api-v1 | 6,389 | — | none |
| flox-core | 3,462 | shell_gen | **crossterm (`cli/flox-core/Cargo.toml:12`), supports-color (`cli/flox-core/Cargo.toml:24`)** ← the leak |
| flox-catalog | 2,556 | catalog-api-v1 | none |
| flox-test-utils | 1,258 | — | none |
| nef-lock-catalog (lib+bin) | 1,122 | flox-catalog, flox-core | none |
| systemd | 603 | — | none |
| mk_data (dev tool) | 553 | — | indicatif (`cli/mk_data/Cargo.toml:12`) — legitimate, it is a standalone progress-bar UI |
| shell_gen | 395 | — | none |
| flox-events | 152 | — (and **nothing depends on it**) | none |
| xtask (dev tool) | 94 | flox-rust-sdk, flox-manifest | none |
| beta | 22 | flox-rust-sdk | none |

Workspace-level declarations of the terminal crates (for completeness):
crossterm `/home/user/flox/Cargo.toml:39`, inquire `:65`, indicatif `:66`,
supports-color `:97`, textwrap(terminal_size) `:106`, tracing-indicatif `:114`.
`minus` is not a workspace dependency; it is declared only in
`cli/flox/Cargo.toml:60`.

## How terminal crates reach each crate (direct + transitive)

**crossterm** — the only terminal crate with transitive reach:

- Direct: `flox` (`cli/flox/Cargo.toml:16`), `flox-core` (`cli/flox-core/Cargo.toml:12`).
- Transitive, every path via the single `flox-core` edge:
  - `flox-activations` → flox-core (`cli/flox-activations/Cargo.toml:20`)
  - `flox-manifest` → flox-core (`cli/flox-manifest/Cargo.toml:22`)
  - `nef-lock-catalog` → flox-core (`cli/nef-lock-catalog/Cargo.toml:17`)
  - `flox-rust-sdk` → flox-core (`cli/flox-rust-sdk/Cargo.toml:16`); also via
    flox-manifest (`:54`) and via nef-lock-catalog (`:23`) — three parallel paths
  - `beta` → flox-rust-sdk (`cli/beta/Cargo.toml:9`)
  - `xtask` → flox-rust-sdk (`cli/xtask/Cargo.toml:9`) and flox-manifest (`:10`)

  Result: **8 of 15 workspace crates carry crossterm**; only one (`flox`)
  has a UI reason to. Cutting the single `flox-core → crossterm` edge cleans
  all six transitive carriers at once.
- Not reached: catalog-api-v1, flox-catalog, flox-events, flox-test-utils,
  shell_gen, systemd, mk_data.

**supports-color** — identical topology to crossterm (direct in `flox`
`cli/flox/Cargo.toml:40` and `flox-core` `cli/flox-core/Cargo.toml:24`; same
six transitive carriers via flox-core). Same root cause, same fix.

**inquire** — direct in `flox` only (`cli/flox/Cargo.toml:26`); used only under
`cli/flox/src/utils/dialog.rs` and friends. No transitive reach. Clean.

**indicatif** — direct in `flox` (`cli/flox/Cargo.toml:24`) and `mk_data`
(`cli/mk_data/Cargo.toml:12`, a dev-only data generator whose whole purpose is
a progress UI). No transitive reach into library crates. Clean.

**tracing-indicatif** — direct in `flox` only (`cli/flox/Cargo.toml:51`); used
in `cli/flox/src/utils/init/logger.rs:7,215,286`. Clean.

**minus** (pager) — direct in `flox` only (`cli/flox/Cargo.toml:60`); used in
`cli/flox/src/utils/message.rs:14`. Clean.

## Verifying the known leak: flox-core → crossterm

- **Manifest edge:** `cli/flox-core/Cargo.toml:12` — `crossterm.workspace = true`
  (and `cli/flox-core/Cargo.toml:24` — `supports-color.workspace = true`).
- **The entire usage surface is one 29-line file**,
  `cli/flox-core/src/util/message.rs` (exported via
  `cli/flox-core/src/util/mod.rs:1`):
  - line 3: `use crossterm::style::Stylize;`
  - line 7: `"✘".red().to_string()` in `format_error`
  - line 17: `"✔".green().to_string()` in `format_updated`
  - lines 23–29: `stdout_supports_color()` / `stderr_supports_color()` wrapping
    `supports_color::on(...)` — the only `supports-color` usage in the crate.
- **No other file in flox-core uses either crate** (grep for `crossterm` and
  `supports_color` across `cli/flox-core/src/` returns only `util/message.rs`).
- **Why it exists:** the helpers are shared by the two binaries —
  `cli/flox/src/utils/message.rs:7-8` (re-exports and builds the CLI's
  `message::*` family on top) and `cli/flox-activations/src/message.rs:3`
  (prints activation errors/updates to stderr). Two ~10-line formatting
  functions were pushed to the bottom of the graph so two binaries could share
  them, at the cost of every intermediate crate carrying a terminal-event
  library (crossterm's `event-stream` feature is even enabled workspace-wide,
  `/home/user/flox/Cargo.toml:39`, pulling in mio/signal machinery).

## Violations list

Severity reflects impact on the GOAL.md targets (SDK reusable by floxhub/
floxdash; enforceable layering), not runtime breakage — nothing here is a bug.

1. **HIGH — `flox-core` → `crossterm`.**
   Evidence: `cli/flox-core/Cargo.toml:12`; usage `cli/flox-core/src/util/message.rs:3,7,17`;
   consumers `cli/flox/src/utils/message.rs:7`, `cli/flox-activations/src/message.rs:3`.
   Impact: contaminates flox-rust-sdk, flox-manifest, flox-activations,
   nef-lock-catalog, beta, and xtask (manifest lines in the reach section
   above). Any web/in-process consumer of the SDK links a terminal-event
   library. Cheapest first refactor: move `format_error`/`format_updated` up
   into the binaries (or a tiny new presentation crate), or inline the two
   ANSI escape sequences without crossterm.

2. **MEDIUM — `flox-core` → `supports-color`.**
   Evidence: `cli/flox-core/Cargo.toml:24`; usage `cli/flox-core/src/util/message.rs:23-29`.
   Same module, same consumers, same fix as #1. Separately noteworthy because
   it is an *ambient terminal-state read* (inspects stdout/stderr at call
   time) sitting in the layer that is supposed to be context-free.

3. **MEDIUM — `nef-lock-catalog` → `flox-core` for one type.**
   Evidence: `cli/nef-lock-catalog/Cargo.toml:17`; the only usages are
   `use flox_core::Version` at `cli/nef-lock-catalog/src/nix_build_lock.rs:6`
   and `cli/nef-lock-catalog/src/nix_build_config.rs:7` (`Version` is
   re-exported at `cli/flox-core/src/lib.rs:21`).
   Impact: a catalog-locking library inherits crossterm, sentry, and sysinfo
   for a version-marker type. Symptom of flox-core being a grab-bag; either
   `Version` moves to a smaller leaf or the edge is dropped.

4. **LOW — `flox-manifest` → `reqwest` for a URL type.**
   Evidence: `cli/flox-manifest/Cargo.toml:20`; sole usage
   `cli/flox-manifest/src/raw/mod.rs:10` (`use reqwest::Url;` — a re-export of
   `url::Url`). Not a terminal crate, but a full HTTP client linked into the
   manifest-parsing layer that floxhub would embed. Replace with the `url`
   crate (already a workspace dependency, `/home/user/flox/Cargo.toml:116`).

5. **LOW — `flox-core` → `sentry`.**
   Evidence: `cli/flox-core/Cargo.toml:20`; `cli/flox-core/src/sentry.rs:6,18-19`
   (telemetry init reading `FLOX_SENTRY_DSN` from the environment).
   Observability bootstrap is a binary/presentation concern living in the
   foundation layer. Flagged for Workstream B's remove/relocate list rather
   than as a terminal leak.

6. **INFO — `flox-events` has zero consumers.**
   Evidence: it appears only in `/home/user/flox/Cargo.toml:6,46` and its own
   manifest; no member `Cargo.toml` declares `flox-events.workspace = true`
   (grep for `flox-events|flox_events` across the repo matches only those
   lines plus `Cargo.lock:1589`). Dead or not-yet-wired; either way the
   layering policy should assign it a layer now so it doesn't grow unanchored.

**Checked and found clean (distinguishing legitimate uses):**

- `flox-rust-sdk` production code contains **no** `println!`/`eprintln!`/
  `print!`/`eprint!`. The six grep hits are all inside `#[cfg(test)]` modules:
  `src/providers/buildenv.rs:1670,1714` (module starts `:1462`),
  `src/providers/flake_installable_locker.rs:283` (module `:185`),
  `src/providers/services/process_compose.rs:1435` (module `:1045`),
  `src/providers/git.rs:1596` (module `:1413`). No `io::stdout()`/`stderr()`
  writes either. This confirms the GOAL.md baseline that the SDK is
  print-clean apart from the transitive crossterm carry.
- `flox-core`, `flox-catalog`, `flox-manifest`: zero print-macro hits in `src/`.
- `flox-activations` prints (`src/message.rs:6,10`, `src/cli/activate.rs:179`,
  `src/cli/prepend_and_dedup.rs:23`, `src/cli/fix_fpath.rs:19`,
  `src/attach.rs:534`) — legitimate: it is a binary, and several of these
  (e.g. `attach.rs:534` `print!("{script}")`) write generated shell code to
  stdout for a shell to `eval`, which is its job, not user-facing decoration.
- `mk_data` → indicatif: legitimate; it is a standalone dev tool whose output
  *is* a progress UI.

## Proposed layering policy (proposal only)

### Layers

| Layer | Name | Crates | Allowed workspace deps | Banned dependencies |
|---|---|---|---|---|
| L3 | Presentation / binaries | `flox`, `flox-activations`, `mk_data`, `xtask` | any | — (terminal crates allowed here only) |
| L2 | Operations (the CLI/API surface) | `flox-rust-sdk`, `beta` | L0–L2 | crossterm, inquire, indicatif, tracing-indicatif, minus, supports-color, dialoguer, console, termion, terminal_size; print macros outside `#[cfg(test)]` |
| L1 | Domain services | `flox-catalog`, `flox-manifest`, `nef-lock-catalog`, `catalog-api-v1` | L0–L1 | everything banned in L2, plus deps on L2/L3 crates |
| L0 | Foundation | `flox-core`, `shell_gen`, `systemd`, `flox-events` | L0 only | everything banned in L1, plus network clients (reqwest/hyper) and telemetry (sentry) |
| — | Test-only | `flox-test-utils` | consumed via dev-deps / `tests` features only | n/a |

Rules in prose:

1. **Terminal/UI crates are direct dependencies of L3 binaries only.** A
   library crate that wants colored output returns *data* (an enum, a typed
   message) and lets the binary render it.
2. **Dependencies point strictly downward.** No L1 crate names an L2 crate; no
   L0 crate names anything above L0. (Today this already holds; the policy
   makes it permanent.)
3. **No user-facing printing below L3** in non-test code. Writing generated
   shell code or subprocess stdin payloads is I/O-as-function-result and is
   exempt; `message::*`-style decoration is not.
4. **L0 is context-free:** no ambient terminal probing (`supports-color`,
   `is_terminal` for decoration), no network, no telemetry init. This is what
   makes L0 safe for any future consumer including floxhub in-process.
5. To comply, the current tree needs exactly the fixes in violations #1–#5;
   #1/#2 (move `format_error`/`format_updated` out of
   `cli/flox-core/src/util/message.rs` into the two binaries or a new L3-only
   helper crate) bring 8 crates into compliance in one change.

### Draft enforcement config (inline proposal — do not commit from this doc)

Option A — **cargo-deny** (no `deny.toml` exists today; checked
`/home/user/flox/deny.toml` and `/home/user/flox/cli/deny.toml`). cargo-deny's
`wrappers` field permits a banned crate *only* where it is a direct dependency
of a listed crate, which encodes rule 1 exactly:

```toml
# deny.toml (PROPOSAL) — run with: cargo deny check bans
[bans]
multiple-versions = "warn"
deny = [
    # Terminal/UI crates: direct deps of presentation binaries only.
    { name = "crossterm",         wrappers = ["flox"] },
    { name = "inquire",           wrappers = ["flox"] },
    { name = "indicatif",         wrappers = ["flox", "mk_data"] },
    { name = "tracing-indicatif", wrappers = ["flox"] },
    { name = "minus",             wrappers = ["flox"] },
    { name = "supports-color",    wrappers = ["flox"] },
    # Belt-and-suspenders: terminal crates nothing should add in the first place.
    { name = "dialoguer" },
    { name = "console" },
    { name = "termion" },
]
```

This config **fails today** on the `flox-core → crossterm` and
`flox-core → supports-color` edges — which is the point: adopt it in the same
change that fixes violations #1/#2, and the leak cannot regress.

Limitations: cargo-deny bans crates, not workspace-edge directions, so rule 2
(L1 must not depend on L2) and rule 4 (no reqwest/sentry in L0 while L1/L2
legitimately use both) need Option B.

Option B — **xtask layering lint** (the workspace already has an `xtask`
crate, `cli/xtask/Cargo.toml`, wired for schema generation). Sketch:

```text
xtask lint-layers:
  1. layers = { flox:3, flox-activations:3, mk_data:3, xtask:3,
                flox-rust-sdk:2, beta:2,
                flox-catalog:1, flox-manifest:1, nef-lock-catalog:1, catalog-api-v1:1,
                flox-core:0, shell_gen:0, systemd:0, flox-events:0 }
     extra_bans = { 0: [reqwest, sentry, crossterm, supports-color, ...TERM],
                    1: [...TERM], 2: [...TERM] }
  2. meta = cargo_metadata::MetadataCommand::new().exec()
  3. for each workspace member m, for each NORMAL (non-dev) dependency d:
       if d is a workspace member and layers[d] > layers[m]  -> error (upward edge)
       if d.name in extra_bans[layers[m]]                    -> error (banned dep)
  4. unknown member (new crate without a layer assignment)   -> error (forces a decision)
```

Run as `cargo run -p xtask -- lint-layers` in the same CI job as clippy.
Optionally pair with clippy's `disallowed-macros` (a `clippy.toml` already
exists at `cli/clippy.toml`; per-crate `clippy.toml` files in L0–L2 crates can
ban `std::println`/`std::eprintln`) to enforce rule 3 mechanically.

Recommendation: adopt **both** — cargo-deny for the terminal-crate ban (zero
code, catches transitive sneak-ins) and the xtask lint for layer-direction and
per-layer bans (cargo-deny cannot express them). Step 4 of the xtask sketch is
what makes the architecture checkable-by-default for future crates, which is
GOAL.md goal 4's structural definition of "what kind of code lives where".

## How to reproduce

All commands from the repo root `/home/user/flox`. None require the Nix dev
shell except the optional `cargo` ones.

```bash
# 1. Enumerate workspace members and shared dep declarations
cat Cargo.toml                          # members: lines 2-18; workspace deps incl.
                                        # crossterm:39 inquire:65 indicatif:66
                                        # supports-color:97 tracing-indicatif:114
for f in cli/*/Cargo.toml; do echo "== $f"; cat -n "$f"; done

# 2. Terminal-crate usage sites (direct evidence)
grep -rn "crossterm" cli --include='*.rs'
grep -rn -E "inquire|indicatif|tracing_indicatif|minus::" cli --include='*.rs' -l
grep -rn "supports_color" cli --include='*.rs'

# 3. The leak, precisely
sed -n '1,30p' cli/flox-core/src/util/message.rs
grep -rn "format_error\|format_updated" cli --include='*.rs'

# 4. Print-macro audit of sub-binary crates (then check hits are in cfg(test))
for c in flox-rust-sdk flox-core flox-catalog flox-manifest flox-activations; do
  echo "== $c"; grep -rn -E '(^|[^a-z_])(println!|eprintln!|eprint!|print!)' cli/$c/src
done
# for each flox-rust-sdk hit, confirm the nearest preceding '#[cfg(test)]':
awk 'NR<=1670 && /#\[cfg\(test\)\]/{t=NR} END{print t}' cli/flox-rust-sdk/src/providers/buildenv.rs

# 5. LOC per crate
for c in cli/*/; do n=$(find $c/src -name '*.rs' 2>/dev/null | xargs wc -l 2>/dev/null \
  | tail -1 | awk '{print $1}'); echo "$c: ${n:-0}"; done

# 6. Single-type dependencies (violations 3 and 4)
grep -rn "flox_core" cli/nef-lock-catalog/src
grep -rn "reqwest" cli/flox-manifest/src

# 7. Orphan-crate check
grep -rn "flox-events\|flox_events" --include='Cargo.toml' .

# 8. (with cargo available) cross-check the transitive reach claims
cargo tree -i crossterm --workspace        # who pulls crossterm, all paths
cargo tree -p flox-rust-sdk -e normal | grep -E 'crossterm|inquire|indicatif|minus'
```
