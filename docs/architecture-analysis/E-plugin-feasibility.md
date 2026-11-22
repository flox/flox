# Workstream E — Plugin System Feasibility Memo

Date: 2026-06-11
Plan: `GOAL.md` (Workstream E)
Status: analysis only — nothing in this memo is implemented.

## Context

Goal 2 of `GOAL.md` is a gh-style plugin system: external `flox-*`
executables discovered on `PATH`, with a documented contract. This memo
answers: what would it cost, what contract must flox offer plugins, and what
must exist first.

## Prior art comparison

| | git | cargo | gh |
|---|---|---|---|
| Discovery | `git-<name>` on `PATH` + `GIT_EXEC_PATH` | `cargo-<name>` on `PATH` | managed extensions dir (`~/.local/share/gh/extensions`), installed via `gh extension install` |
| Built-in precedence | built-ins always win | built-ins always win | built-ins always win; extensions cannot shadow |
| Invocation | `exec git-<name> <args>` | `exec cargo-<name> <name> <args>` (subcommand name repeated as argv[1]) | runs extension binary/script with args |
| Data access for plugins | plumbing commands (stable, parseable output) | `cargo metadata --format-version 1` (versioned JSON) | `gh api`, `gh ... --json`, `gh auth token` |
| Auth handoff | n/a (local) | n/a | explicit: extension calls `gh auth token`; token never auto-exported |
| Listing/help | `git help -a` lists externals | `cargo --list` includes externals | `gh extension list` |

Two models emerge: **bare PATH dispatch** (git/cargo — zero infrastructure,
plugins are just executables) and **managed extensions** (gh — install,
update, list lifecycle). gh's model is strictly a superset built on top of
the same dispatch primitive.

## Current state in flox (evidence)

- Top-level parse: `cli/flox/src/main.rs:131`
  (`commands::flox_cli().run_inner(Args::current_args())`). The `Commands`
  enum is closed: `cli/flox/src/commands/mod.rs:484-500`.
- Unknown subcommand today: bpaf returns `ParseFailure::Stderr`, printed at
  `cli/flox/src/main.rs:139-141`, exit code 1. **There is no fallback hook
  and no prior plugin art in the repo.**
- Auth token: stored as `floxhub_token` in `~/.config/flox/flox.toml`
  (`cli/flox/src/config/mod.rs:70`, location logic `:256-299`), also
  readable from `FLOX_FLOXHUB_TOKEN`
  (`cli/flox-rust-sdk/src/models/floxmeta.rs:26`).
- Version: `flox --version` prints a clean semver-style string
  (`cli/flox/src/main.rs:75-77`; format parsed in
  `cli/flox-core/src/data/flox_version.rs`) — suitable for plugin
  compatibility checks.
- Structured output: only `envs` has `--json`
  (`cli/flox/src/commands/envs.rs:33-35`). **This is the long pole**: every
  datum a plugin needs from flox must be available without scraping human
  output.
- Activation env contract already exists and is plugin-relevant:
  `FLOX_ENV`, `FLOX_ENV_CACHE`, `FLOX_ENV_DESCRIPTION`, `FLOX_ENV_PROJECT`,
  `FLOX_ENV_DIRS` (see `cli/flox-activations/src/attach_diff/mod.rs`),
  plus `_FLOX_ACTIVE_ENVIRONMENTS` (`cli/flox-core/src/activate/vars.rs:11`).

## Recommended dispatch model

**Phase 1 (the primitive): git/cargo-style bare PATH dispatch.**

- Naming: executable `flox-<name>` on `PATH`; `flox <name> [args…]` execs it
  with the remaining args when `<name>` is not a built-in.
- **Dispatch point:** do *not* hook `ParseFailure::Stderr` in
  `main.rs:139` — bpaf returns the same failure variant for bad flags on
  *valid* commands, so intercepting it risks shadowing real usage errors.
  Instead, peek at the first positional token before the bpaf parse (in
  `main()` around `cli/flox/src/main.rs:131`): if it matches no built-in
  command name and `flox-<token>` exists on `PATH`, exec it; otherwise fall
  through to bpaf for the normal error. This is how cargo behaves and keeps
  bpaf's help/error output untouched.
