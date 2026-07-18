# Demo: `flox activate --sandbox` — the Cognition (Devin) backend (prototype)

`cd` into a project and hand its baked environment to **Cognition's
Devin runtime**: an agent runtime whose sessions boot from a
*snapshot* built from a YAML *blueprint*. flox bakes the
reproducible closure into an image, then generates the
`.devin/blueprint.yaml` whose `initialize` step reproduces that
closure inside Devin's snapshot — so a Devin session comes up with
the locked toolchain already present. This is the co-sell shape
from Ben Futoriansky's note (2026-05/06): **Flox is the environment
layer under Devin's runtime + sandbox layer.**

**Bold** lines are what to *say*; fenced blocks are what to *type*.
The OCI-backend walkthrough is `demo/SCRIPT.md`; the OpenShell one
is `demo/OPENSHELL.md`; the Modal one is `demo/MODAL.md`; the Ona
one is `demo/ONA.md`. They all share `demo/setup.sh` and
`demo/cleanup.sh`.

**The pitch:** flox already bakes each environment into an OCI
image. Devin does not consume that image directly — it builds a
snapshot from a blueprint (blueprint ≈ Dockerfile, build ≈
`docker build`, snapshot ≈ image, in Devin's own words). So flox
hands Devin a blueprint that *reproduces* the closure: flox brings
the reproducible environment definition, Devin's runtime brings the
supervised, network-governed sandbox its agent runs inside. Same
manifest, one word changed: `backend = "cognition-devin"`.

> **Honesty up front — this backend is Scaffolded, not
> Implemented.** Devin is a subscription product reached over its
> API: nothing runs on the laptop, and there is **no public
> sandbox/runtime-launch API** that ingests an arbitrary image.
> Devin's builder produces the snapshot from a blueprint, and
> driving that flow needs (1) a **Devin subscription** and, per the
> co-sell note, a **partnership with Cognition's Sandbox/Infra
> team**, and (2) a **registry Devin's build can pull from** for
> the baked substrate image. This host has neither, and no `devin`
> CLI. So flox runs the honest *local* slice — preflight, bake,
> policy compilation, blueprint generation — and stops at the
> launch boundary with a clear error naming both gaps. Beats 2–6
> describe what a completed snapshot build + session looks like.

---

## 0 · Setup

### One-time host prerequisites

1. **Docker Desktop** (or Docker Engine ≥ 28) running — the image
   is baked into the local Docker store as the reproducible
   substrate. This is the one genuinely required host tool.
2. **The presentation shell** — one command, in your presentation
   shell (export `FLOX_BIN` from the dev shell first):

   ```bash
   flox activate -r djsauble/cognition-devin-setup
   ```

   This is the demo's *outer layer* — one setup env per sandbox
   backend is the plan. Like the Ona env, it runs **no local
   service** and installs **no provider CLI**: Devin is cloud-only
   and its CLI is presence-detected, not required. It configures
   the shell (feature flags and the planted `GITHUB_TOKEN` in
   `[vars]`, `FLOX_VERSION` plus a `flox` alias from `$FLOX_BIN` in
   `[profile]`), plants the `~/demo-secrets` fixture, and prints a
   note for each hand-off prerequisite that is missing.
   Deactivating removes the planted secret (`[profile.deactivate]`).
   Stay in this activation for the whole demo.

   > Details, caveats, and troubleshooting:
   > `demo/cognition-devin-setup/README.md`.

3. **A Devin subscription + partnership** (**account beat** —
   required for the snapshot build + session, beats 2+). Devin
   builds a snapshot from a blueprint through its own builder;
   there is no public image-launch API, so a co-sell with
   Cognition's Sandbox/Infra team is the path to a backend-grade
   integration.

4. **A registry Devin's build can pull from** (**registry beat** —
   required for the reproducible substrate). Point flox at it so
   the generated blueprint references the right ref:

   ```bash
   export FLOX_SANDBOX_COGNITION_DEVIN_REGISTRY=docker.io/<your-user>
   ```

### Demo environment

Run once from the dev shell:

```bash
BACKEND=cognition-devin bash demo/setup.sh
```

Same demo env as the other walkthroughs (git, curl, which,
python3, `flox/claude-code`, an auto-starting web service, seeded
`app.py` / `index.html`); the manifest declares
`backend = "cognition-devin"` plus network grants for the agent's
API endpoints:

```toml
[[options.sandbox.network]]
endpoint = "api.anthropic.com:443"
binary   = "claude-code/.claude-wrapped"
# plus an identical grant for statsig.anthropic.com (agent telemetry)
```

flox compiles the **host** of each `:443` grant into the
blueprint's `sandbox.allowed_domains` allowlist. That is Devin's
own CLI-sandbox vocabulary: a loopback egress proxy where only
allowlisted domains pass, and `denied_domains` always blocks. Devin
filters **per-domain, not per-port**, so the `:443` is dropped for
the domains it does allow; the `binary`, `access`, and `protocol`
fields are recorded as comments but **do not** constrain traffic
through the blueprint contract — a declared lossiness, honest about
what the hand-off can express. A grant on any port other than 443
is rejected at compile time rather than silently widened.

