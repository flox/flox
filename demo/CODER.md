# Demo: `flox activate --sandbox` — the Coder backend (prototype)

`cd` into a project and land in a **Coder** workspace: only your
project and your tools exist, your declared services are already
running, and a coding agent runs at full autonomy inside a container
boundary it cannot cross — all driven by a self-hosted Coder control
plane running on your laptop.

**Bold** lines are what to *say*; fenced blocks are what to *type*.
The OCI-backend walkthrough is `demo/SCRIPT.md`; the OpenShell one is
`demo/OPENSHELL.md`. All three share `demo/setup.sh` and
`demo/cleanup.sh`.

**The pitch:** flox already bakes each environment into an OCI image.
This backend hands that image to Coder as a workspace template — flox
brings the reproducible environment, Coder brings a self-hostable
control plane for launching and managing it. Same manifest, one word
changed: `backend = "coder"`. Coder's open-source core is AGPL-3.0
and needs no API key — the whole loop runs locally.

> **Status: Scaffolded — honest about the wall.** On a laptop with
> Docker and the setup env running, flox bakes the image, pushes the
> generated docker-provider template, and **creates the workspace
> container from the baked flox image** (verified end-to-end). The one
> step that does not complete is the final `coder ssh` exec: Coder's
> stock workspace-agent init script assumes `grep`/`head`/`wget` on the
> container `PATH`, and the flox bake's compat layer provides `/bin/sh`
> and the `sandbox` user but not coreutils on that `PATH` — so the
> agent exits before registering and there is no connected agent to
> attach to. `coder create --yes` waits for the agent and then errors,
> which is where the backend honestly bails. The fix is a coreutils
> compat layer in the bake (tracked as an open question); the template,
> push, and workspace-create story is real today.

---

## 0 · Setup

### One-time host prerequisites

1. **Docker Desktop** (or Docker Engine) running — the workspace runs
   in a Linux container via Coder's `docker` provider.
2. **Coder control plane + presentation shell** — one command, in your
   presentation shell (export `FLOX_BIN` from the dev shell first):

   ```bash
   flox activate -r djsauble/coder-setup
   ```

   This is the demo's *outer layer* — one setup env per sandbox
   backend is the plan. It installs `coder` 2.x (Flox Catalog),
   runs `coder server` as a flox service on `127.0.0.1:3000` (with a
   built-in PostgreSQL under the env cache — the first start downloads
   it from Maven), bootstraps the first user and logs the CLI in
   **non-interactively** (no browser), and configures the shell:
   feature flags and the planted `GITHUB_TOKEN` (`[vars]`),
   `FLOX_VERSION` plus a `flox` alias from `$FLOX_BIN` (`[profile]`),
   and the `~/demo-secrets` fixture. Deactivating removes the planted
   secret (`[profile.deactivate]`). Stay in this activation for the
   whole demo; confirm:

   ```bash
   coder whoami            # prints the logged-in user
   coder templates list    # empty until beat 1 pushes one
   ```

   > Details, caveats, and troubleshooting:
   > `demo/coder-setup/README.md`.

### Demo environment

Run once from the dev shell:

```bash
BACKEND=coder bash demo/setup.sh
```

Same demo env as the OCI / OpenShell walkthroughs (git, curl, which,
python3, `flox/claude-code`, an auto-starting web service, seeded
`app.py` / `index.html`); the manifest declares `backend = "coder"`.

> **No network grants here.** Coder is a *control plane*: it delegates
> egress enforcement to the underlying runtime, and the local `docker`
> provider has no L7 domain-egress vocabulary. flox therefore
> **declines** any `[[options.sandbox.network]]` grant on this backend
> rather than pretend to enforce it — so the demo project declares
> none. For enforced per-endpoint egress, that's the `openshell`
> backend (see `demo/OPENSHELL.md` beats 2–3).

The setup env already configured your shell — just make sure the
prompt hook is in your shell's RC:

```bash
eval "$(flox hook-env --shell bash --shell-pid $$)"
```

