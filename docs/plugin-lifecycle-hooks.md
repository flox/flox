# Plugin lifecycle hooks

Status: DESIGN — prototype branch `prototype/sandbox-plugins`.
Author: Daniel Sauble, 2026-08-27. Revised same day after a four-lens
adversarial review (correctness, security, plugin-author DX, migration).

This document designs a complete set of extension points ("hooks") across
the lifecycle of a Flox environment, building on the experimental
`[plugins]` mechanism (schema v1.14.0, PR #4535) and the beta subcommand
extensions (PR #4595). It exists to answer two questions:

1. What is the full lifecycle of a Flox environment, and where can a
   plugin participate in it?
2. What is the minimal set of *new* extension points needed to deliver
   sandboxed activation entirely as plugins — with no sandbox-specific
   code in flox core?

The second question is the forcing function. The
`prototype/sandboxed-activation` branch implemented 15 sandbox backends
(~27k lines in `cli/`) as a first-class CLI feature. That work is being
refactored so that flox core gains only *generic* lifecycle hooks, and
every backend becomes an ordinary plugin package developed in
`flox/flox-plugins` and distributed through the Flox Catalog. Sandboxes
are the validation workload for the hook design, not its subject: core
never says the word "sandbox".

Decisions already made (2026-08-27):

- **Generic hooks only.** No `--sandbox` flag, no `[options.sandbox]`
  manifest table, no `flox sandbox` core subcommand. Sandbox
  configuration lives in each plugin's `[plugins.<name>]` table; the
  grants/review UI ships as a `flox-sandbox` subcommand extension.
- **All 15 providers migrate**, including the in-process libsandbox
  engine (sequenced last — it requires the two hardest hooks).
- **Fresh branch from main** (`prototype/sandbox-plugins`); the old
  branch remains as reference until migration to `flox/flox-plugins`
  completes.
- **Design every phase, implement only what sandboxes exercise.**
  Hooks marked *(build now)* below are implemented in this prototype;
  hooks marked *(design only)* are named and shaped here so future
  extension points stay coherent, but are not built until a consumer
  exists.

## 1. What a plugin is

Unchanged from the v1.14.0 convention established in
`flox/flox-plugins`: **a plugin is an ordinary package** installed via
`[install]`, whose files are merged into the rendered environment by
the buildenv symlink forest. Today the only recognized payload is
`etc/profile.d/*.sh`, sourced during activation
(`assets/environment-interpreter/activate/activate`), with per-plugin
manifest data readable via `flox_plugin_data`
(`assets/environment-interpreter/common/activate.d/helpers.bash`).

This design adds further well-known payload paths (the hook tree,
section 4) and one new piece of typed manifest surface (the
`[plugin-hooks]` section, section 5). Each plugin's `[plugins.<name>]`
table remains fully opaque to Flox, per the v1.14.0 philosophy: "Flox
stores the data without interpreting it."

A plugin may also ship a **subcommand extension** (`flox-<name>`
executable) for imperative UX, installed via the beta
`flox extension install` mechanism. Extensions and environment plugins
are complementary: the plugin participates in the environment
lifecycle; the extension gives it a CLI.

### Threat model for the new hooks

Installing any package already concedes arbitrary code execution at
activation: every installed package's `etc/profile.d/*.sh` is sourced
into the activation shell, runs with the user's privileges, and its
env mutations replay into every attaching shell. The `[plugin-hooks]`
declaration is therefore **not** a code-execution boundary. What it
gates is precisely:

- **session capture** — a `session-wrap` hook execs the user's
  terminal session under code the plugin controls;
- **per-attach env injection** — an `env` hook writes into every
  shell's environment on every attach, for the activation's lifetime;
- **core-supervised daemon lifetime** — a `sidecar` hook gets a
  process supervised by the executive.

A shipped-but-undeclared hook file is ignored with a warning rather
than an error, and that is sound: ignoring the file removes exactly
the powers the declaration guards. (It cannot remove code execution —
nothing can, short of not installing the package.)

## 2. The environment lifecycle

Phases in the life of a Flox environment, with the extension points
that exist on `main` today and the ones this design adds. Evidence for
the "today" column is cited from `main` (8f5162872).

| # | Phase | Driven by | Extension points today | Added by this design |
|---|-------|-----------|------------------------|----------------------|
| 1 | init / edit / lock / build-render | `commands/init`, `commands/edit.rs`, `lock_manifest.rs`, `buildenv.nix` | none (declaration only) | *(design only)* `post-init`, `pre-lock`, `post-lock` |
| 2a | activate: resolve & render | `commands/activate.rs` | none | **`session-wrap`** *(build now)* |
| 2b | activate: start | interpreter `activate` script | interpreter profile.d → `[vars]` → plugin `etc/profile.d` → `hook.on-activate` | — (existing surface is sufficient) |
| 2c | activate: per-shell attach | `flox-activations` `gen_rc/*` | `[profile]` scripts, prompt hooks | **`env` hook** *(built — core wave 2)*; *(design only)* `on-attach` |
| 3 | in-session | executive daemon, `hook-env` prompt hook | auto-activate allow/deny config | **`sidecar`** *(built — core wave 2)*; *(design only)* in-session event hooks |
| 4 | deactivate / exit | emitted teardown scripts; executive | `[profile.deactivate]` (v1.13.0), `hook.on-deactivate` (v1.15.0) | **plugin `on-deactivate.d`** *(build now)*; *(design only)* `pre-deactivate` in-shell |
| 5 | delete / push / pull / containerize / gc | respective commands | none | *(design only)* `pre-push`, `post-pull`, `pre-containerize`, `on-delete` |
| 6 | services | executive + process-compose | `shutdown.command` | *(design only)* `pre-start`/`post-stop` per service, crash notification |

The *(design only)* rows are deliberate deferrals: an extension point
with no consumer is speculative API we would have to support forever.
Their names are reserved here so that when a consumer appears, the
addition is a new hook directory and dispatch site, not a redesign.

## 3. The new hooks, driven by what sandboxes need

The `prototype/sandboxed-activation` branch proved out what a sandbox
provider needs from Flox. Mapping every need onto a generic hook:

| Sandbox need (old branch) | Generic hook |
|---|---|
| Exec the whole session under a boundary (`ActivationSandbox::wrap_activation → Infallible`) | `session-wrap` |
| Detect "already wrapped" on re-entry (`_FLOX_SANDBOX_WRAPPED`) | `_FLOX_SESSION_WRAPPED` scoped marker, core-checked |
| Consent before auto-activation hands over the session (hook_env `SandboxClass`) | `[plugin-hooks]` declaration + generic consent prompt |
| Advisory engine env injection at attach (`double_set_envs`, LD_PRELOAD) | `env` hook |
| Prompt broker living for the activation (`executive/sandbox/*`) | `sidecar` hook |
| Teardown of persistent state | plugin `on-deactivate.d` |
| Baked OCI image of the environment | `flox containerize` (existing command) + plugin-owned compat layers (see §6 wave B for the honest gaps) |
| Read its own policy config | `[plugins.<name>]` table: `flox_plugin_data` in profile.d / `on-deactivate.d` scripts; the ctx's `plugin_table` in `session-wrap`/`env`/`sidecar` executables (which run outside the activation shell and cannot call the bash helper) |
| Resolve an install-id to a store path for policy rules | lockfile (path provided in hook ctx; schema published at `cli/schemas/lockfile-v1.schema.json`) |
| Grants review UI (`flox sandbox …`) | `flox-sandbox` subcommand extension |

### 3.1 `session-wrap` *(build now)*

The marquee hook. Runs in `flox activate` **after lock, build, and
render**, immediately before the CLI would exec into
`flox-activations activate`. This is a *new* dispatch point, later
than the old branch's (which dispatched after lock but before
build/render): hooks are discovered in the rendered `$FLOX_ENV`, so
render-before-wrap is a hard requirement. Two consequences, accepted
deliberately: the host always locks/builds/renders before a wrapper
runs (the old container backends wrapped without host render; the
bake-style plugins re-use the host render, so no wasted work), and
the remote-include trust prompt now runs before the wrap rather than
inside it. The hook executable receives a serialized context and
**execs the activation under its boundary; on success it never
returns** — the process-boundary translation of the old branch's
`wrap_activation(self: Box<Self>) -> Result<Infallible>` contract.

Payload path: `etc/flox/hooks/session-wrap.d/<plugin-name>`
(executable).

Protocol:

- Core writes a versioned JSON context file (mode `0600`, in flox's
  temp dir) and invokes the hook with `FLOX_HOOK_CTX=<path>`,
  `FLOX_HOOK=session-wrap`, `FLOX_PLUGIN_NAME=<name>`,
  `FLOX_BIN=<current flox>`, and `FLOX_HOOK_JQ=<store path of jq>`
  (so shell-scripted hooks have guaranteed JSON tooling). The hook
  inherits the invoking user's full environment, cwd, and stdio.
- **Stdio is inherited, not guaranteed a tty.** `flox activate -- cmd
  | tee` reaches the hook with stdout a pipe. A hook that wants to
  prompt must probe `/dev/tty` (or check stdin with `isatty`) and
  write prompts to stderr or the tty — never stdout. The ctx carries
  `stdin_is_tty` / `stdout_is_tty` so hooks can decide without
  probing.
- The ctx contains at least: `ctx_version`, `dot_flox_path`,
  `env_name`, `activation_mode`, `rendered_env` (store path),
  `lockfile_path`, `plugin_table` (the plugin's own
  `[plugins.<name>]` value, verbatim JSON), `invocation_type` **with
  its full payload** (the command vector for `-- cmd`, the shell
  string for `-c`, serialized shape published with the ctx schema),
  tty state, and `inner_argv`.
- `inner_argv` is the **host-side re-entry argv** — sufficient for
  same-filesystem wrappers (host-native, srt) that re-exec the whole
  activation under a host boundary. Container/remote backends must
  instead compose their own in-boundary command from
  `invocation_type` + their image's entrypoint (the old `oci`/
  `openshell` backends prove a single argv cannot express this). The
  wave-1 fixture exercises both consumption styles.
