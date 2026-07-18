# Demo: `flox activate --sandbox` — the Vercel Sandbox backend (prototype)

`cd` into a project and land in a **Vercel Sandbox**: a remote,
cloud-isolated Firecracker microVM. But this backend is a first for
the roster — it is **bootstrap-shaped**, not image-shaped. Vercel
Sandbox boots a *fixed* base runtime (`node24`, `python3.13`), so
flox cannot hand it the baked OCI image the other cloud backends
push. Instead flox generates a **flox bootstrap** that installs
Flox inside the running sandbox and activates the environment from
FloxHub, plus a `@vercel/sandbox` **launcher** that creates the
sandbox and runs the bootstrap.

**Bold** lines are what to *say*; fenced blocks are what to *type*.
The Modal walkthrough is `demo/MODAL.md`; the OpenShell one is
`demo/OPENSHELL.md`; the OCI one is `demo/SCRIPT.md`. They share
`demo/setup.sh` and `demo/cleanup.sh`.

**The pitch:** every other cloud backend here starts from "flox
bakes an OCI image." Vercel Sandbox cannot ingest one — its runtimes
are fixed. So flox meets it where it is: a self-contained bootstrap
that reproduces the environment *inside* the sandbox from FloxHub.
Same manifest, one word changed: `backend = "vercel-sandbox"`.

**Honest up front — what this host can and cannot do tonight.**
The Vercel Sandbox backend is a *cloud-API* integration: nothing
runs on the laptop. Two prerequisites gate the remote launch, and a
bare checkout has neither:

1. **A Vercel account and token.** The `@vercel/sandbox` SDK
   authenticates with a Vercel OIDC token (`vercel env pull` writes
   it to `.env.local`, 12-hour lifetime) or an access token
   (`VERCEL_TOKEN` + `VERCEL_TEAM_ID` + `VERCEL_PROJECT_ID`). Free
   tier suffices. This host has no account and no token.
2. **The environment reachable from FloxHub.** Because the runtime
   is fixed and no image is pushed, the bootstrap activates
   `flox activate -r <owner>/<env>` — so the environment must be
   pushed to FloxHub first (`flox push`).

Without those, flox goes as deep as it honestly can: it runs
preflight, generates the **flox bootstrap** and the
**`@vercel/sandbox` launcher**, and **stops at the launch boundary**
with a message naming exactly what a credentialed operator must
supply. This walkthrough marks each beat that needs an account or a
FloxHub push.

---

## 0 · Setup

### One-time host prerequisites

1. **The Vercel CLI**, installed via npm (`vercel` is not in the
   Flox Catalog; the `vercel-sandbox-setup` env provides `nodejs`,
   which provides `npm`):

   ```bash
   npm install -g vercel
   vercel --version        # e.g. 56.3.1
   ```

   Install needs no account; only the launch does.

2. **A Vercel account + token** (**account beat** — required for
   the remote launch, beats 1+). On a credentialed operator's
   machine:

   ```bash
   vercel login            # opens a browser
   vercel link             # links a Vercel project
   vercel env pull         # downloads an OIDC token to .env.local
   ```

   The free tier is enough for this demo. (Non-interactively, export
   `VERCEL_TOKEN` with `VERCEL_TEAM_ID`/`VERCEL_PROJECT_ID` instead.)

3. **The environment on FloxHub** (**push beat** — required for the
   remote launch). The bootstrap activates a FloxHub ref, so push
   the demo env and point flox at it:

   ```bash
   cd ~/sandbox-demo && flox push --owner <your-owner>
   export FLOX_SANDBOX_VERCEL_FLOXHUB_REF=<your-owner>/sandbox-demo
   ```

4. **The base runtime** (optional). Vercel Sandbox boots a fixed
   runtime; the default is `node24`. Override it (validated against
   `node22` / `node24` / `python3.13`):

   ```bash
   export FLOX_SANDBOX_VERCEL_RUNTIME=node24
   ```

### Demo environment

Run once from the dev shell:

```bash
BACKEND=vercel-sandbox bash demo/setup.sh
```

Same demo env as the other walkthroughs (git, curl, which, python3,
`flox/claude-code`, an auto-starting web service, seeded `app.py` /
`index.html`); the manifest declares `backend = "vercel-sandbox"`.

**No `[[options.sandbox.network]]` grants** — and that is the
honest part. The `@vercel/sandbox` SDK has **no per-sandbox egress
allowlist or firewall**: its `ports` option governs *inbound*
exposure only. flox cannot compile a domain-egress policy onto this
provider, so rather than silently drop a grant (which would falsely
imply it was honored), flox **declines** any network grant with a
clear error. The demo env therefore declares none; beat 3 shows the
decline directly.

The setup env already configured your shell — make sure the prompt
hook is in your shell's RC:

```bash
eval "$(flox hook-env --shell bash --shell-pid $$)"
```

