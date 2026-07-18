# Demo: `flox activate --sandbox` — the Ona backend (prototype)

`cd` into a project and hand its baked environment to **Ona**
(formerly Gitpod): a control-plane CDE that builds a cloud
workspace from a devcontainer. flox bakes the reproducible closure
into an image, generates the `.devcontainer/devcontainer.json` that
wraps it, and an Ona workspace opened from that devcontainer comes
up with the locked toolchain already present — the BNY / Capital
One deployment shape, where a Flox-baked image is the reproducible
substrate an Ona environment runs on.

**Bold** lines are what to *say*; fenced blocks are what to *type*.
The OCI-backend walkthrough is `demo/SCRIPT.md`; the OpenShell one
is `demo/OPENSHELL.md`; the Modal one is `demo/MODAL.md`; the
Docker Sandboxes one is `demo/DOCKER-SBX.md`. They all share
`demo/setup.sh` and `demo/cleanup.sh`.

**The pitch:** flox already bakes each environment into an OCI
image. This backend hands that image to Ona as a devcontainer —
flox brings the reproducible environment, Ona brings a managed,
governed cloud workspace. Same manifest, one word changed:
`backend = "ona"`.

> **Honesty up front — this backend is Scaffolded, not
> Implemented.** Ona is a control-plane / cloud CDE: nothing runs
> on the laptop, and opening a workspace needs (1) an **Ona
> account and an enterprise workspace** — post-OpenAI-acquisition
> (2026-06-11) trial access is uncertain, so a partnership contact
> is likely required — and (2) a **registry Ona can pull from**,
> since Ona builds the workspace by pulling the image the
> devcontainer references. This host has neither, and no
> `ona`/`gitpod` CLI. So flox runs the honest *local* slice —
> preflight, bake, policy compilation, devcontainer generation —
> and stops at the launch boundary with a clear error naming both
> gaps. Beats 2–5 describe what a completed workspace open looks
> like.

---

## 0 · Setup

### One-time host prerequisites

1. **Docker Desktop** (or Docker Engine ≥ 28) running — the image
   is baked into the local Docker store before it is pushed. This
   is the one genuinely required host tool.
2. **The presentation shell** — one command, in your presentation
   shell (export `FLOX_BIN` from the dev shell first):

   ```bash
   flox activate -r djsauble/ona-setup
   ```

   This is the demo's *outer layer* — one setup env per sandbox
   backend is the plan. Unlike the OpenShell env, it runs **no
   local service** and installs **no provider CLI**: Ona is
   cloud-only and its CLI is presence-detected, not required. It
   configures the shell (feature flags and the planted
   `GITHUB_TOKEN` in `[vars]`, `FLOX_VERSION` plus a `flox` alias
   from `$FLOX_BIN` in `[profile]`), plants the `~/demo-secrets`
   fixture, and prints a note for each hand-off prerequisite that
   is missing. Deactivating removes the planted secret
   (`[profile.deactivate]`). Stay in this activation for the whole
   demo.

   > Details, caveats, and troubleshooting:
   > `demo/ona-setup/README.md`.

3. **An Ona account + enterprise workspace** (**account beat** —
   required for the workspace open, beats 2+). Ona builds a
   workspace from a devcontainer in a git repository through its
   control plane. Trial availability is uncertain after the
   OpenAI acquisition; assume a partnership contact is needed.

4. **A registry Ona can pull from** (**registry beat** — required
   for the workspace open). Point flox at it so the generated
   devcontainer references the right ref:

   ```bash
   export FLOX_SANDBOX_ONA_REGISTRY=docker.io/<your-user>
   ```

### Demo environment

Run once from the dev shell:

```bash
BACKEND=ona bash demo/setup.sh
```

Same demo env as the other walkthroughs (git, curl, which,
python3, `flox/claude-code`, an auto-starting web service, seeded
`app.py` / `index.html`); the manifest declares `backend = "ona"`
plus network grants for the agent's API endpoints:

```toml
[[options.sandbox.network]]
endpoint = "api.anthropic.com:443"
binary   = "claude-code/.claude-wrapped"
# plus an identical grant for statsig.anthropic.com (agent telemetry)
```

flox compiles the **host** of each `:443` grant into the
devcontainer's `flox.sandbox.network.allow` allowlist, with
`default: "deny"`. The devcontainer spec has no native egress
vocabulary, so that allowlist is the expectation an operator wires
into Ona's enterprise network policy; the `binary`, `access`, and
`protocol` fields are recorded as metadata but **do not** constrain
traffic through the devcontainer contract — a declared lossiness,
honest about what the hand-off can express. A grant on any port
other than 443 is rejected at compile time rather than silently
widened.

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