- Re-entry: the hook sets `_FLOX_SESSION_WRAPPED=<scope>` in the
  wrapped process's environment, where `<scope>` is a core-defined
  digest of (`dot_flox_path`, plugin name) provided in the ctx. On
  activation, core skips session-wrap dispatch only when the marker
  **matches** the environment being activated; a mismatch — activating
  env B (which declares its own wrapper) inside env A's boundary — is
  the nested-boundary error promised in §8. The marker is cooperative
  re-entry detection, not boundary integrity; integrity must come
  from the boundary itself.
- A hook that returns instead of exec'ing fails the activation (its
  stderr has already reached the user via inherited stdio). There is
  no "decline and continue unwrapped" path while the feature is on —
  an environment that declares a session-wrap plugin either activates
  wrapped or not at all (the user's escape hatches are editing the
  manifest or declining the consent prompt).

Rules enforced by core:

- **Single wrapper (new rule).** `[plugin-hooks].session-wrap` is
  single-valued (section 5), so two wrappers are unrepresentable in
  one manifest; composition cannot smuggle a second one in (include
  declarations are inert, section 3.2).
- **Declaration↔payload binding.** Core resolves the declared plugin
  name to its locked install (install-id → store path) and verifies
  the discovered hook file's realpath lives inside that package's
  store path. A declaration with no matching install, or a hook file
  shipped by a *different* package (shadowing, typo-squatting a
  plugin name), is an activation error.
