# Zero-setup default environments — demo script

This walkthrough exercises the default-environment onboarding prototype
end-to-end against the local FloxHub stack. It covers both directions:

- **Path A (CLI-first)**: install a package with a single login — the CLI
  authenticates, creates `<handle>/default` on FloxHub, installs, and syncs,
  then skips the web onboarding wizard and keeps working after logout
  (DEV-269).
- **Path B (FloxHub-first)**: create the default environment in the web
  onboarding wizard, then pick it up on a "new machine" with a single
  `flox auth login`.

**Why auth-first (not the Mise model):** Mise has no hosted service, so it
has nothing to log into. FloxHub is the sync backbone for the default
environment, and catalog resolution will require authentication server-side
soon (see [#4637](https://github.com/flox/flox/pull/4637), which warns
unauthenticated users at the `/resolve` call site — the spot where the
interactive login prompt will live once gating is enforced). The prototype
therefore treats login as the *one* gate: any command that creates or
mutates the default environment authenticates first, while *using* an
already-fetched default keeps working logged out or offline.

The prototype spans two branches, both named `prototype/default-env-onboarding`:

| Repo | What changed |
|------|--------------|
| `flox` | DEV-269 offline/logged-out `-D` fallback (always on) + auth-first defaults (`flox install [-D]` logs in, creates `<handle>/default` on FloxHub, auto-syncs mutations; behind `FLOX_FEATURES_AUTO_DEFAULT`) |
| `floxhub` | web-bff reports onboarding complete when `<handle>/default` exists with ≥1 generation (and heals the Auth0 flag); `PATCH /user/:handle/onboarding-completed` accepts `{"completed": false}` for demo resets |

CLI steps below were machine-verified on 2026-08-19 against the local stack
unless marked **[code-verified]** (behavior confirmed in the source, but the
step needs an interactive login or a browser) — see §6.

## 1. One-time setup

### Build the CLI

```bash
cd ~/Code/flox
git switch prototype/default-env-onboarding
nix develop -c just build-cli        # produces ./target/debug/flox
```

### Start the FloxHub stack

```bash
cd ~/Code/floxhub
git switch prototype/default-env-onboarding
flox activate                        # exports ports/URLs, generates nginx config + mkcert certs
just serve-all                       # full stack incl. web-ui; leave this running
```

First-run notes (see `~/Code/floxhub/README.md` for details):

- `/etc/hosts` needs `127.0.0.1 api.local.flox.dev hub.local.flox.dev`
  (the activation hook warns if missing).
- `just floxem setup-gitolite` needs GitHub SSH access on first run.
- mkcert may prompt for sudo once to install its CA.
- If services wedge after a crash, remove the stale socket:
  `rm ~/.cache/flox/run/flox.*.sock` and re-run `just serve-all`.

Verify the web UI loads at <https://hub.local.flox.dev:8000/>.

### Test accounts

Auth is the real Auth0 **dev tenant** (`auth.dev.flox.dev`). The tenant has
**no email+password connection** — login options are GitHub, Google, GitLab,
and SSO. You'll want two fresh identities:

- one for **Path A** (never touches web onboarding until the end),
- one for **Path B** (starts in web onboarding).

Using e.g. your GitHub account for one persona and a Google account for the
other works. Re-running a path with the *same* account requires the reset
steps in §4.

> **Tokens live in the macOS keychain, not the fake `$HOME`.** The keychain
> entry is keyed by FloxHub URL, so the demo's local-stack login never
> touches your production `hub.flox.dev` token — but both personas share the
> `hub.local.flox.dev` entry. Run `flox auth logout` whenever you switch
> personas, or persona B's shell will silently be logged in as persona A.

### Snapshot the stack environment (once)

The flox CLI is pointed at the local stack by env vars that the floxhub
environment's activation hook exports (`FLOX_FLOXHUB_URL`, `FLOX_CATALOG_URL`,
`_FLOX_FLOXHUB_GIT_URL`, the `_FLOX_OAUTH_*` dev-tenant config, and
`SSL_CERT_FILE` for the mkcert TLS certs). **Don't run the demo inside that
activated shell**: with auto-activation, leaving `~/Code/floxhub` deactivates
the environment and strips those vars, and while inside it every `flox`
command targets the floxhub environment.

