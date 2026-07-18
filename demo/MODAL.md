# Demo: `flox activate --sandbox` — the Modal backend (prototype)

`cd` into a project and land in a **Modal Sandbox**: a remote,
cloud-isolated container running your baked environment, with
outbound egress deny-by-default and only your declared domains
allowed over TLS — and a coding agent running inside a boundary
that lives in Modal's cloud, not on your laptop.

**Bold** lines are what to *say*; fenced blocks are what to *type*.
The OpenShell walkthrough is `demo/OPENSHELL.md`; the OCI one is
`demo/SCRIPT.md`. All three share `demo/setup.sh` and
`demo/cleanup.sh`.

**The pitch:** flox already bakes each environment into an OCI
image. This backend hands that image to Modal — flox brings the
reproducible environment, Modal brings remote, cloud-grade
isolation with a native domain allowlist. Same manifest, one word
changed: `backend = "modal"`.

**Honest up front — what this host can and cannot do tonight.**
The Modal backend is a *cloud-API* integration: nothing runs on
the laptop. Two prerequisites gate the remote launch, and a bare
checkout has neither:

1. **A Modal account and token.** The Modal CLI authenticates
   against `~/.modal.toml`. Free tier suffices. This host has no
   account and no token.
2. **A registry Modal can pull from.** Modal ingests images by
   *registry reference only* (`Image.from_registry(...)`) — there
   is no local-Docker or tarball ingestion. So the locally baked
   image must be pushed to a registry Modal can reach.

Without those, flox goes as deep as it honestly can: it runs
preflight, bakes the image, compiles the network policy, and
**generates the Modal launch program** — then stops at the launch
boundary with a message naming exactly what a credentialed
operator must supply. This walkthrough marks each beat that needs
an account or a registry.

---

## 0 · Setup

### One-time host prerequisites

1. **Docker Desktop** (or Docker Engine ≥ 28) running — the image
   is baked into the local Docker store before it is pushed.
2. **The Modal CLI**, installed via flox or pip:

   ```bash
   flox install python313Packages.modal   # or: pip install modal
   ```

   Install needs no account; only the launch does. Verify:

   ```bash
   modal --version        # e.g. 1.4.2
   ```

3. **A Modal account + token** (**account beat** — required for the
   remote launch, beats 1+). On a credentialed operator's machine:

   ```bash
   modal token new        # opens a browser; writes ~/.modal.toml
   modal token info       # confirms the active token
   ```

   The free tier is enough for this demo.

4. **A registry Modal can pull from** (**registry beat** —
   required for the remote launch). Point flox at it so the
   generated launcher references the right ref:

   ```bash
   export FLOX_SANDBOX_MODAL_REGISTRY=docker.io/<your-user>
   ```

### Demo environment

Run once from the dev shell:

```bash
BACKEND=modal bash demo/setup.sh
```

Same demo env as the other walkthroughs (git, curl, which,
python3, `flox/claude-code`, an auto-starting web service, seeded
`app.py` / `index.html`); the manifest declares `backend = "modal"`
plus network grants for the agent's API endpoints:

```toml
[[options.sandbox.network]]
endpoint = "api.anthropic.com:443"
binary   = "claude-code/.claude-wrapped"
# plus an identical grant for statsig.anthropic.com (agent telemetry)
```

flox compiles the **host** of each `:443` grant into Modal's
`outbound_domain_allowlist`. Modal's allowlist is TLS/443-only and
carries no per-binary or read/write scoping, so the `binary`,
`access`, and `protocol` fields are recorded as comments in the
launch program but **do not** constrain traffic — a declared
lossiness, honest about what Modal enforces. A grant on any port
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

The image lands in Docker as `sandbox-demo-openshell:<hash12>`
(the Modal backend reuses the openshell compat-layer bake),
content-addressed to the lockfile — it rebakes only when the
environment actually changes.

---

## 1 · Auto-activate toward a Modal sandbox

**"One `cd`, one `Y`, and flox bakes the image, compiles the
policy, and generates the launch program for a remote Modal
sandbox."**

```bash
cd /tmp && cd ~/sandbox-demo
```

```
Enter '/Users/you/sandbox-demo' (sandboxed via modal)? [Y/n]
```

Type `Y`. flox baked (or reused) the image, then generated the
Modal launch program.

**Without an account or registry** (this host), flox stops at the
launch boundary and tells you precisely what is missing:

```
The 'modal' sandbox backend launches a remote Modal Sandbox, which
requires two prerequisites this host cannot satisfy automatically:
  1. Push the baked image 'sandbox-demo-openshell:<hash12>' to a
     registry Modal can pull (set FLOX_SANDBOX_MODAL_REGISTRY=...).
  2. A Modal account and token (preflight confirmed the CLI; the
     launch itself calls the Modal API).
flox generated the launch program at:
  /Users/you/sandbox-demo/.flox/cache/modal-launch.py
With the image pushed and Modal authenticated, run it with
'modal run /Users/you/sandbox-demo/.flox/cache/modal-launch.py'.
```

**"That is not a failure — that is the honest edge of what a
laptop can do for a cloud provider. flox did everything local:
baked the image, compiled the deny-by-default policy, and wrote the
exact program that launches the sandbox. The two missing pieces are
Modal's, not flox's."**

Look at what flox generated:

```bash
cat ~/sandbox-demo/.flox/cache/modal-launch.py
```