- **No in-place wrapping** *(ported from the old branch)*.
  `eval "$(flox activate)"` cannot exec-wrap the caller's shell;
  declaring a session-wrap plugin makes in-place activation an error.
- **Ephemeral activations skip the hook** *(ported)*. The synthetic
  activation that `flox services start` builds must not recurse into
  a wrapper.
- **Feature-gated** behind a dedicated `features.plugin_hooks` flag
  (section 5.1).

### 3.2 The consent anchor: top-level `[plugin-hooks]`

The auto-activation planner (`hook-env`) must classify "entering this
directory hands your session to a wrapper" **without executing any
plugin code** — it runs on every prompt render. And the consent
signal must be something the top-level author actually wrote:
`[include]` composition unions included environments' `[plugins.*]`
tables into the merged manifest, so any declaration living *inside*
plugin tables could arrive from an included manifest the user never
read — with local includes carrying no trust gate at all, and remote
include trust being sticky and auto-granted for whole orgs. That is
not consent.

Therefore:

- Hook participation is declared in a **typed, top-level manifest
  section** (schema shape in section 5), separate from the opaque
  plugin data tables.
- **Only the top-level (user-authored) manifest's `[plugin-hooks]` is
  effective.** An included manifest's `[plugin-hooks]` section is
  stripped during composition with a warning naming the include and
  the declaration the user must restate. Consequence: an explicit
  `flox activate` of an environment whose *include* declares a
  wrapper activates unwrapped, with the shipped-but-undeclared
  warning pointing at the fix.
- At activation, core cross-checks declarations against the rendered
  environment (declared-but-missing hook file = error; the binding
  rule of §3.1 covers the reverse direction).
- The planner classifies from the user-authored `manifest.toml`
  alone — a cheap TOML parse, and *correct by construction* because
  include-carried declarations are inert anyway. (The old branch's
  planner read only the user manifest too, but was wrong to under
  its semantics; this design makes the cheap read sound.)
- The consent prompt (core-owned, generically worded) offers a
  foreground wrapped session, ported from the old branch's consent
  leg, with the same suppression semantics (prompt once per shell
  visit, clear on leaving the directory) but a **default of No** —
  bare Enter declines; handing the terminal to third-party code from
  a `cd` must be an affirmative choice:

  ```
  Enter '<path>'? Activation hands this session to plugin '<name>'. [y/N]
  ```

- On fish, tcsh, or without a tty, auto-activation of a wrapping
  environment emits a notice pointing at `flox activate` — no
  prompt, no session. (Same posture as the old branch; stated
  plainly so reviewers know a whole shell class gets notice-only
  UX.)

profile.d participation stays undeclared — see the threat model in
section 1: declarations gate session capture, per-attach injection,
and supervised lifetime, none of which profile.d has. Its
arbitrary-code-at-activation surface is inherent to installing
packages and is not something a declaration could revoke.

### 3.3 Plugin `on-deactivate.d` *(build now)*

Package counterpart of the user-authored `hook.on-deactivate`
(v1.15.0). Payload path: `etc/flox/hooks/on-deactivate.d/*.sh`,
sourced by the executive's on-deactivate leg
(`cli/flox-activations/src/on_deactivate.rs`) in lexical order,
before the user's `hook.on-deactivate`, with the same execution
posture (activation-end environment replayed from the env trace,
output to the executive log, failures swallowed).

`flox_plugin_data` works here only with explicit wiring: it is a bash
function from the interpreter's `helpers.bash`, and the on-deactivate
runner spawns a bare bash with replayed *variables* (functions do not
replay). The runner therefore sources the interpreter's
`helpers.bash` (the interpreter path is recorded in the attach ctx)
before sourcing plugin teardown scripts.