**No pre-bake step.** Unlike every OCI backend, nothing is baked
here: Vercel Sandbox boots a stock runtime and flox reproduces the
environment inside it from FloxHub. The "bake" is the push
(prerequisite 3) plus the in-sandbox `flox activate -r`.

---

## 1 · Auto-activate toward a Vercel Sandbox

**"One `cd`, one `Y`, and flox generates the bootstrap and the
`@vercel/sandbox` launcher for a remote Firecracker microVM — no
image, because Vercel's runtimes are fixed."**

```bash
cd /tmp && cd ~/sandbox-demo
```

```
Enter '/Users/you/sandbox-demo' (sandboxed via vercel-sandbox)? [Y/n]
```

Type `Y`. flox ran preflight (Vercel CLI + auth), generated the
bootstrap and launcher, then — **without an account or a FloxHub
push** (this host) — stopped at the launch boundary and told you
precisely what is missing:

```
The 'vercel-sandbox' backend launches a remote Vercel Sandbox, which
requires two prerequisites this host cannot satisfy automatically:
  1. A Vercel account and token (preflight confirmed the CLI; the
     launch calls the Vercel API — run 'vercel env pull' for an OIDC
     token or export VERCEL_TOKEN).
  2. the environment pushed to FloxHub ('flox push'), then set
     FLOX_SANDBOX_VERCEL_FLOXHUB_REF=<owner>/sandbox-demo and re-run —
     Vercel Sandbox boots a fixed runtime and cannot ingest the baked
     image, so the bootstrap installs Flox in-sandbox and activates
     from FloxHub.
flox generated the bootstrap-shaped hand-off at:
  /Users/you/sandbox-demo/.flox/cache/vercel-sandbox-bootstrap.sh
  /Users/you/sandbox-demo/.flox/cache/vercel-sandbox-launch.mjs
With Vercel authenticated and the environment on FloxHub, run
'node /Users/you/sandbox-demo/.flox/cache/vercel-sandbox-launch.mjs'.
```

**"That is not a failure — that is the honest edge of a
bootstrap-shaped provider. flox did everything local: ran preflight,
wrote the bootstrap that reproduces the environment inside the
sandbox, and wrote the exact launcher that creates it. The two
missing pieces are Vercel's account and a FloxHub push, not flox's."**

Look at what flox generated — first the bootstrap:

```bash
cat ~/sandbox-demo/.flox/cache/vercel-sandbox-bootstrap.sh
```

```bash
#!/usr/bin/env bash
# ... runs INSIDE a Vercel Sandbox (fixed Amazon Linux 2023 runtime).
# Vercel Sandbox boots a stock runtime rather than a baked image, so
# flox installs Flox and activates the environment from FloxHub.
set -euo pipefail

if ! command -v flox >/dev/null 2>&1; then
  curl -fsSL https://install.flox.dev/install.sh | bash
  ...
fi

exec flox activate -r <owner>/sandbox-demo
```

Then the launcher:

```bash
cat ~/sandbox-demo/.flox/cache/vercel-sandbox-launch.mjs
```

```javascript
#!/usr/bin/env node
// Generated by `flox activate --sandbox --sandbox-backend vercel-sandbox`.
import { Sandbox } from "@vercel/sandbox";

const BOOTSTRAP = "#!/usr/bin/env bash\n...";

async function main() {
  const sandbox = await Sandbox.create({
    name: "flox-sandbox-demo",
    runtime: "node24",
    timeout: 300000,
  });
  await sandbox.writeFiles([
    { path: "flox-bootstrap.sh", content: Buffer.from(BOOTSTRAP) },
  ]);
  const run = await sandbox.runCommand({
    cmd: "bash", args: ["flox-bootstrap.sh"],
    stdout: process.stdout, stderr: process.stderr,
  });
  await sandbox.stop();
  process.exit(run.exitCode);
}
main().catch((err) => { console.error(err); process.exit(1); });
```

**"The launcher is real, and it is valid JavaScript — it creates a
fixed-runtime sandbox, uploads the flox bootstrap, and runs it.
Notice what is *not* there: no egress-policy argument, because the
SDK has none to set. That absence is the honest part, not an
oversight."**

---

## 2 · Push to FloxHub and launch (account + push beat)

**"With a Vercel account and the environment on FloxHub, this is the
whole remaining path."** On a credentialed operator's machine, with
`vercel env pull` done and `FLOX_SANDBOX_VERCEL_FLOXHUB_REF` set:

```bash
# Push the environment so the bootstrap can activate it:
flox push --owner <your-owner>

# Run the launcher flox generated (needs Node 22+):
node ~/sandbox-demo/.flox/cache/vercel-sandbox-launch.mjs
```

Vercel creates a remote Firecracker microVM on the fixed runtime,
the launcher uploads the bootstrap, the bootstrap installs Flox and
runs `flox activate -r <owner>/sandbox-demo` — output streams back
to your terminal.

> This beat requires a live Vercel account and a FloxHub push,
> neither of which this host has tonight. The generated launcher is
> exactly what runs; nothing is faked.

