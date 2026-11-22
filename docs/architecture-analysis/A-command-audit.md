# Workstream A — Command Audit Matrix

**Date:** 2026-06-11

This audit measures every user-facing command wired in
`cli/flox/src/commands/mod.rs` (the `Manage`/`Use`/`Discover`/`Modify`/`Share`/
`Admin` groups; `Internal` and `Beta` commands are excluded) against the
metrics defined in GOAL.md Workstream A: size, how much business logic is
stranded in the command file rather than delegated to `flox-rust-sdk`, how much
terminal I/O it performs (`message::` calls, `Dialog`/`Select`/`Confirm`
usage), whether prompts occur **mid-operation** (after an SDK operation has
started, or between steps of a multi-step flow) versus only **up-front**
(environment/auth selection before the operation), whether the underlying SDK
calls return structured result types, and whether `--json` output would be
feasible today. The matrix is sorted by estimated migration difficulty toward
the target shape (`parse → call operation → render`), easiest first. All line
numbers refer to the working tree at the date above.

## Reading the matrix

- **LOC / non-test** — `wc -l` of the file(s); non-test excludes the
  `#[cfg(test)]` module (line where `mod tests` begins).
- **Biz-logic %** — estimated share of *non-test* LOC that is business logic
  living in the command file rather than delegated to the SDK. Rendering
  (formatting structured results into text) and bpaf arg structs are *not*
  counted as business logic; orchestration, error recovery, subprocess
  invocation, and filesystem mutation are.
- **msg** — count of lines containing `message::` (grep, per file).
- **dlg** — count of real `Dialog { … }` / `Dialog::can_prompt()` /
  `typed: Confirm|Select` usages (a naive grep for `Select|Confirm` is
  inflated by unrelated type names like `EnvironmentSelect`, `CommandSelect`,
  `BaseCatalogUrlSelect`; those are excluded).
- **Prompts** — `none`, `up-front` (env selection / auth / confirmation
  before the operation), or `MID` (after the operation started or between
  steps).
- All commands additionally inherit two *up-front* shared prompts from
  `commands/mod.rs`: environment disambiguation
  (`query_which_environment`, `mod.rs:1224`) and the auth login fallback
  (`ensure_auth`, `mod.rs:1397`).

## The matrix (easiest migration first)

