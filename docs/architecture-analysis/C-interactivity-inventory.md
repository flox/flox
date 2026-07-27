# Workstream C — Interactivity & Side-Effects Inventory

**Date:** 2026-06-11

This inventory answers GOAL.md Workstream C's question: what in the flox
command layer breaks headless use, and what single progress/input contract
could the CLI, floxdash (TUI), and floxhub (web) share? It starts from the
eight mid-operation prompts cataloged in
`docs/architecture-analysis/A-command-audit.md` ("Mid-operation
interactivity"), verifies each at its cited source location, characterizes
what state exists when the prompt fires and what each answer does, and
classifies every interaction point as **hoistable** (resolvable before the
operation runs), **modelable** (the operation must return a typed
"needs input" outcome the caller loops on), or **structural** (can never be a
pure API call). It then covers the surrounding interaction machinery —
up-front prompts, progress reporting, pager, exit codes, ambient-state reads —
and closes with a design memo recommending the shared contract, worked through
on the four hardest real cases. All line numbers refer to the working tree on
the date above; paths are relative to the repository root.

## Classified inventory

### Mid-operation prompts (verified, one row per prompt)

| # | Interaction point | Location | Classification | Rationale |
|---|---|---|---|---|
| 1 | `edit`: "Continue editing?" `Confirm` loop | `cli/flox/src/commands/edit.rs:298-326` | **modelable** (already 90% modeled) + structural `$EDITOR` leg | Each loop iteration is already a pure call: `environment.edit(flox, new_manifest)` (`edit.rs:310`) returns `Result<EditResult, EnvironmentError>`, and `make_interactively_recoverable` (`edit.rs:330-352`) is a typed classifier of which errors are retryable. The loop, the prompt, and the `$EDITOR` spawn (`edit.rs:430-437`) are all caller-side. The `$EDITOR` subprocess itself is structural (CLI-only). |
| 2 | `pull`: add-your-system `Select` | `cli/flox/src/commands/pull.rs:536-559`, invoked at `pull.rs:454` | **modelable** — the existing seam | Fires inside `handle_pull_result` (`pull.rs:415-530`) after the clone (`ManagedEnvironment::open`, `pull.rs:329`), pointer write (`pull.rs:324-326`), and a failed build (`pull.rs:353`) — i.e. `.flox/` exists on disk and a network clone has completed. "No" deletes `.flox/` and bails (`pull.rs:456-465`); "Yes" mutates the manifest via `amend_current_system` (`pull.rs:467`, defined `pull.rs:562-579`) and rebuilds (`pull.rs:468-474`). The decision is injected as `QueryFunctions` (`pull.rs:86-89`, wired at `pull.rs:362-365`), so the operation core is already prompt-free; see "The QueryFunctions seam" below. `--force` is the hoisted answer (`pull.rs:454`). |
| 3 | `pull`: ignore-build-errors `Select` | `cli/flox/src/commands/pull.rs:582-604`, invoked at `pull.rs:512` | **modelable** — same seam | Fires on `BuildEnvError::Realise2` (`pull.rs:492-494`) after the same clone/pointer-write/build state as #2. "Yes" keeps the broken environment with a warning (`pull.rs:512-517`); "No" deletes `.flox/` and bails (`pull.rs:518-522`). Also pre-answerable with `--force` (`pull.rs:512`). |
| 4 | `install`: onboarding create-default-env `Select` | `cli/flox/src/commands/install.rs:597-606` | **hoistable** (by decomposition) | Fires when environment detection fails (`install.rs:175-183`). State at prompt time: nothing has been mutated — only the user-state file has been locked and read (`install.rs:580-581`; the file lives in the SDK, `cli/flox-rust-sdk/src/models/user_state.rs:39,93`). "No" records the refusal and exits 1 (`install.rs:611-621`); "Yes" *then* runs `ensure_auth` and creates/pulls the remote default env (`install.rs:626-649`). Since the answer precedes every mutation, the decision can be hoisted: resolve "no environment found" up front and treat env-creation as its own operation. |
| 5 | `install`: modify-RC-file `Select` | `cli/flox/src/commands/install.rs:704-711`, called at `install.rs:658` | **hoistable** (by decomposition); the RC mutation itself is CLI-machine-only | Fires after the remote default environment has been created (`install.rs:638-648`) and the choice recorded (`install.rs:654-656`). But the prompt's subject — appending an activation line to `~/.bashrc` etc. (`add_activation_to_rc_file`, `install.rs:764-780`) — is an independent follow-up side effect, not a recovery from the prior step. Modeled as a second operation ("add shell auto-activation"), its confirmation hoists to the front of that operation. For floxhub the operation is meaningless (the RC file lives on the user's machine), making it structural for that consumer. |
| 6 | `activate`: trust `Select` loop for remote includes | `cli/flox/src/commands/activate.rs:352-367` → `ensure_environment_trust`, `cli/flox/src/commands/mod.rs:1341-1390` | **modelable** (with hoistable fast paths) | Fires per remote include after `lockfile()` has run (`activate.rs:343-349`) — the set of remote includes is only knowable post-locking (`lockfile.compose`, `activate.rs:353-356`). Policy short-circuits already exist and are pure: `--trust` flag (`activate.rs:352`), `flox`-owned envs (`mod.rs:1280-1283`), own envs (`mod.rs:1285-1289`), config `trusted_environments` (`mod.rs:1291-1304`). The residual 5-way dialog (`mod.rs:1343-1357`) loops on Trust/Deny (persisted via `update_config` + config re-parse, `mod.rs:1362-1382`), temporary trust/deny, or show-manifest (`mod.rs:1388`). Typed outcome: "needs trust decision for {env_ref, manifest}". |
| 7 | `auth login`: Enter-key `Checkpoint` raced against token polling | `cli/flox/src/commands/auth.rs:156-187` | **modelable** core; the keypress/browser leg is structural | The device-code grant has been requested (`auth.rs:106-116`) and token polling is in flight (`auth.rs:130-135`) when the `tokio::select!` races an Enter keypress (raw-mode terminal listener, `cli/flox/src/utils/dialog.rs:45-74,102-105`) against token arrival (`auth.rs:168-187`). Enter spawns a browser (`auth.rs:174-182`). The *operation* is modelable as: start device flow → return `{verification_uri, code, expires_in}` (`auth.rs:122-127`) → await token; only the press-Enter/open-browser presentation is terminal-bound. The no-browser path (`auth.rs:189-200`) already proves the prompt is optional. |
| 8 | `init`: language-hook accept prompts and version `Select`s | `cli/flox/src/commands/init/mod.rs:325-330`; `init/node.rs:930-951`; `init/python.rs:114-142`; `init/go.rs:115-134` | **hoistable** (by decomposition) | Detection (`Node::new`/`Python::new`/`Go::new`, `init/mod.rs:311-321`) performs read-only filesystem inspection and catalog resolution; no environment exists yet when the prompts fire. Each hook already separates `prompt_user` from `get_init_customization` (e.g. `init/python.rs:146-160`), and `--auto-setup` bypasses every prompt (`init/mod.rs:327`). Hoisted form: operation A "detect" returns suggested `InitCustomization`s; caller chooses; operation B "init with chosen customizations". The show-modifications loop options (`node.rs:944-948`, `python.rs:135-140`, `go.rs:129-131`) are render concerns. |

