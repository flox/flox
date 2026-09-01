# Plugin lifecycle hooks: `session-wrap`

Status: DESIGN — scoped branch `daniel/session-wrap-hook`.
Author: Daniel Sauble, 2026-08-27; scoped 2026-09-01.

The `prototype/sandbox-plugins` branch designed a complete set of
extension points ("hooks") across the lifecycle of a Flox environment
(building on the experimental `[plugins]` mechanism of schema v1.14.0)
and validated it by carrying sandboxed activation entirely as plugins.
This branch scopes that design down to what shipping a
**session-boundary plugin** such as OpenShell requires from flox core:
one hook (`session-wrap`, dispatched from `flox activate`), one typed
manifest section (`[plugin-hooks]`, with one key), one feature flag
(`features.plugin_hooks`), include-stripping during composition, and a
consent prompt in the auto-activation planner. Everything else stays
on the prototype branch until a consumer ships (§2). Core never says
the word "sandbox": the hook is generic session capture, and OpenShell
is its first consumer.

## 1. What a plugin is

Unchanged from the v1.14.0 convention established in
`flox/flox-plugins`: **a plugin is an ordinary package** installed via
`[install]`, whose files are merged into the rendered environment by
the buildenv symlink forest. Today the only recognized payload is
`etc/profile.d/*.sh`, sourced during activation by the interpreter,
with per-plugin manifest data readable via `flox_plugin_data`.

This design adds one payload path (the `session-wrap.d` hook
directory, §4) and one piece of typed manifest surface (the
`[plugin-hooks]` section, §5). Each plugin's `[plugins.<name>]` table
remains opaque to Flox: "Flox stores the data without interpreting it."

### Threat model

Installing any package already concedes arbitrary code execution at
activation: every installed package's `etc/profile.d/*.sh` is sourced
into the activation shell, runs with the user's privileges, and its
env mutations replay into every attaching shell. The `[plugin-hooks]`
declaration is therefore **not** a code-execution boundary. What it
gates is exactly one power today: **session capture** — a
`session-wrap` hook execs the user's terminal session under code the
plugin controls.

A shipped-but-undeclared hook file is ignored with a warning rather
than an error, and that is sound: ignoring the file removes exactly
the power the declaration guards. (It cannot remove code execution —
nothing can, short of not installing the package.)

## 2. Where `session-wrap` sits in the lifecycle

| Phase | Driven by | Extension points today | Added here |
|---|---|---|---|
| activate: resolve & render | `commands/activate.rs` | none | **`session-wrap`** — after lock, build, and render; before the exec into `flox-activations` |
| activate: start | interpreter `activate` script | interpreter profile.d → `[vars]` → plugin `etc/profile.d` → `hook.on-activate` | — |
| activate: per-shell attach | `flox-activations` `gen_rc/*` | `[profile]` scripts, prompt hooks | — |
| in-session | executive daemon, `hook-env` prompt hook | auto-activate allow/deny config | consent prompt for wrapping environments (§3.2) |
| deactivate / exit | emitted teardown scripts; executive | `[profile.deactivate]`, `hook.on-deactivate` | — |

Every other extension point — `env` and `sidecar` hooks, plugin
`on-deactivate.d` scripts, and the init/lock/push/pull/containerize/
services hooks — was designed or prototyped on `prototype/sandbox-plugins`
and is deferred until a consumer ships: an extension point with no
consumer is speculative API we would have to support forever.

## 3. The `session-wrap` hook

### 3.1 Contract

Runs in `flox activate` **after lock, build, and render**, immediately
before the CLI would exec into `flox-activations activate`. Hooks are
discovered in the rendered `$FLOX_ENV`, so render-before-wrap is a
hard requirement (a bake-style plugin re-uses that render, and the
remote-include trust prompt runs before the wrap rather than inside
it). The hook executable receives a serialized context and **execs
the activation under its boundary; on success it never returns.**

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
  prompt must write to stderr or `/dev/tty` — never stdout. The ctx
  carries `stdin_is_tty` / `stdout_is_tty` so hooks need not probe.
