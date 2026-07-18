# Demo: `flox activate --sandbox` — the OpenShell backend (prototype)

`cd` into a project and land in an **NVIDIA OpenShell**-supervised
sandbox: only your project and your tools exist, your declared
services are already running, every outbound connection is governed
by a hot-reloadable L7 policy — and a coding agent runs at full
autonomy inside a boundary it cannot cross.

**Bold** lines are what to *say*; fenced blocks are what to *type*.
The OCI-backend walkthrough is `demo/SCRIPT.md`; the two share
`demo/setup.sh` and `demo/cleanup.sh`.

**The pitch:** flox already bakes each environment into an OCI
image. This backend hands that image to OpenShell's gateway — flox
brings the reproducible environment, OpenShell brings supervised
isolation with L7 egress policy. Same manifest, one word changed:
`backend = "openshell"`.

**Verification status** (macOS arm64, Docker Desktop 28.5.1,
OpenShell 0.0.82):

- Beats 1–5 verified 2026-07-13/14; full end-to-end run including
  beat 4's live agent on 2026-07-16.
- Re-verified exec-mode 2026-07-17 end-to-end: the beat 2 log-tail
  resequencing, the demo-secrets probe, the manifest-declared agent
  grants (the proxy identified `.claude-wrapped` by store path and
  allowed api.anthropic.com per the manifest rule while denying its
  ungranted Datadog telemetry endpoint), the beat 3 hot-reload flip,
  and `--no-keep` teardown. Note `--tail` *replays* recent events —
  starting it a beat late still shows the deny.
- **Known issue until the builder pin is bumped:** in-guest
  `flox services status` (beat 1) fails to parse the project
  lockfile — the pinned guest flox predates the
  `options.sandbox.network` field. The services themselves start
  and serve fine. Fix: push the branch, bump the openshell
  `FROZEN_FALLBACK_REV` to the pushed head, re-dispatch the
  frozen-builder-cache workflow, rebake.
- Still needing a live interactive rehearsal: the full `cd` +
  consent + interactive-session flow, beat 4's real agent run
  (needs the pre-seeded token), and the layered one-command setup
  (`djsauble/openshell-setup`, published 2026-07-18, including its
  profile/deactivate handlers — untestable on a host whose gateway
  already owns port 17670).
- Grant-follows-binary confirmed in the allow direction: `claude`
  (scoped grant) reached its API through the proxy while `curl` in
  the same session stayed denied against ungranted endpoints. A
  binary-mismatch denial against a *granted* endpoint has not been
  staged.
- `read-only` does **not** block write methods on 0.0.82 — keep
  write-denial out of the talk track (see beat 3).
- In-guest `claude` login is a dead end (the OAuth URL can't be
  copied out); beat 4 pre-seeds a token instead.

---

## 0 · Setup

### One-time host prerequisites