The setup env already configured your shell — make sure the prompt
hook is in your shell's RC:

```bash
eval "$(flox hook-env --shell bash --shell-pid $$)"
```

**Pre-bake off-camera.** The first bake takes ~5–15 min on a
machine that compiles the pinned flox rev in-VM, or ~2–5 min if the
pin is cached. Later bakes reuse layers:

```bash
cd ~/sandbox-demo && FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate -- true
```

The image lands in Docker as
`sandbox-demo-cognition-devin:<hash12>` (the Devin backend reuses
the openshell compat-layer bake), content-addressed to the
lockfile — it rebakes only when the environment actually changes.

---

## 1 · Auto-activate toward a Devin snapshot

**"One `cd`, one `Y`, and flox bakes the image, compiles the
policy, and generates the Devin blueprint the snapshot builds
from."**

```bash
cd /tmp && cd ~/sandbox-demo
```

```
Enter '/Users/you/sandbox-demo' (sandboxed via cognition-devin)? [Y/n]
```

Type `Y`. flox baked (or reused) the image, then generated the
blueprint hand-off.

**Without a subscription or registry** (this host), flox stops at
the launch boundary and tells you precisely what is missing:

```
The 'cognition-devin' sandbox backend hands the baked environment
off to Cognition's Devin runtime, which requires prerequisites this
host cannot satisfy automatically:
  1. Push the baked image
     'sandbox-demo-cognition-devin:<hash12>' to a registry Devin's
     build can pull (set
     FLOX_SANDBOX_COGNITION_DEVIN_REGISTRY=<registry-prefix> and
     re-run, then push
     '<prefix>/sandbox-demo-cognition-devin:<hash12>').
  2. A Devin subscription and a partnership: no 'devin' CLI was
     found on PATH, and Devin builds a snapshot from the committed
     blueprint through its own builder — there is no public
     image-launch API. A co-sell with Cognition's Sandbox/Infra
     team is the path to a backend-grade integration.
flox generated the Devin blueprint hand-off at:
  /Users/you/sandbox-demo/.devin/blueprint.yaml
Commit it as '.devin/blueprint.yaml' on the repo's default branch,
push the image, then sync + build the snapshot through Devin.
```

**"That is not a failure — that is the honest edge of what a laptop
can do for a partner runtime with no public launch API. flox did
everything local: baked the image, compiled the deny-by-default
policy, and wrote the exact blueprint Devin builds a snapshot from.
The two missing pieces are Cognition's, not flox's — and closing
them is exactly the co-sell conversation."**

Look at what flox generated:

```bash
cat ~/sandbox-demo/.devin/blueprint.yaml
```

```yaml
# Generated by `flox activate --sandbox --sandbox-backend cognition-devin`.
# ... blueprint contract: Devin builds a snapshot from this file's
# initialize step; blueprint ~= Dockerfile, snapshot ~= image ...
#   policy: allowed: api.anthropic.com, statsig.anthropic.com
initialize: |
  # Install Flox and activate the locked environment so the snapshot
  # boots with the same closure flox baked into the image.
  curl -fsSL https://downloads.flox.dev/by-env/stable/deb/flox.x86_64-linux.deb -o /tmp/flox.deb
  sudo dpkg -i /tmp/flox.deb || sudo apt-get -f install -y
  flox activate -- true

sandbox:
  allowed_domains: ["api.anthropic.com", "statsig.anthropic.com"]
  denied_domains: []
  network_mode: "full"
```

**"The manifest's `:443` grants became the blueprint's
`allowed_domains` allowlist, deny-by-default. That is Devin's own
sandbox vocabulary — flox authored it from the environment's own
declared network needs, nothing more. And the `initialize` step is
the real inversion: flox doesn't hand Devin an image, it hands Devin
the *recipe* to reproduce the locked closure inside Devin's
snapshot."**

---

## 2 · Push the image and build the snapshot (account + registry beat)

**"With a Devin subscription and a registry, this is the whole
remaining path."** On a credentialed operator's machine, with
`FLOX_SANDBOX_COGNITION_DEVIN_REGISTRY` set:

```bash
# Tag the local bake as the devin-namespaced registry ref and push:
docker tag sandbox-demo-cognition-devin:<hash12> \
  "$FLOX_SANDBOX_COGNITION_DEVIN_REGISTRY/sandbox-demo-cognition-devin:<hash12>"
docker push \
  "$FLOX_SANDBOX_COGNITION_DEVIN_REGISTRY/sandbox-demo-cognition-devin:<hash12>"

# Commit the generated blueprint to the repo Devin syncs, then sync
# + build the snapshot through Devin's API or UI.
git -C ~/sandbox-demo add .devin/blueprint.yaml
git -C ~/sandbox-demo commit -m "add flox-generated Devin blueprint"
git -C ~/sandbox-demo push
```