- The ctx contains: `ctx_version`, `dot_flox_path`, `env_name`,
  `activation_mode` (`dev` or `run`), `rendered_env` (store path),
  `lockfile_path`, `plugin_table` (the plugin's own `[plugins.<name>]`
  value, verbatim JSON; `null` when absent), `invocation_type` **with
  its full payload** (the command vector for `-- cmd`, the shell
  string for `-c`), `stdin_is_tty`, `stdout_is_tty`, `inner_argv`, and
  `wrap_scope`.
- `inner_argv` is the **host-side re-entry argv** — sufficient for
  wrappers that re-exec the whole activation under a boundary on the
  host filesystem. Wrappers that run the session elsewhere — in a
  container or a guest, as OpenShell does — must instead compose their
  own in-boundary command from `invocation_type` plus their image's
  entrypoint; a single argv cannot express that. The bats fixture
  (`cli/tests/session_wrap.bats`) exercises the re-exec style.
- Re-entry: the hook sets `_FLOX_SESSION_WRAPPED=<wrap_scope>` in the
  wrapped process's environment, where `wrap_scope` is a core-defined
  digest of (`dot_flox_path`, plugin name) provided in the ctx. Core
  skips dispatch only when the marker **matches** the environment
  being activated; a mismatch — activating env B (which declares its
  own wrapper) inside env A's boundary — is the nested-boundary error
  of §8. The marker is cooperative re-entry detection, not boundary
  integrity; integrity must come from the boundary itself.
- The hook **replaces** the flox process (`exec`), so its exit status
  is the activation's exit status. A hook that cannot hand off must
  exit non-zero and say why on stderr (which reaches the user via
  inherited stdio); a hook that exits 0 without exec'ing produces a
  silent no-op activation, which is a plugin bug rather than a state
  core can detect. There is no "decline and continue unwrapped" path
  while the feature is on — an environment that declares a
  session-wrap plugin either activates wrapped or not at all (the
  escape hatches are editing the manifest or declining the consent
  prompt).

Rules enforced by core:

- **Single wrapper.** `[plugin-hooks].session-wrap` is single-valued
  (§5), so two wrappers are unrepresentable in one manifest;
  composition cannot smuggle a second one in (include declarations are
  inert, §3.2).
- **Declaration↔payload binding.** Core resolves the declared plugin
  name to its locked install (install-id → store path) and verifies
  the discovered hook file's realpath lives inside that package's
  store path. A declaration with no matching install, a hook file
  shipped by a *different* package (shadowing, typo-squatting a plugin
  name), a declared-but-missing hook file, or a non-executable hook
  file is an activation error.
- **No in-place wrapping.** `eval "$(flox activate)"` cannot exec-wrap
  the caller's shell; declaring a session-wrap plugin makes in-place
  activation an error.
- **Ephemeral activations skip the hook.** The synthetic activation
  that `flox services start` builds must not recurse into a wrapper.
- **Feature-gated** behind a dedicated `features.plugin_hooks` flag
  (§5.1).

### 3.2 The consent anchor: top-level `[plugin-hooks]`

The auto-activation planner (`hook-env`) must classify "entering this
directory hands your session to a wrapper" **without executing any
plugin code** — it runs on every prompt render. And the consent signal
must be something the top-level author actually wrote: `[include]`
composition unions included environments' `[plugins.*]` tables, so a
declaration living *inside* plugin tables could arrive from an
included manifest the user never read. That is not consent. Therefore:

- Hook participation is declared in a **typed, top-level manifest
  section** (§5), separate from the opaque plugin data tables.
- **Only the top-level (user-authored) manifest's `[plugin-hooks]` is
  effective.** An included manifest's section is stripped during
  composition with a notice naming the include; an environment whose
  *include* declares a wrapper activates unwrapped, with the
  shipped-but-undeclared warning pointing at the fix.
