# Zero-setup default environments — demo script

This walkthrough exercises the default-environment onboarding prototype
end-to-end against the local FloxHub stack. It covers both directions:

- **Path A (CLI-first)**: install a package with zero setup, watch the default
  environment appear locally, sync to FloxHub on login, skip the web
  onboarding wizard, and keep working after logout (DEV-269).
- **Path B (FloxHub-first)**: create the default environment in the web
  onboarding wizard, then pick it up on a "new machine" with a single
  `flox auth login`.

The prototype spans two branches, both named `prototype/default-env-onboarding`:

| Repo | What changed |
|------|--------------|
| `flox` | DEV-269 offline/logged-out `-D` fallback (always on) + zero-setup ladder (`flox install [-D]` creates the default env), login auto-sync, post-install auto-push (behind `FLOX_FEATURES_AUTO_DEFAULT`) |
| `floxhub` | web-bff reports onboarding complete when `<handle>/default` exists with ≥1 generation (and heals the Auth0 flag); `PATCH /user/:handle/onboarding-completed` accepts `{"completed": false}` for demo resets |

Every CLI step below was machine-verified on 2026-08-19 against the local
stack unless marked **[code-verified]** (behavior confirmed in the source, but
the step needs a login or a browser and so wasn't executed) — see §6.

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
looks factory-fresh, none of your real Flox state is touched, and your shell
RC's auto-activation hooks stay out of the picture:

```bash
/bin/zsh -f                          # bare shell: no RC files, no auto-activation
source /tmp/flox-demo-stack.env
export DEMO_ROOT="$TMPDIR/flox-demo"; mkdir -p "$DEMO_ROOT/home-a"
export HOME="$DEMO_ROOT/home-a"      # fresh machine simulation
alias flox=~/Code/flox/target/debug/flox
export FLOX_FEATURES_AUTO_DEFAULT=true   # zero-setup prototype flag
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
  in.` once per command (it's suppressed inside an activated shell). The
  expected outputs below omit both.

> **Catalog note:** package resolution goes through the local catalog-server,
> which serves at least `jq` and `ripgrep` anonymously (verified). If some
> other package fails to resolve, drop the local catalog for the demo:
> `unset FLOX_CATALOG_URL` (falls back to the production catalog while
> environment hosting stays on the local stack).

## 2. Path A — CLI-first

*Persona: developer who found Flox before FloxHub. Uses `home-a`.*

### A1. Install a package with zero setup

```bash
flox install -D jq        # the `mise use --global` moment
```

Expected: **no prompt, no `flox init`, no login** — works from any directory.

```
⚡︎ Created your default environment at '~/.flox'. Log in with 'flox auth login' to sync it with FloxHub.
✔ 'jq' installed to environment 'default'
ℹ 'jq' has additional outputs, use 'flox list -a' to see more
```

Plain `flox install jq` does the same **when no environment is in reach**
(none in cwd or active); with an environment in cwd it installs there
instead — `-D` is the unambiguous form.

In an interactive terminal, creation also offers to add
`eval "$(flox activate --default -m run)"` to your shell RC files — the fake
`$HOME` means it edits the demo home, so saying *Yes* is safe.

```bash
flox activate -D -- jq --version     # the default env works immediately
flox envs                            # shows: default at $HOME
```

This is the Mise moment: install → use, no environment ceremony, no account.

### A2. Log in — the default environment follows you to FloxHub **[code-verified]**

```bash
flox auth login
```

Expected after the device-flow dance (log in with persona A's identity):

```
✔ Authentication complete
✔ Logged in as <handle>
✔ Synced your default environment to FloxHub as '<handle>/default'. It will now sync across your machines.
```

`~/.flox` is now a checkout of `<handle>/default` (see `cat ~/.flox/env.json`).

### A3. Web onboarding is skipped **[browser]**

Open <https://hub.local.flox.dev:8000/> in a private browser window and log in
as the same account (complete the profile step if prompted).

Expected: **you land on the dashboard, not the onboarding wizard** — the
BFF saw `<handle>/default` exists and marked onboarding complete. The
environment page shows `jq`.

### A4. Installs keep FloxHub in sync **[code-verified]**

```bash
flox install -D ripgrep
```

Expected:

```
✔ 'ripgrep' installed to environment 'default'
✔ Synced your default environment to FloxHub.
```

Refresh the environment page on the web — `ripgrep` is there, no `flox push`
needed.

### A5. Logout does not break your shell (DEV-269)

```bash
flox auth logout
eval "$(flox activate -D -m run)"    # what the shell RC line runs
jq --version && rg --version         # still works
```

Expected: activation succeeds from the `~/.flox` checkout, silently — no
forced login, no "You are not logged in" hard error (the once-per-shell
logged-out reminder still appears). Resolution knows the checkout is yours
because logout records your handle.

Also try it with FloxHub fully unreachable: stop the stack (Ctrl-C on
`just serve-all`) and activate again — still works. Restart the stack
afterwards. (Verified for the logged-out-from-the-start case with all
FloxHub endpoints pointed at a dead port; the post-login variant is
code-verified.)

## 3. Path B — FloxHub-first

*Persona: developer invited to FloxHub by a teammate. Uses a second identity
and a second fake home. In a NEW terminal, repeat the §1 persona block but
with:*

```bash
export HOME="$DEMO_ROOT/home-b"; mkdir -p "$HOME"
```

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
`~/.cache/flox/remote/<handle-b>/default`, with one info line:

```
ℹ Using the cached default environment '<handle-b>/default'. Run 'flox auth login' to sync with FloxHub.
```

## 4. Resetting between runs

```bash
# CLI side: wipe a persona's fake home (config, cache, and ~/.flox all live
# under it, so this is a complete reset)
rm -rf "$DEMO_ROOT/home-a"          # or home-b

# FloxHub side: delete <handle>/default via the web UI
#   (environment page -> Settings -> Delete), then re-arm onboarding.
```

Re-arm the onboarding wizard for an account (run in the browser devtools
console while logged in on hub.local.flox.dev):

```js
await fetch(`https://api.local.flox.dev:8000/web-bff/api/user/<handle>/onboarding-completed`, {
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
   Path A? Is anything left that could still be removed?
2. **Messaging**: does each transition (create → sync → skip-onboarding →
   offline) explain itself? Is anything chatty enough to become noise in
   every new shell?
3. **Trust**: after A2, did anything happen to your environment you didn't
   expect? Is auto-pushing on install comfortable, or should it batch/ask?
4. **Symmetry**: does Path B's "log in and it's just there" feel like the
   Chrome-profile moment? What's missing (RC-file setup isn't offered on
   the login path yet)?
5. **Failure honesty**: unplug the stack mid-flow — are the warnings
   actionable, and does the primary command still succeed?

Known prototype limitations (deliberate scope cuts, confirmed by review):

- Auto-sync-on-mutation covers `install`/`uninstall`, not `edit`/`upgrade` —
  and it triggers for *any* checkout of `<handle>/default` (including one you
  explicitly `flox pull`ed elsewhere), not just `~/.flox` and the `-D` cache.
- `flox uninstall -D` (unlike `install -D`) never creates a default
  environment — there is nothing to uninstall from — so with no default env
  anywhere it requires a login to resolve.
- Login auto-sync doesn't offer RC-file setup; only env creation does.
- Web-side deletion of the default env doesn't stick while a local checkout
  keeps auto-pushing (a tombstone needs designing).
- Info-line noise: the logged-out reminder prints once per shell outside
  activations, the B4 cached-use line prints on every activation, and the
  "local + FloxHub default both exist" explainer prints on every `-D`
  resolution until the conflict is resolved (needs announce-once state).
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
logged out, `FLOX_FEATURES_AUTO_DEFAULT=true`):

- `flox install -D jq` in a fresh home creates `~/.flox` + installs (A1),
  including from inside another environment's directory (installs to
  `default`, not the cwd env).
- Plain `flox install jq` with no env in reach behaves identically.
- `flox install -D ripgrep` into the existing default; `flox uninstall -D`.
- `flox activate -D -- <cmd>`, `flox envs`, and
  `eval "$(flox activate -D -m run)"` in a bare zsh.
- Activation with all FloxHub endpoints unreachable (dead port).
- Local catalog resolves `jq`/`ripgrep` anonymously.

Code-verified only (need a dev-tenant login and/or a browser; the tenant has
no email+password connection, so these can't run headlessly): A2, A4 sync
line, B2, B3, B4, and both web steps (A3, B1). Message texts quoted above are
copied from the source.