| # | Command | File(s) (`cli/flox/src/commands/`) | LOC | non-test | Biz-logic % | msg | dlg | Prompts | SDK result type | `--json` feasible today? |
|---|---------|------------------------------------|-----|----------|-------------|-----|-----|---------|-----------------|--------------------------|
| 1 | `envs` | `envs.rs` | 292 | 241 | ~15% | 5 | 0 | none | reads `EnvRegistry` via `env_registry::garbage_collect` | **already has `--json`** (`envs.rs:35`) |
| 2 | `show` | `show.rs` | 298 | 193 | ~10% | 0 | 0 | none | `PackageDetails` (catalog client) | yes — pure render of one structured response |
| 3 | `search` | `search.rs` | 147 | 147 | ~15% | 3 | 0 | none | `SearchResults` | **already has `--json`** (`search.rs:34`) |
| 4 | `include upgrade` | `include.rs` | 108 | 108 | ~5% | 5 | 0 | none | `UpgradeResult` (`Environment::include_upgrade`, sdk `environment/mod.rs:143`) | yes |
| 5 | `uninstall` | `uninstall.rs` | 129 | 129 | ~10% | 3 | 0 | none | `UninstallationAttempt` (sdk `environment/mod.rs:97`) | yes |
| 6 | `upgrade` | `upgrade.rs` | 274 | 167 | ~10% | 8 | 0 | none | `UpgradeResult` + `SingleSystemUpgradeDiff` (sdk `core_environment.rs:1000`) | yes — `render_diff` (`upgrade.rs:152`) is a trivial render fn |
| 7 | `push` | `push.rs` | 518 | 263 | ~15% | 5 | 0 | up-front only (`ensure_auth`, `push.rs:55`) | `PushResult::{Updated,UpToDate}` (sdk `managed_environment.rs:1209`) | yes — the cleanest model in the codebase |
| 8 | `delete` | `delete.rs` | 89 | 89 | ~10% | 2 | 3 | up-front only (Confirm, `delete.rs:67-77`) | `Result<()>` from `delete()` — no payload, none needed | yes (with `--force` or after confirm) |
| 9 | `generations` (list/history/switch/rollback) | `generations/*.rs` (5 files) | 812 | ~590 | ~20% | 4 | 0 | none | `AllGenerationsMetadata`, `GenerationId` (sdk `generations.rs`) | **list & history already have `--json`** (`generations/list.rs:47`, `history.rs:42`) |
| 10 | `list` | `list.rs` | 977 | 443 | ~15% | 4 | 0 | none | `Lockfile::list_packages` → `Vec<PackageToList>` | yes — most of the file is alternate text renderers |
| 11 | `config` | `general.rs` | 253 | 186 | ~40% | 1 | 0 | none | none — mutates config TOML via `update_config` (`general.rs:147`) | yes for `--list`; set/delete need no output |
| 12 | `services status/logs/stop/persist` | `services/{status,logs,stop,persist}.rs` | 582 | ~460 | ~30% | 6 | 0 | none | `ProcessStates`/`ProcessState` (sdk `process_compose.rs`) | **status already has `--json`** (`services/status.rs:30`) |
| 13 | `containerize` | `containerize/mod.rs`, `containerize/macos_containerize_proxy.rs` | 743 | ~666 | ~40% | 1 | 0 | none | `ContainerBuilder::create_container_source` → streamable source | partially — success payload is a file path/tag; runtime detection & streaming are side effects |
| 14 | `publish` | `publish.rs` | 504 | 367 | ~30% | 3 | 0 | up-front only (`ensure_auth`, `publish.rs:152`) | typed metadata from `check_environment_metadata` / `check_package_metadata` / `check_build_metadata`; `publish()` returns `needs_wait: bool` | yes — pipeline of SDK calls, no prompts |
| 15 | `gc` | `gc.rs` | 466 | 335 | ~60% | 2 | 0 | none | none — parses `nix store gc` stderr in the command file | partially — freed-space figure is a parsed string, not a typed result |
| 16 | `services start/restart` | `services/{start,restart}.rs`, `services/mod.rs` | 998 | ~750 | ~50% | 8 | 0 | none, but may *re-enter `activate`* (ephemeral activation via `ActivateOptions::activate`, `services/mod.rs:358`) | `ProcessStates`; start-via-activation returns names started | mostly — needs the activate coupling untangled |
| 17 | `build` | `build.rs` | 1,176 | 733 | ~60% | 7 | 0 | none | `FloxBuildMk::build` → results with `result_links` | yes for outputs; large stranded pre-flight logic (see below) |
| 18 | `auth` | `auth.rs` | 342 | 342 | ~80% | 9 | 2 | **MID** — Enter-key `Checkpoint` dialog raced against OAuth token polling (`auth.rs:156-187`) | none — whole OAuth device flow implemented in the command file | status/token: yes; login: device-flow data (URL+code) could be emitted as JSON |
| 19 | `install` | `install.rs` | 1,090 | 781 | ~45% | 19 | 5 | **MID** — default-env onboarding `Select` (`install.rs:597`) and RC-file `Select` (`install.rs:704`) fire mid-flow, after `RemoteEnvironment` creation has begun | `InstallationAttempt` (sdk `environment/mod.rs:88`) | main path yes; onboarding path is inherently interactive |
| 20 | `pull` | `pull.rs` | 906 | 699 | ~50% | 9 | 6 | **MID** — `query_add_system` (`pull.rs:536`) and `query_ignore_build_errors` (`pull.rs:582`) fire after clone/build has already run | `PullResult::{Updated,UpToDate}` (sdk `managed_environment.rs:1201`) | `--force` path yes; recovery paths are interactive |
| 21 | `edit` | `edit.rs` | 973 | 443 | ~50% | 10 | 4 | **MID** — edit→build→error→"Continue editing?" Confirm loop (`edit.rs:298-326`); spawns `$EDITOR` (`edit.rs:425-441`) | `EditResult::{Unchanged,Changed}` (sdk `core_environment.rs:953`), `SyncToGenerationResult` (sdk `generations.rs:656`) | `--file`/`--sync`/`--reset` paths yes; interactive path structural |
| 22 | `init` | `init/{mod,node,python,go}.rs` | 4,476 | ~3,046 | ~85% | 16 | 7 | **MID** — language-hook prompts fire *after* catalog detection work has run (`init/mod.rs:327`; `node.rs:935`, `python.rs:115`, `go.rs:116`) | `PathEnvironment::init` → `PathEnvironment`; `InitCustomization` is an SDK type | `--auto-setup`/`--bare` paths yes; suggestion flow interactive |
| 23 | `activate` | `activate.rs` | 1,240 | 906 | ~60% | 7 | 0 (delegates) | up-front trust for the remote env (`activate.rs:206-219`); **MID** trust prompts for remote *includes* after locking has begun (`activate.rs:352-367` → `mod.rs:1263`) | `LockResult`, `UpgradeResult`, `BranchOrd` consumed; terminal action is `command.exec()` (`activate.rs:597`) | structurally **no** — ends in process replacement; the *resolution* phase could emit JSON |

