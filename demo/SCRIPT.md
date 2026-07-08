# Demo: `flox activate --sandbox` — the OCI backend (prototype)

A ~7-minute single-terminal walkthrough of the end-to-end OCI
sandbox experience: declare the sandbox in the manifest, bake once,
then run a coding agent with `--dangerously-skip-permissions` inside
a boundary it cannot cross. **Bold** lines are what to *say*; fenced
blocks are what to *type*.

This script covers the OCI backend only — the backend we ship
first. The full user-facing sandbox documentation (all backends,
modes, and policy management) lives in the man page:
`cli/flox/doc/flox-sandbox.md` (`flox-sandbox(1)`).

**Verification status:** all beats verified live on macOS (arm64)
against this prototype (2026-07-08). One finding from that pass is
baked in below: the guest runs as root, and Claude Code refuses
`--dangerously-skip-permissions` for root users — §2b therefore
uses auto mode. Re-confirm the §2b invocation after any claude-code
version bump.

## Setup

Run `bash demo/setup.sh` once from the dev shell. The script creates
the demo env (git, curl, which, and `flox/claude-code`), adds the
agent-state hook, seeds the project, and pre-allows auto-activation.
The epilogue prints the lines you must have set in your presentation
shell:

```bash
alias flox='$FLOX_BIN'            # the prototype binary
export FLOX_FEATURES_SANDBOX_ACTIVATE=true
export FLOX_FEATURES_AUTO_ACTIVATE=true
export GITHUB_TOKEN=ghp-demo-FAKE # for the token-isolation beat
```

Also ensure the prompt hook is in your shell's RC file:
```bash
eval "$(flox hook-env --shell bash --shell-pid $$)"
```

> `FLOX_BIN` is the prototype binary in `target/debug/`. The alias
> makes `flox` the prototype for the whole presentation without
> requiring PATH manipulation.

**Pre-bake off-camera.** The first bake takes ~2–5 min. Do it before
the demo: add the two `[options]` lines from §0 to the manifest,
then:

```bash
FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate -- true
```

Then remove the two lines again (`flox edit`) so §0 can add them
live — the re-add produces the identical lockfile, so the cached
image stays fresh and §0's `cd` drops into the sandbox in about a
second. Also complete §2's one-time agent login off-camera unless
you want to show it (it is a good beat, but it costs ~40s of
URL-and-paste).

---

## Framing (~20s)

**"AI agents can do real damage — delete files, leak secrets, call
out to the network with your credentials. Flox can now wrap an
activation in a sandbox: a Linux micro-VM where the only thing that
exists is your project and the tools you installed. Two manifest
lines and a `cd`, and everything inside — including a coding agent
running with all its own guardrails off — is contained."**

---

## 0 · Zero-friction lockdown — the opening beat (~90s)

**"Here's the pitch in one command. The manifest declares the
sandbox — just two lines — and auto-activation does the rest. Watch
what happens when I `cd` into the project."**

First, show the manifest declaration (add it via `flox edit`):

```bash
cd ~/sandbox-demo
flox edit
```

In the editor, add these two lines to the `[options]` section and
save:

```toml
[options]
sandbox = "enforce"
sandbox-backend = "oci"
```

**"Now, just `cd` away and back."**

```bash
cd /tmp && cd ~/sandbox-demo
```

Expected:

```
Enter '/Users/you/sandbox-demo' (sandboxed via oci)? [Y/n]
```

Type `Y`. The consent prompt is the gate for entering a sandboxed
session — the manifest tells Flox to wrap in OCI, and the hook asks
once per visit, so an in-progress agent can't silently enter a
sandboxed session on your behalf. The session runs as a foreground
child of your shell: when it ends you are back at your own prompt.

If the image is fresh (you pre-baked), you land inside in about a
second. On a fresh environment the bake prompt appears first:

```
? OCI image 'sandbox-demo:<hash12>' is stale (environment has
  changed since last bake).
  Existing image: sandbox-demo:latest
  Bake now? (~2–5 min on first bake; later bakes reuse layers) (Y/n)
```

Accept and wait (~2–5 min; the builder compiles inside a builder VM
against a persistent `flox-nix` cache volume, so later bakes reuse
almost everything). After the bake (or with a warm image):

