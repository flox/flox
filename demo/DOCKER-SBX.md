# Demo: `flox activate --sandbox` — the Docker Sandboxes backend (prototype)

`cd` into a project and hand its baked environment to **Docker
Sandboxes** (`sbx`): the reproducible closure runs inside a local
Linux microVM with its own dockerd, its own filesystem, and a
host-side egress proxy that enforces the network policy flox
compiled from the manifest — before the microVM ever starts.

**Bold** lines are what to *say*; fenced blocks are what to *type*.
The OCI-backend walkthrough is `demo/SCRIPT.md`; the OpenShell one is
`demo/OPENSHELL.md`; the Modal one is `demo/MODAL.md`. They all share
`demo/setup.sh` and `demo/cleanup.sh`.

**The pitch:** flox already bakes each environment into an OCI
image. This backend hands that image to Docker's `sbx` as a kit's
base image — flox brings the reproducible environment, Docker
Sandboxes brings a microVM boundary with domain-level egress policy.
Same manifest, one word changed: `backend = "docker-sbx"`.

> **Honesty up front — this backend is Scaffolded, not
> Implemented.** The microVM launch (`sbx run`) needs two things
> this prototype cannot produce on its own: (1) the `sbx` CLI, signed
> in with `sbx login` (browser OAuth; the free CLI tier suffices —
> only org governance is paid), and, on hosts using the bundled
> `docker sbx` subcommand, **Docker Desktop 4.60+**; and (2) a base
> image that satisfies sbx's kit contract (a non-root `agent` user at
> uid 1000, passwordless sudo, `/home/agent`, preserved HTTP proxy
> env). The flox bake adds a `sandbox` user, not sbx's `agent` user,
> so the image must be adapted first. This walkthrough runs the honest
> local slice — preflight, bake, policy compilation, kit generation —
> and stops at the launch boundary with a clear error naming both
> gaps. Beats 4–6 describe what a completed launch looks like.

---

## 0 · Setup

### One-time host prerequisites

1. **Docker daemon** running (Docker Desktop or the Docker service).
   Only for baking and loading the image — the `sbx` microVM runs on
   its own hypervisor.
2. **The `sbx` CLI + presentation shell** — one command, in your
   presentation shell (export `FLOX_BIN` from the dev shell first):

   ```bash
   flox activate -r djsauble/docker-sbx-setup
   ```

   This is the demo's *outer layer* — one setup env per sandbox
   backend is the plan. It installs `docker-sbx` (the `sbx` CLI),
   configures the shell (feature flags and the planted `GITHUB_TOKEN`
   in `[vars]`, `FLOX_VERSION` plus a `flox` alias from `$FLOX_BIN`
   in `[profile]`), plants the `~/demo-secrets` fixture, and prints a
   note for each launch prerequisite that is missing. Deactivating
   removes the planted secret (`[profile.deactivate]`). Stay in this
   activation for the whole demo; then sign in once:

   ```bash
   sbx login          # browser OAuth; picks a default network policy
   ```

   `sbx login` prompts for a default network preset (Open /
   Balanced / Locked Down). **Locked Down** matches the
   deny-by-default story below; pick it, or `sbx policy init
   deny-all` non-interactively.

   > Details, caveats, and troubleshooting:
   > `demo/docker-sbx-setup/README.md`.

### Demo environment

Run once from the dev shell:

```bash
BACKEND=docker-sbx bash demo/setup.sh
```

Same demo env as the other walkthroughs (git, curl, which, python3,
`flox/claude-code`, an auto-starting web service, seeded `app.py` /
`index.html`); the manifest declares `backend = "docker-sbx"` plus
network grants for the agent's API endpoints:

```toml
[[options.sandbox.network]]
endpoint = "api.anthropic.com:443"
binary   = "claude-code/.claude-wrapped"
# plus an identical grant for statsig.anthropic.com (agent telemetry)
```

flox compiles these into the `sbx` kit's `network.allowedDomains` at
kit generation. Docker Sandboxes governs egress by **domain over
HTTP/HTTPS**, so each `:443` host becomes an allow entry; the
`binary`/`access`/`protocol` scoping is recorded as a comment but is
**not enforceable** on sbx — a declared lossiness (contrast
OpenShell, which enforces per-binary at L7). Everything else stays
deny-by-default.