The image lands in Docker as `sandbox-demo-ona:<hash12>`
(the Ona backend reuses the openshell compat-layer bake),
content-addressed to the lockfile — it rebakes only when the
environment actually changes.

---

## 1 · Auto-activate toward an Ona workspace

**"One `cd`, one `Y`, and flox bakes the image, compiles the
policy, and generates the devcontainer for an Ona workspace."**

```bash
cd /tmp && cd ~/sandbox-demo
```

```
Enter '/Users/you/sandbox-demo' (sandboxed via ona)? [Y/n]
```

Type `Y`. flox baked (or reused) the image, then generated the
devcontainer hand-off.

**Without an account or registry** (this host), flox stops at the
launch boundary and tells you precisely what is missing:

```
The 'ona' sandbox backend hands the baked environment off to an Ona
(formerly Gitpod) workspace, which requires prerequisites this host
cannot satisfy automatically:
  1. Push the baked image 'sandbox-demo-ona:<hash12>' to a registry
     Ona can pull (set FLOX_SANDBOX_ONA_REGISTRY=<registry-prefix>
     and re-run, then push '<prefix>/sandbox-demo-ona:<hash12>').
  2. An Ona account and an enterprise workspace: no Ona/Gitpod CLI
     was found on PATH, and Ona builds the workspace from the
     committed devcontainer through its control plane.
     Post-OpenAI-acquisition trial access is uncertain — a
     partnership contact is likely required.
flox generated the devcontainer hand-off at:
  /Users/you/sandbox-demo/.devcontainer/devcontainer.json
Commit it to the repo Ona opens, push the image, then create a
workspace from the repo.
```

**"That is not a failure — that is the honest edge of what a laptop
can do for an enterprise control plane. flox did everything local:
baked the image, compiled the deny-by-default policy, and wrote the
exact devcontainer Ona builds a workspace from. The two missing
pieces are Ona's, not flox's."**

Look at what flox generated:

```bash
cat ~/sandbox-demo/.devcontainer/devcontainer.json
```

```jsonc
// Generated by `flox activate --sandbox --sandbox-backend ona`.
// ... hand-off contract: push the image, commit this file, open a
// workspace from the repo ...
//   policy: HTTPS/443 allowed: api.anthropic.com, statsig.anthropic.com
{
  "name": "flox-sandbox-demo",
  "image": "sandbox-demo-ona:<hash12>",
  "containerEnv": {
    "FLOX_SANDBOX_BACKEND": "ona"
  },
  "customizations": {
    "flox": {
      "sandbox": {
        "backend": "ona",
        "network": {
          "default": "deny",
          "allow": ["api.anthropic.com", "statsig.anthropic.com"]
        }
      }
    }
  },
  "overrideCommand": false
}
```

**"The manifest's `:443` grants became the devcontainer's egress
allowlist, deny-by-default. That is the exact allowlist an operator
wires into Ona's enterprise network policy — flox authored it from
the environment's own declared network needs, nothing more."**

---

## 2 · Push the image and open the workspace (account + registry beat)

**"With an Ona account and a registry, this is the whole remaining
path."** On a credentialed operator's machine, with
`FLOX_SANDBOX_ONA_REGISTRY` set:

```bash
# Tag the local bake as the ona-namespaced registry ref and push:
docker tag sandbox-demo-ona:<hash12> \
  "$FLOX_SANDBOX_ONA_REGISTRY/sandbox-demo-ona:<hash12>"
docker push "$FLOX_SANDBOX_ONA_REGISTRY/sandbox-demo-ona:<hash12>"

# Commit the generated devcontainer to the repo Ona opens, then
# create a workspace from that repo through the Ona dashboard or CLI.
git -C ~/sandbox-demo add .devcontainer/devcontainer.json
git -C ~/sandbox-demo commit -m "add flox-generated Ona devcontainer"
git -C ~/sandbox-demo push
```

Ona pulls the image the devcontainer references, builds the
workspace, and opens it — the locked toolchain is present the
moment the workspace comes up.

> This beat requires a live Ona account, an enterprise workspace,
> and a reachable registry, none of which this host has tonight.
> The generated devcontainer is exactly what Ona builds from;
> nothing is faked.

---

## 3 · The workspace opens with the locked toolchain — contrast a
hand-provisioned one