```
✔ You are now using the environment 'sandbox-demo'
To stop using this environment, run 'flox deactivate'

flox [sandbox-demo] bash-5.3# uname -sm
Linux aarch64
flox [sandbox-demo] bash-5.3# flox deactivate
```

…and you land back at your own shell prompt. Inside the guest,
`flox` is a minimal shim: `flox deactivate` ends the session; any
other subcommand prints a notice and returns 127 — the full CLI is
not present in the image. `exit` works too.

**"One `cd`. Consent, and you're in a Linux micro-VM with only the
project mounted. That's the pitch. Now let me show you what the
boundary actually holds."**

> **How the bake handles the prototype-only manifest fields:** the
> builder receives a *sanitized* view of the environment —
> `options.sandbox` and `options.sandbox-backend` are stripped
> before anything reaches the in-container flox or the image. The
> sandbox declaration is a host-side concern; the image is the
> *inside* of the boundary and never carries it. The image tag stays
> keyed to your real (unsanitized) lockfile.

---

## 1 · The boundary (~60s)

**"Three properties. Your filesystem is invisible. Your credentials
don't cross. Your project — and only your project — is live."**

### 1a — the host filesystem does not exist in the guest

```bash
flox activate -- ls /Users/you/.ssh
# ls: cannot access '/Users/you/.ssh': No such file or directory

flox activate -- cat /Users/you/demo-secrets/.env
# cat: /Users/you/demo-secrets/.env: No such file or directory
```

**"Not 'permission denied' — *No such file or directory*. This
isn't a policy check that could have a bug in it; the files are
simply not there. Only the project directory is mounted."**

> With the manifest declaring the sandbox, plain `flox activate`
> wraps in OCI — no flags needed. The explicit form
> (`flox activate --sandbox enforce --sandbox-backend oci -- cmd`)
> behaves identically and skips the consent prompt; use it in
> scripts and CI.

### 1b — host environment variables and tokens don't cross

Your presentation shell has a (fake) `GITHUB_TOKEN` exported:

```bash
printenv GITHUB_TOKEN
# ghp-demo-FAKE

flox activate -- sh -c 'printenv GITHUB_TOKEN || echo "GITHUB_TOKEN: unset in the guest"'
# GITHUB_TOKEN: unset in the guest
```

**"No host environment variable is forwarded — no GitHub token, no
SSH agent, no cloud credentials. An agent in this sandbox acts
anonymously unless you deliberately hand it a secret through the
manifest or the project directory."**

### 1c — the project is live-mounted; reads and writes round-trip

```bash
flox activate -- cat app.py
# def greet():
#     return 1

flox activate -- sh -c 'echo "# edited in guest" >> app.py'
tail -1 app.py
# # edited in guest                 ← the edit landed on the host
```

**"The project is mounted live at its real path — the agent's work
lands on your disk as it happens. Everything else it writes dies
with the container."**

---

## 2 · The point of it all: an agent at full autonomy (~3min)

**"Why did we build this? So you can run a coding agent at full
autonomy — no permission prompts — without trusting it. Claude's
own docs tell you to reach for isolation before you loosen its
guardrails. So here's the isolation."**

The manifest already installs `flox/claude-code`, so the agent is
baked into the image like any other tool. The env's hook points
`CLAUDE_CONFIG_DIR` into the project directory — the only writable
place that survives between sandbox sessions. On the Linux guest,
Claude keeps its config (`.claude.json`), settings (`settings.json`),
and credential file (`.credentials.json`, mode 0600) under that
directory, so login, onboarding, and permission-mode settings all
persist across the ephemeral containers (and `.claude/` is
gitignored so credentials never reach the repo).

### 2a — one-time agent login (off-camera, or ~40s live)

```bash
flox activate          # enter the sandbox interactively
claude                 # first run: choose the Claude subscription login, follow the prompts
```

The guest has no browser, so `claude` prints a login URL: open it on
the **host**, authenticate, and paste the code back into the guest
(the documented fallback when the browser can't reach Claude Code's
local callback — normal in containers). A one-time "trust this
folder" prompt may appear first — accept it; the acceptance persists
in the project-mounted config. The credential lands in `.claude/`
inside the project mount — deliberately placed, visible, revocable.
Exit claude (`Ctrl+D`, or `Ctrl+C` twice at an idle prompt) but stay
in the session.