Instead, snapshot the vars to a file. In any shell where the floxhub
environment is active (e.g. the `just serve-all` terminal before starting the
services, or a second terminal that auto-activated on cd):

```bash
env | grep -E '^(FLOX_FLOXHUB_URL|FLOX_CATALOG_URL|_FLOX_FLOXHUB_GIT_URL|_FLOX_OAUTH_(AUTH_URL|TOKEN_URL|DEVICE_AUTH_URL|CLIENT_ID)|SSL_CERT_FILE|NIX_SSL_CERT_FILE)=' \
  | sed 's/^/export /' > /tmp/flox-demo-stack.env
```

The snapshot survives any cd and any number of terminals.

### Demo persona shell (each new terminal)

Each persona runs in a **bare shell** with an isolated `$HOME`, so the machine
looks factory-fresh, none of your real Flox files are touched, and your shell
RC's auto-activation hooks stay out of the picture:

```bash
/bin/zsh -f                          # bare shell: no RC files, no auto-activation
source /tmp/flox-demo-stack.env
export DEMO_ROOT="$TMPDIR/flox-demo"; mkdir -p "$DEMO_ROOT/home-a"
export HOME="$DEMO_ROOT/home-a"      # fresh machine simulation
alias flox=~/Code/flox/target/debug/flox
export FLOX_FEATURES_AUTO_DEFAULT=true   # auth-first defaults prototype flag
flox auth logout 2>/dev/null             # clean keychain slate for this persona
cd "$HOME"
```

Notes:

- Keep `DEMO_ROOT` short. Activation creates a unix socket under
  `$HOME/.cache/flox/run/`, and socket paths are capped at ~104 characters —
  a deeply nested `$HOME` fails with "path for services socket is too long".
- Because nothing is activated in this shell, you can cd anywhere; `-D`
  always names the default environment regardless of cwd.
- **Output noise to expect:** every command prints a two-line
  `Using … as the FloxHub git endpoint … intended for testing only` warning
  (an artifact of `_FLOX_FLOXHUB_GIT_URL`; harmless), and — while logged
  out — `! You are not logged in to FloxHub. Run 'flox auth login' to log
  in.` (suppressed inside an activated shell). The expected outputs below
  omit both.

> **Catalog note:** package resolution goes through the local catalog-server,
> which serves at least `jq` and `ripgrep` (verified). If some other package
> fails to resolve, drop the local catalog for the demo:
> `unset FLOX_CATALOG_URL` (falls back to the production catalog while
> environment hosting stays on the local stack).

## 2. Path A — CLI-first

*Persona: developer who found Flox before FloxHub. Uses `home-a`.*

### A1. Install a package with one login **[code-verified]**

```bash
flox install -D jq        # the global-install moment (`mise use --global`, plus an account)
```

Expected: the CLI notices you're logged out and runs the device flow inline —
authenticate in the browser tab it opens with persona A's identity — then
creates the environment on FloxHub, installs, and syncs:

```
You are not logged in to FloxHub. Re-authenticating...
Logging in to https://hub.local.flox.dev:8000
Your one-time activation code is: XXXX-XXXX
✔ Authentication complete
✔ Logged in as <handle>
⚡︎ Created your default environment '<handle>/default' on FloxHub.
✔ 'jq' installed to environment 'default'
✔ Synced your default environment to FloxHub.
```

Between creation and install, an interactive terminal also offers to add
`eval "$(flox activate --default -m run)"` to your shell RC files — the fake
`$HOME` means it edits the demo home, so saying *Yes* is safe.

Plain `flox install jq` behaves identically **when no environment is in
reach** (none in cwd or active); with an environment in cwd it installs there
instead, without any login — `-D` is the unambiguous form.

(Machine-verified half: logged out **without** a TTY, `install -D` fails
cleanly with the login instructions and creates nothing — scripts never
trigger the device flow or create environments.)

```bash
flox activate -D -- jq --version     # the default env works immediately
flox list -D                         # shows jq
```

One command, one login — and the environment is already on FloxHub for
every other machine.

