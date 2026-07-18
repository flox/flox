# Demo: `flox activate --sandbox` — the E2B backend (prototype)

`cd` into a project and hand its baked environment to **E2B**: a
cloud-API sandbox provider. flox bakes the reproducible closure
into an image, generates the E2B template hand-off — an
`e2b.Dockerfile` whose `FROM` is that image plus an `e2b.toml`
template config — and `e2b template build` turns it into a sandbox
template a sandbox boots from with the locked toolchain already
present.

**Bold** lines are what to *say*; fenced blocks are what to *type*.
The OCI-backend walkthrough is `demo/SCRIPT.md`; the OpenShell one
is `demo/OPENSHELL.md`; the Modal one is `demo/MODAL.md`; the
Docker Sandboxes one is `demo/DOCKER-SBX.md`; the Ona one is
`demo/ONA.md`. They all share `demo/setup.sh` and
`demo/cleanup.sh`.

**The pitch:** flox already bakes each environment into an OCI
image. This backend hands that image to E2B as a template base —
flox brings the reproducible environment, E2B brings a fast,
governed cloud sandbox with a **live** network-policy update. Same
manifest, one word changed: `backend = "e2b"`.

> **Honesty up front — this backend is Scaffolded, not
> Implemented.** E2B is a cloud-API provider: nothing runs on the
> laptop, and launching a sandbox needs (1) an **E2B account and
> API key** (free tier with $100 credit; `E2B_API_KEY` /
> `e2b auth login`) and (2) a **template built from the baked
> image** (`e2b template build` reads an `e2b.Dockerfile` whose
> `FROM` is the image, so the image must be pushed to a registry
> E2B's builder can pull). This host has no `e2b` CLI and no API
> key, so flox runs the honest *local* slice — preflight, bake,
> policy compilation, template generation — and stops at the launch
> boundary with a clear error naming both gaps. Beats 2–6 describe
> what a completed sandbox launch looks like.

---

## 0 · Setup

### One-time host prerequisites

1. **Docker Desktop** (or Docker Engine ≥ 28) running — the image
   is baked into the local Docker store before it is pushed. This
   is one of the two genuinely required host tools.
2. **The E2B CLI** — `e2b` on PATH. The `@e2b/cli` package is not
   in the Flox Catalog (only the Python SDK is), so the setup env
   below installs `nodejs` to provide `npm` and you run `npm
   install -g @e2b/cli`. It builds the template from the generated
   Dockerfile.
3. **The presentation shell** — one command, in your presentation
   shell (export `FLOX_BIN` from the dev shell first):

   ```bash
   flox activate -r djsauble/e2b-setup
   ```

   This is the demo's *outer layer* — one setup env per sandbox
   backend is the plan. It installs `nodejs` (so `npm install -g
   @e2b/cli` works) and runs **no local service** (E2B is
   cloud-only). It configures the shell
   (feature flags and the planted `GITHUB_TOKEN` in `[vars]`,
   `FLOX_VERSION` plus a `flox` alias from `$FLOX_BIN` in
   `[profile]`), plants the `~/demo-secrets` fixture, and prints a
   note for each launch prerequisite that is missing. Deactivating
   removes the planted secret (`[profile.deactivate]`). Stay in this
   activation for the whole demo.

   > Details, caveats, and troubleshooting:
   > `demo/e2b-setup/README.md`.

4. **An E2B account + API key** (**account beat** — required for the
   template build and sandbox launch, beats 2+). Sign in with
   `e2b auth login` (browser OAuth), or export the key from the
   dashboard:

   ```bash
   export E2B_API_KEY=e2b_<your-key>
   ```

5. **A registry E2B's builder can pull from** (**registry beat** —
   required for the template build). Point flox at it so the
   generated Dockerfile's `FROM` references the right ref:

   ```bash
   export FLOX_SANDBOX_E2B_REGISTRY=docker.io/<your-user>
   ```

### Demo environment

Run once from the dev shell:

```bash
BACKEND=e2b bash demo/setup.sh
```

Same demo env as the other walkthroughs (git, curl, which,
python3, `flox/claude-code`, an auto-starting web service, seeded
`app.py` / `index.html`); the manifest declares `backend = "e2b"`
plus network grants for the agent's API endpoints:

```toml
[[options.sandbox.network]]
endpoint = "api.anthropic.com:443"
binary   = "claude-code/.claude-wrapped"
# plus an identical grant for statsig.anthropic.com (agent telemetry)
```

flox compiles the **host** of each `:443`/`:80` grant into the
`e2b.toml` `allowed_hosts` allowlist — and, crucially, forces
`allow_internet_access = false`. **E2B's own default is
`allowInternetAccess = true` (default-OPEN)**, so flox always
writes the explicit deny posture and layers the allowlist on top;
the absence of a grant is not enough to close egress on E2B the way
it is on the other backends. E2B filters by host/SNI on ports 80
and 443 only and does **not** filter QUIC/UDP — a declared
lossiness, honest about what the hand-off enforces. The `binary`,
`access`, and `protocol` fields are recorded as metadata but do not
constrain traffic. A grant on any port other than 80/443 is
rejected at compile time rather than silently widened.

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