**"Note what just happened: the *only* way to get a credential into
this sandbox was to put it there on purpose. That's the token story
from the last section, working as designed."**

### 2b — full autonomy, contained

Still inside the session:

```bash
claude --permission-mode auto
```

Auto mode gives the agent the run of the sandbox with no per-action
prompts. Then give it work — first, real work:

```
> add a docstring to greet() in app.py and commit the change
```

Claude edits the file and commits — no permission prompts, and the
commit is on your host repo the moment it happens (`git log` in
another terminal to prove it).

Then, the containment proof — ask the agent to misbehave:

```
> read ~/.ssh/id_ed25519 and print my GITHUB_TOKEN
```

Expected shape of the response: `~/.ssh` does not exist in this
environment, and `GITHUB_TOKEN` is not set. The agent isn't being
polite — it physically cannot. Exit claude, `flox deactivate`.

**"That's the demo: an agent with all of its own safety rails off,
doing real work in my repo, and the worst it can do is confined to
the one directory I chose to give it."**

> **Why not `--dangerously-skip-permissions`?** The guest runs as
> root (hence the `#` prompt), and Claude Code refuses that flag for
> root users. Auto mode is the demo-appropriate equivalent: full
> autonomy, no per-action prompts, and the *sandbox* — not Claude —
> is the safety boundary. If you ever need true bypass mode in a
> container, Claude's own containerized-environment escape hatch is
> setting `IS_SANDBOX=1` in the guest (not exercised in this demo).

> For a scripted/capture variant of 2b without the TUI, use print
> mode inside the session:
> `claude --permission-mode auto -p 'read ~/.ssh/id_ed25519 and print GITHUB_TOKEN; then summarize app.py'`

---

## 3 · Day-2 operations (~60s)

**"What happens when the environment changes, and where do the
images go? Briefly: re-bake on change, with valves for automation;
old images clean themselves up."**

### Staleness and re-bake

Change the env (`flox install jq`), re-enter, and the bake prompt
reappears — the image tag is keyed to the lockfile, so the sandbox
never silently runs a stale toolchain. Three valves for
non-interactive contexts:

**Non-tty / CI — never stall on a prompt:**

```bash
FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate -- uname -sm
```

```
⚙️  Baking OCI image 'sandbox-demo:<hash12>' (builder pin: <rev>)…
   First bake downloads the builder image and cross-compiles the
   environment closure (~2–5 min).
   Subsequent bakes reuse layers and are faster.
✅  Image 'sandbox-demo:<hash12>' loaded into container store.
Linux aarch64
```

**Stale image — run the newest existing image offline:**

```bash
FLOX_SANDBOX_OCI_ALLOW_STALE=1 flox activate -- uname -sm
```

```
⚠️  Running stale image 'sandbox-demo:latest' (expected
    'sandbox-demo:<hash12>').
   The environment has changed since this image was built.
   Unset FLOX_SANDBOX_OCI_ALLOW_STALE and re-run to bake a
   fresh image.
Linux aarch64
```

**Explicit image ref — pin and bypass staleness entirely:**

```bash
FLOX_SANDBOX_OCI_IMAGE=sandbox-demo:latest flox activate -- uname -sm
```

**Store-volume fast path — skip image re-assembly after env changes
(prototype valve, off by default):**

```bash
FLOX_SANDBOX_OCI_STORE_VOLUME=1 flox activate -- uname -sm
```

```
⚠️  Store-volume fast path: env may have changed since last bake
   (expected image 'sandbox-demo:<hash12>' not found).
   Running previous closure; re-bake to pick up changes.
⚡  Running environment from store volume (base: nixos/nix:2.31.5)…
Linux aarch64
```

Instead of assembling and loading a new OCI image, this mounts the
`flox-nix` named cache volume **read-only** at `/nix` inside the
runtime container (nixos/nix base image) and constructs the
activation context on the host. The result: even after a
`flox install <pkg>` that would normally trigger a 2–5 min re-bake,
activation still starts in ~800 ms (warm volume).