Inherits `hook.on-deactivate`'s known coverage holes (not run on
unclean shutdown of the executive, in containers, or when activation
state is removed externally) — acceptable for cache/janitorial
teardown, and documented for plugin authors. Undeclared, like
profile.d: teardown scripts have no session-capture or injection
powers.

### 3.4 `env` hook *(built — core wave 2)*

An executable contributing environment variables at activation start
and every attach, inside `flox-activations` — the seam the old branch
used for LD_PRELOAD/policy injection via `double_set_envs`. Payload:
`etc/flox/hooks/env.d/<plugin-name>` (executable). Contract:

- Invoked with the hook env (`FLOX_HOOK=env`, ctx file with
  `dot_flox_path`, `rendered_env`, `plugin_table`,
  `phase: start|attach`, the activation's **runtime dir and
  services-socket path** — so injected processes can find a sibling
  sidecar's sockets — and `session_root_pid`); inherits the current
  environment, so PATH-like variables are read-modify-write on the
  plugin side.
- Prints a JSON object `{ "VAR": "value" }` on stdout; stderr goes to
  the log. Core **rejects `_FLOX_*`-prefixed keys** (its own control
  state, including the session-wrap marker, must not be forgeable
  through this channel); everything else is allowed — a deny-list on
  `LD_PRELOAD` would be futile since wave D exists to set it.
- Multiple declared env hooks run in lexical plugin-name order,
  last-writer-wins.
- Fail-closed: non-zero exit or malformed JSON is an activation
  (or attach) error — this is a declared control surface, not
  best-effort decoration.
- Must be fast (it runs on every attach) and idempotent.

Named wave-D design items this hook deliberately does not yet cover
(they live in the old branch's same seam and need their own shape
when libsandbox ports): the SIP shell-swap (substituting bundled bash
for unmediable shells) and one-time grants-dir creation/seeding.
Attach-conflict handling ports as config-hash comparison: an env hook
whose `plugin_table` changed since the activation started errors
rather than silently diverging.

Implementation notes (core wave 2, 2026-08-28): the CLI resolves and
validates declarations with the session-wrap machinery and records
them (`AttachCtx.env_hooks`, resolved paths + tables); dispatch runs
at three sites — activation start before the activate script, services
start in the executive (both `phase: start`), and every shell attach —
with the contributions folded into the double-set channel after the
trace replay, so an injecting plugin wins over profile/hook mutations.
Teardown's env replay for `on-deactivate` scripts does not re-run
hooks. The attach-conflict config-hash comparison remains a wave-D
item. Covered end to end by `cli/tests/plugin_hooks_exec.bats`.

### 3.5 `sidecar` hook *(built — core wave 2)*

A long-running process with the activation's lifetime, hosted by the
executive next to process-compose — the generic form of the old
branch's prompt broker (which was in-process threads; a spawned child
introduces failure states the old design structurally lacked, so they
are specified here). Payload: `etc/flox/hooks/sidecar.d/<plugin-name>`
(executable).

Supervision contract:

- Spawned at activation start with the hook ctx plus a private `0700`
  runtime dir (for sockets, sibling to the services socket) and
  `session_root_pid` + the executive's pid (required to port the
  broker's peer-credential self-approval guard, which moves into
  plugin code).
- **Spawn failure fails the activation.**
- **Crash mid-activation is logged and non-fatal; no automatic
  restart.** Plugins must fail closed on a dead peer (libsandbox
  already does: an absent broker socket means deny).
- The sidecar must die with the executive: `PR_SET_PDEATHSIG` on
  Linux, a parent-watch contract on macOS (kqueue `NOTE_EXIT` or
  mandated parent-pid polling) — stated in the hook contract so
  plugin authors can rely on it.
- Terminated and reaped during teardown before `on-deactivate.d`;
  its runtime dir (including any ctx file) is removed by the
  executive.

Implementation notes (core wave 2, 2026-08-28): sidecars spawn in the
executive between watcher setup and the readiness handshake, so spawn
failure fails the activation; the private `0700` runtime dir is
`sc.<executive-pid>.<n>` beside the services socket (short, for the
104-byte `sun_path` cap) and holds the ctx file. Exits are logged by a
per-sidecar watch thread (crash non-fatal, no restart; the signal
thread reaps). Teardown SIGTERMs with a 5s grace then SIGKILLs, after
services shut down and before `on-deactivate.d`, on both the normal
and the state-removed paths; the executive's deliberate
exit-without-cleanup on a termination signal leans on
`PR_SET_PDEATHSIG`/the macOS parent-watch obligation, as designed.
Covered end to end by `cli/tests/plugin_hooks_exec.bats`.

## 4. Hook tree layout and rendering

```
<plugin package>/
├── etc/profile.d/1000_<name>.sh            # existing: activation env setup
└── etc/flox/hooks/
    ├── session-wrap.d/<name>               # executable
    ├── env.d/<name>                        # executable
    ├── sidecar.d/<name>                    # executable
    └── on-deactivate.d/1000_<name>.sh      # sourced
```