- At activation, core cross-checks declarations against the rendered
  environment (declared-but-missing hook file = error; the binding
  rule of §3.1 covers the reverse direction).
- The planner classifies from the user-authored `manifest.toml`
  alone — a cheap TOML parse, and *correct by construction* because
  include-carried declarations are inert anyway.
- The consent prompt (core-owned, generically worded) offers a
  foreground wrapped session, once per shell visit, clearing on
  leaving the directory, with a **default of No** — bare Enter
  declines; handing the terminal to third-party code from a `cd` must
  be an affirmative choice. It is asked even for directories on the
  allow list, since a prior allow may predate the declaration:

  ```
  Enter '<path>'? Activation hands this session to plugin '<name>'. [y/N]
  ```

- On fish, tcsh, or without a tty, auto-activation of a wrapping
  environment emits a notice pointing at `flox activate` — no prompt.

## 4. Hook tree layout and rendering

```
<plugin package>/
├── etc/profile.d/1000_<name>.sh            # existing: activation env setup
└── etc/flox/hooks/
    └── session-wrap.d/<name>               # executable
```

The buildenv symlink forest merges nested package directories at
arbitrary depth (`pathsToLink = "/"`), so per-plugin files inside the
hook directory merge exactly like profile.d. One caveat is
load-bearing: an identical *leaf filename* from two packages is a hard
build failure, so the `<plugin-name>` naming convention is a
requirement, not tidiness. Discovery at dispatch time is a directory
listing of `$FLOX_ENV/etc/flox/hooks/session-wrap.d/`, cross-checked
against the `[plugin-hooks]` declaration (§3.1–3.2).

Other contract details plugin authors need on day one:

- **Execution environment:** hooks run with the invoking user's
  environment and `PATH`, before any activation setup. On macOS
  `/usr/bin/env bash` can resolve to the system bash 3.2, so shell
  hooks must stay 3.2-compatible — notably, bash 3.2 cannot parse a
  heredoc inside `$(...)` when the body has unbalanced parentheses
  (policy files often do); write such payloads to a temp file instead.
- **Cache/state:** blessed location is
  `<project>/.flox/cache/plugins/<plugin-name>/`.
- **Ctx schema:** the ctx JSON is versioned (`ctx_version`, currently
  `1`); publishing its schema under `cli/schemas/` is a follow-up.