```python
#!/usr/bin/env python3
# Generated by `flox activate --sandbox --sandbox-backend modal`.
import sys
import modal

app = modal.App.lookup('flox-sandbox-demo', create_if_missing=True)
image = modal.Image.from_registry('sandbox-demo-modal:<hash12>')

sb = modal.Sandbox.create(
    ...
    app=app,
    image=image,
    workdir='/Users/you/sandbox-demo',
    timeout=3600,
    outbound_domain_allowlist=['api.anthropic.com', 'statsig.anthropic.com'],
)
...
```

**"The manifest's `:443` grants became Modal's native domain
allowlist. Everything else is deny-by-default — that is Modal's
secure-by-default posture, and flox compiled the policy onto it."**

---

## 2 · Push the image and launch (account + registry beat)

**"With a Modal account and a registry, this is the whole
remaining path."** On a credentialed operator's machine, with
`FLOX_SANDBOX_MODAL_REGISTRY` set:

```bash
# Tag the local bake as the modal-namespaced registry ref and push:
docker tag sandbox-demo-openshell:<hash12> \
  "$FLOX_SANDBOX_MODAL_REGISTRY/sandbox-demo-modal:<hash12>"
docker push "$FLOX_SANDBOX_MODAL_REGISTRY/sandbox-demo-modal:<hash12>"

# Launch the sandbox flox generated:
modal run ~/sandbox-demo/.flox/cache/modal-launch.py
```

Modal pulls the image, builds a `modal.Image` from it, creates a
remote Sandbox, and runs the activation inside it — output streams
back to your terminal.

> This beat requires a live Modal account and a reachable
> registry, neither of which this host has tonight. The generated
> program is exactly what runs; nothing is faked.

---

## 3 · Prove the boundary — deny-by-default egress

**"Egress is deny-by-default at the network layer. Only the
manifest's `:443` domains are allowed, over TLS."**

Inside the launched sandbox (account + registry beat), a granted
endpoint works:

```bash
curl -sS https://api.anthropic.com/  # allowed: in the domain allowlist
```

An ungranted endpoint is blocked by Modal:

```bash
curl -sS https://api.github.com/zen
# blocked — api.github.com is not in outbound_domain_allowlist
```

**"Modal blocks and logs the connection to any domain the manifest
did not grant. flox authored that allowlist from the environment's
own declared network needs — nothing more."**

The threat model **inverts** here versus the local backends: the
host filesystem is unreachable from the remote sandbox (there is no
bind mount of your laptop), but the code and any injected secrets
run in Modal's cloud. That is the honest tradeoff of a remote
provider, and flox states it in the backend capabilities.

---

## 4 · Policy is fixed at creation — redemption is recreation

**"Modal fixes the sandbox's policy at creation. There is no live
'ask' — to widen egress, you recreate the sandbox with a wider
allowlist."**

Grant a new domain by editing the manifest and re-activating:

```toml
[[options.sandbox.network]]
endpoint = "api.github.com:443"
```

```bash
flox edit                       # add the grant
flox deactivate && cd ~/sandbox-demo   # recreate with the new policy
```

flox recompiles the allowlist, regenerates the launch program, and
(with account + registry) the next launch creates a sandbox that
allows `api.github.com`. **"Recreation-as-redemption — the common
path for cloud sandbox providers, and honest about it: no live
verdict, a fresh sandbox with the new policy."**

---

## 5 · Run a coding agent, at full autonomy (account + registry beat)

**"A coding agent with no permission prompts, running in Modal's
cloud — the sandbox, not the agent, is the boundary, and the
boundary is remote."**

The manifest already grants the agent's Anthropic endpoints, so
inside the launched sandbox:

```bash
claude --permission-mode auto
```

```
> add a docstring to greet() in app.py and commit the change
```

Claude's API traffic to `api.anthropic.com` is allowed; anything it
reaches for outside the allowlist is blocked by Modal. Because the
sandbox is remote and ephemeral, the blast radius of anything the
agent does is a cloud container that Modal tears down on exit.

> Agent auth (`CLAUDE_CODE_OAUTH_TOKEN`) must be injected as a
> Modal secret/env into the sandbox — the remote guest has no
> access to your laptop's `.env`. This is the credential-leaves-the
> -laptop tradeoff the inverted threat model names.

---

## 6 · Exit — the sandbox is remote and ephemeral

With account + registry, the launched sandbox terminates when the
activation exits (the generated program calls `sb.terminate()`).
Nothing lingers on Modal, and nothing ran on your laptop.

On this host tonight, there is nothing to tear down — no sandbox
was launched. The only local artifacts are the baked image and the
generated `modal-launch.py`, both removed by cleanup.

---

## 7 · Reset

```bash
bash demo/cleanup.sh
```

Removes the demo env, fixtures, the generated `modal-launch.py`,
and the Docker-side `sandbox-demo-openshell:*` / `sandbox-demo-modal:*`
images. (Any sandboxes launched on Modal are ephemeral and already
gone; images pushed to your registry are yours to prune.)

> Integration notes for the Modal conversation (image-ingestion is
> registry-only, TLS/443-only domain allowlist, policy-fixed-at
> -creation, the inverted threat model): the backend module docs at
> `cli/flox/src/commands/sandbox_backends/modal.rs` and the backend
> contract at
> `slices/2026/06-sandboxed-activation-prototype/artifacts/backend-contract.md`.