The buildenv symlink forest merges nested package directories at
arbitrary depth (`pathsToLink = "/"`; `findFiles` recurses and
converts colliding directory nodes into real merged directories), so
per-plugin files inside per-hook directories merge exactly like
profile.d. One caveat is load-bearing: an identical *leaf filename*
from two packages is a hard build failure, so the
`<plugin-name>` / `1000_<plugin-name>.sh` naming convention is a
requirement, not tidiness. Discovery at dispatch time is a directory
listing of `$FLOX_ENV/etc/flox/hooks/<hook>.d/`, cross-checked
against `[plugin-hooks]` declarations (sections 3.1–3.2).

Other contract details plugin authors need on day one:

- **Execution environment:** hooks run with the invoking user's
  environment and `PATH`, before any activation setup. On macOS
  `/usr/bin/env bash` can resolve to the system bash 3.2, so shell
  hooks must stay 3.2-compatible — notably, bash 3.2 cannot parse a
  heredoc inside `$(...)` when the body has unbalanced parentheses
  (which SBPL profiles always do); write such payloads to a temp
  file instead.
- **What a boundary must admit (macOS):** the wrapped activation's
  executive binds unix sockets under Flox's cache dir and watches
  its state via FSEvents, so a wrapping boundary must allow local
  socket binds and the `com.apple.FSEvents` mach-lookup, or the
  activation fails after entry. (Discovered by the wave-A srt port;
  its hook is the reference policy.)
- **Cache/state:** blessed location is
  `<dot_flox_path>/cache/plugins/<plugin-name>/` (the old branch's
  grants store precedent, one level down).
- **Ctx schema:** the ctx JSON is versioned (`ctx_version`) and its
  schema is published alongside the manifest/lockfile schemas in
  `cli/schemas/`.
- **Local dev loop:** hooks are testable without publishing — build
  the plugin package and path-install it (store-path installs are
  the mechanism main's own bats suite uses to exercise `[plugins]`).

## 5. Manifest schema: the `[plugin-hooks]` section

A **new typed, top-level section** — not a reserved key inside the
opaque plugin tables. (A reserved key was the first draft and fails
mechanically: opaque `serde_json::Value` content can't trip
`deny_unknown_fields` on older schemas, so hooks-bearing manifests
would silently parse — and activate unwrapped, fail-open — on every
released CLI, and the downgrade-guard machinery would never bump the
schema version. A new section gets both properties for free: released
CLIs reject it wholesale, and `as_original_schema` refuses downgrade
while it is present.)

```toml
[plugin-hooks]
session-wrap = "openshell"      # at most one — single-wrapper rule is structural
env = ["libsandbox"]           # zero or more
sidecar = ["libsandbox"]       # zero or more
```

- Typed as
  `PluginHooks { session_wrap: Option<String>, env: Vec<String>, sidecar: Vec<String> }`
  with `deny_unknown_fields`, so an unknown hook kind fails at parse
  time (lock/edit), not activation time.
- Values name plugins, i.e. keys of `[plugins.<name>]` /
  installed packages; the activation-time binding check (§3.1) ties
  the name to the locked install that must ship the hook file.
- Introduced in the **next unreleased schema version**. At the time
  of writing that is v1.16.0, but the in-flight CLI-153 branch
  (service health checks) also mints v1.16.0 — coordinate: land into
  the same unreleased v1.16.0 if CLI-153 merges first, otherwise
  renumber. The prototype tracks whichever number is free when core
  wave 1 lands.
- Migration to/from the previous version is lossless when the section
  is absent; a manifest using it does not downgrade (new-field
  precedent, same as the `[plugins]` introduction).
- Composition: included manifests' `[plugin-hooks]` sections are
  stripped with a warning (§3.2). `[plugins.<name>]` data tables keep
  their existing whole-table merge — data flows through includes,
  *participation* does not.

### 5.1 Feature gating

A dedicated `features.plugin_hooks` flag (env:
`FLOX_FEATURES_PLUGIN_HOOKS`), not `features.beta` — enabling beta to
try subcommand extensions must not silently arm session handoff. The
old branch used a dedicated flag for the same reason.

- **Flag off (the default):** hooks-declaring manifests
  **warn-and-ignore** — the activation proceeds unwrapped with a
  warning naming the flag. This keeps a shared environment usable for
  teammates who haven't opted in (the old branch's choice, ported).
  Note the asymmetry this buys: *released* CLIs reject the manifest
  at parse (unknown section, fail-closed); *flag-off prototype* CLIs
  warn-and-ignore (fail-open, but loudly). Acceptable for a
  prototype-only flag; revisit before the flag defaults on.
- The gate is evaluated in the `flox` CLI only. `flox-activations`
  has no config plumbing and gains none: the CLI records the
  resolved hook participation (which env hooks, which sidecar, which
  on-deactivate scripts) into the activation context it already
  serializes, and `flox-activations`/the executive execute what was
  recorded. No re-checking downstream.

## 6. What each sandbox provider becomes

