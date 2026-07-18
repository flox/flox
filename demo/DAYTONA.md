# Demo: `flox activate --sandbox` — the Daytona backend (prototype)

`cd` into a project and land in a **Daytona sandbox**: a remote,
cloud-isolated environment built from your baked image as a Daytona
*snapshot*, with outbound egress deny-by-default and only your
declared domains allowed — and a coding agent running inside a
boundary that lives in Daytona's cloud, not on your laptop.

**Bold** lines are what to *say*; fenced blocks are what to *type*.
The Modal walkthrough is `demo/MODAL.md`; the E2B one is
`demo/E2B.md`; the OCI one is `demo/SCRIPT.md`. All share
`demo/setup.sh` and `demo/cleanup.sh`.

**The pitch:** flox already bakes each environment into an OCI
image. This backend hands that image to Daytona as a snapshot — flox
brings the reproducible environment, Daytona brings remote,
cloud-grade isolation with a native domain allowlist. Same manifest,
one word changed: `backend = "daytona"`.

**Honest up front — what this host can and cannot do tonight.**
The Daytona backend is a *cloud-API* integration: nothing runs on
the laptop. Two prerequisites gate the remote launch, and a bare
checkout has neither:

1. **A Daytona account and API key.** The Daytona CLI/SDK
   authenticates with `DAYTONA_API_KEY` (the REST API takes
   `Authorization: Bearer <key>`). Free tier suffices. This host has
   no account and no key.
2. **A registry Daytona can pull from.** Daytona ingests images as
   the base of a *snapshot* (`Image.base(<ref>)` via the declarative
   builder), so the locally baked image must be pushed to a registry
   Daytona can reach.

Without those, flox goes as deep as it honestly can: it runs
preflight, bakes the image, compiles the network policy, and
**generates the Daytona launch program** — then stops at the launch
boundary with a message naming exactly what a credentialed operator
must supply. This walkthrough marks each beat that needs an account
or a registry.

---

## 0 · Setup

### One-time host prerequisites

1. **Docker Desktop** (or Docker Engine ≥ 28) running — the image
   is baked into the local Docker store before it is pushed.
2. **The Daytona CLI** — one command, in your presentation shell
   (export `FLOX_BIN` from the dev shell first):

   ```bash
   flox activate -r djsauble/daytona-setup
   ```

   This is the demo's *outer layer* — one setup env per sandbox
   backend is the plan. It installs `daytona-bin` (binary name
   `daytona`; unlike the E2B CLI, Daytona's CLI is in the Flox
   catalog), exports the demo's feature flags and the planted
   `GITHUB_TOKEN` (`[vars]`), sets `FLOX_VERSION` plus a `flox` alias
   from `$FLOX_BIN` (`[profile]`), plants the `~/demo-secrets`
   fixture, and prints a non-interactive note for each missing launch
   prerequisite (Docker, auth, registry). Deactivating removes the
   planted secret (`[profile.deactivate]`). Stay in this activation
   for the whole demo; confirm:

   ```bash
   daytona --version      # e.g. 0.12.0
   ```

   > Details, caveats, and troubleshooting:
   > `demo/daytona-setup/README.md`.

3. **A Daytona account + API key** (**account beat** — required for
   the remote launch, beats 1+). On a credentialed operator's
   machine:

   ```bash
   daytona login          # opens a browser; or:
   export DAYTONA_API_KEY=<key>   # from the Daytona dashboard
   ```

   The free tier is enough for this demo.

4. **A registry Daytona can pull from** (**registry beat** —
   required for the remote launch). Point flox at it so the generated
   launcher references the right ref:

   ```bash
   export FLOX_SANDBOX_DAYTONA_REGISTRY=docker.io/<your-user>
   ```

### Demo environment

Run once from the dev shell:

```bash
BACKEND=daytona bash demo/setup.sh
```

Same demo env as the other walkthroughs (git, curl, which, python3,
`flox/claude-code`, an auto-starting web service, seeded `app.py` /
`index.html`); the manifest declares `backend = "daytona"` plus
network grants for the agent's API endpoints:

```toml
[[options.sandbox.network]]
endpoint = "api.anthropic.com:443"
binary   = "claude-code/.claude-wrapped"
# plus an identical grant for statsig.anthropic.com (agent telemetry)
```