The session is *layered*: the setup env is the outer layer, and beat
1's `cd` activates the project env on top of it. Cleanup is symmetric
— deactivate the sandbox, then the setup env.

**Pre-bake off-camera.** The first bake takes ~5–15 min on a machine
that has to compile the pinned flox rev in-VM, or ~2–5 min if the pin
is in the flox cache. Later bakes reuse the builder's store:

```bash
cd ~/sandbox-demo && FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate -- true
```

The image lands in Docker as `sandbox-demo-coder:<hash12>`,
content-addressed to the lockfile — it rebakes only when the
environment actually changes.

---

## 1 · Auto-activate into a Coder workspace

**"One `cd`, one `Y`, and flox baked my environment, pushed it to
Coder as a workspace template, launched a workspace, and dropped me
inside — my project mounted, my service already running."**

```bash
cd /tmp && cd ~/sandbox-demo
```

```
Enter '/Users/you/sandbox-demo' (sandboxed via coder)? [Y/n]
```

Type `Y`. flox bakes the image (first run), generates a docker-provider
Terraform template under `.flox/cache/coder-template/main.tf`, pushes it
(`coder templates push`), and creates a workspace (`coder create`). In a
**second terminal on the host** — the control plane — watch the
workspace materialize:

```bash
coder list
WORKSPACE                 TEMPLATE            STATUS   HEALTHY
flox-sandbox-demo-coder-# flox-sandbox-demo-coder  Started  false

docker ps
# coder-flox-flox-sandbox-demo-coder-#  sandbox-demo-coder:<hash12>  Up
```

**"flox baked my environment, pushed it to Coder as a template, and
Coder stood up a Docker workspace from that exact image — the whole
lifecycle on a self-hosted control plane, zero cloud account."**

> **The honest wall (Scaffolded).** `HEALTHY` reads `false` and the
> `cd` does not drop you into a shell: `coder create` is still waiting
> for the workspace **agent** to connect, and it never will on a
> flox-baked image. Coder's stock agent init script (the container
> entrypoint) does `coder --version | grep Coder` before registering,
> but the flox bake's compat layer gives the container `/bin/sh` and
> the `sandbox` user — not `grep`/`head`/`wget` on the default `PATH`.
> The init script hits `grep: command not found`, exits, and the agent
> never comes up. `coder create` times out and flox bails there.
>
> What IS proven, live: the image bakes, the generated template
> validates through Coder's Terraform provisioner, and the workspace
> **container runs from the baked flox image** (you can `docker exec`
> into it and see your closure). The missing piece is a coreutils
> compat layer in the bake so Coder's agent script can run — an open
> question, not a design flaw. Until then, show the container and the
> template on the control plane; the in-workspace shell is the beat
> that waits on the fix.

---

## 2 · Prove the boundary is intact

> Beats 2–5 land you *inside* the workspace, which needs the agent
> connected — so they assume the coreutils compat-layer fix from beat 1
> is in place. You can still prove the boundary **today** without the
> agent by `docker exec`-ing into the running workspace container
> directly (`docker exec -it coder-flox-flox-sandbox-demo-coder-# sh`),
> which is exactly the filesystem the agent would land you in. The
> narration below is written for the connected-agent shell.

**"My filesystem is invisible and my host credentials don't cross —
the workspace is the container, not my machine."**

In the **host terminal** — a real (planted) secret, seeded by the setup
layer (and by `setup.sh` on the manual path):

```bash
ls -a ~/demo-secrets/
# .  ..  .env
```

Inside the guest, the directory doesn't exist — `$HOME` is outside the
project mount, so it simply isn't there:

```bash
flox [sandbox-demo] bash-5.3$ ls -a /Users/you/demo-secrets/
ls: cannot access '/Users/you/demo-secrets/': No such file or directory

flox [sandbox-demo] bash-5.3$ printenv GITHUB_TOKEN
flox [sandbox-demo] bash-5.3$
```

**"The container is a fresh Linux root filesystem — the baked flox
closure plus whatever the template mounts. Nothing from my host `HOME`
crosses. The boundary is the container the docker provider runs, and
Coder is the control plane that stood it up."**