### Other interaction points (patterns, classified)

| Interaction point | Location | Classification | Rationale |
|---|---|---|---|
| Environment disambiguation prompt (active vs. found-in-dir) | `detect_environment` `cli/flox/src/commands/mod.rs:1171-1221`; prompt in `query_which_environment` `mod.rs:1224-1255` | **hoistable** | Pure pre-operation selection; resolution policy (`mod.rs:1178-1219`) is prompt-free except one branch, and already falls back deterministically when no TTY (`mod.rs:1199-1204`). API consumers pass an explicit environment ref instead. |
| Auth login fallback | `ensure_auth`, `cli/flox/src/commands/mod.rs:1397-1430` | **hoistable** | Up-front credential check before the operation; non-TTY path bails with instructions (`mod.rs:1412-1420`). API consumers authenticate out of band. |
| `delete` confirmation | `cli/flox/src/commands/delete.rs:67-77` | **hoistable** | Classic pre-operation confirm; `-f` is the hoisted answer (`delete.rs:75`). |
| `activate` trust for the directly-activated remote env | `cli/flox/src/commands/activate.rs:206-219` | **hoistable** | Same `ensure_environment_trust` machinery as row 6 but fires before any locking/mutation; `--trust` and config pre-answer it. |
| `edit` interactive-mode gate | `cli/flox/src/commands/edit.rs:276-278` | **hoistable** | `Dialog::can_prompt()` TTY check; non-interactive callers use `--file` (`edit.rs:408-422`). |
| `activate` shell exec | `cli/flox/src/commands/activate.rs:597` (`command.exec()`) | **structural** | Replaces the CLI process with the activation subprocess; by definition not a returnable API call. The ephemeral variant (`activate.rs:575-592`) captures output instead and is modelable. |
| `$EDITOR` subprocess | `cli/flox/src/commands/edit.rs:430-437` | **structural** | Spawns and waits on an interactive terminal program on the user's machine. |
| Pager | `page_output`, `cli/flox/src/utils/message.rs:94-106` | **structural** (presentation-only) | Terminal-window-aware paging via `minus`; pure render concern, never blocks an operation. |
| Raw-mode key listening | `cli/flox/src/utils/dialog.rs:45-74` | **structural** | crossterm raw-mode keyboard handling; CLI presentation primitive. |