- **Local dev loop:** hooks are testable without publishing — build
  the plugin package and path-install it (store-path installs are the
  mechanism main's own bats suite uses to exercise `[plugins]`).

## 5. Manifest schema: the `[plugin-hooks]` section

A **new typed, top-level section** — not a reserved key inside the
opaque plugin tables, whose `serde_json::Value` content can't trip
`deny_unknown_fields` on older schemas (hooks-bearing manifests would
silently activate unwrapped on every released CLI). A new section makes
released CLIs reject it wholesale and blocks downgrade while present.

```toml
[plugin-hooks]
session-wrap = "plugin-openshell"   # at most one — single-wrapper rule is structural
```

- Typed as `PluginHooks { session_wrap: Option<String> }` with
  `deny_unknown_fields`, so an unknown hook kind fails at parse time
  (lock/edit), not activation time. Future hooks add keys here.
- The value names a plugin, i.e. the key of its `[plugins.<name>]`
  table and the install id of the package that ships the hook file;
  the activation-time binding check (§3.1) ties the name to the locked
  install.
- Introduced in schema **v1.16.0**.
- Migration to/from the previous version is lossless when the section
  is absent; a manifest using it does not downgrade (new-field
  precedent, same as the `[plugins]` introduction).
- Composition: included manifests' `[plugin-hooks]` sections are
  stripped with a notice (§3.2). `[plugins.<name>]` data tables keep
  their existing whole-table merge — data flows through includes,
  *participation* does not.

### 5.1 Feature gating

A dedicated `features.plugin_hooks` flag (env:
`FLOX_FEATURES_PLUGIN_HOOKS`), not `features.beta` — enabling beta to
try subcommand extensions must not silently arm session handoff.

- **Flag off (the default):** hooks-declaring manifests
  **warn-and-ignore** — the activation proceeds unwrapped with a
  warning naming the flag, so a shared environment stays usable for
  teammates who haven't opted in. Asymmetry to revisit before the flag
  defaults on: *released* CLIs reject the manifest at parse
  (fail-closed); *flag-off* CLIs warn-and-ignore (fail-open, loudly).
- The gate is evaluated in the `flox` CLI only. Session-wrap dispatch
  runs before the CLI hands off to `flox-activations`, which has no
  config plumbing and gains none.

## 6. What OpenShell becomes

An ordinary package, `plugin-openshell`, developed in
`flox/flox-plugins` and shipping the `session-wrap.d/plugin-openshell`
hook. The hook bakes the rendered environment into a guest image via
`flox containerize`, compiles its network policy per activation from
its own `[plugins.plugin-openshell]` table (`autobake`, `allow-stale`,
`image`, and `[[plugins.plugin-openshell.network]]` entries with
`endpoint`, `access`, `protocol`, `binary`), and composes the in-guest
command from `invocation_type`. It is intended to be the first plugin
released to the Flox Catalog (`flox` org) for internal and user
testing; until then it is built from `flox/flox-plugins` and installed
by store path.

## 7. Branches

| Repo | Branch | Carries |
|---|---|---|
| `flox/flox` | `daniel/session-wrap-hook` (this one) | this design; `[plugin-hooks]` schema v1.16.0 + `features.plugin_hooks`; session-wrap dispatch in `activate.rs`; include-stripping in composition; the consent leg in `hook_env.rs`; `cli/tests/session_wrap.bats` |
| `flox/flox-plugins` | `daniel/openshell-plugin` | the `plugin-openshell` package (§6) |
| `flox/docs` | `daniel/session-wrap-openshell` | plugin-hook and OpenShell documentation |

## 8. Open questions / future work

- **Run-mode plugins**: profile.d (and thus plugin env setup) is
  skipped in run mode today; session-wrap is mode-independent by
  design, but a wrapped run-mode env whose plugin needs profile.d data
  must carry everything in the ctx. Revisit if a real case appears.
- **Remote/managed environments**: consent semantics for
  `flox activate -r` pulling a manifest that declares session-wrap —
  the remote-env trust prompt and the hook consent prompt should
  compose, not stack. (Include-stripping already prevents *transitive*
  arrival; this is about the remote env's own top-level declaration.)
- **Nested boundaries**: a scope-mismatched `_FLOX_SESSION_WRAPPED`
  (env B activating inside env A's boundary) errors, per §3.1.
  Composing boundaries is future work.
- **Catalog metadata**: a catalog-side "this package declares hooks X"
  signal could let `flox install` surface consent before activation.
- **Ctx cleanup**: the hook execs away, so nobody deletes its 0600 ctx
  file deterministically; it is cleaned with flox's temp dir. If that
  proves unsatisfying, hand the ctx on an inherited fd instead.
- **Duplicate warnings on re-entry**: the wrapped inner activation
  re-runs session-wrap resolution (skipping via the marker), so the
  undeclared-hook warning prints twice. Cosmetic; dedupe if it grates.
- **Double-counted activation events for same-filesystem wrappers**:
  the outer process records its `activate` metrics and events before
  dispatch execs the hook, and a wrapper that re-enters via
  `inner_argv` records them again. Container wrappers such as
  OpenShell run no flox CLI in the guest, so they count once. Fix when
  a same-filesystem wrapper ships: record events after dispatch, or
  skip them in the outer process when it is about to exec.
- **Flag-off is fail-open**: with `features.plugin_hooks` off a
  declaring manifest activates unwrapped with a warning (§5.1). For an
  isolation consumer that may be the wrong default; failing closed is
  fewer lines. Decide before the flag defaults on.