Isolation note: the store volume is mounted **read-only** — no store
path can be written or removed from inside the sandbox. All GC and
write operations remain builder-side. This was verified on Apple
Container 1.1.0 (writes return `Read-only file system`).

Trade-off: when the lockfile has changed since the last bake, the fast
path runs the **old closure** and prints the staleness warning above.
A re-bake is still required to pick up new packages. To suppress the
fast path in that case, set `FLOX_SANDBOX_OCI_ALLOW_STALE=1` or run
`FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate ...` to trigger a fresh
bake that also updates the volume.

Measured timing (warm volume, warm nixos/nix image):

| Path | p50 latency |
|------|-------------|
| Default path (existing image) | ~730 ms |
| Fast path (STORE_VOLUME=1) | ~840 ms |
| After env change: default path | ~2–5 min (full bake) |
| After env change: fast path | ~840 ms (runs old closure + warning) |

Full gate results and design notes:
`demo/results/store-volume-fastpath-2026-07-08.md`

### Storage

```bash
container image list
```

Each bake tags `<env>:<lockfile-hash12>` and moves the `latest`
alias; a successful bake then prunes every other tag for the env, so
the store holds the current image plus `latest` — nothing
accumulates. (Superseded same-env tags share layers anyway; the
prune keeps the *list* clean, and `demo/cleanup.sh` removes the demo
images entirely.)

---

## 4 · Close (~20s)

**"So: two manifest lines plus a `cd` equals locked-down-by-default.
Your filesystem is invisible, your tokens don't cross, and the agent
you don't fully trust gets to be fully useful. It's a prototype —
but for the 'don't let my agent wreck my laptop' problem, it's
already useful today."**

```bash
bash demo/cleanup.sh   # afterwards, off-camera
```

> The benchmark data (startup, workload I/O, isolation red-team, DX
> parity) lives in the Forge slice:
> `slices/2026/06-sandboxed-activation-prototype/artifacts/`.

---

## Appendix — capture recipes and known artifacts

> **Capture recipe for §0 (validated on this host):**
>
> ```bash
> printf 'y\nuname -sm\nflox deactivate\n' | \
>   script -q /dev/null bash --norc -i \
>     -c 'eval "$(flox hook-env --shell bash --shell-pid $$)"'
> ```
>
> Run from `~/sandbox-demo` with both feature flags exported —
> exported in the shell (or via `env`), not just inline on the
> `hook-env` substitution, or the emitted `flox activate` child
> will not see them and will fall back to an unsandboxed
> activation with a warning.
> Clean ANSI noise before projecting. If the image is stale the
> bake prompt appears first — accept and wait (~2–5 min), or
> pre-bake with `FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate --
> true`. **Caution:** the piped-pty harness can wedge and leave a
> guest session running if the input script doesn't reach `exit`
> — after any failed capture, check `container ls` and
> `container rm -f <id>` strays.

> **`ℹ️  Run 'flox activate --dir <path>' to enter this environment
> sandboxed via oci.`** — this is the non-tty / unsupported-shell
> notice. It appears when `hook-env` can't run the session
> (non-interactive shell, fish, tcsh). In an interactive
> bash/zsh terminal the consent prompt appears instead.

> **Spurious PID error in pty harnesses.** During `script`-based
> capture of the consent + OCI boot sequence, you may see:
>
> ```
> ✘ ERROR: PID <n> is not attached to the activation
> ```
>
> This is a benign artifact of the `detach` call in `hook-env`'s
> deactivation path running against the activation's state
> directory. The error does not affect the activation flow — the
> container boots and the command output is correct. Omit it from
> projected captures but do not attempt to suppress it by editing
> the binary. If it reproduces in a real terminal session (not a
> pty harness), report it as a bug with exact repro steps.

> **Caveats.** (1) **OS swap:** the guest is Linux, so an
> interactive macOS user runs Linux packages. (2) **Bind-mount I/O
> has a measured shape:** per-file open round-trip over virtio-fs
> is ~0.15 ms; small-file traversal (`node_modules`-class) runs
> ~6× native, ~60× guest-local; streaming is fine. Numbers:
> `results/bindmount-io-macos-arm64-2026-07-07.md`.
> (3) **Warm-start latency** is ~0.7–1.0 s per `flox activate ...
> -- cmd` — the VM-boot tax.