### A2. Web onboarding is skipped **[browser]**

Open <https://hub.local.flox.dev:8000/> in a private browser window and log in
as the same account (complete the profile step if prompted).

Expected: **you land on the dashboard, not the onboarding wizard** — the
BFF saw `<handle>/default` exists and marked onboarding complete. The
environment page shows `jq`.

### A3. Installs keep FloxHub in sync **[code-verified]**

```bash
flox install -D ripgrep
```

Expected (no login prompt — the token from A1 is in the keychain):

```
✔ 'ripgrep' installed to environment 'default'
✔ Synced your default environment to FloxHub.
```

Refresh the environment page on the web — `ripgrep` is there, no `flox push`
needed.

### A4. Logout does not break your shell (DEV-269)

```bash
flox auth logout
eval "$(flox activate -D -m run)"    # what the shell RC line runs
jq --version && rg --version         # still works
```

Expected: activation succeeds from the local checkout (fetched under
`~/.cache/flox/remote/<handle>/default` when the environment was created),
with one info line:

```
ℹ Using the cached default environment '<handle>/default'. Run 'flox auth login' to sync with FloxHub.
```

No forced login, no hard error. Resolution knows the checkout is yours
because logout records your handle.

Also try it with FloxHub fully unreachable: stop the stack (Ctrl-C on
`just serve-all`) and activate again — still works. Restart the stack
afterwards. (Machine-verified with all FloxHub endpoints pointed at a dead
port, using a pre-existing local default; the cached-managed variant is
code-verified.)

Mutations while logged out are the flip side: `flox install -D` now asks you
to log in again (machine-verified) — using what you have is free, changing
it goes through FloxHub.

## 3. Path B — FloxHub-first

*Persona: developer invited to FloxHub by a teammate. Uses a second identity
and a second fake home. In a NEW terminal, repeat the §1 persona block but
with:*

```bash
export HOME="$DEMO_ROOT/home-b"; mkdir -p "$HOME"
```

(The persona block's `flox auth logout` matters here — it clears persona A's
keychain token.)

### B1. Onboard on the web **[browser]**

In a private browser window, sign up at <https://hub.local.flox.dev:8000/>
with the second identity. The onboarding wizard appears; on the *Select
packages* step keep/select a couple of packages and continue — this creates
`<handle-b>/default` on FloxHub.

> If package creation fails here, the local catalog is missing those
> packages — see the catalog note in §1. A default environment with **zero
> generations does not count as onboarded** (by design), so retry after
> fixing the catalog.

### B2. First contact with the CLI **[code-verified]**

```bash
flox auth login                      # log in as the second identity
```

Expected:

```
✔ Authentication complete
✔ Logged in as <handle-b>
ℹ Fetched your default environment '<handle-b>/default' from FloxHub. Activate it with 'flox activate --default'.
```

### B3. Use it **[code-verified]**

```bash
flox activate -D                     # packages picked in the wizard are on PATH
```

The wizard's *Install Flox / Activate* steps map exactly onto what just
happened — one login and the environment was already there.

### B4. Offline / logged out **[code-verified]**

```bash
flox auth logout
flox activate -D -- true && echo "still works"
```

Expected: activation from the cached checkout at
`~/.cache/flox/remote/<handle-b>/default`, with the same info line as A4.

## 4. Resetting between runs

```bash
# CLI side: log out first (the keychain outlives the fake home), then wipe
# the persona's fake home (config, cache, and any ~/.flox live under it)
flox auth logout
rm -rf "$DEMO_ROOT/home-a"          # or home-b

# FloxHub side: delete <handle>/default via the web UI
#   (environment page -> Settings -> Delete), then re-arm onboarding.
```

