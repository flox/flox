# Workstream B — flox-rust-sdk Fitness Review

**Date:** 2026-06-11
**Status:** Analysis only, per GOAL.md ground rules. Inputs: the surface of
`cli/flox-rust-sdk` read from source (no `cargo public-api`; `cargo` is
unavailable in this environment), the "Stranded logic" list from
`A-command-audit.md`, and the violations list from `D-dependency-layering.md`.

This document answers GOAL.md's reuse question: can `flox-rust-sdk` (~37k LOC,
`D-dependency-layering.md` per-crate table) serve as the shared operations
layer for the CLI, floxhub (web portal), and floxdash (TUI) — and exactly what
must be added to it, removed from it, or relocated? The short answer, defended
in the Verdict section: the SDK is structurally fit to be that layer directly —
it is print-clean, returns typed results, and takes its context via a `Flox`
struct rather than ambient state — but it is *incomplete* (the Pass-2 add list
is large, dominated by `init` and `install` logic stranded in the command
layer), carries a handful of relocatable leaks (Pass 3), and no crate shape
fixes the fact that every build-touching operation mutates the host-global Nix
store via subprocess — for in-process floxhub, that is a deployment boundary,
not a refactoring target.

## Assumptions

Per GOAL.md Workstream B defaults (no human-provided capability sketch was
available):

- **floxdash** = full CLI parity minus `activate` (a local, single-user TUI on
  the user's own machine; subprocesses, `$HOME` state, and the local Nix store
  are all in scope).
- **floxhub** = read operations (search/show/list/generations browsing) plus
  push/pull/publish-adjacent mutations, evaluated as an **in-process,
  multi-tenant async web service** linking the SDK as a library — the
  expensive-to-discover-late case GOAL.md names.
- "Leaky" follows GOAL.md's definition: prints, reads ambient/global state,
  assumes an interactive flow, or returns strings whose only purpose is
  terminal display.
- Public-API inventory is at module/major-type level from `lib.rs` and the
  module tree, not an exhaustive `cargo public-api` diff; method-level
  judgments are from reading the cited files.

---

## Pass 1 — Surface inventory

`cli/flox-rust-sdk/src/lib.rs:1-5` exports exactly five modules: `data`,
`flox`, `models`, `providers`, `utils`.

### Global hygiene checks (the GOAL.md "verify" items)