- Built-ins always win; shadowing is not permitted. Document this.

**Phase 2 (optional, later): gh-style managed extensions** (`flox plugin
install/list/upgrade`) layered on the same dispatch primitive, with a
dedicated extensions directory prepended to the search path. Not needed for
the primitive to be useful; decide after observing Phase 1 adoption.

## Required plugin contract

What flox must guarantee to a `flox-*` subprocess:

1. **Environment.** Pass through the caller's environment plus:
   `FLOX_VERSION` (set explicitly — note `main.rs:51` currently *removes*
   it before parsing), `_FLOX_SUBSYSTEM_VERBOSITY`
   (`cli/flox/src/main.rs:117-120`), and `FLOX_DISABLE_METRICS` when set.
   Inside an activated environment, the existing `FLOX_ENV*` contract is
   already inherited for free.
2. **Auth: explicit, not ambient.** Follow gh's model — do **not**
   auto-export `FLOX_FLOXHUB_TOKEN` to every plugin. Plugins that need auth
   should call `flox auth token` (a new, trivial built-in that prints the
   token) or read the documented config path. This makes token access an
   auditable, opt-in act by the plugin rather than something every spawned
   process silently receives.
3. **Structured data via `--json`.** Plugins must never scrape human
   output. Minimum viable JSON surface for useful plugins: `envs` (exists),
   `list`, `search`/`show`, and a machine-readable `--version`. The full
   rollout is Workstream B's structured-result work; the plugin system can
   ship with the minimum set.
4. **Exit codes pass through** unchanged; flox adds nothing.
5. **Stability statement.** A short doc page declaring the contract
   (env vars, JSON shapes, dispatch rules) stable, with a version field in
   any JSON output (`cargo metadata --format-version` is the model).

## Security considerations

- **PATH hijacking:** dispatch inherits the user's `PATH` trust model, same
  as git/cargo/gh. Mitigations: never dispatch when running as root unless
  explicitly enabled; document that plugins execute with full user
  privileges; Phase 2's dedicated extensions dir narrows the search surface.
- **No built-in shadowing** (enforced by checking built-ins first).
- **Token policy** is the main genuine decision — recommendation above
  (explicit `flox auth token`) is the conservative default.

## Dependencies on other workstreams

- **Workstream B/A (`--json` coverage):** the dispatch primitive has no
  dependency, but plugin *usefulness* scales directly with structured
  output coverage. Ship dispatch with the minimum JSON set; grow with B.
- **Workstream D (layering):** none — dispatch lives entirely in the `flox`
  binary's entry path, which is the correct layer.

## Effort estimate

| Item | Estimate |
|---|---|
| PATH dispatch + built-in precedence check in `main.rs` | 1–2 days |
| `flox auth token` built-in | < 1 day |
| Contract documentation page | 1 day |
| Minimum `--json` additions (`list`, `search`) | 2–4 days (depends on B's findings per command) |
| Phase 2 managed extensions | ~2 weeks, defer |

Conclusion: **goal 2 is small and mostly independent**, as hypothesized in
`GOAL.md`. It is sequencable as an early win; only the token-access policy
and the minimum JSON set need a deliberate decision first.

## Assumptions

- floxhub/floxdash do not consume plugins; plugins are a CLI-only surface.
- Plugin authors are external/community; therefore the contract must be
  documented and versioned from day one.

## How to reproduce

```sh
# dispatch/failure path
sed -n '125,150p' cli/flox/src/main.rs
sed -n '484,500p' cli/flox/src/commands/mod.rs
# token storage and env var
grep -n "floxhub_token" cli/flox/src/config/mod.rs
grep -rn "FLOX_FLOXHUB_TOKEN" cli/flox-rust-sdk/src/models/floxmeta.rs
# --json coverage
grep -rn "json" cli/flox/src/commands/*.rs | grep -i "bpaf\|flag"
# env var contract
cat cli/flox-core/src/vars.rs cli/flox-core/src/activate/vars.rs
```