Notes on the count of "commands": the 6 visible bpaf groups wire 22 top-level
user commands; counting `services` and `generations` subcommands separately
yields the ~26 cited in GOAL.md. `mod.rs` itself (1,508 lines) carries 14
`message::` lines and 7 dialog usages shared by all commands.

## Summary

### The 3 thinnest commands

**`include upgrade` (`include.rs`, 108 LOC).** The purest example of the
target shape in the codebase: parse args, call
`environment.include_upgrade(&flox, names)` (`include.rs:80`) which returns an
`UpgradeResult`, then render `result.include_diff()` into messages
(`include.rs:82-104`). Zero prompts, zero subprocesses, zero filesystem
access. A `--json` flag would be ~10 lines.

**`delete` (`delete.rs`, 89 LOC).** One up-front `Confirm` dialog
(`delete.rs:67-77`, skippable with `-f`), then a single SDK call
(`environment.delete(&flox)`, `delete.rs:79-83`), then one message. The only
"logic" is the remote/managed special-case messaging (`delete.rs:37-59`).

**`uninstall` (`uninstall.rs`, 129 LOC).** Parses `UninstallSpec`s (an SDK
type, `uninstall.rs:80-85`), calls `concrete_environment.uninstall(...)`
which returns a structured `UninstallationAttempt`, and renders the
`modifications` list (`uninstall.rs:96-123`). `push` (263 non-test LOC)
deserves honorable mention as the cleanest *non-trivial* model: typed
`PushResult` in, three small render branches out, all error rewriting
concentrated in one `convert_error` fn (`push.rs:204-251`).

### The 3 fattest commands

**`init` (4,476 LOC across 4 files, ~3,046 non-test).** The entire
language-detection subsystem — Node version resolution against the catalog
(`init/node.rs`, 1,747 lines), Python interpreter/poetry/pyproject handling
(`init/python.rs`, 1,157 lines), Go module detection (`init/go.rs`, 590
lines) — lives in the command layer, interleaved with `Select` dialogs for
version choice (`node.rs:935`, `python.rs:115`, `go.rs:116`) and the
accept/decline prompt loop (`init/mod.rs:325-330`). Only the final
`PathEnvironment::init` call (`init/mod.rs:235`) is SDK. floxdash/floxhub
could not reuse any of the detection logic today without linking the CLI
binary's internals.