The image lands in Docker as `sandbox-demo-e2b:<hash12>`
(the E2B backend reuses the openshell compat-layer bake),
content-addressed to the lockfile — it rebakes only when the
environment actually changes.

---

## 1 · Auto-activate toward an E2B sandbox

**"One `cd`, one `Y`, and flox bakes the image, compiles the
deny-by-default policy, and generates the E2B template hand-off."**

```bash
cd /tmp && cd ~/sandbox-demo
```

```
Enter '/Users/you/sandbox-demo' (sandboxed via e2b)? [Y/n]
```

Type `Y`. flox baked (or reused) the image, then generated the
template artifacts.

**Without an account or registry** (this host), flox stops at the
launch boundary and tells you precisely what is missing:

```
The 'e2b' sandbox backend launches a remote E2B sandbox, which
requires two prerequisites this host cannot satisfy automatically:
  1. Push the baked image 'sandbox-demo-e2b:<hash12>' to a registry
     E2B's builder can pull (set FLOX_SANDBOX_E2B_REGISTRY=<prefix>
     and re-run, then push '<prefix>/sandbox-demo-e2b:<hash12>'),
     then build the template with 'e2b template build'.
  2. An E2B account and API key (preflight confirmed the CLI; the
     template build and sandbox launch call the E2B API).
flox generated the E2B template hand-off at:
  /Users/you/sandbox-demo/e2b.Dockerfile
  /Users/you/sandbox-demo/e2b.toml
With the image pushed and E2B authenticated, run 'e2b template
build' in that directory, then launch a sandbox from the template.
```

**"That is not a failure — that is the honest edge of what a laptop
can do for a cloud sandbox provider. flox did everything local:
baked the image, compiled the deny-by-default policy, and wrote the
exact template E2B builds a sandbox from. The two missing pieces are
E2B's, not flox's."**

Look at what flox generated:

```bash
cat ~/sandbox-demo/e2b.Dockerfile
cat ~/sandbox-demo/e2b.toml
```

```dockerfile
# syntax=docker/dockerfile:1
# Generated by `flox activate --sandbox --sandbox-backend e2b`.
# ... push the image, then run `e2b template build` in this directory.
FROM docker.io/<you>/sandbox-demo-e2b:<hash12>
```

```toml
# Generated by `flox activate --sandbox --sandbox-backend e2b`.
# ... E2B's own default is allow_internet_access = true (default-OPEN),
# so flox forces it to false below ...
#   policy: deny-by-default; 80/443 SNI allowed: api.anthropic.com, statsig.anthropic.com
template_name = "flox-sandbox-demo"
dockerfile = "e2b.Dockerfile"
start_cmd = "/bin/sh -lc 'exec \"$@\"' _"

[network]
allow_internet_access = false
allowed_hosts = ["api.anthropic.com", "statsig.anthropic.com"]
```

**"The manifest's `:443` grants became `allowed_hosts` — and note
`allow_internet_access = false`. E2B defaults to open; flox always
writes the closed posture and adds only what the manifest grants.
That is the load-bearing move for a default-open provider: flox
never inherits E2B's open default, it overrides it."**

---

## 2 · Push the image, build the template, launch the sandbox (account + registry beat)

**"With an E2B account and a registry, this is the whole remaining
path."** On a credentialed operator's machine, with `E2B_API_KEY`
and `FLOX_SANDBOX_E2B_REGISTRY` set:

```bash
# Tag the local bake as the e2b-namespaced registry ref and push:
docker tag sandbox-demo-e2b:<hash12> \
  "$FLOX_SANDBOX_E2B_REGISTRY/sandbox-demo-e2b:<hash12>"
docker push "$FLOX_SANDBOX_E2B_REGISTRY/sandbox-demo-e2b:<hash12>"

# Build the template from the generated Dockerfile, then start a sandbox:
cd ~/sandbox-demo
e2b template build            # reads e2b.Dockerfile + e2b.toml
e2b sandbox spawn flox-sandbox-demo
```

E2B pulls the image the Dockerfile references, builds the template,
and boots a sandbox — the locked toolchain is present the moment it
comes up.

> This beat requires a live E2B account and a reachable registry,
> neither of which this host has tonight. The generated template is
> exactly what E2B builds from; nothing is faked.

---

## 3 · The sandbox boots with the locked toolchain

**"The whole pitch: the toolchain is present on boot. No
`apt install`, no version drift, no 'works on my machine' — the
image is the environment, content-addressed to the lockfile."**

Inside the E2B sandbox (account + registry beat), the locked tools
are already there:

```bash
which python3 curl git       # all present, at the locked versions
flox list                    # the baked closure, exactly as declared
```

**"That reproducible closure is the base image every E2B sandbox
opens from — the same guarantee across every teammate and every
run."**

---

## 4 · Prove the boundary — deny-by-default over a default-open provider