Every provider ports to a directory in `flox/flox-plugins` following
that repo's existing convention (self-contained dir, `.flox` build
env producing a `plugin-<name>` package, README, demo). The hook
contract is language-agnostic (executables); wrappers with real logic
may build a small Rust binary in their build env, simple ones stay
shell (with `FLOX_HOOK_JQ` for ctx parsing — `inner_argv` and the
`invocation_type` payload are arrays, unrecoverable in pure bash
string-mangling). Ordered by migration wave:

| Wave | Provider | Hooks used | Notes |
|---|---|---|---|
| A | host-native | session-wrap | SBPL generation + `sandbox-exec` re-exec of `inner_argv`; validates the same-filesystem contract on macOS |
| A | srt | session-wrap | settings file + `srt` re-exec of `inner_argv`; both platforms |
| B | oci | session-wrap (+ on-deactivate.d for cache GC) | bake via `flox containerize` + docker; see the honest-gaps list below |
| B | openshell | session-wrap, on-deactivate.d | policy YAML compiled from its own `[plugins.openshell]` table; binary→store-path resolution from the lockfile; version preflight ≥ 0.0.62; retags under its own `<env>-openshell` repo. **First released plugin** (Flox Catalog, `flox` org) |
| C | coder, modal, docker-sbx, ona, e2b, daytona, cognition-devin, anjuna, cursor, vercel-sandbox | session-wrap | the OCI-handoff/artifact-writer slices; port artifact generation + declared-lossiness policy compilation as-is; each bails where it bailed before |
| D | libsandbox (+ `flox-sandbox` extension) | env, sidecar | **DONE** — C engine ships as a package library; env hook injects preload + policy env; sidecar hosts the broker (peer-cred guard moved with it); grants store under the plugin cache dir; review UI is the `flox-sandbox` beta extension. `on-deactivate.d` proved unnecessary (grants/audit persist; the broker self-cleans its sockets). Remaining follow-ups: SIP shell-swap, Linux leg (§6 wave-D outcomes) |

**Wave A outcomes** (2026-08-27; both plugins live on the local
`daniel/wave-a-session-wrap` branch of flox-plugins, validated end to
end on macOS — wrapped entry, re-entry marker skip, deny-home /
project / `.env` / network probes; the srt Linux leg is not yet
exercised):

- The ports surfaced one core incompatibility: `flox-core`'s
  `proc_status` shelled out to `/bin/ps`, which Seatbelt-based
  boundaries refuse to exec, killing the executive. Fixed in core
  wave 1 by reading process status via `proc_pidinfo` + a
  `kill(pid, 0)` zombie probe — syscalls the boundaries permit.
- srt (0.0.71) needs three policy grants beyond the old branch's
  settings — `allowLocalBinding`, `allowUnixSockets` for Flox's run
  sockets, and the `com.apple.FSEvents` mach-lookup (§4) — and an
  explicit `--` terminator so its CLI does not consume the one
  inside `inner_argv`.
- One accepted policy delta: srt's read rules have no pattern
  matching and its allow list wins inside the project, so
  host-native's `.env` deny cannot be replicated there (documented
  in the plugin README).

**Wave B outcomes** (2026-08-27/28; both plugins on the flox-plugins
`daniel/wave-a-session-wrap` branch, validated end to end on macOS —
bake, digest-tag caching, staleness decision table, wrapped entry,
project rw at its host path, host home absent; openshell additionally:
gateway preflight, session as the sandbox user, deny-all egress, a
binary-scoped allow rule reaching its endpoint with everything else
403, policy edits applying without a rebake, `--no-keep` teardown):

- **Shim-less guests it is** (the §6 decision recorded): the plugins
  ride main's `flox containerize` unmodified, so guests carry no flox
  CLI — no in-guest `flox list`/services, a documented regression vs.
  the old demos. Revisit only if a catalog-served guest flox makes the
  entrypoint rewrite worth owning.
- The old branch's frozen-builder pins, sanitized-view machinery, and
  release-guard collapsed into one small mechanism: bake from a /tmp
  view rewritten to schema 1.15.0 with `[plugin-hooks]` and the
  wrapper's own install entry stripped, and pin the macOS proxy to the
  cached `v1.15.0` release builder. Stripping the wrapper's install
  entry is load-bearing twice over: its host-only store path cannot be
  realised for the guest, and hashing the stripped lockfile keeps
  plugin upgrades from invalidating images (`strip your own footprint`
  now includes the install entry, not just the data table).
- The openshell compat layer ported from Nix to a plain Dockerfile on
  top of the shared base image, with two deltas: `/etc/hosts` is left
  alone (BuildKit mounts it read-only during builds and Docker injects
  one at runtime), and the guest-arch `ip`/`nsenter` come from a
  pre-locked tools environment bundled in the plugin package,
  containerized once and multi-staged in — the 0.0.82 supervisor
  refuses to start without a trusted `ip`.
- Two upstream drifts absorbed in plugin code: OpenShell 0.0.8x runs
  its policy engine in binary-identity mode, so egress rules
  effectively require `binary` now, and its CLI needs an explicit
  `--` so it does not consume the one inside a composed command.