flox compiles the **host** of each grant into Daytona's
`domainAllowList`. Daytona filters **per-domain, not per-port**, so
the `:443` is dropped — every port to `api.anthropic.com` is
reachable — and the allowlist carries no per-binary or read/write
scoping, so `binary`, `access`, and `protocol` are recorded as
comments in the launch program but **do not** constrain traffic. All
declared lossiness, honest about what Daytona enforces.

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

The image lands in Docker as `sandbox-demo-daytona:<hash12>`,
content-addressed to the lockfile — it rebakes only when the
environment actually changes.

---

## 1 · Auto-activate toward a Daytona sandbox

**"One `cd`, one `Y`, and flox bakes the image, compiles the
policy, and generates the launch program for a remote Daytona
sandbox."**

```bash
cd /tmp && cd ~/sandbox-demo
```

```
Enter '/Users/you/sandbox-demo' (sandboxed via daytona)? [Y/n]
```

Type `Y`. flox baked (or reused) the image, then generated the
Daytona launch program.

**Without an account or registry** (this host), flox stops at the
launch boundary and tells you precisely what is missing:

```
The 'daytona' sandbox backend launches a remote Daytona sandbox, which
requires two prerequisites this host cannot satisfy automatically:
  1. Push the baked image 'sandbox-demo-daytona:<hash12>' to a
     registry Daytona can pull (set FLOX_SANDBOX_DAYTONA_REGISTRY=...),
     which the launcher registers as the snapshot base.
  2. A Daytona account and API key (preflight confirmed the CLI; the
     snapshot registration and sandbox launch call the Daytona API).
flox generated the launch program at:
  /Users/you/sandbox-demo/.flox/cache/daytona-launch.py
With the image pushed and DAYTONA_API_KEY set, run it with
'python /Users/you/sandbox-demo/.flox/cache/daytona-launch.py'.
```

**"That is not a failure — that is the honest edge of what a laptop
can do for a cloud provider. flox did everything local: baked the
image, compiled the deny-by-default policy, and wrote the exact
program that launches the sandbox. The two missing pieces are
Daytona's, not flox's."**

Look at what flox generated:

```bash
cat ~/sandbox-demo/.flox/cache/daytona-launch.py
```

```python
#!/usr/bin/env python3
# Generated by `flox activate --sandbox --sandbox-backend daytona`.
import sys
from daytona import (
    CreateSandboxFromSnapshotParams,
    CreateSnapshotParams,
    Daytona,
    Image,
    Resources,
)

daytona = Daytona()

image = Image.base('sandbox-demo-daytona:<hash12>')
daytona.snapshot.create(
    CreateSnapshotParams(
        name='flox-sandbox-demo',
        image=image,
        resources=Resources(cpu=2, memory=4, disk=8),
    ),
)

sandbox = daytona.create(
    CreateSandboxFromSnapshotParams(
        snapshot='flox-sandbox-demo',
        domain_allow_list='api.anthropic.com,statsig.anthropic.com',
    )
)
...
```

**"The manifest's grants became Daytona's native domain allowlist.
Everything else is deny-by-default — flox compiled the policy onto
Daytona's secure-by-default posture."**

---

## 2 · Push the image and launch (account + registry beat)

**"With a Daytona account and a registry, this is the whole
remaining path."** On a credentialed operator's machine, with
`FLOX_SANDBOX_DAYTONA_REGISTRY` set and `DAYTONA_API_KEY` exported:

```bash
# Tag the local bake as the registry ref and push:
docker tag sandbox-demo-daytona:<hash12> \
  "$FLOX_SANDBOX_DAYTONA_REGISTRY/sandbox-demo-daytona:<hash12>"
docker push "$FLOX_SANDBOX_DAYTONA_REGISTRY/sandbox-demo-daytona:<hash12>"

# Launch the sandbox flox generated:
python ~/sandbox-demo/.flox/cache/daytona-launch.py
```

Daytona pulls the image, registers it as a snapshot, creates a
sandbox from that snapshot, and runs the activation inside it —
output streams back to your terminal.

> This beat requires a live Daytona account and a reachable
> registry, neither of which this host has tonight. The generated
> program is exactly what runs; nothing is faked.

---

## 3 · Prove the boundary — deny-by-default egress

**"Egress is deny-by-default. Only the manifest's domains are
allowed."**

Inside the launched sandbox (account + registry beat), a granted
endpoint works:

```bash
curl -sS https://api.anthropic.com/  # allowed: in the domain allowlist
```

An ungranted endpoint is blocked by Daytona:

```bash
curl -sS https://api.github.com/zen
# blocked — api.github.com is not in domainAllowList
```