## Up-front prompt machinery (pattern summary)

All prompting funnels through one utility, `Dialog`
(`cli/flox/src/utils/dialog.rs:91-95`), with three typed shapes: `Confirm`
(`dialog.rs:78-80,108-133`), `Select` (`dialog.rs:82-84,142-204`), and
`Checkpoint` (press Enter; `dialog.rs:88,97-106`). Prompts render via
`inquire` under a global stderr lock (`TERMINAL_STDERR`,
`cli/flox/src/utils/mod.rs:22`; taken at `dialog.rs:116,157,187`).
Headless-ness is decided by exactly one predicate:
`Dialog::can_prompt()` — stdin *and* stdout *and* stderr must all be TTYs
(`dialog.rs:206-213`). Every interactive flow either checks it and bails
(`edit.rs:276-278`, `auth.rs:96-98`, `install.rs:576-578`,
`mod.rs:1335-1339`) or checks it and degrades to a default
(`pull.rs:362` passes `None` query functions; `pull.rs:583-585` answers "no";
`delete.rs:75` skips the confirm; `mod.rs:1199-1204` picks the found env;
`init/mod.rs:327` skips hooks unless `--auto-setup`).

Environment selection is the dominant up-front pattern: the bpaf-external
`EnvironmentSelect` enum (variants `Dir`/`Remote`/`Default`/`Unspecified`)
resolves to a `ConcreteEnvironment` either without detection
(`to_concrete_environment`, `cli/flox/src/commands/mod.rs:997-1059`) or with
active-environment detection plus the disambiguation prompt
(`detect_environment` → `query_which_environment`, `mod.rs:1171-1255`).
Commands embed it as a field and call it first in `handle` (e.g.
`install.rs:69-70,159-186`; `delete.rs:22-31`). This is the cleanest evidence
that ~90% of prompting is already hoisted: the prompt resolves an *input*
(which environment), not a mid-flight condition.

## Progress reporting: the tracing-indicatif span pattern

Progress is **already an event-stream contract**, not direct terminal
writes:

- Any tracing span carrying a field literally named `progress`
  (`PROGRESS_TAG`, `cli/flox/src/utils/init/logger.rs:222`) is rendered as an
  indicatif spinner by an `IndicatifLayer` installed at logger init
  (`logger.rs:286-298`); a filter shows *only* spans with that field
  (`logger.rs:292-296`). Span nesting renders as child-prefixed spinners
  (template at `logger.rs:273`), with elapsed time after 1s
  (`logger.rs:275-284`).