The setup env already configured your shell — just make sure the
prompt hook is in your shell's RC:

```bash
eval "$(flox hook-env --shell bash --shell-pid $$)"
```

**Pre-bake off-camera.** The first bake compiles the pinned flox rev
in a builder VM (~5–15 min cold, ~2–5 min if the pin is cached).
Later bakes reuse layers:

```bash
cd ~/sandbox-demo && FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate -- true
```

The image lands in Docker as `sandbox-demo-docker-sbx:<hash12>`,
content-addressed to the lockfile — it rebakes only when the
environment actually changes, and never collides with the `oci`,
`openshell`, or `modal` backends' tags.

---

## 1 · Auto-activate — flox bakes, compiles policy, and hands off

**"One `cd`, and flox bakes the environment, compiles the manifest's
network grants into a Docker Sandboxes kit, and prepares the microVM
launch."**

```bash
cd /tmp && cd ~/sandbox-demo
```

```
Enter '/Users/you/sandbox-demo' (sandboxed via docker-sbx)? [Y/n]
```

Type `Y`. flox runs preflight (`sbx` on PATH, version ≥ 0.32.0,
Docker daemon reachable), bakes the image under
`sandbox-demo-docker-sbx:<hash12>` if needed, compiles the policy,
and generates the kit manifest.

On this prototype it then **stops at the launch boundary** with an
honest error — the microVM launch needs the two prerequisites the
top of this doc names:

```
❌ ERROR: The 'docker-sbx' sandbox backend launches a local Docker
Sandboxes microVM, which requires two prerequisites this host cannot
satisfy automatically:
  1. The 'sbx' CLI (install with 'brew install docker/tap/sbx' and
     run 'sbx login'); the bundled 'docker sbx' subcommand instead
     needs Docker Desktop 4.60 or newer.
  2. A base image that satisfies sbx's kit contract (a non-root
     'agent' user at uid 1000 with passwordless sudo, a /home/agent
     home, and preserved HTTP proxy env). The flox bake adds a
     'sandbox' user, not sbx's 'agent' user, so the baked image
     'sandbox-demo-docker-sbx:<hash12>' must be adapted first (build
     on 'docker/sandbox-templates:shell-docker').
flox generated the kit manifest at:
  /Users/you/sandbox-demo/.flox/cache/docker-sbx-kit/spec.yaml
With 'sbx' installed and the image adapted, load it with 'sbx kit
load .../docker-sbx-kit' and run it with 'sbx run --kit flox-sandbox-demo'.
```

**"That's the deepest honest slice: flox baked the real image,
compiled the real policy, and generated the real kit manifest. The
only thing it won't do is fake a launch it can't complete."**

Show the generated kit:

```bash
cat ~/sandbox-demo/.flox/cache/docker-sbx-kit/spec.yaml
```

```yaml
schemaVersion: "1"
kind: sandbox
name: flox-sandbox-demo
displayName: flox-sandbox-demo
description: Flox environment sandbox baked by flox activate --sandbox.
network:
  allowedDomains:
    - 'api.anthropic.com'
    - 'statsig.anthropic.com'
sandbox:
  image: 'sandbox-demo-docker-sbx:<hash12>'
```

**"The manifest's `:443` grants are already compiled into
`allowedDomains`. The base image points at the flox bake. An operator
adapts that image once, and `sbx run --kit` launches straight into
the microVM."**

---

## 2 · Prove the policy is deny-by-default

**"Even before a launch, the compiled policy tells the whole egress
story — deny-by-default, with only the manifest's grants opened."**

`sbx` can evaluate a policy without starting a sandbox. After `sbx
login` (Locked Down preset), check what the deny-by-default posture
blocks and what the grants allow:

```bash
sbx policy check network api.github.com
# Denied: api.github.com          ← no grant, blocked

sbx policy check network api.anthropic.com
# (after `sbx policy allow network api.anthropic.com`, or once the
#  kit's allowedDomains are loaded) Allowed: api.anthropic.com
```

