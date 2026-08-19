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
| `flox` | DEV-269 offline/logged-out `-D` fallback (always on) + zero-setup ladder, login auto-sync, post-install auto-push (behind `FLOX_FEATURES_AUTO_DEFAULT`) |
| `floxhub` | web-bff reports onboarding complete when `<handle>/default` exists with ≥1 generation (and heals the Auth0 flag); `PATCH /user/:handle/onboarding-completed` accepts `{"completed": false}` for demo resets |

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

Auth is the real Auth0 **dev tenant** (`auth.dev.flox.dev`). You'll want two
fresh accounts (sign up with email+password during the first login):

- one for **Path A** (never touches web onboarding until the end),
- one for **Path B** (starts in web onboarding).

Re-running a path with the *same* account requires the reset steps in §4.

### Demo shell setup (each new terminal)

Each persona runs with an isolated `$HOME` so the machine looks factory-fresh
and none of your real Flox state is touched.

First, activate the floxhub environment — its activation exports everything
that points the CLI at the local stack (`FLOX_FLOXHUB_URL`,
`_FLOX_FLOXHUB_GIT_URL`, `FLOX_CATALOG_URL`, the `_FLOX_OAUTH_*` dev-tenant
config, and `SSL_CERT_FILE` for the mkcert TLS certs):

```bash
cd ~/Code/floxhub && flox activate
```

Then paste the persona setup **inside that activated shell**:

```bash
# ---- demo persona shell setup ----
export DEMO_ROOT="$TMPDIR/flox-demo"; mkdir -p "$DEMO_ROOT/home-a"
export HOME="$DEMO_ROOT/home-a"                     # fresh machine simulation
alias flox=~/Code/flox/target/debug/flox

# MUST come after `flox activate` — the activation itself sets these, and
# `flox install` targets the last-activated env when the cwd has none, so
# leaving them set would install into the floxhub env instead of creating
# the demo default env
unset FLOX_FLOXHUB_TOKEN FLOX_CONFIG_DIR _FLOX_ACTIVE_ENVIRONMENTS \
      FLOX_ENV FLOX_ENV_CACHE FLOX_ENV_PROJECT FLOX_ENV_DIRS \
      FLOX_ENV_DESCRIPTION FLOX_PROMPT_ENVIRONMENTS

# Feature flag for the zero-setup prototype (not exported by the activation)
export FLOX_FEATURES_AUTO_DEFAULT=true
# ----------------------------------
```

> **Catalog note:** package resolution goes through the local catalog-server.
> If `flox search jq` errors (e.g. the local test catalog lacks the package or
> rejects anonymous access), drop the local catalog for the demo:
> `unset FLOX_CATALOG_URL` (falls back to the production catalog while
> environment hosting stays on the local stack).

## 2. Path A — CLI-first

*Persona: developer who found Flox before FloxHub. Uses `home-a`.*

### A1. Install a package with zero setup

```bash
cd $HOME
flox install jq
```

Expected: **no prompt, no `flox init`**. The CLI prints something like

```
⚡︎ Created your default environment at '~/.flox'. Log in with 'flox auth login' to sync it with FloxHub.
✔  'jq' installed to environment default
```

(In an interactive terminal it also offers to add
`eval "$(flox activate --default -m run)"` to your shell RC files — the fake
`$HOME` means it edits the demo home, so saying *Yes* is safe.)

```bash
flox activate -D -- jq --version     # the default env works immediately
flox envs                            # shows: default at ~/.flox
```

This is the Mise moment: install → use, no environment ceremony, no account.

### A2. Log in — the default environment follows you to FloxHub

```bash
flox auth login
```

Expected after the device-flow dance:

```
✔  Authentication complete
✔  Logged in as <handle>
✔  Synced your default environment to FloxHub as '<handle>/default'. It will now sync across your machines.
```

`~/.flox` is now a checkout of `<handle>/default` (see `cat ~/.flox/env.json`).

### A3. Web onboarding is skipped

Open <https://hub.local.flox.dev:8000/> in a private browser window and log in
as the same account (complete the profile step if prompted).

Expected: **you land on the dashboard, not the onboarding wizard** — the
BFF saw `<handle>/default` exists and marked onboarding complete. The
environment page shows `jq`.

### A4. Installs keep FloxHub in sync

```bash
flox install -D ripgrep
```

Expected:

```
✔  'ripgrep' installed to environment default
✔  Synced your default environment to FloxHub.
```

Refresh the environment page on the web — `ripgrep` is there, no `flox push`
needed.

### A5. Logout does not break your shell (DEV-269)

```bash
flox auth logout
eval "$(flox activate -D -m run)"    # what the shell RC line runs
jq --version && rg --version         # still works
```

Expected: activation succeeds from local state. No forced login, no
"You are not logged in" hard error — at most an informational line.

Also try it with FloxHub fully unreachable: stop the stack (Ctrl-C on
`just serve-all`) and activate again — still works. Restart the stack
afterwards.

## 3. Path B — FloxHub-first

*Persona: developer invited to FloxHub by a teammate. Uses a second account
and a second fake home:*

```bash
# in a NEW terminal: repeat the §1 setup (flox activate + persona block), but with
export HOME="$DEMO_ROOT/home-b"; mkdir -p "$HOME"
```

### B1. Onboard on the web

In a private browser window, sign up at <https://hub.local.flox.dev:8000/>
with the second account. The onboarding wizard appears; on the *Select
packages* step keep/select a couple of packages and continue — this creates
`<handle-b>/default` on FloxHub.

> If package creation fails here, the local catalog is missing those
> packages — see the catalog note in §1. A default environment with **zero
> generations does not count as onboarded** (by design), so retry after
> fixing the catalog.

### B2. First contact with the CLI

```bash
cd $HOME
flox auth login                      # log in as the second account
```

Expected:

```
✔  Authentication complete
✔  Logged in as <handle-b>
ℹ  Fetched your default environment '<handle-b>/default' from FloxHub. Activate it with 'flox activate --default'.
```

### B3. Use it

```bash
flox activate -D                     # packages picked in the wizard are on PATH
```

The wizard's *Install Flox / Activate* steps map exactly onto what just
happened — one login and the environment was already there.

### B4. Offline / logged out

```bash
flox auth logout
flox activate -D -- true && echo "still works"
```

Expected: activation from the cached checkout, logged out (DEV-269 again,
this time via the `~/.cache/flox/remote/<handle-b>/default` checkout).

## 4. Resetting between runs

```bash
# CLI side: wipe a persona's fake home
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
- `flox install -D` with *no* default env anywhere still errors instead of
  creating one (plain `flox install` and `flox activate -D` cover creation).
- Login auto-sync doesn't offer RC-file setup; only env creation does.
- Web-side deletion of the default env doesn't stick while a local checkout
  keeps auto-pushing (a tombstone needs designing).
- Info-line noise: the logged-out cached-use line prints on every activation,
  and the "local + FloxHub default both exist" explainer prints on every `-D`
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