**"Egress is deny-by-default — which on E2B means flox actively
*closed* what E2B leaves open. Only the manifest's `:443` hosts are
in the allowlist flox wrote into `e2b.toml`."**

Inside the sandbox (account + registry beat), a granted endpoint
works and an ungranted one is blocked:

```bash
curl -sS https://api.anthropic.com/    # allowed: in allowed_hosts
curl -sS https://api.github.com/zen    # blocked: not in allowed_hosts
```

**"The threat model inverts versus the local backends: the host
filesystem is unreachable from the E2B sandbox (there is no bind
mount of your laptop), but the code and any injected secrets run in
E2B's cloud. And the honesty flox states in its capabilities: E2B
filters host/SNI on 80/443 only and does not filter QUIC/UDP — so
the allowlist governs HTTP(S), not every protocol. That is the
declared lossiness, not a hidden gap."**

In the **host terminal**, the planted secret is visible; inside the
sandbox it simply does not exist (the host `$HOME` is not mounted):

```bash
ls -a ~/demo-secrets/    # host: .env present
```

```bash
# inside the sandbox:
ls -a /Users/you/demo-secrets/   # No such file or directory
printenv GITHUB_TOKEN            # empty — host env does not cross
```

---

## 5 · Redeem a grant **live** — E2B's `updateNetwork`

> **This is E2B's differentiator.** Modal and Ona fix the policy at
> sandbox/workspace creation — widening egress there means
> recreating the sandbox. E2B exposes `updateNetwork` on a
> **running** sandbox: a replace-not-merge update of the allowlist,
> no restart. It is the one true live network-grant redemption in
> the cloud tier.

**"I'm going to grant this running sandbox GitHub access — no
restart, no recreate."** Add the grant to the manifest, recompile
the allowlist, and apply it live:

```toml
[[options.sandbox.network]]
endpoint = "api.github.com:443"
```

```bash
flox edit                     # add the grant
# flox recompiles allowed_hosts and rewrites e2b.toml; apply live:
e2b sandbox update-network <sandbox-id> \
  --allow api.anthropic.com,statsig.anthropic.com,api.github.com
```

Back in the same sandbox — nothing restarted:

```bash
curl -sS https://api.github.com/zen
# Practicality beats purity.        ← now allowed, live
```

**"That is replace-not-merge: the new allowlist *is* the full set,
so flox recompiles the whole list from the manifest — it never
silently keeps a stale grant. Live redemption without recreation is
what sets E2B apart from the other cloud backends; the capabilities
row still reads `live-ask: no`, because this is an operator-driven
policy *replacement*, not a per-request prompt. flox states that
distinction honestly."**

> Why `live-ask: no` despite the live update: the contract's
> `live-ask` means adjudicating a *specific out-of-policy access
> mid-flight* (a prompt). E2B has no per-request ask API;
> `updateNetwork` replaces the policy on the operator's initiative.
> Real, live, and honestly not the same thing.

---

## 6 · Run a coding agent, at full autonomy (account + registry beat)

**"A coding agent with no permission prompts, running in E2B's
cloud — the sandbox, not the agent, is the boundary."**

The manifest already grants the agent's Anthropic endpoints, so
inside the sandbox:

```bash
claude --permission-mode auto
```

```
> add a docstring to greet() in app.py and commit the change
```

Claude's API traffic to `api.anthropic.com` is allowed; anything it
reaches for outside `allowed_hosts` is blocked by E2B's network
policy. Because the sandbox is remote and governed, the blast radius
of anything the agent does is a cloud sandbox the operator controls.

> Agent auth (`CLAUDE_CODE_OAUTH_TOKEN`) must be injected through
> E2B's own secret mechanism, not your laptop's `.env` — the remote
> sandbox has no access to it. This is the credential-leaves-the-
> laptop tradeoff the inverted threat model names.

---

## 7 · Exit — the sandbox is remote and governed

With account + registry, the sandbox is stopped or killed through
the E2B API / CLI when you are done; nothing ran on your laptop.

On this host tonight, there is nothing to tear down — no sandbox was
launched. The only local artifacts are the baked image and the
generated `e2b.Dockerfile` / `e2b.toml`, both removed by cleanup.

---

## 8 · Reset

```bash
bash demo/cleanup.sh
```

Removes the demo env, fixtures, the generated `e2b.Dockerfile` and
`e2b.toml`, and the Docker-side `sandbox-demo-e2b:*` images. (Any
sandboxes or templates created on E2B are governed through its API
and are yours to stop or delete; images pushed to your registry are
yours to prune.)

> Integration notes for the E2B conversation (Dockerfile-based
> template ingestion, the default-open network that forces an
> explicit deny compile, the 80/443-SNI + unfiltered-QUIC lossiness,
> and the live `updateNetwork` redemption): the backend module docs
> at `cli/flox/src/commands/sandbox_backends/e2b.rs` and the backend
> contract at
> `slices/2026/06-sandboxed-activation-prototype/artifacts/backend-contract.md`.