**`activate` (1,240 LOC, 906 non-test).** Beyond the structural shell-exec
(`activate.rs:597`), the file assembles the whole activation context by hand:
mode/link resolution (`activate.rs:399-414`), prompt-environment construction
(`activate.rs:451-482, 663-678`), service auto-start decisions
(`services_to_start`, `activate.rs:611-658`), upgrade-notification logic
reading SDK state files (`activate.rs:703-860`), and `AttachCtx`/`ActivateCtx`
serialization to a temp file consumed by `flox-activations`
(`activate.rs:511-573`). Mid-operation trust prompts for remote includes
(`activate.rs:352-367`) fire after locking has begun.

**`install` (1,090 LOC, 781 non-test).** The retry-with-valid-systems
recovery (`need_retry_with_valid_systems`, `install.rs:357-422`;
`retry_install_for_valid_systems`, `install.rs:424-458`), result partitioning
(`install.rs:309-355`), unfree/broken warning generation
(`install.rs:522-561`), and — most stranded of all — the first-run onboarding
flow that creates a remote default environment and *edits the user's shell RC
files* (`try_create_default_environment_interactive`, `install.rs:564-661`;
`prompt_to_modify_rc_file` + `locate_rc_file` + `ensure_rc_file_exists` +
`add_activation_to_rc_file`, `install.rs:673-780`) all live in the command
file. (`build.rs` at 733 non-test LOC with ~60% stranded logic is a close
fourth; see the list below.)

## Stranded logic (candidates for moving into flox-rust-sdk — feeds Workstream B)

Business logic currently in `cli/flox/src/commands/` that a non-CLI consumer
(floxhub, floxdash, plugins) would need:

- **install.rs**
  - `need_retry_with_valid_systems` — `install.rs:357-422` (classifying resolution failures, computing per-system retry sets)
  - `retry_install_for_valid_systems` — `install.rs:424-458`
  - `partition_installed_packages` — `install.rs:309-355` (interpreting `InstallationAttempt` modifications)
  - `generate_unfree_and_broken_warnings` — `install.rs:522-561` (lockfile policy inspection)
  - default-env creation in `try_create_default_environment_interactive` — `install.rs:626-649` (the `RemoteEnvironment::new`-or-`init_floxhub_environment` dance)
  - RC-file mutation: `locate_rc_file` / `ensure_rc_file_exists` / `add_activation_to_rc_file` — `install.rs:736-780`
- **pull.rs**
  - `pull_new_environment` — `pull.rs:277-409` (writes `env.json` pointer, creates/cleans `.flox/`, orchestrates open→generation-switch→build)
  - `handle_pull_result` — `pull.rs:415-530` (typed recovery for incompatible-system and build-failure outcomes; already prompt-free via the injected `QueryFunctions` seam at `pull.rs:86-89` — a ready-made "modelable outcome" pattern)
  - `amend_current_system` — `pull.rs:562-579` (manifest mutation to add a system)
- **edit.rs**
  - `determine_editor_from_vars` — `edit.rs:374-405` ($VISUAL/$EDITOR/PATH resolution)
  - `make_interactively_recoverable` — `edit.rs:330-352` (classification of which `EnvironmentError`s are recoverable; this is SDK error taxonomy, not UI)
- **activate.rs**
  - `services_to_start` — `activate.rs:611-658` (auto-start policy against manifest + running state)
  - `ActivateCtx`/`AttachCtx` assembly — `activate.rs:511-557` (environment resolution → activation context)
  - `notify_upgrades_if_available` / `notify_environment_upgrades` / `notify_package_upgrades` — `activate.rs:703-860` (branch comparison and upgrade-diff policy; only the final `message::info` is UI)
  - `allow` / `deny` auto-activation config writes — `activate.rs:865-890`
