# Demo: `flox activate --sandbox` — the OCI backend (prototype)

A single-terminal walkthrough of the end-to-end OCI sandbox: `cd`
into a project, consent, and land in a Linux micro-VM where your
declared services are already running and the only files that exist
are your project and the tools you installed — then run a coding
agent at full autonomy inside a boundary it cannot cross.

**Bold** lines are what to *say*; fenced blocks are what to *type*.
This script covers the OCI backend only. The full user-facing
sandbox reference (all backends, modes, policy) is the man page:
`flox-sandbox(1)` (`cli/flox/doc/flox-sandbox.md`).

**Verification status:** all beats verified live on macOS (arm64,
Apple Container 1.1.0) against this prototype (2026-07-09): services
auto-start on entry, the boundary holds, and in-guest file changes
persist to the host. The guest now carries a **real** flox binary —
`flox list`, `flox services`, and `flox deactivate` all work inside
the sandbox.

---

## 0 · Setup

Run `bash demo/setup.sh` once from the dev shell. It creates the demo
env (git, curl, which, python3, and `flox/claude-code`), declares the
OCI sandbox and an auto-starting web service in the manifest, seeds
the project (`app.py`, `index.html`), and pre-allows auto-activation.

Then, in your presentation shell:

```bash
alias flox='$FLOX_BIN'                 # the prototype binary
export FLOX_FEATURES_SANDBOX_ACTIVATE=true
export FLOX_FEATURES_AUTO_ACTIVATE=true
export GITHUB_TOKEN=ghp-demo-FAKE      # for the token-isolation beat
export _FLOX_CONTAINERIZE_FLAKE_REF_OR_REV=$(git -C /path/to/flox rev-parse origin/prototype/sandboxed-activation)
```

Ensure the prompt hook is in your shell's RC:

```bash
eval "$(flox hook-env --shell bash --shell-pid $$)"
```

> `_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV` pins the in-VM builder to the
> prototype revision. A `target/debug` build reports a plain release
> version and would otherwise pick the release-tag builder, which
> lacks the guest-flox/services work. A nix-built prototype flox
> (dev version) picks it up automatically and needs no override.

**Pre-bake off-camera.** The first bake takes ~2–5 min. Do it before
the demo so `cd` drops you in in about a second:

```bash
FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate -- true
```

---

## 1 · Auto-activate a sandboxed environment, services and all

**"The manifest declares the sandbox and a web service. One `cd`,
one `Y`, and I'm inside a Linux micro-VM with my project mounted and
my service already running — no extra commands."**

```bash
cd /tmp && cd ~/sandbox-demo
```

The consent prompt is the gate — the manifest tells Flox to wrap the
activation in OCI, and the hook asks once per visit, so an
in-progress agent can't silently enter a sandboxed session for you:

```
Enter '/Users/you/sandbox-demo' (sandboxed via oci)? [Y/n]
```

Type `Y`. The session runs as a foreground child of your shell; when
it ends you are back at your own prompt.

```
✔ You are now using the environment 'sandbox-demo'
To stop using this environment, run 'flox deactivate'

flox [sandbox-demo] bash-5.3# uname -sm
Linux aarch64
```

**"The declared service came up on its own."**

```
flox [sandbox-demo] bash-5.3# flox services status
NAME       STATUS       PID
web        Running       53

flox [sandbox-demo] bash-5.3# curl -s localhost:8080
<!doctype html><title>sandbox-demo</title>
<h1>Hello from inside the flox sandbox</h1>
```

**"`[services].auto-start` did that — the same lifecycle you get on
the host, now inside the sandbox. `flox services start/stop/logs`
all work here too."**

---

## 2 · Prove the boundary is intact

**"Three properties, checked by hand. My filesystem is invisible, my
credentials don't cross, and only my project is live."**

Still inside the guest:

```bash
flox [sandbox-demo] bash-5.3# ls /Users/you/.ssh
ls: cannot access '/Users/you/.ssh': No such file or directory

flox [sandbox-demo] bash-5.3# cat /Users/you/demo-secrets/.env
cat: /Users/you/demo-secrets/.env: No such file or directory

flox [sandbox-demo] bash-5.3# printenv GITHUB_TOKEN
flox [sandbox-demo] bash-5.3#
```

**"Not 'permission denied' — *No such file or directory*. The host
filesystem isn't there to be reached, and no host environment
variable crosses: no GitHub token, no SSH agent, no cloud
credentials. An agent in here acts anonymously unless I hand it a
secret on purpose."**

---

## 3 · Run a coding agent, at full autonomy

**"This is why we built it: a coding agent with no permission
prompts, that I don't have to trust — because the sandbox, not the
agent, is the boundary."**

`flox/claude-code` is baked into the image; the env hook points
`CLAUDE_CONFIG_DIR` into the project (the one writable place that
survives between the ephemeral containers, so login and settings
persist). After a one-time `claude` login (off-camera — the guest
has no browser, so it prints a URL you open on the host and paste the
code back), run it at full autonomy:

```bash
flox [sandbox-demo] bash-5.3# claude --permission-mode auto
```

Give it real work:

```
> add a docstring to greet() in app.py and commit the change
```

Claude edits `app.py` and commits — no per-action prompts.

> Auto mode gives the agent the run of the sandbox with no per-action
> prompts. The guest runs as root, and Claude Code refuses
> `--dangerously-skip-permissions` for root, so auto mode is the
> demo-appropriate equivalent. We don't need to ask the agent to try
> to escape — §2 already showed, by hand, that the secrets simply
> aren't reachable.

---

## 4 · Exit the sandbox — the work persists

**"The project is mounted live at its real path, so everything the
agent did is already on my disk. Watch."**

```bash
flox [sandbox-demo] bash-5.3# flox deactivate
```

You land back at your own shell. On the host:

```bash
git -C ~/sandbox-demo log --oneline -1
# <hash> add docstring to greet()          ← the agent's commit

tail -3 ~/sandbox-demo/app.py
# def greet():
#     """..."""                            ← the edit is here
```

**"The commit and the edit are on my host repo. Everything else the
container wrote died with it — only the project directory round-trips.
An agent with its own guardrails off did real work in my repo, and
the worst it could do was confined to the one directory I gave it."**

---

## 5 · Reset

```bash
bash demo/cleanup.sh
```

> Benchmark data (startup, workload I/O, isolation red-team, DX
> parity) lives in the Forge slice:
> `slices/2026/06-sandboxed-activation-prototype/artifacts/`.