**"api.github.com has no rule, so it's denied. The Anthropic
endpoints the manifest granted are the only outbound the agent gets.
Docker Sandboxes enforces that at the host-side HTTP/HTTPS proxy —
every request the microVM makes goes through it."**

> **Lossiness to call out honestly:** sbx's allowlist is
> domain-scoped over HTTP/HTTPS only. It carries no per-binary
> identity and no read/write method distinction — so unlike the
> OpenShell beat, you cannot say "only *this* curl binary may reach
> the endpoint." Non-HTTP TCP needs an explicit IP:port rule (`sbx
> policy allow network "10.1.2.3:22"`); UDP and ICMP are blocked at
> the network layer and cannot be unblocked. flox *declines* a
> non-80/443 grant at compile time rather than silently widening it.

---

## 3 · The isolation story (what the microVM boundary buys)

**"When the launch does run, the boundary is a microVM — stronger
than a shared-kernel container."**

Describe (don't demo, on this host) what `sbx run --kit
flox-sandbox-demo` produces:

- The agent gets its **own Linux filesystem, its own dockerd, and its
  own network** inside the microVM. Packages it installs and images
  it pulls stay inside and vanish on `sbx rm`.
- Only the **project workspace** is shared into the microVM
  (read-write by default), so `~/demo-secrets` and `$GITHUB_TOKEN`
  simply do not exist inside it — the same "a secret the agent cannot
  even see" story as the other backends, enforced by the microVM
  boundary rather than a bind-mount allowlist.
- Credentials are injected as **sentinel values**: the host-side
  proxy overwrites the auth header on outbound requests, so the
  microVM sees a placeholder (`proxy-managed`), never the real key.
  Agent secrets go through `sbx secret set`, not a baked-in `.env`.

```bash
# On a fully-provisioned host, this is the launch:
sbx kit load ~/sandbox-demo/.flox/cache/docker-sbx-kit
sbx run --kit flox-sandbox-demo
```

---

## 4 · Run a coding agent (fully-provisioned host)

**"With the image adapted and `sbx` signed in, a coding agent runs at
full autonomy inside the microVM — the sandbox, not the agent, is the
boundary."**

Docker Sandboxes is built for exactly this: `sbx run claude` launches
Claude Code inside the microVM. In the flox flow, the kit's base
image *is* the flox environment, so the agent runs with the project's
whole reproducible toolbox:

```bash
sbx run --kit flox-sandbox-demo claude
```

Authenticate with `sbx secret set` (API key) or the agent's OAuth
(`/login` inside the sandbox — the token stays on the host). Give it
real work; anything it tries outside the compiled `allowedDomains` is
denied at the proxy. This beat is **described, not run** tonight: the
base image isn't adapted on this host.

---

## 5 · Exit — the work persists, the sandbox is disposable

On a fully-provisioned host:

```bash
sbx stop my-sandbox      # pause; installed packages and state persist
sbx rm my-sandbox        # discard the microVM entirely
```

**"The workspace edits are on your host repo — the project was
shared live. Stop keeps the microVM's state for next time; `rm`
throws it all away. State-lost-on-remove is the reinstall gap flox
closes: the environment is content-addressed, so re-baking and
re-launching reproduces it exactly."**

---

## 6 · Reset

Deactivate the setup layer (its `profile.deactivate` removes the
planted secret), then:

```bash
bash demo/cleanup.sh
```

Removes the env, fixtures, and the Docker-side
`sandbox-demo-docker-sbx:*` images. On a host where you launched real
sandboxes, also clear them:

```bash
sbx ls
sbx rm --force <name>     # for any lingering flox-sandbox-demo microVM
```

---

> Integration notes for the Docker conversation (kit base-image
> contract, policy vocabulary, `--version` vs `version` surface, and
> the released-vs-docs gaps): the `docker-sbx` backend module docs in
> `cli/flox/src/commands/sandbox_backends/docker_sbx.rs`, and the
> slice artifacts under
> `slices/2026/06-sandboxed-activation-prototype/`.