- **build.rs**
  - `import_nixpkgs` — `build.rs:345-423` (runs `nix eval`, copies package definition files)
  - `update_catalogs` — `build.rs:425-461` (lock `nix-builds.toml`)
  - `base_nixpkgs_url_from_url_select` — `build.rs:539-574` (stability → base catalog URL policy)
  - `check_git_tracking_for_expression_builds` — `build.rs:582-655` (git subprocess checks)
  - `prefetch_flake_ref` — `build.rs:665-680` (`nix flake prefetch` subprocess)
  - `packages_to_build` — `build.rs:708-732` (target selection)
- **auth.rs**
  - `create_oauth_client` — `auth.rs:71-93` and `authorize` — `auth.rs:95-219` (the entire OAuth device flow) plus `login_flox` — `auth.rs:312-341` (token persistence). Only the browser-open/Enter-key race is UI.
- **gc.rs**
  - `run_store_gc` + the `GcProgress` state machine — `gc.rs:93-335` (spawning and parsing `nix store gc`)
- **init/**
  - the whole detection subsystem: `init/node.rs` (1,747), `init/python.rs` (1,157), `init/go.rs` (590); `combine_customizations` — `init/mod.rs:336-430`. Each hook already separates "detect/resolve" from "prompt", so extraction is mechanical but large.
- **services/mod.rs**
  - `ProcessComposeState` detection — `services/mod.rs:196-238` (activation-state file + store-path comparison)
  - `guard_service_commands_available` / `guard_is_within_activation` — `services/mod.rs:248-289`
  - `processes_by_name_or_default_to_all` — `services/mod.rs:314`
  - `start_services_with_new_process_compose` — `services/mod.rs:358` (couples services to `ActivateOptions::activate`)
- **mod.rs (shared)**
  - `detect_environment` — `mod.rs:1171-1221` (active-vs-found resolution policy; only `query_which_environment` is UI)
  - trust policy inside `ensure_environment_trust` — `mod.rs:1263-1304` (owner/handle/config checks are policy; the dialog loop at `mod.rs:1341-1390` is UI)
- **general.rs**
  - `update_config` — `general.rs:147` (TOML config mutation used by auth, activate, and trust handling — already imported across command files, a clear sign it belongs in a lower layer)
- **envs.rs**
  - `get_inactive_environments` — `envs.rs:225` (registry set arithmetic)

## Mid-operation interactivity (feeds Workstream C)

Prompts that occur after the operation has started, ordered by design
difficulty:

| # | Command | Prompt | Location | When it fires |
|---|---------|--------|----------|---------------|
| 1 | `edit` | "Continue editing?" `Confirm` loop | `edit.rs:298-326` | After each failed edit/build attempt, in a loop with `$EDITOR` re-spawn (`edit.rs:308-326`) |
| 2 | `pull` | add-your-system `Select` | `pull.rs:536-559` (invoked via `pull.rs:454`) | After clone + pointer write + build attempt failed with incompatible-system |
| 3 | `pull` | ignore-build-errors `Select` | `pull.rs:582-604` (invoked via `pull.rs:512`) | After clone + build attempt failed with `Realise2` build error |
| 4 | `install` | onboarding: create-default-env `Select` | `install.rs:597-606` | Mid-flow, after env detection failed and user state file was locked/read (`install.rs:580-586`) |
| 5 | `install` | modify-RC-file `Select` | `install.rs:704-711` (called at `install.rs:658`) | After the remote default environment has already been created (`install.rs:638-648`), before the install proceeds |
| 6 | `activate` | trust `Select` loop for remote *includes* | `activate.rs:352-367` → `ensure_environment_trust` `mod.rs:1341-1390` | After `lockfile()` resolution has run (`activate.rs:343-349`), per remote include |
| 7 | `auth login` | Enter-key `Checkpoint` raced against token polling | `auth.rs:156-187` | While the OAuth device-code grant is in flight (`tokio::select!` between key press and token arrival) |
| 8 | `init` | language-hook accept prompts and version `Select`s | `init/mod.rs:327`; `init/node.rs:935`; `init/python.rs:115`; `init/go.rs:116` | After catalog resolution for suggestions has already run inside `Node::new`/`Python::new`/`Go::new` (`init/mod.rs:311-321`), before env creation |

Up-front-only prompts (for contrast, all hoistable): `delete` confirm
(`delete.rs:67-77`), environment disambiguation (`mod.rs:1224-1255`), auth
login fallback (`mod.rs:1397-1411`), `activate` trust for the directly
activated remote env (`activate.rs:206-219`).

Structural (can never be a pure API call): `activate`'s
`command.exec()` (`activate.rs:597`) and `edit`'s `$EDITOR` subprocess
(`edit.rs:430-437`).

## Assumptions

- **Biz-logic % is a judgment-based estimate** from reading each file, not a
  tool measurement. Rendering helpers (e.g. `list.rs`'s `print_extended`,
  `upgrade.rs`'s `render_diff`) are counted as render, not business logic;
  reasonable readers could shift any figure ±10 points.
- Test modules were located via the first `#[cfg(test)]` line; inline
  `#[test]`-free helper code below that line (rare) is counted as test code.
- "Mid-operation" is defined as: the prompt fires after any SDK mutation,
  network call, build, or subprocess for the operation has begun. Init's
  hooks are classified MID because catalog resolution runs before the prompt,
  even though no environment exists yet.
- GOAL.md's baseline "JSON output exists on exactly one command (`envs`)" is
  **stale**: `search` (`search.rs:34`), `services status`
  (`services/status.rs:30`), `generations list` (`generations/list.rs:47`),
  and `generations history` (`generations/history.rs:42`) also have `--json`
  today. There is still no *systemic* pattern.
- Migration-difficulty ranking weighs (in order): mid-operation prompts >
  stranded-logic volume > subprocess/filesystem side effects > rendering
  volume. `cargo`/`nix` were unavailable in the analysis container, so no
  compile-based verification (e.g. `cargo public-api`) was performed.

## How to reproduce

All commands run from the repository root.

```bash
# 1. Total LOC per command file
wc -l cli/flox/src/commands/*.rs cli/flox/src/commands/**/*.rs | sort -n