---

## 3 · Prove the boundary — and the honest network gap

**"The filesystem story is the strong one: the sandbox is a remote
microVM, so your laptop's filesystem is unreachable by
construction — no bind mount, nothing to leak."** The threat model
**inverts** here versus the local backends: the host filesystem is
invisible to the sandbox, but the code and any injected secrets run
in Vercel's cloud.

**"The network story is where flox is honest about a limit."** Add
a network grant to the manifest and re-activate:

```toml
[[options.sandbox.network]]
endpoint = "api.github.com:443"
```

```bash
flox edit                       # add the grant
flox deactivate && cd ~/sandbox-demo
```

flox refuses to proceed, naming the exact limitation:

```
The 'vercel-sandbox' backend cannot enforce network egress grants:
the @vercel/sandbox SDK has no per-sandbox egress allowlist or
firewall — its `ports` option governs INBOUND exposure only.
The manifest declares 1 [[options.sandbox.network]] grant(s) that
this backend cannot express, so they are declined rather than
silently ignored.
Remove the grants to run on Vercel Sandbox with its default network
posture, or select a backend with domain egress (e.g. 'openshell',
'e2b', or 'daytona').
```

**"This is the load-bearing honesty of the whole prototype: when a
provider cannot express what the manifest asks for, flox declines —
it never silently widens the grant to all-traffic, and never drops
it while pretending it was applied. Vercel Sandbox has no egress
vocabulary, so a grant is a hard stop, with a pointer to a backend
that does. That is why its capabilities row reads `domain-egress:
no`, unlike every other cloud backend here."**

Remove the grant to run on Vercel Sandbox's default posture.

---

## 4 · The determinism tradeoff — bootstrap vs bake

**"A bootstrap-shaped provider forces a choice the image backends
never face: how do you get a *locked* environment into a runtime you
cannot pre-seed with an image?"**

flox chose **FloxHub-remote activation**: the bootstrap runs
`flox activate -r <owner>/<env>`, pulling the environment from
FloxHub inside the sandbox. The tradeoff, stated in the generated
bootstrap itself:

```bash
# Determinism note: this activates the FloxHub-pushed revision, not a
# byte-for-byte closure captured at `flox activate --sandbox` time.
```

**"The reproducibility is bounded but real: the FloxHub revision is
itself a locked environment, and pushing it is one command. The
fully-deterministic alternative — copying the content-addressed store
closure to an artifact store the sandbox pulls from — needs plumbing
this prototype does not have yet. That is the next piece of design
this backend surfaces: a shared 'bootstrap bundle' stage the whole
bootstrap tier would share."**

---

## 5 · Run a coding agent (account + push beat)

**"A coding agent running in Vercel's cloud microVM — the sandbox,
not the agent, is the boundary, and the boundary is remote and
ephemeral."**

Once the sandbox is live (beat 2), the in-sandbox activation is a
full flox environment, so the agent runs the same as anywhere:

```bash
claude --permission-mode auto
```

```
> add a docstring to greet() in app.py and commit the change
```

Because the sandbox is remote and ephemeral, the blast radius of
anything the agent does is a Vercel microVM that stops on exit.

> Agent auth (`CLAUDE_CODE_OAUTH_TOKEN`) must be injected into the
> sandbox as a Vercel environment variable — the remote guest has no
> access to your laptop's `.env`. This is the credential-leaves-the
> -laptop tradeoff the inverted threat model names. And with no
> egress allowlist, the agent's outbound traffic runs under Vercel's
> default network posture, which flox does not control — a point to
> raise honestly with any security-minded audience.

---

## 6 · Exit — the sandbox is remote and ephemeral

With account + push, the launcher calls `sandbox.stop()` when the
activation exits, so nothing lingers on Vercel and nothing ran on
your laptop.

On this host tonight, there is nothing to tear down — no sandbox was
launched. The only local artifacts are the generated
`vercel-sandbox-bootstrap.sh` and `vercel-sandbox-launch.mjs`, both
removed by cleanup.

---

## 7 · Reset

```bash
bash demo/cleanup.sh
```

Removes the demo env, fixtures, and the generated
`vercel-sandbox-*` artifacts under `.flox/cache/`. (Any sandboxes
launched on Vercel are ephemeral and already stopped; there is no
image to prune because nothing was baked. If you pushed the env to
FloxHub, `flox delete --owner <owner> --remote sandbox-demo` prunes
it.)

> Integration notes for the Vercel conversation (fixed-runtime
> bootstrap shape, no image ingestion on the stock-runtime path, no
> egress vocabulary in the SDK, the FloxHub-remote determinism
> tradeoff, and the shared "bootstrap bundle" stage this backend
> asks for): the backend module docs at
> `cli/flox/src/commands/sandbox_backends/vercel_sandbox.rs` and the
> backend contract at
> `slices/2026/06-sandboxed-activation-prototype/artifacts/backend-contract.md`.