Devin syncs `.devin/blueprint.yaml` from the default branch, runs
its `initialize` step to build the snapshot, and every session
boots from it — the locked toolchain is present the moment the
session comes up.

> This beat requires a live Devin subscription, a partnership, and a
> reachable registry, none of which this host has tonight. The
> generated blueprint is exactly what Devin builds from; nothing is
> faked.

---

## 3 · The session boots with the locked toolchain — contrast a
hand-provisioned one

**"The whole pitch: the toolchain is present on boot. No
`apt install`, no version drift, no 'works on my machine' — the
blueprint reproduces the locked closure, content-addressed to the
lockfile."**

Inside a Devin session (account + registry beat), the locked tools
are already there:

```bash
which python3 curl git       # all present, at the locked versions
flox list                    # the baked closure, exactly as declared
```

Contrast a hand-authored Devin blueprint (ad-hoc `initialize`
commands with no Flox): the developer installs each tool by hand,
pins nothing, and the next snapshot build drifts. **"That drift is
exactly what the Flox layer removes from Devin's runtime: the
reproducible closure is what every snapshot boots from."**

---

## 4 · Prove the boundary — deny-by-default egress

**"Egress is deny-by-default. Only the manifest's `:443` domains
are in the `allowed_domains` allowlist flox compiled into the
blueprint, which Devin's CLI-sandbox loopback proxy enforces."**

Inside the session (account + registry beat), a granted endpoint
works:

```bash
curl -sS https://api.anthropic.com/  # allowed: in allowed_domains
```

An ungranted endpoint is blocked by Devin's sandbox proxy:

```bash
curl -sS https://api.github.com/zen
# blocked — api.github.com is not in allowed_domains
```

**"Devin's runtime governs egress; flox authored the allowlist from
the environment's own declared network needs. The threat model
inverts versus the local backends: the host filesystem is
unreachable from Devin's runtime (there is no bind mount of your
laptop), but the code and any injected secrets run in Devin's
cloud. That is the honest tradeoff of a partner runtime, and flox
states it in the backend capabilities."**

---

## 5 · Policy is fixed at snapshot build — redemption is rebuild

**"Devin fixes the snapshot's policy when it builds from the
blueprint. There is no live 'ask' — to widen egress, you edit the
manifest, regenerate the blueprint, and rebuild the snapshot."**

Grant a new domain by editing the manifest and re-activating:

```toml
[[options.sandbox.network]]
endpoint = "api.github.com:443"
```

```bash
flox edit                       # add the grant
flox deactivate && cd ~/sandbox-demo   # regenerate the blueprint
```

flox recompiles the allowlist and rewrites `.devin/blueprint.yaml`;
committing, syncing, and rebuilding the snapshot (with account +
registry) yields one that allows `api.github.com`.
**"Rebuild-as-redemption — the common path for control-plane
sandbox providers, and honest about it: no live verdict, a fresh
snapshot built from the new blueprint."**

---

## 6 · Run a coding agent, at full autonomy (account + registry beat)

**"A coding agent with no permission prompts, running in Devin's
runtime — the sandbox, not the agent, is the boundary, and the
boundary is a governed runtime with its own egress proxy."**

The manifest already grants the agent's Anthropic endpoints, so
inside a Devin session:

```bash
claude --permission-mode auto
```

```
> add a docstring to greet() in app.py and commit the change
```

Claude's API traffic to `api.anthropic.com` is allowed; anything it
reaches for outside the allowlist is blocked by Devin's sandbox
proxy. Because the runtime is remote and governed, the blast radius
of anything the agent does is a cloud sandbox the operator
controls.

> Agent auth (`CLAUDE_CODE_OAUTH_TOKEN`) must be injected through
> Devin's own secret mechanism (blueprint secrets / env-var
> material), not your laptop's `.env` — the remote runtime has no
> access to it. This is the credential-leaves-the-laptop tradeoff
> the inverted threat model names.

---

## 7 · Exit — the runtime is remote and governed

With account + registry, the Devin session is stopped or discarded
through Devin's control plane when you are done; nothing ran on your
laptop.

On this host tonight, there is nothing to tear down — no snapshot
was built and no session opened. The only local artifacts are the
baked image and the generated `.devin/blueprint.yaml`, both removed
by cleanup.

---

## 8 · Reset

```bash
bash demo/cleanup.sh
```

Removes the demo env, fixtures, the generated
`.devin/blueprint.yaml`, and the Docker-side
`sandbox-demo-cognition-devin:*` images. (Any snapshots or sessions
in Devin are governed through its control plane and are yours to
stop or delete; images pushed to your registry are yours to prune.)

> Integration notes for the Cognition conversation (blueprint —
> not devcontainer — hand-off, the `allowed_domains` CLI-sandbox
> egress vocabulary, no public image-launch API so the motion is
> the partner consuming the artifact, the inverted threat model):
> the backend module docs at
> `cli/flox/src/commands/sandbox_backends/cognition_devin.rs` and
> the backend contract at
> `slices/2026/06-sandboxed-activation-prototype/artifacts/backend-contract.md`.