- Producers use either `#[instrument(fields(progress = "..."))]` (e.g.
  `cli/flox/src/commands/gc.rs:250`,
  `cli/flox-catalog/src/client.rs:292-302` — `search_with_spinner` is just
  `search` wrapped in a progress span) or
  `info_span!(..., progress = "...").in_scope(...)`
  (e.g. `pull.rs:468-474`).
- Crucially, **the SDK already emits progress this way**: 25 `progress =`
  span fields across 9 `flox-rust-sdk` files (7 in
  `cli/flox-rust-sdk/src/providers/buildenv.rs`, 4 in
  `models/environment/managed_environment.rs`, 3 in `providers/publish.rs`,
  etc. — see "How to reproduce"). The SDK has no indicatif dependency
  (GOAL.md baseline); the field is inert until a subscriber renders it.
- Even **user-facing messages are tracing events**: `message::*` calls
  `info!` (`cli/flox/src/utils/message.rs:20-22`), and a dedicated fmt layer
  selects events by target `flox::utils::message` (`logger.rs:148-157`),
  writing through the indicatif writer so spinners and messages interleave
  cleanly (`logger.rs:149,174`).
- The most elaborate producer is `gc`: a scoped thread parses `nix store gc`
  stderr into a `GcProgress` state machine and re-opens a child
  `progress`-tagged span per state change (`gc.rs:250-315`).

Implication: a headless consumer can swap the indicatif layer for its own
subscriber today and receive identical progress semantics. This is the
foundation the design memo builds on.

## Pager usage

One pager helper, `message::page_output`
(`cli/flox/src/utils/message.rs:94-106`), built on `minus::Pager` with
`run_no_overflow(false)` so output prints straight through when it fits the
terminal or when stdout is not interactive. Exactly two call sites:
`generations list` (`cli/flox/src/commands/generations/list.rs:88`) and
`generations history` (`generations/history.rs:79`); both commands also offer
`--json` (`generations/list.rs:47`, `generations/history.rs:42`), so paging
is purely a human-output affordance. Classification: structural
presentation, no API impact.

## Exit-code mapping

The process exit surface is effectively binary plus a silent-exit escape
hatch:

- `Ok(())` → 0; any `Err` → 1, after downcast-driven message formatting for
  `EnvironmentError`, `ManagedEnvironmentError`, `RemoteEnvironmentError`,
  `EnvironmentSelectError`, and `ServiceError`
  (`cli/flox/src/main.rs:157-207`).
- `struct Exit(ExitCode)` (`main.rs:241-247`) is an error type meaning "exit
  with this code, print nothing" (`main.rs:162-164`). All four uses carry
  code 1: declined onboarding (`install.rs:621`), failed ephemeral activation
  whose stderr was already relayed (`activate.rs:586`), and `auth status` /
  `auth token` when not logged in (`auth.rs:282`, `auth.rs:300`).
- bpaf parse failures: `--help`-style stdout → 0, parse error → 1,
  completion output → 0 (`main.rs:133-148`); early exits for
  `--bpaf-complete-style-bash`, `--prefix`, `--version` → 0
  (`main.rs:63-78`); `set_user` failure → 1 (`main.rs:95-98`).
- `activate`'s exec path surrenders the exit code to the replacement process
  entirely (`activate.rs:597`).

There is no semantic exit-code taxonomy (no "conflict = 2, auth = 3, …"),
which matters for Workstream E's plugin contract: plugins and scripts can
distinguish only success/failure today.

## Ambient-state reads

What an operation's outcome depends on besides its arguments:

- **Config.** `config::Config::parse()` runs at startup (`main.rs:100`) and
  again inside the command runner (`main.rs:36`); it layers system config
  (`$FLOX_SYSTEM_CONFIG_DIR`, `cli/flox/src/config/mod.rs:236-248`), user
  config (`$FLOX_CONFIG_DIR` / XDG discovery — which *sets* the env var as a
  side effect, `config/mod.rs:256-298`, set at `:292`), the TOML files, and
  `FLOX_*` environment-variable overrides (`config/mod.rs:343`). The trust
  loop both writes config and re-parses it mid-operation
  (`mod.rs:1362-1382`).