> **On egress, be honest.** Coder does not enforce a network policy on
> this backend: it delegates to the docker provider, which has no L7
> egress vocabulary, so outbound from the workspace follows the
> container's default (open). flox *declines* network grants here
> rather than fake enforcement — that's why the demo manifest declares
> none. If the pitch calls for *enforced, per-endpoint* egress with a
> live audit log, switch to `demo/OPENSHELL.md` (its gateway ships an
> L7 proxy). Naming this ceiling out loud is the honest move — the
> division of labor is "flox defines what the environment is, Coder
> launches and manages it," and egress enforcement is a separate
> backend's job.

---

## 3 · Rebuild the workspace from a changed environment

**"Change the manifest, and the workspace image changes with it —
Coder rebuilds from the new template, deterministically."**

Coder's redemption path is a *rebuild*, not a live policy edit (there
is no per-request ask surface here). Install a tool on the host:

```bash
flox [sandbox-demo]$ exit          # back to the host shell
flox install jq -d ~/sandbox-demo
```

Re-activate — flox re-bakes to a new `sandbox-demo-coder:<hash12>`,
re-pushes the template, and recreates the workspace from it:

```bash
cd /tmp && cd ~/sandbox-demo       # 'Y' again
flox [sandbox-demo] bash-5.3$ command -v jq
/nix/store/…-jq-1.x/bin/jq
```

**"Same content-addressed image contract as every flox sandbox: the
tag is the lockfile hash, so the workspace only rebuilds when the
environment actually changes — and when it does, Coder gets a new
template version, fully reproducible."**

On the host, the new template version is visible:

```bash
coder templates versions list flox-sandbox-demo
```

---

## 4 · Run a coding agent, at full autonomy

**"A coding agent with no permission prompts, that I don't have to
trust — the container, not the agent, is the boundary."**

> **Authenticate before the demo.** In-guest login is a dead end (the
> OAuth URL can't be copied out of a workspace session). On the
> **host**:
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
> The file lives under the project — the one directory shared into the
> workspace — so beat 2's isolation story is unchanged. (The guest runs
> unprivileged, so `claude --dangerously-skip-permissions` also works.)

With the token pre-seeded, start the agent:

```bash
flox [sandbox-demo] bash-5.3$ claude --permission-mode auto
```

Give it real work:

```
> add a docstring to greet() in app.py and commit the change
```

Claude edits `app.py` and commits — no per-action prompts. The
container is the boundary: anything it touches is confined to the
workspace filesystem and the shared project mount.

> Egress caveat holds here too: the workspace can reach the network by
> default (no L7 policy on this backend). If the threat model requires
> *governed* agent egress, that's the `openshell` backend.

---

## 5 · Exit the workspace — the work persists, the workspace doesn't

```bash
flox [sandbox-demo] bash-5.3$ flox deactivate
```

You land back at your own shell. On the host:

```bash
git -C ~/sandbox-demo log --oneline -1
# <hash> add docstring to greet()          ← the agent's commit

coder list
# (the demo workspace, ready to delete)
```

**"The commit is on my host repo — the project was mounted live. The
workspace is on Coder's control plane, mine to keep or delete.
Reproducible environment in, governed session out."**

> Workspaces are not auto-deleted (Coder keeps them for reuse). The
> reset step below removes the demo workspace and template.

---

## 6 · Reset

Deactivate the setup layer (its `profile.deactivate` removes the
planted secret; the server + built-in DB stop with the activation),
then:

```bash
bash demo/cleanup.sh
```

Removes the env, fixtures, Docker-side `sandbox-demo-coder:*` images,
any `flox-sandbox-demo` Coder workspaces and templates, and the
`coder-setup` server state under the setup env cache.

> Coder's open-source core is AGPL-3.0, self-hostable, no API key: this
> whole demo runs on a laptop with Docker. For the customer story —
> Deutsche Telekom / T-Mobile run Coder for remote dev on self-hosted
> Kubernetes — the same template contract points at `kubernetes` /
> `envbuilder` providers instead of `docker`, with flox's baked image
> as the deterministic workspace substrate.
