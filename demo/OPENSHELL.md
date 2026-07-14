# Demo: `flox activate --sandbox` — the OpenShell backend (prototype)

A single-terminal-plus-sidecar walkthrough of flox activating into
**NVIDIA OpenShell**: `cd` into a project, consent, and land in an
OpenShell-supervised sandbox where your declared services are already
running, the only files that exist are your project and the tools you
installed — and every outbound connection is governed by a
hot-reloadable, HTTP-method-level network policy. Then run a coding
agent at full autonomy inside a boundary it cannot cross.

**Bold** lines are what to *say*; fenced blocks are what to *type*.
This script covers the `openshell` backend only. The OCI-backend
walkthrough is `demo/SCRIPT.md`; the two share `demo/setup.sh` and
`demo/cleanup.sh`.

**Why this demo exists:** flox already bakes each environment into an
OCI image. The `openshell` backend hands that image to OpenShell's
gateway instead of a bare container runtime — flox brings the
reproducible environment, OpenShell brings supervised isolation with
L7 egress policy. Same manifest, one word changed:
`backend = "openshell"`.

**Verification status:** see the "verified" notes per beat. Beats
1, 2, and 5 were verified live on macOS (arm64, Docker Desktop
28.5.1, OpenShell 0.0.82) on 2026-07-13, as were beat 3's
deny-by-default, hot-reload, and binary-scoped GET. Two things need
one off-camera rehearsal before recording: beat 3's write-denial
output shape (L7 read-only semantics vary with the endpoint's
protocol config), and beat 4 (agent login/run, which needs an
interactive Anthropic login).

---

## 0 · Setup

### One-time host prerequisites

1. **Docker Desktop** (or Docker Engine ≥ 28) running.
2. **OpenShell CLI + gateway** (v0.0.82 tested). Homebrew install:
   `curl -LsSf https://raw.githubusercontent.com/NVIDIA/OpenShell/main/install.sh | sh`
   — or install the release tarballs (`openshell`,
   `openshell-gateway`) from
   https://github.com/NVIDIA/OpenShell/releases and provision the
   gateway manually (generate-certs + `gateway add`; see the
   openshell.rb formula for the exact service recipe).
3. **Gateway config** — the demo needs the Docker driver and bind
   mounts enabled. In `~/.config/openshell/gateway.toml`:

   ```toml
   [openshell.gateway]
   compute_drivers = ["docker"]

   [openshell.drivers.docker]
   enable_bind_mounts = true
   ```

   Restart the gateway after editing, then confirm:

   ```bash
   openshell status        # Status: Connected
   ```

   > `enable_bind_mounts` is what lets flox live-mount the project
   > into the sandbox at its real path. OpenShell documents bind
   > mounts as an isolation tradeoff — that tradeoff is scoped to
   > exactly one directory: the project you're asking the agent to
   > work on. Everything else stays invisible.

### Demo environment

Run once from the dev shell:

```bash
BACKEND=openshell bash demo/setup.sh
```

Same demo env as the OCI walkthrough (git, curl, which, python3,
`flox/claude-code`, an auto-starting web service, seeded `app.py` /
`index.html`) — the only difference is the manifest declares
`backend = "openshell"`.

Then, in your presentation shell:

```bash
alias flox='$FLOX_BIN'                 # the prototype binary
export FLOX_FEATURES_SANDBOX_ACTIVATE=true
export FLOX_FEATURES_AUTO_ACTIVATE=true
export GITHUB_TOKEN=ghp-demo-FAKE      # for the token-isolation beat
export FLOX_VERSION="1.13.1-g$(git -C /path/to/flox rev-parse --short origin/prototype/sandboxed-activation-openshell)"
```

Ensure the prompt hook is in your shell's RC:

```bash
eval "$(flox hook-env --shell bash --shell-pid $$)"
```

> The `FLOX_VERSION` export matters for `target/debug` builds: they
> report a plain release version, which routes the in-VM builder to
> the release tag — and the release builder doesn't know the
> OpenShell compat layer. The `-g<sha>` suffix routes it to the
> frozen builder pin on this branch. (A nix-built prototype flox
> reports a dev version on its own and needs no override.)

**Pre-bake off-camera.** The first bake takes ~5–15 min (the builder
VM cross-compiles the pinned flox rev on first use; later bakes
reuse its store). Do it before the demo so `cd` drops you in in
about a second:

```bash
cd ~/sandbox-demo && FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate -- true
```

The image lands in Docker as `sandbox-demo-openshell:<hash12>` —
content-addressed to the lockfile, so it rebakes only when the
environment actually changes.

---

## 1 · Auto-activate into an OpenShell sandbox

*(verified 2026-07-13)*

**"The manifest declares the sandbox backend and a web service. One
`cd`, one `Y`, and I'm inside an OpenShell-supervised sandbox with my
project mounted and my service already running."**

```bash
cd /tmp && cd ~/sandbox-demo
```

```
Enter '/Users/you/sandbox-demo' (sandboxed via openshell)? [Y/n]
```

Type `Y`. flox creates the sandbox through the OpenShell gateway
(`--no-keep`, so it lives exactly as long as the session) and execs
the environment's activation entrypoint inside it.