- **Process environment mutation at startup.** `main` removes
  `FLOX_VERSION_VAR` (`main.rs:50-51`), sets `_FLOX_SUBSYSTEM_VERBOSITY`
  (`main.rs:116-121`), and rewrites `$USER`/`$HOME` to match the effective
  uid (`set_user`, `main.rs:254-280`).
- **Env-var reads inside command handlers** (grep inventory; see
  reproduction): `$VISUAL`/`$EDITOR`/`$PATH` (`edit.rs:366-368`),
  `FLOX_PROMPT_COLOR_1/2` (`activate.rs:479-481`), `_FLOX_FLOXHUB_GIT_URL`
  (`mod.rs:254`), `$USER` (`mod.rs:288`), `_FLOX_OAUTH_*` endpoint overrides
  (`auth.rs:73-86`), `$PATH` for container runtimes
  (`containerize/mod.rs:293,325`), `$GOWORK` (`init/go.rs:292-360`),
  prompt-hook and verbosity vars (`deactivate.rs:170,227`), test kill-switch
  (`check_for_upgrades.rs:185`).
- **cwd dependence.** Environment detection walks up from
  `env::current_dir()` (`mod.rs:1012-1013`, `mod.rs:1174-1176`); `pull`
  defaults its target dir to cwd (`pull.rs:109`); `init` defaults to cwd
  (`init/mod.rs:121`); `build` shortens result links relative to cwd
  (`build.rs:303-310`); `hook_env` reads cwd (`hook_env.rs:85`);
  active-vs-current comparison canonicalizes cwd (`mod.rs:1464-1469`).
- **TTY-ness as control flow.** `Dialog::can_prompt()`
  (`dialog.rs:206-213`) and `stdout().is_tty()` choosing interactive vs.
  in-place activation (`activate.rs:223`).
- **Cross-invocation user state.** The onboarding choice persists in a
  locked user-state file (`install.rs:580-586,654-656`;
  `cli/flox-rust-sdk/src/models/user_state.rs:39,93`).

For an API consumer, every one of these must become either an explicit
parameter (cwd → environment ref; config → typed settings struct) or a
documented server-side default. None is conceptually hard; the risk is only
that they are *implicit* today.

---

## Design memo: one progress/input contract for CLI, floxdash, and floxhub

**Recommendation.** Adopt a two-channel contract on the operations layer:

1. **Progress = tracing spans with the `progress` field** — i.e. standardize
   what already exists. The SDK emits 25 such spans with zero terminal
   dependencies; the CLI renders them with `IndicatifLayer`
   (`logger.rs:286-298`), floxdash renders the same stream as TUI widgets,
   and floxhub forwards them as SSE/log lines. The only work is declaring
   the field name (`PROGRESS_TAG`, `logger.rs:222`) and span-nesting
   semantics a public contract, and finishing the migration of direct
   terminal writes (`message::*` is already a tracing event with a filterable
   target, `message.rs:20-22` + `logger.rs:155-157`, so even final messages
   can ride this channel for floxdash).

2. **Input = hoist by default; typed `NeedsX` outcomes for the rest.** Every
   decision resolvable before the operation becomes a parameter (the ~90%:
   environment ref, confirmations, auth, trust-policy overrides, init
   customization choices). The residual mid-operation decisions — those whose
   *subject* is only discovered mid-flight — are returned as serializable
   variants of the operation's result enum, and the caller answers by making
   a follow-up call. Operations must additionally leave the world in a named
   state when they return a `NeedsX` outcome (today `pull` deletes `.flox/`
   on abort, `pull.rs:441,458,519,526` — under the contract, "abort" becomes
   an explicit cleanup call rather than an implicit consequence of answering
   "no").

   **Rejected alternative:** injected callbacks. The codebase already
   contains the modelable pattern as `QueryFunctions` (`pull.rs:86-89`):
   `handle_pull_result` takes `Option<{query_add_system, query_ignore_build_errors}>`
   where `None` *means* "non-interactive" and forces the conservative
   default (`pull.rs:440-448,502-505`). This proves prompts can be pulled
   out of operation logic and that "cannot prompt" is a first-class state —
   but function pointers do not cross an HTTP boundary, and a callback
   blocks the operation's transaction while a human thinks. Keep the
   *semantics* (every decision has a typed question, a conservative
   no-answer default, and a forced path), change the *mechanism* from
   injected functions to returned outcomes.

   For **floxhub (non-interactive) the rule is uniform**: every `NeedsX`
   outcome must map to either a request parameter that pre-answers it (the
   `--force` analogue) or a typed API error naming the unanswered decision.
   No decision point may default to hanging or to an interactive fallback.