1. **Docker Desktop** (or Docker Engine ≥ 28) running.
2. **OpenShell control plane + presentation shell** — one command,
   in your presentation shell (export `FLOX_BIN` from the dev
   shell first):

   ```bash
   flox activate -r djsauble/openshell-setup
   ```

   This is the demo's *outer layer* — one setup env per sandbox
   backend is the plan. It installs `djsauble/openshell` (0.0.86),
   generates gateway TLS, renders a gateway config (docker driver,
   bind mounts), runs `openshell-gateway` as a flox service,
   registers it as gateway `flox-demo` — and configures the shell:
   feature flags and the planted `GITHUB_TOKEN` (`[vars]`),
   `FLOX_VERSION` plus a `flox` alias from `$FLOX_BIN`
   (`[profile]`), and the `~/demo-secrets` fixture. Deactivating
   removes the planted secret (`[profile.deactivate]`). Stay in
   this activation for the whole demo; confirm:

   ```bash
   openshell status        # Status: Connected
   ```

   > ⚠️ Not yet rehearsed end-to-end (it cannot run beside an
   > already-provisioned gateway — both want port 17670), and
   > registration writes `~/.config/openshell/gateways/flox-demo`
   > and may switch your active gateway (`openshell gateway
   > select <name>` switches back; `demo/cleanup.sh` removes the
   > registration). The env is private to djsauble; the in-repo
   > definition is `demo/openshell-setup/`. On a machine with a
   > working manual setup, skip this and use that gateway.

   **Manual alternative** (the path every prior verification
   used): install OpenShell ≥ 0.0.62 (0.0.82 tested) via
   `curl -LsSf https://raw.githubusercontent.com/NVIDIA/OpenShell/main/install.sh | sh`,
   then in `~/.config/openshell/gateway.toml`:

   ```toml
   [openshell.gateway]
   compute_drivers = ["docker"]

   [openshell.drivers.docker]
   enable_bind_mounts = true
   ```

   and restart the gateway; `openshell status` should report
   Connected.

   > **PATH pitfall** (either path): the Flox catalog's own
   > `openshell` (0.0.36) is far too old. If any active flox
   > environment installs it, it shadows a newer install and
   > preflight fails with "OpenShell CLI version … is too old".
   > Check `which openshell`.

   > `enable_bind_mounts` live-mounts the project into the sandbox
   > at its real path — an isolation tradeoff scoped to exactly the
   > one directory you're asking the agent to work on.

### Demo environment

Run once from the dev shell:

```bash
BACKEND=openshell bash demo/setup.sh
```

Same demo env as the OCI walkthrough (git, curl, which, python3,
`flox/claude-code`, an auto-starting web service, seeded `app.py` /
`index.html`); the manifest declares `backend = "openshell"` plus
network grants for the agent's API endpoints, scoped to the exact
claude binary:

```toml
[[options.sandbox.network]]
endpoint = "api.anthropic.com:443"
binary   = "claude-code/.claude-wrapped"
# plus an identical grant for statsig.anthropic.com (agent telemetry)
```

flox compiles these into the OpenShell policy at sandbox create,
resolving `binary` to the locked store path for the guest. Policy
edits never rebake the image — the image tag ignores
`[options.sandbox]`.

If you used the setup env (prerequisite 2), your shell is already
configured — just make sure the prompt hook is in your shell's RC:

```bash
eval "$(flox hook-env --shell bash --shell-pid $$)"
```

The session is *layered*: the setup env is the outer layer, and
beat 1's `cd` activates the project env on top of it. Cleanup is
symmetric — deactivate the sandbox, then the setup env.

On the manual path, configure the presentation shell by hand:

```bash
alias flox='$FLOX_BIN'                 # the prototype binary
export FLOX_FEATURES_SANDBOX_ACTIVATE=true
export FLOX_FEATURES_AUTO_ACTIVATE=true
export GITHUB_TOKEN=ghp-demo-FAKE      # for the token-isolation beat
export FLOX_VERSION=`flox --version`
```

> `FLOX_VERSION` routes the bake: a `-g<sha>` version pins the
> in-VM builder to this branch's frozen rev; a plain release
> version routes to the release builder, which lacks the OpenShell
> compat layer.

**Pre-bake off-camera.** The first bake takes ~5–15 min on a
machine that has to compile the pinned flox rev in-VM, or ~2–5 min
if the pin is in the flox cache (dispatch
`.github/workflows/frozen-builder-cache.yml` once per pin bump).
Later bakes reuse the builder's store:

```bash
cd ~/sandbox-demo && FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate -- true
```

The image lands in Docker as `sandbox-demo-openshell:<hash12>`,
content-addressed to the lockfile — it rebakes only when the
environment actually changes.

---

## 1 · Auto-activate into an OpenShell sandbox

*(verified 2026-07-13)*

**"One `cd`, one `Y`, and I'm inside an OpenShell-supervised
sandbox with my project mounted and my service already running."**

```bash
cd /tmp && cd ~/sandbox-demo
```

```
Enter '/Users/you/sandbox-demo' (sandboxed via openshell)? [Y/n]
```