- **Printing:** no `println!`/`eprintln!`/`print!`/`eprint!` and no
  `io::stdout()`/`stderr()` writes in non-test SDK code. Verified
  independently and consistent with `D-dependency-layering.md` ("Checked and
  found clean"): the only print-macro hits are inside `#[cfg(test)]` modules
  (`src/providers/buildenv.rs:1670,1714`, `flake_installable_locker.rs:283`,
  `services/process_compose.rs:1435`, `git.rs:1596`).
- **TTY assumptions:** zero hits for `is_terminal`/`isatty`/`atty`/`stdin()`
  across `cli/flox-rust-sdk/src` (grep in How-to-reproduce #2). The SDK never
  probes the terminal.
- **Ambient env-var reads at depth:** ~20 sites, but almost all are
  *tool-path/host-config overrides* resolved through `LazyLock` with
  compile-time `env!` defaults — `NIX_BIN` (`providers/nix.rs:10`), `GIT_PKG`
  (`providers/git.rs:21`), `FLOX_BUILD_MK`/`FLOX_EXPRESSION_BUILD_NIX`/
  `GNUMAKE_BIN`/`COMMON_NIXPKGS_URL` (`providers/build.rs:29-47`),
  `FLOX_BUILDENV_NIX` (`providers/buildenv.rs:38`), `FLOX_MK_CONTAINER_NIX`
  (`providers/container_builder.rs:22`), `PROCESS_COMPOSE_BIN`/`SLEEP_BIN`
  (`providers/services/process_compose.rs:41-45`), `NIX_PLUGINS`
  (`models/nix_plugins.rs:6`), `FLOX_INTERPRETER` (`utils/mod.rs:41`). These
  configure *which binaries the host runs* and are acceptable for any consumer,
  though as process-wide statics they are per-process, not per-tenant.
  Genuinely ambient reads that deserve flags: `FLOX_MAX_PARALLEL_DOWNLOADS`
  (`providers/buildenv.rs:308`), `_FLOX_NIX_STORE_URL`
  (`providers/buildenv.rs:1040`), `FLOX_INVOCATION_SOURCE` plus ~CI/agent
  heuristic vars (`utils/invocation_sources.rs:52-78`), and one
  `std::env::current_dir()` inside `RemoteEnvironment`
  (`models/environment/remote_environment.rs:469`).
- **Error `Display` content:** the SDK *does* format user-facing prose,
  including CLI command suggestions, inside error types — e.g.
  `RecoverableMergeError::PathOutOfSync`/`ManagedOutOfSync` embed "run
  'flox edit -d {0}'" copy (`providers/lock_manifest.rs:128-142`),
  `impl Display for ResolutionFailures` renders indented multi-paragraph
  advice with 'flox edit' suggestions (`providers/lock_manifest.rs:1266-1330`),
  `BuildEnvError::NoPackageStoreLocation` suggests 'flox auth login'
  (`providers/buildenv.rs:149-151`), and `fetcher.rs:272` and
  `publish.rs:792,1964` carry similar copy. Note: this is the *intended*
  error architecture per AGENTS.md ("Credential sanitization … belong in
  `Display` impls"), so it is classified leaky-by-GOAL.md's-definition but
  deliberate — see Pass 3 item R9 for the recommendation.
- **Progress reporting:** the SDK has a de-facto headless-safe progress
  contract already: `#[instrument(fields(progress = "..."))]` span fields
  (27 sites: `providers/buildenv.rs:437-1283`, `publish.rs:413,499,1050`,
  `lock_manifest.rs:425,639`, `container_builder.rs:136,222`,
  `managed_environment.rs:1238-1510`, `floxmeta_branch.rs:295`, etc.), which
  the CLI's tracing-indicatif layer renders as spinners. The SDK emits data;
  the binary decides presentation. This is the pattern Workstream C should
  standardize.
- **Execution model:** the surface is *mixed sync/async*. Catalog and locking
  are async (`LockManifest::lock_manifest`, `providers/lock_manifest.rs:315`;
  `ClientTrait` async fns, `cli/flox-catalog/src/client.rs:159-200`), but the
  `Environment` trait is sync and bridges with `pollster::block_on`
  (`models/environment/core_environment.rs:22,208,258,657,712`;
  `providers/buildenv.rs:24,333`). The CLI drives this from its own tokio
  runtime (`cli/flox/src/main.rs:154`). An async web server must wrap these
  sync entry points in `spawn_blocking`; calling them on a runtime worker
  thread blocks it.

### Classified inventory (module / major type level)

Classification: **clean** = structured in/out, no printing, no TTY assumption;
**leaky** = per GOAL.md definition above; **internal** = should not be public.

| Public item | Location | Classification | Notes |
|---|---|---|---|
| `Flox` context struct | `flox.rs:38-71` | clean | All dirs, catalog client, auth, features injected by constructor — the DI seam that makes per-tenant instances possible |
| `Flox::set_auth_context` | `flox.rs:78` | clean | |
| `Floxhub`, `Features`, `FLOX_VERSION` | `flox.rs:108,93,16` | clean | `Floxhub::git_url` doc mentions an env override but the override is applied by the *caller*, not read here |
| `Environment` trait (install/uninstall/edit/upgrade/include_upgrade/lockfile/build/delete/…) | `models/environment/mod.rs:110-254` | clean | The core operations surface; every mutation returns a typed result |
| `ConcreteEnvironment` (Path/Managed/Remote enum-dispatch) | `models/environment/mod.rs:259` | clean | |
| `InstallationAttempt`, `UninstallationAttempt` | `models/environment/mod.rs:88,98` | clean | Typed modification lists |
| `EnvironmentPointer`/`PathPointer`/`ManagedPointer`, `DotFlox`, `open_path`, `find_dot_flox` | `models/environment/mod.rs:438,560,929,987` | clean | Resolution takes explicit paths; the *cwd policy* lives (stranded) in the CLI |
| `UninitializedEnvironment::bare_description` | `models/environment/mod.rs:738` | **leaky** | Returns a string "when displayed in a prompt" — display formatting in the SDK (Pass 3 R7) |
| `CoreEnvironment::{ensure_locked,lock,lock_without_writing,build}` | `core_environment.rs:150,189,241,282` | clean | `lock` writes the lockfile atomically (`core_environment.rs:234`) |
| `EditResult`, `UpgradeResult` (+`diff`, `diff_for_system`, `include_diff`), `SingleSystemUpgradeDiff` | `core_environment.rs:953,1000,1022,1084,1099` | clean | `include_diff` returns include *names*, not prose |
| `ManagedEnvironment::{open,push,push_new,pull,fetch_remote_state,into_path_environment,has_local_changes}` | `managed_environment.rs:842,1409,1262,1511,1239,1594,1151` | clean | `PushResult`/`PullResult` enums at `managed_environment.rs:1209,1201` |
| `RemoteEnvironment::{new,push,pull,init_floxhub_environment}` | `remote_environment.rs:116,294,305,321` | clean except `remote_environment.rs:469` | reads `std::env::current_dir()` for `project_path` — ambient (Pass 3 R10) |
| `PathEnvironment::{open,init,init_bare}` | `path_environment.rs:422,483,440` | clean | |
| `Generations<State>` type-state, `GenerationsExt`, `AllGenerationsMetadata`, `SyncToGenerationResult` | `generations.rs:95,584,709,656` | clean | Already serves `generations list/history --json` (A matrix row 9) |
| `FloxMeta`, `floxmeta_git_options` | `floxmeta.rs:29,206` | clean | Builds git config flags incl. auth headers from `AuthContext`; credentials flow into a subprocess env, never printed |
| `env_registry::{ensure_registered,deregister,garbage_collect,read_environment_registry}` | `env_registry.rs:292,317,337,252` | clean | File-locked JSON registry |
| `user_state`, `upgrade_checks::UpgradeInformationGuard` | `user_state.rs:27-93`, `upgrade_checks.rs:44-141` | clean | Lock-guarded typed state files |
| `catalog::Client`/`MockClient`, `SearchTerm`, `base_catalog_url_for_stability_arg`, `get_base_nixpkgs_url` | `providers/catalog.rs:117,338,594,620,670` | clean | |
| `ClientTrait::search_with_spinner` | `cli/flox-catalog/src/client.rs:167,295` | **leaky (naming)** | A UI concept in the API trait; the impl is just an instrumented span with a `progress` field (Pass 3 R8) |
| `LockManifest::lock_manifest`, `LockResult`, `ResolutionFailure(s)` | `lock_manifest.rs:315,157,1229,1242` | clean types / **leaky Display** | Typed failures; the `Display` impl renders CLI advice (`lock_manifest.rs:1266-1330`, Pass 3 R9) |
| `BuildEnv` trait, `BuildEnvNix`, `BuildEnvOutputs` | `buildenv.rs:225,235,199` | clean | nix subprocess inside; error prose at `buildenv.rs:151` (R9) |
| `ManifestBuilder`/`FloxBuildMk`, `BuildResults`, `PackageTargets` | `build.rs:54,200,102,703` | clean | `new_with_buffers` (`build.rs:240`) injects output sinks — the right seam for streaming build logs headlessly |
| `Publisher` trait, `PublishProvider`, `check_environment_metadata`, `check_package_metadata`, `check_build_metadata` | `publish.rs:114,554,1192,1238,887` | clean | The publish pipeline is already SDK-resident (A matrix row 14) |
| `ContainerBuilder`, `ContainerSource::stream_container` | `container_builder.rs:27,223` | clean | Sink-injected streaming |
| `services::process_compose` (`ProcessStates::read`, `start_service`, `stop_services`, `restart_service`, log tail types) | `process_compose.rs:370,457,428,486,617-776` | clean | Talks to a Unix socket; socket override env var is test-only (`models/environment/mod.rs:83`) |
| `GitProvider` trait, `GitCommandProvider`, `GitRemoteCommandError` | `git.rs:54,315,809` | clean | Typed remote-failure classification (`AccessDenied`, `Diverged`, …) per AGENTS.md architecture |
| `nix::nix_base_command`, `NIX_VERSION` | `nix.rs:17,14` | **internal** | Subprocess plumbing; public only because CLI code (e.g. the stranded `gc.rs` store-gc) builds raw nix commands — should become unnecessary once Pass-2 A17 lands |
| `manifest_init::ManifestInitializer` | `manifest_init.rs:32` | clean | The 'flox install gum' prose at `manifest_init.rs:136` etc. is *manifest template content written to the user's file* — product data, not terminal decoration |
| `nix_auth::NixAuth`, `git_auth` | `nix_auth.rs:83`, `git_auth.rs` | clean | Writes ad-hoc netrc into `flox.temp_dir` (`nix_auth.rs:93-107`) — injected dir, fine |
| `flake_installable_locker::{InstallableLocker,InstallableLockerImpl}` | `flake_installable_locker.rs:44,54` | clean | |
| `utils::{copy_file_without_permissions, …}` | `utils/mod.rs` | **internal** | Grab-bag helpers; nothing display-related, but not an API |
| `utils::invocation_sources`, `HEADER_DEVICE_UUID`/`HEADER_INVOCATION_SOURCE` | `invocation_sources.rs:78`, `utils/mod.rs:20-22` | **leaky** | Telemetry heuristics scanning ~30 ambient env vars (CI, Copilot, …) — CLI-invocation concern in the SDK (Pass 3 R6) |
| `utils::logging::test_helpers`, `flox::test_helpers`, per-module `test_helpers` | `logging.rs:1`, `flox.rs:174` | clean (gated) | Behind `feature = "tests"`/`cfg(test)`; correctly excluded from production surface |
| `data::{System, FloxVersion, CanonicalPath}` | `data/mod.rs:5-6` | clean | |

**Tally:** of ~30 major public items, ~24 are clean operations, 4 are leaky
(`bare_description`, `invocation_sources`+headers, `search_with_spinner`
naming, error-Display prose as a cross-cutting pattern), and 2 are
internal-but-public plumbing (`nix_base_command`, `utils` grab-bag). The
GOAL.md baseline ("returns typed results, no direct terminal deps") is
confirmed and, on this inspection, even somewhat understated — the SDK also
has working seams for progress (span fields), output streaming (buffer/sink
injection), and prompt-free recovery (`PullResult` + the CLI's injected
`QueryFunctions`, see Pass 2).

### Keep as-is (the explicit list)

Everything in the table above marked **clean**, headlined by: the `Flox`
context struct (`flox.rs:38`); the `Environment` trait and its typed results
(`models/environment/mod.rs:110,88,98`); `ManagedEnvironment::push/pull` with
`PushResult`/`PullResult` (`managed_environment.rs:1409,1511,1209,1201`);
`UpgradeResult` diffs (`core_environment.rs:1000-1099`); the generations
type-state API (`generations.rs:95-245`); the catalog client trait minus the
`_with_spinner` name (`cli/flox-catalog/src/client.rs:159`); the publish
check/provider pipeline (`publish.rs:114-1238`); the git provider with typed
remote errors (`git.rs:54,809`); process-compose service control
(`process_compose.rs:370-486`); and the `progress =` span-field convention
(27 sites, How-to-reproduce #5) which should be promoted from convention to
documented contract by Workstream C rather than changed.

---

## Pass 2 — Add candidates

Source: the "Stranded logic" list in `A-command-audit.md`, prioritized by who
needs it under the Assumptions (floxhub = read + push/pull/publish-adjacent;
floxdash = CLI parity minus activate). **P1** = blocks floxhub, **P2** =
blocks floxdash parity on core flows, **P3** = floxdash long tail.

| # | Pri | Source (cli/flox/src/commands/) | What it does | Destination in flox-rust-sdk |
|---|-----|------|--------------|------------------------------|
| A1 | P1 | `pull.rs:277-409` `pull_new_environment` | Writes `env.json` pointer, creates/cleans `.flox/`, orchestrates open → generation switch → build for a first-time pull | `models/environment/managed_environment.rs` — a `ManagedEnvironment::clone_into(path, …)`-style constructor |
| A2 | P1 | `pull.rs:415-530` `handle_pull_result` | Typed recovery for incompatible-system and build-failure outcomes; already prompt-free via the injected `QueryFunctions` seam (`pull.rs:86-89`) | `managed_environment.rs` as a returned outcome enum (the model case for Workstream C's "modelable" pattern — port the seam, not the prompts) |
| A3 | P1 | `pull.rs:562-579` `amend_current_system` | Manifest mutation adding the current system | `models/environment/core_environment.rs` (next to other manifest transactions) |
| A4 | P1 | `install.rs:309-355` `partition_installed_packages` | Interprets `InstallationAttempt` modifications into installed/already-present sets | `models/environment/install.rs` (module exists, `models/environment/mod.rs:62`) |
| A5 | P1 | `install.rs:357-458` `need_retry_with_valid_systems` + `retry_install_for_valid_systems` | Classifies resolution failures, computes per-system retry sets, re-runs install | `models/environment/install.rs`, consuming the typed `ResolutionFailure` (`lock_manifest.rs:1242`) instead of re-deriving |
| A6 | P1 | `install.rs:522-561` `generate_unfree_and_broken_warnings` | Lockfile license/broken policy inspection → warnings | `models/environment/install.rs`, returning a typed `Vec<PackageWarning>`, not strings |
| A7 | P1 | `build.rs:582-655` `check_git_tracking_for_expression_builds` | Git cleanliness/tracking preconditions for expression builds (publish-adjacent) | `providers/publish.rs` or `providers/build.rs` (publish already does sibling checks at `publish.rs:1050`) |
| A8 | P2 | `mod.rs:1171-1221` `detect_environment` | Active-vs-found environment resolution policy (only `query_which_environment` at `mod.rs:1224` is UI) | `models/environment/mod.rs`, returning a typed `Ambiguous{…}` outcome the caller resolves |
| A9 | P2 | `mod.rs:1263-1304` trust policy inside `ensure_environment_trust` | Owner/handle/config trust decision (dialog loop at `mod.rs:1341-1390` stays CLI) | new `models/environment/trust.rs`, returning `Trusted/Untrusted{owner,…}` |
| A10 | P2 | `general.rs:147` `update_config` | TOML config mutation already imported by auth/activate/trust command files — "a clear sign it belongs in a lower layer" (A) | a config module in the SDK (or `flox-core`, given `flox-activations` may need it); note D's L0 rule bans network/telemetry, not config I/O |
| A11 | P2 | `auth.rs:71-219,312-341` OAuth device flow + `login_flox` token persistence | The entire device-code grant; only the Enter-key/poll race (`auth.rs:156-187`) is UI | new `providers/auth.rs` (floxdash needs login; floxhub authenticates its own users and would not link this) |
| A12 | P2 | `edit.rs:330-352` `make_interactively_recoverable` | Classifies which `EnvironmentError`s are recoverable — "SDK error taxonomy, not UI" (A) | fold into `EnvironmentError` as a `fn is_recoverable_with_edit(&self)` or new variants, per AGENTS.md error-architecture rules |
| A13 | P2 | `services/mod.rs:196-238,248-289,314` `ProcessComposeState` detection, guards, name resolution | Activation-state + store-path comparison and service-command preconditions | `providers/services/` (new `state.rs` beside `process_compose.rs`) |
| A14 | P2 | `activate.rs:611-658` `services_to_start` | Auto-start policy from manifest + running state | `providers/services/` |
| A15 | P2 | `activate.rs:703-860` `notify_upgrades_if_available` + helpers | Branch comparison and upgrade-diff policy reading SDK state files; only the final `message::info` is UI | `providers/upgrade_checks.rs` (the state-file types already live there, `upgrade_checks.rs:27-141`) |
| A16 | P2 | `envs.rs:225` `get_inactive_environments` | Registry set arithmetic | `models/env_registry.rs` |
| A17 | P2 | `gc.rs:93-335` `run_store_gc` + `GcProgress` | Spawns and parses `nix store gc` | new `providers/store_gc.rs`, returning typed progress events/freed-bytes instead of parsed strings (also retires the CLI's need for `nix_base_command`) |
| A18 | P3 | `install.rs:564-661` default-env creation (`RemoteEnvironment::new`-or-`init_floxhub_environment` dance) | First-run onboarding env creation | `models/environment/remote_environment.rs` helper |
| A19 | P3 | `install.rs:673-780` `locate_rc_file`/`ensure_rc_file_exists`/`add_activation_to_rc_file` | Shell RC-file mutation | new `providers/shell_setup.rs` — floxdash needs it for onboarding parity; floxhub must never link a `$HOME`-mutating module, which is an argument for an isolated module with its own feature flag |
| A20 | P3 | `build.rs:345-423,425-461,539-574,665-680,708-732` `import_nixpkgs`, `update_catalogs`, `base_nixpkgs_url_from_url_select`, `prefetch_flake_ref`, `packages_to_build` | Build pre-flight: nix eval/prefetch subprocesses, `nix-builds.toml` locking, stability→base-URL policy, target selection | `providers/build.rs` (target selection beside `PackageTargets`, `build.rs:703`; stability policy beside `base_catalog_url_for_stability_arg`, `catalog.rs:620`) |
| A21 | P3 | `activate.rs:511-557` `ActivateCtx`/`AttachCtx` assembly | Environment resolution → activation context (types already in `flox-core/src/activate/context.rs`) | new `models/activation.rs`; floxdash excludes `activate` but `services start` re-enters activation (A matrix row 16), so this unblocks untangling services |
| A22 | P3 | `activate.rs:865-890` allow/deny auto-activation config writes | Config mutation | rides on A10's config module |
| A23 | P3 | `edit.rs:374-405` `determine_editor_from_vars` | `$VISUAL`/`$EDITOR`/PATH resolution | borderline: ambient-by-design and interactive-only; acceptable as a small SDK util consumed by CLI+floxdash, or left in a shared CLI/TUI helper crate. Lowest priority |
| A24 | P3 | `init/node.rs` (1,747 LOC), `init/python.rs` (1,157), `init/go.rs` (590), `init/mod.rs:336-430` `combine_customizations` | The entire language-detection subsystem; each hook already separates detect/resolve from prompt (A) | new `providers/init_detection/` (or a sibling crate if SDK size is a concern); `InitCustomization` is already an SDK-adjacent type and the sink (`ManifestInitializer`, `manifest_init.rs:32-60`) is already in the SDK. Largest single item (~3.5k LOC) but mechanical |

Items A1–A7 are what the assumed floxhub actually blocks on: pull/push
orchestration and recovery (A1–A3), install-result interpretation for any
env-mutation UI (A4–A6), and publish pre-flight (A7). Everything else is
floxdash parity.

---

## Pass 3 — Remove / relocate candidates

From `D-dependency-layering.md` violations plus this workstream's own pass.
"Destination" is where the code should live, stated as a proposal.

| # | Item | Evidence | Destination / fix |
|---|------|----------|-------------------|
| R1 | `flox-core` → `crossterm` (D #1, HIGH) | `cli/flox-core/Cargo.toml:12`; sole usage `cli/flox-core/src/util/message.rs:3,7,17` | Move `format_error`/`format_updated` into the two consuming binaries (`cli/flox/src/utils/message.rs:7`, `cli/flox-activations/src/message.rs:3`) or a tiny L3 helper crate. Cleans crossterm out of flox-rust-sdk and five other crates in one change |
| R2 | `flox-core` → `supports-color` (D #2, MEDIUM) | `cli/flox-core/Cargo.toml:24`; usage `cli/flox-core/src/util/message.rs:23-29` | Same move as R1; ambient terminal probing has no place in L0 |
| R3 | `nef-lock-catalog` → `flox-core` for one type (D #3, MEDIUM) | `cli/nef-lock-catalog/Cargo.toml:17`; usages `nix_build_lock.rs:6`, `nix_build_config.rs:7` (`flox_core::Version`) | Move `Version` (`cli/flox-core/src/version.rs`) to a smaller leaf crate or inline it; drops crossterm/sentry/sysinfo from a catalog-locking library |
| R4 | `flox-manifest` → `reqwest` for a URL type (D #4, LOW) | `cli/flox-manifest/Cargo.toml:20`; sole usage `cli/flox-manifest/src/raw/mod.rs:10` (`use reqwest::Url;`) | Replace with the `url` crate (already a workspace dep, `Cargo.toml:116`); removes a full HTTP client from the manifest layer floxhub would embed |
| R5 | `flox-core` → `sentry` (D #5, LOW) | `cli/flox-core/Cargo.toml:20`; `cli/flox-core/src/sentry.rs:18-19` reads `FLOX_SENTRY_DSN` from the environment | Relocate `init_sentry` to the binaries (it is only an init helper for them) or a dedicated telemetry crate at L3; telemetry bootstrap is a binary concern |
| R6 | SDK telemetry plumbing: `utils/invocation_sources.rs` + `HEADER_DEVICE_UUID`/`HEADER_INVOCATION_SOURCE` | `cli/flox-rust-sdk/src/utils/invocation_sources.rs:40-90` (scans `COPILOT_*`, CI vars, `FLOX_INVOCATION_SOURCE`); `utils/mod.rs:20-22`; `metrics_device_uuid` field `flox.rs:70` | Relocate detection to the CLI; keep only the header *names* (or an injected `telemetry_headers: HashMap<…>` on `Flox`) in the SDK. A multi-tenant web service must not fingerprint its own process environment per request |
| R7 | `UninitializedEnvironment::bare_description` | `models/environment/mod.rs:736-746` ("description when displayed in a prompt") | Move formatting to the CLI render layer; the SDK already exposes the parts (`name()`, `owner_if_*`) |
| R8 | `ClientTrait::search_with_spinner` | `cli/flox-catalog/src/client.rs:167,295` — a UI word in the API trait; the impl is `search` plus an `#[instrument(fields(progress = …))]` span | Rename (e.g. `search_with_progress_span`) or collapse into `search` once Workstream C's progress contract is documented; no behavioral change needed |
| R9 | CLI-flavored prose in error `Display` impls | `lock_manifest.rs:128-142` ('flox edit -d …'), `lock_manifest.rs:1266-1330` (rendered, indented multi-line advice), `buildenv.rs:151` ('flox auth login'), `fetcher.rs:272`, `publish.rs:792,1964` | Do **not** flatten the enums (AGENTS.md mandates typed variants). Keep variants and their data; move the "run 'flox …'" *suggestion copy* into the CLI's error-rendering layer, or accept the copy as product-level text shared by all consumers — a deliberate decision Workstream C/REPORT should record. floxhub rendering "run 'flox edit'" in a web UI is the failure mode |
| R10 | `RemoteEnvironment` cwd read | `models/environment/remote_environment.rs:469` (`std::env::current_dir()` for `project_path`) | Take the project path as a constructor parameter; the only ambient-cwd read in the SDK |
| R11 | `nix::nix_base_command` publicness | `providers/nix.rs:17`; external consumers exist only because store-gc logic is stranded in `cli/flox/src/commands/gc.rs:93-335` | After A17 lands, narrow to `pub(crate)` |
| R12 | `flox-events` orphan crate (D #6, INFO) | No member `Cargo.toml` depends on it (grep matches only workspace `Cargo.toml:6,46` and `Cargo.lock:1589`) | Not an SDK change: assign it a layer (D policy step 4) or delete; flagged here so the facade question doesn't silently inherit an unanchored crate |

Notably absent from this list: there is no `message`/dialog/pager code inside
`flox-rust-sdk` itself to remove — the terminal contamination is entirely the
transitive `flox-core` carry (R1/R2). The SDK's own remove list is small and
surgical.

---

## Side-effect profile

For each major operation: what it touches, and whether that is acceptable for
**floxdash** (local single-user TUI — side effects on the user's own machine
are the product) and **in-process floxhub** (multi-tenant async web service
linking the SDK). "Per-tenant dirs" means: acceptable only because every path
derives from the injected `Flox{cache,data,state,temp,runtime}_dir`
(`flox.rs:42-47`), so a per-request `Flox` can isolate tenants on disk.

| Operation (entry point) | Filesystem writes (where) | git subprocess | nix subprocess | Network | floxdash | in-process floxhub |
|---|---|---|---|---|---|---|
| **lock** — `CoreEnvironment::lock` (`core_environment.rs:189`) → `LockManifest::lock_manifest` (`lock_manifest.rs:315`) | lockfile next to manifest (`write_atomically`, `core_environment.rs:234`); include fetching may read other envs (`fetcher.rs`) | only for managed/remote includes | none | catalog resolve (reqwest via flox-catalog) | **OK** | **Mostly OK** — pure resolve+serialize; needs per-tenant dirs and `spawn_blocking` (pollster bridge, `core_environment.rs:208`) |
| **install / add-packages** — `Environment::install` (`models/environment/mod.rs:112`) → transact (`core_environment.rs:822`) | manifest+lockfile in `.flox`, temp checkout in `flox.temp_dir`, rendered env out-links | none (path env); floxmeta branch commit for managed | **yes** — `nix build` via `BuildEnvNix` (`buildenv.rs:1283`), writes host-global `/nix/store` | catalog + substituter downloads | **OK** | **NO** — host-global store mutation, unbounded subprocess; needs a build-service boundary |
| **upgrade** — `Environment::upgrade`/`dry_upgrade` (`models/environment/mod.rs:129-140`) | same as install (`dry_upgrade` skips the on-disk manifest write) | as install | **yes** (validation build) | catalog | **OK** | **NO** (same as install); `dry_upgrade`'s resolve-only half would be OK if split from the build |
| **push** — `ManagedEnvironment::push` (`managed_environment.rs:1409`) | local checkout copy + floxmeta git dir under `flox.data_dir` | **yes** — fetch/compare/`push_ref` (`managed_environment.rs:1444-1497`) with token in subprocess config (`floxmeta.rs:206-241`) | **yes** — pre-push validation build (`managed_environment.rs:1440`) | git push to FloxHub; catalog (lock) | **OK** | **Partial** — the git/generation half is per-tenant-safe; the mandatory validation build is the blocker. Server-side, "push" is also conceptually inverted (the server *is* the remote) |
| **pull** — `ManagedEnvironment::pull` (`managed_environment.rs:1511`) | floxmeta + project branch reset; `.flox` updates | **yes** — fetch/reset | none in the SDK call itself (rebuild happens in the caller, `pull.rs:415-530` — stranded item A2) | git fetch | **OK** | **Partial** — metadata sync OK; the follow-up build is the blocker |
| **build (packages)** — `FloxBuildMk::build` (`build.rs:200`, trait `build.rs:54`) | result symlinks + cache dirs in the project | for expression-build preflight (today stranded, A7/A20) | **yes** — make + nix, executes *user-authored build scripts* | substituters | **OK** | **NO** — arbitrary code execution; hard service boundary regardless of crate shape |
| **publish** — `check_*` (`publish.rs:887,1192,1238`) + `PublishProvider` (`publish.rs:554`) | temp netrc in `flox.temp_dir` (`nix_auth.rs:93-107`); clone/checkout temp dirs | **yes** — repo-state checks (`publish.rs:1050`) | **yes** — build metadata + `nix copy` upload | catalog API + store upload | **OK** | **Partial** — metadata checks are in-process-safe; upload/signing path is subprocess+store-bound |
| **activate context assembly** — today stranded in `activate.rs:399-573` (A21); SDK parts: `lockfile()`+`build()`+`rendered_env_links()` (`models/environment/mod.rs:153,188,178`) | env links, temp context file consumed by flox-activations | trust prompts aside, none | **yes** (build) | catalog (lock) | n/a — activate excluded by assumption (services re-entry caveat, A matrix row 16) | **NO** — meaningless server-side (ends in shell exec) |
| **search / show** — `ClientTrait::search`/`package_versions` (`cli/flox-catalog/src/client.rs:177,185`) | none | none | none | catalog HTTPS only | **OK** | **OK** — the only fully web-safe operations today; async end-to-end, no disk |
| **gc** — `env_registry::garbage_collect` (`env_registry.rs:337`) + stranded `nix store gc` (`cli/flox/src/commands/gc.rs:93-335`, A17) | registry JSON + lock file at `env_registry_path(flox)` (`env_registry.rs:239`) | none | `nix store gc` (stranded part) — host-global | none | **OK** | **Split** — registry GC per-tenant OK; store GC is host-global and must never run per-request |

**The structural reading:** every "NO/Partial" in the floxhub column shares one
root cause — the Nix store is host-global and reached by subprocess
(`buildenv.rs:1283`, `build.rs:200`, `publish.rs` upload). That is invariant
under any SDK refactor. The floxhub-viable in-process subset is: search/show,
lockfile/manifest/generations *reading* (`generations.rs:117-180`), lock
(resolve-only), pull/push metadata sync minus builds, and publish metadata
checks. Everything heavier needs a worker/service boundary, which is a
deployment decision for REPORT.md, not a fitness defect of the SDK.

---

## Verdict

**flox-rust-sdk can serve as the API layer directly. A separate `flox-ops`
facade crate is not warranted at this time.**

Criteria that drove the verdict:

1. **Print/TTY cleanliness — pass.** Zero non-test prints, zero TTY probes,
   zero direct terminal-crate deps (Pass 1; `D-dependency-layering.md`
   per-crate table). The one transitive contamination (crossterm via
   flox-core) is fixed by R1/R2 *below* the SDK, not by wrapping it.
2. **Structured results — pass.** Every operation a consumer would call
   returns a typed result (`InstallationAttempt`, `PushResult`, `PullResult`,
   `UpgradeResult`, `LockResult`, `ProcessStates`, `BuildEnvOutputs`,
   generations metadata). A facade would re-export these, adding a layer
   without adding information.
3. **Context injection — pass.** The `Flox` struct (`flox.rs:38-71`) already
   carries dirs, catalog client, auth, and feature flags explicitly; ambient
   reads are confined to host tool paths plus the four flagged items
   (R6, R10, `buildenv.rs:308,1040`). Per-tenant/per-task isolation is a
   constructor call, not a redesign.
4. **The real gaps are additive, not structural.** What floxhub/floxdash
   cannot do today is missing *logic* (Pass 2: ~24 items, dominated by init's
   ~3.5k-LOC detection subsystem and install/pull recovery), not a
   wrongly-shaped SDK. Moving that logic into a hypothetical facade instead of
   the SDK would just rename the SDK problem.
5. **The hard floxhub constraint is orthogonal to crate shape.** The
   side-effect profile shows builds/store mutation gate in-process use; a
   facade crate cannot change that. The honest boundary is
   operation-subsetting (in-process read+metadata ops; worker-dispatched
   builds), which the existing module structure already supports.

What would change the verdict to "a thin `flox-ops` facade is warranted":

- If the SDK's public surface **cannot be tightened** — i.e. if narrowing
  internals (`utils`, `nix_base_command` R11, provider plumbing) to
  `pub(crate)` breaks the CLI in ways that reveal load-bearing coupling, a
  facade becomes the cheaper way to publish a curated surface. (Testable
  mechanically once `cargo` is available: tighten visibility, build the
  workspace.)
- If Workstream C concludes the progress/input contract requires **API-level
  mediation** (e.g. every operation wrapped to emit progress events and accept
  an input-resolver), that wrapper *is* a facade and should be built as one
  rather than rewriting every SDK signature.
- If the sync/async duality (pollster bridges, `core_environment.rs:22,208`)
  must be hidden for an async floxhub — an async-native facade over the sync
  `Environment` trait would be the natural place for `spawn_blocking`
  policy — and the SDK keeps its sync surface for the CLI.
- If real floxhub/floxdash capability requirements (replacing this document's
  Assumptions) demand a stability-versioned API contract: a facade crate is
  the standard tool for "stable narrow surface over a moving implementation".

Conditions attached to the direct-use verdict: execute R1–R5 (the dependency
hygiene below the SDK), R6–R10 (the SDK's own leaks), land the P1 add items
(A1–A7), and adopt D's layering enforcement so the now-clean state is
checkable. Without R1/R2 in particular, "the SDK is the API layer" remains
false in the only sense a web service cares about: its dependency closure.

---

## How to reproduce

All commands from the repository root `/home/user/flox`. None require the dev
shell.

```bash
# 1. SDK module tree and sizes
cat cli/flox-rust-sdk/src/lib.rs
find cli/flox-rust-sdk/src -name '*.rs' | xargs wc -l | sort -rn

# 2. Hygiene checks: prints, TTY probes, ambient reads, cwd
grep -rnE '(^|[^a-z_])(println!|eprintln!|eprint!|print!)' cli/flox-rust-sdk/src   # then confirm each hit is under a preceding #[cfg(test)]
grep -rn 'is_terminal\|isatty\|atty\|stdin()' cli/flox-rust-sdk/src --include='*.rs'   # no hits
grep -rn 'env::var\|var_os' cli/flox-rust-sdk/src --include='*.rs'
grep -rn 'current_dir()' cli/flox-rust-sdk/src --include='*.rs'                        # one hit: remote_environment.rs:469

# 3. CLI-flavored prose inside SDK error types
grep -rn "run 'flox\|'flox auth login'\|'flox edit\|'flox config" cli/flox-rust-sdk/src --include='*.rs'
sed -n '110,160p;1260,1335p' cli/flox-rust-sdk/src/providers/lock_manifest.rs

# 4. Typed result types (the clean-operation evidence)
grep -rn 'pub struct InstallationAttempt\|pub struct UninstallationAttempt\|pub enum PushResult\|pub enum PullResult\|pub enum EditResult\|pub struct UpgradeResult\|pub enum LockResult\|pub enum SyncToGenerationResult' cli/flox-rust-sdk/src

# 5. Progress span-field convention (the headless progress seam)
grep -rn 'progress = ' cli/flox-rust-sdk/src cli/flox-catalog/src --include='*.rs'

# 6. Sync-over-async bridges and the CLI runtime
grep -rn 'use pollster\|block_on' cli/flox-rust-sdk/src --include='*.rs'
grep -n 'Runtime::new' cli/flox/src/main.rs

# 7. Side-effect anchors: subprocesses and store writes
grep -n 'NIX_BIN\|nix_base_command' cli/flox-rust-sdk/src/providers/nix.rs
sed -n '1409,1505p' cli/flox-rust-sdk/src/models/environment/managed_environment.rs  # push: build+git+network
sed -n '337,345p' cli/flox-rust-sdk/src/models/env_registry.rs                       # registry gc
grep -n 'GcProgress\|nix store gc' cli/flox/src/commands/gc.rs                       # stranded store gc

# 8. Pass-2 sources: the stranded-logic list
#    docs/architecture-analysis/A-command-audit.md, section "Stranded logic";
#    spot-check any row, e.g.:
sed -n '309,458p' cli/flox/src/commands/install.rs
sed -n '277,409p' cli/flox/src/commands/pull.rs

# 9. Pass-3 sources: dependency violations
#    docs/architecture-analysis/D-dependency-layering.md, "Violations list";
grep -rn 'crossterm\|supports-color\|sentry' cli/flox-core/Cargo.toml
grep -rn 'reqwest' cli/flox-manifest/src cli/flox-manifest/Cargo.toml
grep -rn 'flox_core' cli/nef-lock-catalog/src
grep -rn 'flox-events\|flox_events' --include='Cargo.toml' .
```