```
✔ You are now using the environment 'sandbox-demo'
To stop using this environment, run 'flox deactivate'

flox [sandbox-demo] bash-5.3$ uname -sm
Linux aarch64
flox [sandbox-demo] bash-5.3$ whoami
sandbox
```

**"Notice the prompt: not root. OpenShell runs the workload as an
unprivileged `sandbox` user under a supervisor that owns the network
namespace — flox baked that user into the image as part of the
OpenShell compat layer."**

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
it's the control plane for beat 3):

```bash
openshell sandbox list
NAME                     CREATED              PHASE
flox-sandbox-demo-#####  2026-07-13 ...       Ready
```

**"That's flox's sandbox, visible to OpenShell's control plane —
`openshell logs`, `openshell term`, policy management: NVIDIA's whole
operational surface applies to a flox environment with zero extra
wiring."**

---

## 2 · Prove the boundary is intact

*(verified 2026-07-13)*

**"My filesystem is invisible, my credentials don't cross, and only
my project is live."**

Still inside the guest:

```bash
flox [sandbox-demo] bash-5.3$ ls /Users/you/.ssh
ls: cannot access '/Users/you/.ssh': No such file or directory

flox [sandbox-demo] bash-5.3$ cat /Users/you/demo-secrets/.env
cat: /Users/you/demo-secrets/.env: No such file or directory

flox [sandbox-demo] bash-5.3$ printenv GITHUB_TOKEN
flox [sandbox-demo] bash-5.3$
```

**"And unlike a plain container, the network is deny-by-default at
layer 7:"**

```bash
flox [sandbox-demo] bash-5.3$ curl -sS https://api.github.com/zen
curl: (56) Received HTTP code 403 from proxy after CONNECT
```

**"flox generated an OpenShell policy for this activation — Nix
store read-only, project read-write, zero network. Every outbound
connection goes through OpenShell's proxy and is denied unless a
policy allows it."**

---

## 3 · Hot-reload a network policy — no restart

*(deny-by-default, hot-reload, and the binary-scoped GET verified
2026-07-13; rehearse the write-denial output once off-camera — L7
read-only semantics vary with the endpoint's protocol config)*

**"Here's what OpenShell adds that a bare container can't do: I'm
going to grant this running sandbox read-only GitHub access — without
restarting it, without touching my session. And the grant names the
exact binary allowed to use it."**

First, in the guest, resolve the tool you're granting — in a flox
environment every binary is a content-addressed Nix store path:

```bash
flox [sandbox-demo] bash-5.3$ readlink -f $(command -v curl)
/nix/store/…-curl-8.x.x/bin/curl
```

In the **host terminal** (sandbox name from `openshell sandbox
list`):

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

**"OpenShell enforces per-binary network identity, and flox makes
that precise: a Nix store path pins the policy to the exact build
of curl the environment shipped — not 'anything named curl'. Watch
the verdicts live:"**

In the host terminal:

```bash
openshell logs flox-sandbox-demo-##### --tail
# ... l7_decision=allow GET /zen ...
```

**"Every allow and deny is an audit event. This is the division of
labor: flox defines *what the environment is* — reproducibly, from
the manifest — and OpenShell governs *what it's allowed to do*,
live."**

> For the agent beat next, grant the Anthropic API the same way
> (the claude CLI runs on the env's node — scope the rule to that
> store path, or omit `--binary` scoping per your OpenShell
> version's identity mode):
>
> ```bash
> openshell policy update flox-sandbox-demo-##### \
>   --add-endpoint 'api.anthropic.com:443:full:https' \
>   --add-endpoint 'statsig.anthropic.com:443:full:https' \
>   --binary "$(readlink -f $(command -v node))" --wait
> ```

---

## 4 · Run a coding agent, at full autonomy

*(agent flow identical to demo/SCRIPT.md §3; requires the Anthropic
endpoints granted via `openshell policy update` — see the tip at the
end of beat 3. Rehearse once off-camera.)*

**"A coding agent with no permission prompts, that I don't have to
trust — the sandbox, not the agent, is the boundary. And this time
the agent's network access is an auditable policy, not all-or-
nothing."**

```bash
flox [sandbox-demo] bash-5.3$ claude --permission-mode auto
```

Give it real work:

```
> add a docstring to greet() in app.py and commit the change
```

Claude edits `app.py` and commits — no per-action prompts. If it
tries to reach anywhere outside the policy, the proxy denies it and
the denial shows up in `openshell logs`.

> One-time `claude` login persists in `$CLAUDE_CONFIG_DIR` under the
> project mount, exactly as in the OCI demo. The guest runs as the
> unprivileged `sandbox` user, so
> `claude --dangerously-skip-permissions` also works here if you
> prefer it to auto mode.

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

**"The commit is on my host repo — the project was mounted live at
its real path. The sandbox itself is already gone: flox created it
for the session and OpenShell deleted it on exit. Reproducible
environment in, governed session out, nothing left running."**

---

## 6 · Reset

```bash
bash demo/cleanup.sh
```

Removes the env, fixtures, Docker-side `sandbox-demo-openshell:*`
images, and any lingering demo sandboxes.

> Integration notes for the NVIDIA conversation (image
> requirements, policy compilation, released-vs-docs gaps) live in
> the Forge slice:
> `slices/2026/06-sandboxed-activation-prototype/artifacts/openshell-integration.md`.