**"The whole pitch: the toolchain is present on open. No
`apt install`, no version drift, no 'works on my machine' — the
image is the environment, content-addressed to the lockfile."**

Inside the opened Ona workspace (account + registry beat), the
locked tools are already there:

```bash
which python3 curl git       # all present, at the locked versions
flox list                    # the baked closure, exactly as declared
```

Contrast a hand-provisioned Ona workspace (a bare devcontainer with
no Flox image): the developer installs each tool by hand, pins
nothing, and the next teammate gets a different set. **"That drift
is precisely what BNY and Capital One are buying Flox to remove
from their Ona deployment: the reproducible closure is the base
image every workspace opens from."**

---

## 4 · Prove the boundary — deny-by-default egress

**"Egress is deny-by-default. Only the manifest's `:443` domains
are in the allowlist flox compiled into the devcontainer, which the
workspace's Ona network policy enforces."**

Inside the workspace (account + registry beat), a granted endpoint
works:

```bash
curl -sS https://api.anthropic.com/  # allowed: in the allowlist
```

An ungranted endpoint is blocked by Ona's network policy:

```bash
curl -sS https://api.github.com/zen
# blocked — api.github.com is not in flox.sandbox.network.allow
```

**"Ona's control plane governs egress; flox authored the allowlist
from the environment's own declared network needs. The threat model
inverts versus the local backends: the host filesystem is
unreachable from the remote workspace (there is no bind mount of
your laptop), but the code and any injected secrets run in Ona's
cloud. That is the honest tradeoff of a control-plane provider, and
flox states it in the backend capabilities."**

---

## 5 · Policy is fixed at workspace creation — redemption is recreation

**"Ona fixes the workspace's policy when it is built from the
devcontainer. There is no live 'ask' — to widen egress, you edit
the manifest, regenerate the devcontainer, and recreate the
workspace."**

Grant a new domain by editing the manifest and re-activating:

```toml
[[options.sandbox.network]]
endpoint = "api.github.com:443"
```

```bash
flox edit                       # add the grant
flox deactivate && cd ~/sandbox-demo   # regenerate the devcontainer
```

flox recompiles the allowlist and rewrites
`.devcontainer/devcontainer.json`; committing and recreating the
workspace (with account + registry) yields one that allows
`api.github.com`. **"Recreation-as-redemption — the common path for
control-plane sandbox providers, and honest about it: no live
verdict, a fresh workspace built from the new devcontainer."**

---

## 6 · Run a coding agent, at full autonomy (account + registry beat)

**"A coding agent with no permission prompts, running in Ona's
cloud — the workspace, not the agent, is the boundary, and the
boundary is a governed CDE."**

The manifest already grants the agent's Anthropic endpoints, so
inside the opened workspace:

```bash
claude --permission-mode auto
```

```
> add a docstring to greet() in app.py and commit the change
```

Claude's API traffic to `api.anthropic.com` is allowed; anything it
reaches for outside the allowlist is blocked by Ona's network
policy. Because the workspace is remote and governed, the blast
radius of anything the agent does is a cloud CDE the operator
controls.

> Agent auth (`CLAUDE_CODE_OAUTH_TOKEN`) must be injected through
> Ona's own environment-variable / secret mechanism, not your
> laptop's `.env` — the remote workspace has no access to it. This
> is the credential-leaves-the-laptop tradeoff the inverted threat
> model names.

---

## 7 · Exit — the workspace is remote and governed

With account + registry, the workspace is stopped or deleted
through Ona's control plane when you are done; nothing ran on your
laptop.

On this host tonight, there is nothing to tear down — no workspace
was opened. The only local artifacts are the baked image and the
generated `.devcontainer/devcontainer.json`, both removed by
cleanup.

---

## 8 · Reset

```bash
bash demo/cleanup.sh
```

Removes the demo env, fixtures, the generated
`.devcontainer/devcontainer.json`, and the Docker-side
`sandbox-demo-ona:*` images. (Any workspaces opened on Ona are
governed through its control plane and are yours to stop or delete;
images pushed to your registry are yours to prune.)

> Integration notes for the Ona conversation (devcontainer-only
> hand-off, no native egress vocabulary so policy is an operator
> expectation, control-plane account/partnership wall, the inverted
> threat model): the backend module docs at
> `cli/flox/src/commands/sandbox_backends/ona.rs` and the backend
> contract at
> `slices/2026/06-sandboxed-activation-prototype/artifacts/backend-contract.md`.