Type `Y`. flox creates the sandbox through the OpenShell gateway
(`--no-keep`: it lives exactly as long as the session) and execs
the activation entrypoint inside it.

```
✔ You are now using the environment 'sandbox-demo'
To stop using this environment, run 'flox deactivate'

flox [sandbox-demo] bash-5.3$ uname -sm
Linux aarch64
flox [sandbox-demo] bash-5.3$ whoami
sandbox
```

**"Not root: OpenShell runs the workload as an unprivileged
`sandbox` user under a supervisor that owns the network namespace —
flox baked that user in as part of the compat layer."**

**"The declared service came up on its own."**

```
flox [sandbox-demo] bash-5.3$ flox services status
NAME       STATUS       PID
web        Running       ##

flox [sandbox-demo] bash-5.3$ curl -s localhost:8080
<!doctype html><title>sandbox-demo</title>
<h1>Hello from inside the flox sandbox</h1>
```

Meanwhile, in a **second terminal on the host** (keep it visible —
it's the control plane for beats 2 and 3):

```bash
openshell sandbox list
NAME                     CREATED              PHASE
flox-sandbox-demo-#####  2026-07-13 ...       Ready
```

**"That's flox's sandbox on OpenShell's control plane — logs,
terminals, policy: NVIDIA's whole operational surface, zero extra
wiring."**

---

## 2 · Prove the boundary is intact

*(verified 2026-07-13; resequenced tail flow re-verified exec-mode
2026-07-17 — `--tail` replays recent events, so starting it late
still shows the deny)*

**"My filesystem is invisible, my credentials don't cross, and only
my project is live."**

In the **host terminal** — a real (planted) secret, seeded by the
setup layer (and by setup.sh on the manual path):

```bash
ls -a ~/demo-secrets/
# .  ..  .env
```

Inside the guest, the directory doesn't exist:

```bash
flox [sandbox-demo] bash-5.3$ ls -a /Users/you/demo-secrets/
ls: cannot access '/Users/you/demo-secrets/': No such file or directory

flox [sandbox-demo] bash-5.3$ printenv GITHUB_TOKEN
flox [sandbox-demo] bash-5.3$
```

**"And the network is deny-by-default at layer 7 — every verdict an
audit event. Watch live:"**

In a **third host terminal** (or split pane — beat 3 needs the
control terminal free), tail verdicts and leave it running:

```bash
openshell logs flox-sandbox-demo-##### --tail
```

Back in the guest:

```bash
flox [sandbox-demo] bash-5.3$ curl -sS https://api.github.com/zen
curl: (7) CONNECT tunnel failed, response 403
```

The tail prints the denial as it happens:

```
# [ocsf] NET:OPEN [MED] DENIED /nix/store/…-curl-8.x.x/bin/curl(…) -> api.github.com:443 [policy:- engine:opa] [reason:endpoint api.github.com:443 is not allowed by any policy]
```

**"flox generated this policy for the activation — Nix store
read-only, project read-write, and only the network the manifest
grants. api.github.com has no rule, so curl is denied at layer 7 —
and every denial lands on the audit log, down to the store path of
the binary that tried. The grants that do exist are scoped to the
exact claude binary; even curl against the agent's endpoints would
be denied."**

---

## 3 · Hot-reload a network policy — no restart

*(hot-reload and the binary-scoped GET verified 2026-07-13,
re-verified 2026-07-14 and exec-mode 2026-07-17)*

> **Do not demo write-denial:** on 0.0.82 a `read-only:rest` grant
> still lets POST/DELETE through (logged ALLOWED). Stick to
> deny-by-default, hot-reload, per-binary identity, and the audit
> log; raise method enforcement with NVIDIA (integration notes
> pointer at the bottom).

**"I'm going to grant this running sandbox GitHub access — no
restart, and the grant names the exact binary allowed to use it."**

In the guest, resolve the tool you're granting — in a flox
environment every binary is a content-addressed store path:

```bash
flox [sandbox-demo] bash-5.3$ readlink -f $(command -v curl)
/nix/store/…-curl-8.x.x/bin/curl
```

In the **host terminal**:

```bash
openshell policy update flox-sandbox-demo-##### \
  --add-endpoint 'api.github.com:443:read-only:rest' \
  --binary /nix/store/…-curl-8.x.x/bin/curl \
  --wait
```

Back in the guest — same session, nothing restarted:

```bash
flox [sandbox-demo] bash-5.3$ curl -sS https://api.github.com/zen
Practicality beats purity.
```

**"The store path pins the policy to the exact build of curl the
environment shipped — not 'anything named curl'. And the verdict
flipped in the tail:"**

```
# [ocsf] HTTP:GET [INFO] ALLOWED GET http://api.github.com:443/zen [policy:allow_api_github_com_443 engine:l7]
```

**"That's the division of labor: flox defines *what the environment
is*; OpenShell governs *what it's allowed to do* — live."**

> The agent beat needs no grant of its own: the manifest already
> declares the Anthropic endpoints, scoped to the claude binary
> (see §0). Beat 3's live `policy update` is the hot-reload story;
> the manifest is the declarative one — same policy engine.

---

## 4 · Run a coding agent, at full autonomy

*(verified end-to-end 2026-07-16 with manual Anthropic grants;
the manifest-declared grants that replace them are not yet
rehearsed. Agent flow identical to demo/SCRIPT.md §3.)*

**"A coding agent with no permission prompts, that I don't have to
trust — the sandbox, not the agent, is the boundary."**

> **Authenticate before the demo.** In-guest login is a dead end
> (the OAuth URL can't be copied out of a sandboxed session).
> Instead, on the **host**:
>
> ```bash
> claude setup-token
> ```
>
> (browser OAuth; one-year `sk-ant-oat01-…` token; needs a Claude
> subscription), then park it in a gitignored `.env` at the project
> root — the env hook sources it into every activation:
>
> ```bash
> printf 'CLAUDE_CODE_OAUTH_TOKEN=%s\n' '<token>' > ~/sandbox-demo/.env
> ```
>
> The file lives under the project mount — the one directory you
> chose to share — so beat 2's isolation story is unchanged. (The
> guest runs unprivileged, so
> `claude --dangerously-skip-permissions` also works if you prefer
> it to `--permission-mode auto`.)

With the token pre-seeded, start the agent:

```bash
flox [sandbox-demo] bash-5.3$ claude --permission-mode auto
```

Give it real work:

```
> add a docstring to greet() in app.py and commit the change
```

Claude edits `app.py` and commits — no per-action prompts. Anything
it tries outside the policy is denied and lands in the log tail —
in rehearsal the agent's own Datadog telemetry endpoint showed up
DENIED while its granted API traffic flowed, which makes a nice
closing line.

---

## 5 · Exit the sandbox — the work persists, the sandbox doesn't

*(verified 2026-07-13)*

```bash
flox [sandbox-demo] bash-5.3$ flox deactivate
```

You land back at your own shell. On the host:

```bash
git -C ~/sandbox-demo log --oneline -1
# <hash> add docstring to greet()          ← the agent's commit

openshell sandbox list
# No sandboxes found.                      ← --no-keep cleaned up
```

**"The commit is on my host repo — the project was mounted live.
The sandbox is already gone. Reproducible environment in, governed
session out, nothing left running."**

---

## 6 · Reset

Deactivate the setup layer (its `profile.deactivate` removes the
planted secret and its exit stops the gateway service), then:

```bash
bash demo/cleanup.sh
```

Removes the env, fixtures, Docker-side `sandbox-demo-openshell:*`
images, any lingering demo sandboxes, and the `flox-demo` gateway
registration.

> Integration notes for the NVIDIA conversation (image
> requirements, policy compilation, released-vs-docs gaps):
> `slices/2026/06-sandboxed-activation-prototype/artifacts/openshell-integration.md`.