### Worked case (a): `edit`'s continue-editing loop

```
CURRENT (edit.rs:298-326)
  CLI: write manifest copy to tmpfile (edit.rs:286-296)
  loop:
    CLI: spawn $EDITOR on tmpfile, wait (edit.rs:309 -> 430-437)
    CLI: env.edit(flox, contents)            -- pure SDK call (edit.rs:310)
    SDK: -> Ok(EditResult) ............ exit loop, render
         -> recoverable err (classified by edit.rs:330-352)
              CLI: print error; prompt "Continue editing?" (edit.rs:316-323)
              Yes -> loop again    No -> bail "cancelled"

MODELED
  op edit(env, contents) -> EditOutcome::Applied(EditResult)
                          | EditOutcome::Invalid(RecoverableError)   [typed, serializable]
  CLI: loop { spawn $EDITOR; call edit(); on Invalid -> confirm -> retry }
  floxdash: same loop with an in-TUI editor pane
  floxhub: web editor POSTs manifest; Invalid -> 422 with the typed error
           rendered next to the editor. The "loop" IS the request/response
           cycle; no prompt exists. Nothing further to hoist: the prompt was
           never operation state, only CLI pacing.
```
The operation side needs no change beyond promoting the
`make_interactively_recoverable` classification (`edit.rs:330-352`) into the
SDK error taxonomy (it inspects `EnvironmentError` variants — SDK knowledge
stranded in the CLI; also flagged in A's stranded-logic list).

### Worked case (b): `pull`'s post-build recovery dialogs

```
CURRENT (pull.rs:277-409, 415-530)
  op: write pointer (324-326); clone via ManagedEnvironment::open (329);
      build (353)
  build fails:
    incompatible system -> query_add_system (454 -> 536-559)
         No  -> rm .flox/, bail (456-465)
         Yes -> amend manifest + rebuild (467-474); broken? warn (476-486)
    Realise2 build error -> query_ignore_build_errors (512 -> 582-604)
         No  -> rm .flox/, bail (518-522)
         Yes -> keep broken env, warn (512-517)
  (--force pre-answers both: 454, 512; no-TTY forces No: 440-448, 502-505)

MODELED
  op pull(ref, dir, policy{add_system: bool?, allow_broken: bool?})
     -> PullOutcome::Complete
      | PullOutcome::IncompatibleSystem { system }     [env kept, state=pending]
      | PullOutcome::Broken { build_error }            [env kept, state=pending]
  follow-ups: op amend_system(env) ; op accept_broken(env) ; op discard(env)
  CLI: on IncompatibleSystem -> Select -> amend_system | discard
  floxdash: same, as a modal
  floxhub: caller sets policy in the request (the --force analogue,
           split per decision). Without policy, IncompatibleSystem/Broken
           return as typed 409-class errors carrying the same payload;
           the client retries with policy set. No pending server state
           needed if floxhub always supplies policy up front.
```
Note the contract change hiding here: today "No" *destroys* the clone
(`pull.rs:458,519`). The modeled flow keeps it in a pending state with an
explicit `discard`, because a returned outcome must not require the answer
to arrive in the same process lifetime.

### Worked case (c): `install`'s onboarding flow

```
CURRENT (install.rs:159-186, 564-661, 673-734)
  env detection fails (175) ->
    read+lock user state (580-586); bail if previously answered (584-586)
    PROMPT "create default env?" (597-606)
      No  -> persist refusal; Exit(1) (611-621)
      Yes -> ensure_auth (628); create/pull remote default env (638-648);
             persist choice (654-656)
             PROMPT "modify RC files?" (704-711)
               Yes -> append eval line to ~/.bashrc etc. (724-733, 764-780)
               No  -> print docs link (720-722)
    ... then the actual install proceeds into the new env

HOISTED
  op resolve_env(...) -> ResolvedEnv | NoEnvironment        [read-only]
  op create_default_env(auth) -> RemoteEnv                  [own operation]
  op add_shell_autoactivation(shell) -> ()                  [own op, CLI-local]
  op install(env, pkgs) -> InstallationAttempt              [unchanged]
  CLI on NoEnvironment: prompt -> create_default_env -> prompt ->
       add_shell_autoactivation -> install. All prompts now sit at the
       FRONT of the operation they gate.
  floxdash: same sequence; RC-file step offered only when running locally.
  floxhub: never reaches the flow — the environment is explicit in every
       request, so NoEnvironment is a plain 4xx; create-default-env is a
       deliberate endpoint; add_shell_autoactivation does not exist
       (the RC file is on the user's machine -> structural for web).
```

### Worked case (d): `activate`'s include-trust loop

```
CURRENT (activate.rs:343-367 -> mod.rs:1263-1390)
  op: lock environment (343-349)  -- discovers remote includes (353-356)
  per remote include, unless --trust (352):
    pure policy chain: flox-owned (1280) / own env (1285) / config
    trust (1291) / config deny -> bail (1296-1304)
    else PROMPT 5-way loop (1341-1390):
      Trust(save)  -> write config, re-parse, proceed (1362-1372)
      Deny(save)   -> write config, bail (1373-1382)
      Trust(once)  -> proceed (1383-1386)
      Abort        -> bail (1387)
      ShowManifest -> print, re-prompt (1388)

MODELED
  op lock(env, trust_overrides) ->
       LockOutcome::Locked(Lockfile)
     | LockOutcome::NeedsTrust(Vec<{env_ref, manifest}>)   [nothing mutated;
                                                            locking is re-runnable]
  op record_trust(env_ref, Trust|Deny) -> ()               [the config write]
  CLI: on NeedsTrust -> per entry, 5-way Select (ShowManifest is render);
       persistent answers call record_trust; re-invoke lock.
  floxdash: same, as a list of trust cards.
  floxhub: trust must be pre-recorded (account-level trusted_environments,
       mirroring config.flox.trusted_environments at mod.rs:1270) or
       supplied as trust_overrides in the request; NeedsTrust surfaces as
       a typed error enumerating the untrusted refs. The terminal step of
       activate (exec, activate.rs:597) remains structural — floxhub shares
       only the resolution/locking/trust phase, which is exactly the part
       this models.
```

**Why one contract suffices:** all eight mid-operation prompts reduce to
three shapes — a retryable-input loop (a), a recovery decision over a kept
intermediate state (b), and a discovered-set approval (d) — plus pure
hoisting (c, and #4/#5/#8). Each shape is expressible as "operation returns
typed outcome; caller answers via parameter or follow-up call", and progress
is orthogonal on the span channel. The cost of *not* deciding this once is
visible in the codebase already: `pull` invented callbacks
(`pull.rs:86-89`), `edit` invented an error-classifier loop
(`edit.rs:330-352`), `init` invented `prompt_user`/`get_init_customization`
splits (`init/python.rs:146-160`), and `activate` invented a config-backed
policy chain (`mod.rs:1270-1304`) — four bespoke solutions to the same
problem.

## Structural list (what floxhub can never share)

- `activate`'s process replacement — `command.exec()`
  (`cli/flox/src/commands/activate.rs:597`). The resolution/locking/trust
  phase above it is shareable; the exec is not.
- `edit`'s `$EDITOR` subprocess (`edit.rs:430-437`) — replaced by a web
  editor; the surrounding edit/validate loop is shareable (case a).
- `install`'s RC-file mutation (`install.rs:724-733,736-780`) — writes to
  shell init files on the invoking machine.
- `auth login`'s raw-mode Enter listener and browser spawn
  (`dialog.rs:45-74`, `auth.rs:168-182`) — floxhub *is* the other end of
  the device flow; the CLI presentation has no server analogue.
- Pager (`message.rs:94-106`), spinner rendering (`logger.rs:286-298`),
  inquire prompts under the stderr lock (`dialog.rs:108-204`), and the
  `Dialog::can_prompt` TTY gate (`dialog.rs:206-213`) — terminal
  presentation primitives; consumers bring their own.
- cwd-based environment discovery (`mod.rs:1171-1176`) — meaningful only
  where a working directory exists; a web request supplies the ref.
- (floxdash, running on the user's machine, shares everything above except
  nothing — only `activate`'s exec is off-limits to it if it must keep its
  own UI process alive; it can use the ephemeral path,
  `activate.rs:575-592`.)

## Assumptions

- floxhub is assumed non-interactive request/response (GOAL.md Workstream B
  default: read operations plus push/pull/publish-adjacent mutations);
  floxdash is assumed interactive, local, full-parity-minus-activate. No
  capability sketch was provided.
- "State when the prompt fires" describes mutations performed by the
  *current* invocation before the prompt, verified by reading the call path;
  background processes (e.g. the upgrade checker spawned at
  `activate.rs:260-265`) were not traced.
- Classification of install onboarding (#4) as hoistable departs from A's
  MID label deliberately: A's criterion was "any work has begun" (the
  user-state read/lock at `install.rs:580-586`); this inventory's criterion
  is "any mutation, or any discovery the answer depends on, has occurred" —
  neither holds at `install.rs:597`.
- The memo's floxhub sequence sketches (HTTP status pairings, account-level
  trust store) are illustrative design intent, not requirements gathered
  from floxhub.
- `cargo`/`nix` were unavailable; all evidence is from grep and reading.
  Line numbers are from the working tree on 2026-06-11 and will drift.

## How to reproduce

All commands run from the repository root.

```bash
# 1. Verify the eight mid-operation prompts (read at the cited lines)
sed -n '298,326p;330,352p;425,441p' cli/flox/src/commands/edit.rs
sed -n '83,89p;277,409p;415,604p'   cli/flox/src/commands/pull.rs
sed -n '564,780p'                   cli/flox/src/commands/install.rs
sed -n '206,219p;343,367p;575,598p' cli/flox/src/commands/activate.rs
sed -n '1171,1430p'                 cli/flox/src/commands/mod.rs
sed -n '95,219p'                    cli/flox/src/commands/auth.rs
sed -n '303,333p'                   cli/flox/src/commands/init/mod.rs

# 2. All real dialog usages in the command layer
grep -rnE 'Dialog \{|Dialog::can_prompt|raw_prompt|checkpoint_async' \
  cli/flox/src/commands cli/flox/src/utils

# 3. Progress span pattern and producers
grep -n 'PROGRESS_TAG\|IndicatifLayer' cli/flox/src/utils/init/logger.rs
grep -rnc 'progress\s*=' cli/flox/src/commands cli/flox-rust-sdk/src \
  cli/flox-catalog/src

# 4. Pager
grep -rn 'page_output\|minus' cli/flox/src --include='*.rs'

# 5. Exit-code mapping
grep -n 'struct Exit\|ExitCode' cli/flox/src/main.rs
grep -rn 'Exit(' cli/flox/src

# 6. Ambient state
grep -rn 'env::var\|std::env::var' cli/flox/src/commands --include='*.rs'
grep -rn 'current_dir' cli/flox/src/commands --include='*.rs'
grep -n 'FLOX_CONFIG_DIR\|FLOX_SYSTEM_CONFIG_DIR\|strip_prefix("FLOX_")' \
  cli/flox/src/config/mod.rs

# 7. The QueryFunctions seam and its no-prompt defaults
sed -n '83,90p;355,366p;440,455p;500,513p' cli/flox/src/commands/pull.rs
```