- Neither plugin needs `on-deactivate.d` after all: the policy file is
  regenerated per activation and the docker images are the cache
  (never GC'd by flox, matching the old backends).
- Not yet exercised: the Linux host leg, interactive-tty sessions
  (command mode validated), and in-guest services (no
  `PROCESS_COMPOSE_BIN` in the compat layer).

The original gap list, kept for the record (all items now dispositioned
as above):

- Main's `flox containerize` has no `include_guest_flox`, no compat
  knobs, and its baked activate-ctx JSON lives in `/nix/store` behind
  the image entrypoint — a post-hoc docker layer cannot flip
  `flox_bin`/`disable_hook` without also rewriting the entrypoint to
  a plugin-authored ctx copy. Wave B either ships **shim-less guests**
  first (no in-guest `flox list`/services — a documented regression
  vs. the old demos) or the plugin rewrites the entrypoint; decide
  during the oci port, and record the outcome here.
- The old `openshellCompat` layer (uid 1000660000 passwd/group,
  chowns, `/var/run → /run`, `/bin/sh` symlink, guest-arch
  `iproute2`/`nsenter`) is Dockerfile-replicable *except*: the base
  image has no `/bin/sh` (plugin Dockerfile must start with
  `SHELL ["/bin/bash", "-c"]` — bash is in the env closure) and the
  guest-arch Linux binaries need a source (a second minimal
  containerized flox env / multi-stage build).
- Main's containerize `Runtime` is Docker/Podman only — no Apple
  Container sink. The ported oci plugin initially targets Docker on
  macOS (roster change vs. the old backend), or loads via
  `-f image.tar` + `container image load`.
- Image tag hashing: the old branch stripped sandbox options from the
  lockfile before hashing so policy edits don't force a rebake.
  Plugin convention: **strip your own `[plugins.<name>]` table and
  `[plugin-hooks]` before hashing** — runtime-applied policy must not
  invalidate the image, but *other* plugins' tables stay in the hash
  (their data is baked into the image's lockfile and read by their
  profile.d scripts at container start).

**Wave C outcomes** (2026-08-28; all ten handoff slices on the
flox-plugins branch, each validated through a real activation): every
port preserves its old module's contract — same preflight gates and
guidance, same policy compilation with the same declared lossiness,
same artifacts at the same paths, and a bail at exactly the point the
old backend bailed. Seven bail at CLI preflight on this machine class
(coder, modal, docker-sbx, e2b, daytona, cursor, vercel-sandbox);
three compile policy, write their handoff artifacts, and bail at the
launch boundary (ona, cognition-devin, anjuna). The shared toolkit
stayed *duplicated* across the bash hooks for now — factoring a
common library inside flox-plugins is deferred to the migration-out
step, when the per-plugin branch layout is settled.

**Wave D outcomes** (2026-08-28; `plugin-libsandbox` on the flox-plugins
branch, validated end to end on macOS): the advisory engine is now a
plugin with no sandbox-specific code left in core — it rides only the
generic env + sidecar hooks built in core wave 2.

- The C engine (`sandbox.c` + `closure.c`) ships **unchanged** as the
  plugin's package library, compiled in the build env (`clang`,
  `-pthread`, `-dynamiclib`/`-shared`). Repackaging the engine was the
  easy part; the work was replacing the in-tree Rust injection and
  broker with hook executables.
- The **env hook** (bash) composes the engine's policy environment.
  Two facts made it correct: it must be idempotent (it runs at start
  and every attach — the preload compose checks for the lib before
  appending), and the seed allow-set is reproduced from the reference
  `SeedAllowSet` (system dirs + `/nix/store` as allow-dirs, shell /
  interpreter / flox-config trees as globs), folded with saved grants
  by shape.
- The **broker** (C) is the sidecar — the load-bearing new code. It
  binds the verdict and control sockets, both derived from the
  services-socket path in its ctx (the same pure function the env hook
  holds, so the rendezvous needs no second channel). The
  peer-credential self-approval guard moved into it verbatim
  (`LOCAL_PEERPID`/`SO_PEERCRED`, refuse when the peer is the
  session-root pid or a descendant), using the `session_root_pid` the
  sidecar ctx carries; validated refusing in-session and permitting
  out-of-session. Sockets self-clean on the teardown SIGTERM.
- The **review UI** is a self-contained C `flox-sandbox` binary
  installed as a beta subcommand extension
  (`flox extension install --from-path share/flox-sandbox`, dispatched
  as `flox sandbox`). It reads the NDJSON grants/audit store off disk
  and reaches the broker's control socket derived via
  `flox services-socket` — never from the session env, so an
  in-session process cannot find it.