**"Daytona blocks any domain the manifest did not grant. flox
authored that allowlist from the environment's own declared network
needs — nothing more."**

The threat model **inverts** here versus the local backends: the
host filesystem is unreachable from the remote sandbox (there is no
bind mount of your laptop), but the code and any injected secrets
run in Daytona's cloud. That is the honest tradeoff of a remote
provider, and flox states it in the backend capabilities.

Show the isolation directly — the planted secret the host has:

```bash
ls -a ~/demo-secrets/   # on the host: .env exists
```

is simply invisible to the remote sandbox — there is no host mount
at all.

---

## 4 · The load-bearing lossiness — Daytona's exclusive allowlists

**"Here is the honest part a customer needs to hear. Daytona's
per-sandbox network vocabulary is three parameters — a domain list,
a CIDR list, and block-all — and they are *mutually exclusive*: at
most one non-empty."**

flox compiles host grants onto the **domain** list. So a CIDR-shaped
grant cannot ride along on the same sandbox — flox declines it
rather than silently dropping it or widening the policy. Show it:

```toml
[[options.sandbox.network]]
endpoint = "10.0.0.0/24:443"
```

```bash
flox edit                       # add the CIDR grant
flox deactivate && cd ~/sandbox-demo
```

```
The 'daytona' sandbox backend compiles host grants onto its domain
allowlist, which is mutually exclusive with its CIDR allowlist, but
rule '10.0.0.0/24:443' is a CIDR range.
Daytona accepts at most one of a domain list or a CIDR list per
sandbox — mixing them is not expressible. Rewrite the grant as a host
(e.g. 'api.github.com:443'), or select a backend that expresses CIDR
and domain grants together (e.g. 'openshell').
```

**"flox declines what Daytona cannot express, and names the ceiling
in the same breath. Two more honest limits it records: Daytona
filters per-domain, not per-port, so the grant's `:443` doesn't
scope the rule; and on Tier 1/2 organizations the org-level network
policy overrides sandbox-level settings entirely. flox writes all of
this into the launch program's header so the operator sees it."**

Remove the CIDR grant to continue:

```bash
flox edit                       # delete the CIDR line
```

---

## 5 · Policy is fixed at creation — with a live-update caveat

**"Daytona chooses the sandbox's network policy at creation. It does
expose a live update-network on a running sandbox — but that is an
operator replacing the whole policy, not a per-request verdict. So
the redemption path flox models is recreation."**

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
path for cloud sandbox providers. Daytona's live update-network is
there if an operator wants it, but it is a policy *replacement*, not
the per-request 'ask' the advisory backend gives you."**

---

## 6 · Run a coding agent, at full autonomy (account + registry beat)

**"A coding agent with no permission prompts, running in Daytona's
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
reaches for outside the allowlist is blocked by Daytona. Because the
sandbox is remote and ephemeral, the blast radius of anything the
agent does is a cloud sandbox that Daytona tears down on exit.

> Agent auth (`CLAUDE_CODE_OAUTH_TOKEN`) must be injected via
> Daytona's own secret mechanism into the sandbox — the remote guest
> has no access to your laptop's `.env`. This is the
> credential-leaves-the-laptop tradeoff the inverted threat model
> names.

---

## 7 · Exit — the sandbox is remote and ephemeral

With account + registry, the launched sandbox is deleted when the
activation exits (the generated program calls `daytona.delete(...)`).
Nothing lingers in Daytona, and nothing ran on your laptop.

On this host tonight, there is nothing to tear down — no sandbox was
launched. The only local artifacts are the baked image and the
generated `daytona-launch.py`, both removed by cleanup.

---

## 8 · Reset

Deactivate the setup layer (its `profile.deactivate` removes the
planted secret), then:

```bash
bash demo/cleanup.sh
```

Removes the demo env, fixtures, the generated `daytona-launch.py`,
and the Docker-side `sandbox-demo-daytona:*` images. (Any sandboxes
launched on Daytona are ephemeral and deleted on exit; images pushed
to your registry are yours to prune.)

> Integration notes for the Daytona conversation (image-ingestion is
> snapshot-from-registry, the three mutually-exclusive allowlists,
> per-domain-not-per-port filtering, the Tier 1/2 org override, the
> inverted threat model): the backend module docs at
> `cli/flox/src/commands/sandbox_backends/daytona.rs` and the backend
> contract at
> `slices/2026/06-sandboxed-activation-prototype/artifacts/backend-contract.md`.