Re-arm the onboarding wizard for an account (run in the browser devtools
console while logged in on hub.local.flox.dev — the snippet resolves your
handle itself; the endpoint returns **403 Forbidden** if the handle in the
URL is not the logged-in user's):

```js
const me = await (await fetch('https://api.local.flox.dev:8000/web-bff/api/auth/me', {
  credentials: 'include',
})).json();
await fetch(`https://api.local.flox.dev:8000/web-bff/api/user/${me.handle}/onboarding-completed`, {
  method: 'PATCH',
  credentials: 'include',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify({ completed: false }),
});
```

Note: the heal re-marks onboarding complete whenever `<handle>/default`
exists with generations — delete the environment first if you want the
wizard back.

## 5. What to evaluate

While walking through, judge the experience against these questions:

1. **Time-to-value**: how long from "fresh machine" to a working package in
   Path A? Is the inline device flow an acceptable cost for the one gate, or
   does it need trimming (e.g. fewer lines, auto-opening the browser)?
2. **Messaging**: does each transition (login → create → sync →
   skip-onboarding → offline) explain itself? Is anything chatty enough to
   become noise in every new shell?
3. **Trust**: did anything happen to your environment you didn't expect?
   Is auto-pushing on install comfortable, or should it batch/ask?
4. **Symmetry**: does Path B's "log in and it's just there" feel like the
   Chrome-profile moment? What's missing (RC-file setup isn't offered on
   the login path yet)?
5. **Failure honesty**: unplug the stack mid-flow — are the warnings
   actionable, and does the primary command still succeed?

Known prototype limitations (deliberate scope cuts, confirmed by review):

- Auto-sync-on-mutation covers `install`/`uninstall`, not `edit`/`upgrade` —
  and it triggers for *any* checkout of `<handle>/default` (including one you
  explicitly `flox pull`ed elsewhere), not just the `-D` cache.
- The implicit login inside `flox install` does not run the login-time
  reconcile that explicit `flox auth login` does (push local-only default /
  pre-fetch remote-only); by design, implicit re-auth must not grow side
  effects. A `~/.flox` default from the earlier zero-setup build converts to
  FloxHub on the next authenticated `install -D`, or on explicit login.
- `flox uninstall -D` is not auth-gated up front; with a cached checkout it
  works logged out (and skips the sync). End-state `/resolve` gating
  (DEV-236) will cover it.
- Login auto-sync doesn't offer RC-file setup; only env creation does.
- Web-side deletion of the default env doesn't stick while a local checkout
  keeps auto-pushing (a tombstone needs designing).
- Info-line noise: the logged-out reminder prints outside activations, and
  the cached-use line (A4/B4) prints on every activation until the next
  login (needs announce-once state).
- A failed FloxHub probe during authed `-D` resolution says "Could not reach
  FloxHub" even when the real cause is an auth rejection; classifying
  AccessDenied vs NetworkUnreachable in that message is a follow-up.
- The SDK's stale-fetch fallback logs via tracing only; a product-level
  "using the last fetched version" message needs plumbing.
- The web-bff onboarding heal probes floxem on every `/auth/me` for
  not-yet-onboarded users with no timeout/negative-cache — fine locally,
  needs bounding before production.
- `confirmed_create_default_env` (the old prompt's answer) is reused as the
  opt-out for both auto-creation and login auto-push; a dedicated field
  should replace that overload.

## 6. Verification status (2026-08-19)

Machine-verified by execution against the local stack (fresh fake homes,
scrubbed env, `FLOX_FEATURES_AUTO_DEFAULT=true`):

- Logged out without a TTY, `flox install -D jq` and plain `flox install jq`
  (no env in reach) fail with the login instructions and create nothing.
- With a pre-existing local `~/.flox` default (earlier zero-setup build):
  `flox activate -D -- <cmd>` and `flox list -D` work logged out;
  `flox install -D` demands a login; activation still works with every
  FloxHub endpoint pointed at a dead port.
- `eval "$(flox activate -D -m run)"` in a bare zsh.
- Local catalog resolves `jq`/`ripgrep` anonymously today (server-side
  gating is the upcoming change, #4637).
- Keychain entries are keyed by FloxHub URL (`hub.local.flox.dev` ≠
  `hub.flox.dev`), so the demo cannot touch a production token.
- The §4 reset endpoint 403s when the URL handle ≠ session user (hence the
  self-resolving snippet).

Code-verified only (need an interactive device-flow login and/or a browser;
the dev tenant has no email+password connection, so these can't run
headlessly): the interactive halves of A1 and A3, and A2, B1–B4. Message
texts quoted above are copied from the source.