- Grants persist as **NDJSON** under
  `.flox/cache/plugins/plugin-libsandbox/` (the plugin owns the
  format; the engine's `audit.ndjson` sits beside it). `on-deactivate.d`
  proved unnecessary and was dropped — grants and audit persist by
  design and the broker cleans its own sockets.
- Two follow-ups remain, both documented in the plugin README: the
  macOS **SIP shell-swap** (a SIP-protected session shell strips
  `DYLD_INSERT_LIBRARIES`, so its own builtins escape mediation while
  non-SIP children and `flox activate -- <tool>` are covered — the
  automatic swap-in of a bundled bash is not yet ported), and the
  **Linux `LD_PRELOAD` leg** (builds from the same sources, not yet
  exercised here). The "grants seeding" open item from §3.4 landed as
  the env hook's reproduction of the seed allow-set.

The shared toolkit (`preflight.rs`, `bake.rs`, `handoff.rs` pure
helpers) ports to a shared library *inside flox-plugins* (a common
package or vendored module the plugin builds consume) — not into
flox core.

What does **not** migrate (dies with the old branch): `--sandbox` /
`--sandbox-backend` flags, `[options.sandbox]`, `SandboxBackend` +
`BackendCapabilities` roster, `flox sandbox` core subcommand, the
mode vocabulary (`off|warn|enforce|prompt` — each plugin defines its
own mode key in its table if it wants one), `sandbox_oci_autobake`
config (becomes plugin-table config), and the guest-side
`container_active_env` / `flox()` shim plumbing (superseded per the
wave-B decision above).

Related prototype disposition: the local `manifest-plugins-prototype`
branch (the `flox_plugin_entries` helper, `demo-secrets-plugin`
example, plugin-contract README) is independent of this design — the
hook ctx's `plugin_table` supersedes the entries helper for hook
executables, while the profile.d convention it documents is
untouched. It is included in step 7's retirement diff: port or drop
explicitly, don't strand it.

## 7. Implementation plan (this prototype)

1. **Core wave 1** *(flox/flox, this branch)*: `[plugin-hooks]`
   schema (next unreleased version, see §5); `features.plugin_hooks`;
   session-wrap dispatch in `activate.rs` (discovery, declaration
   cross-check + install binding, ctx serialization, scoped marker,
   guards); include-stripping + warning in composition; generic
   consent leg in `hook_env.rs` (default-No); plugin
   `on-deactivate.d` sourcing (helpers preamble) in
   `on_deactivate.rs`; bats coverage on both platforms with a
   store-path-installed fixture plugin exercising both `inner_argv`
   re-exec and `invocation_type`-composed consumption. Automated
   contract coverage rests on this fixture; waves A–D validate
   manually via their demo environments (CI has no docker daemon,
   srt, or macOS `sandbox-exec` harness).
2. **Wave A ports** (host-native, srt) as local flox-plugins-layout
   directories; end-to-end manual validation of the contract.
3. **Wave B ports** (oci, openshell) — bake via containerize,
   prompting from a hook, teardown, compat-parity exit criterion.
4. **Wave C ports** (the ten handoff slices).
5. **Core wave 2**: `env` + `sidecar` hooks in flox-activations/
   executive (contracts per §3.4–3.5).
6. **Wave D**: libsandbox plugin + `flox-sandbox` extension.
7. **Migration out**: one branch per plugin on `flox/flox-plugins`;
   OpenShell first to Catalog (`flox` org) for internal/user testing;
   then retire `prototype/sandboxed-activation` (and disposition
   `manifest-plugins-prototype`, §6) after diffing that nothing
   (docs, demos, tests) is left behind.
8. **Spec-review artifacts** *(flox/docs, new branch)*: update
   `concepts/plugins.mdx` to present the full lifecycle
   extension-point model (profile.d + the hook tree + declarations +
   consent), and add a sandbox concept page mirroring
   `concepts/secrets-management.mdx` — describing sandboxing as a
   class of plugin built on the framework. These are the documents
   the team reviews.

## 8. Open questions / future work

- **Extension distribution**: `flox extension install` is
  local-path-only on main; Catalog-served extensions are needed
  before `flox-sandbox` can ship cleanly. Interim: install from the
  plugin package's rendered path.
- **Run-mode plugins**: profile.d (and thus plugin env setup) is
  skipped in run mode today; session-wrap is mode-independent by
  design, but a wrapped run-mode env whose plugin needs profile.d
  data must carry everything in the ctx. Revisit if a real case
  appears.
- **Remote/managed environments**: consent semantics for
  `flox activate -r` pulling a manifest that declares session-wrap —
  the remote-env trust prompt and the hook consent prompt should
  compose, not stack. (The include-stripping rule already prevents
  *transitive* arrival of declarations; this question is about the
  remote env's own top-level declaration.)
- **Nested boundaries**: a scope-mismatched `_FLOX_SESSION_WRAPPED`
  (env B activating inside env A's boundary) errors, per §3.1.
  Composing boundaries is future work.
- **Catalog metadata**: a future catalog-side "this package is a
  plugin declaring hooks X" signal could let `flox install` surface
  consent even earlier than activation.
- **Ctx cleanup for session-wrap**: the hook execs away, so nobody
  deletes its 0600 ctx file deterministically; it lives in flox's
  temp dir and is cleaned with it. If that proves unsatisfying,
  hand the ctx on an inherited fd instead of a path.
- **Duplicate warnings on re-entry**: the wrapped inner activation
  re-runs session-wrap resolution (skipping via the marker), so
  advisory output like the undeclared-hook warning prints twice per
  wrapped activation. Cosmetic; dedupe if it grates.