# 2. Enumerate wired commands
grep -nE '\(#\[bpaf\(external' cli/flox/src/commands/mod.rs

# 3. message:: and (naive) dialog counts per file
cd cli/flox/src/commands
for f in $(find . -name '*.rs' | sort); do
  msg=$(grep -c 'message::' "$f")
  dlg=$(grep -cE 'Dialog|Select|Confirm' "$f")   # inflated; see next step
  echo "$f msg=$msg dialog=$dlg"
done

# 4. Real dialog usages (filters out EnvironmentSelect/CommandSelect etc.)
grep -nE 'Dialog \{|Dialog::can_prompt|typed: Confirm|typed: Select' \
  *.rs */*.rs

# 5. Where test modules start (for non-test LOC)
for f in <files>; do
  grep -n '^#\[cfg(test)\]' "$f" | head -1
done

# 6. SDK result types
grep -rn 'pub struct InstallationAttempt\|pub struct UninstallationAttempt\|pub enum PushResult\|pub enum PullResult\|pub enum EditResult\|pub struct UpgradeResult\|pub enum SyncToGenerationResult' \
  cli/flox-rust-sdk/src/models/

# 7. Existing --json flags
grep -rn 'json' cli/flox/src/commands --include='*.rs' | grep -iE 'bpaf|--json'

# 8. Per-command judgments (biz-logic %, prompt timing): read the files,
#    in particular install.rs, pull.rs, edit.rs, activate.rs, delete.rs,
#    upgrade.rs, build.rs, publish.rs, auth.rs, init/mod.rs,
#    services/{mod,start}.rs, and mod.rs:1050-1440 (shared helpers).
```
