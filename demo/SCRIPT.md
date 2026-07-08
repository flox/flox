# Demo: `flox activate --sandbox` (prototype)

A ~7-minute single-terminal walkthrough. **Bold** lines are what
to *say*; fenced blocks are what to *type*. Every command and its
output below was verified on macOS (arm64) against this prototype
at HEAD b0ebb29de.

## Setup

Run `bash demo/setup.sh` once from the dev shell. The script
creates the demo env, installs tools, seeds the project, and
pre-allows auto-activation. The epilogue prints the three lines
you must have set in your presentation shell:

```bash
alias flox='$FLOX_BIN'            # the prototype binary
export FLOX_FEATURES_SANDBOX_ACTIVATE=true
export FLOX_FEATURES_AUTO_ACTIVATE=true
```

Also ensure the prompt hook is in your shell's RC file:
```bash
eval "$(flox hook-env --shell bash --shell-pid $$)"
```

> `FLOX_BIN` is the prototype binary in `target/debug/`. The alias
> makes `flox` the prototype for the whole presentation without
> requiring PATH manipulation.

---

## Framing (~20s)

**"AI agents can do real damage — delete files, leak secrets,
call out to the network. Flox can now wrap an activation in a
sandbox so anything running inside — including a coding agent —
is contained. And with two manifest lines and a `cd`, the whole
thing is locked down by default."**

---

## 0 · Zero-friction lockdown — the opening beat (~60s)

**"Here's the pitch in one command. The manifest declares the
sandbox — just two lines — and auto-activation does the rest.
Watch what happens when I `cd` into the project."**

First, show the manifest declaration (add it via `flox edit`):

```bash
cd ~/sandbox-demo
flox edit
```

In the editor, add these two lines to the `[options]` section
and save:

```toml
[options]
sandbox = "enforce"
sandbox-backend = "oci"
```

**"Now, just `cd` away and back."**

```bash
cd /tmp && cd ~/sandbox-demo
```

Expected (piped through the `script` harness for capture;
the live terminal shows the same):

```
Enter '/Users/you/sandbox-demo' (sandboxed via oci)? [Y/n]
```

Type `Y`. The consent prompt is the gate for entering a
sandboxed session — the manifest tells Flox to wrap in OCI,
and the hook asks once per visit. The session runs as a
foreground child of your shell: when it ends you are back at
your own prompt, and the hook stays quiet until you leave the
directory and return. Accept:

```
? OCI image 'sandbox-demo:<hash12>' is stale (environment has
  changed since last bake).
  Existing image: sandbox-demo:latest
  Bake now? (~2–5 min on first bake; later bakes reuse layers) (Y/n)
```

On a **fresh environment** (first bake), accept the bake prompt
and wait ~2–5 min. Subsequent entries find the cached image and
launch in under a second. The builder is pinned to a
prototype-branch rev and compiles inside the builder VM against
a persistent `flox-nix` cache volume — the very first bake on a
machine (cold volume) also pays that one-time compile; every
bake after reuses it. After the bake (or with a warm image):

```
✔ You are now using the environment 'sandbox-demo'
To stop using this environment, run 'flox deactivate'

flox [sandbox-demo] bash-5.3# uname -sm
Linux aarch64
flox [sandbox-demo] bash-5.3# flox deactivate
```

…and you land back at your own shell prompt. Inside the guest,
`flox` is a minimal shim: `flox deactivate` ends the session
(so the banner's instruction is true, mirroring subshell
activations); any other subcommand prints a notice and returns
127 — the full CLI is not present in the image. `exit` works
too.

**"One `cd`. Consent — so an in-progress agent can't silently
enter a sandboxed session — and you're in a Linux micro-VM with
only the project mounted. That's the pitch."**

> **Capture recipe (validated on this host):**
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

> **`ℹ️  This environment declares a libsandbox sandbox;
> in-place auto-activation is not mediated — run 'flox activate
> --sandbox <MODE> --dir <path>' for a sandboxed session.`** —
> this notice appears for environments that declare a libsandbox
> backend. libsandbox is advisory, so in-place activation is not
> blocked; the notice points you at the explicit form.

### Auto-bake valves

**Non-tty / CI — never stall on a prompt:**

```bash
FLOX_SANDBOX_OCI_AUTOBAKE=true \
  flox activate --sandbox enforce --sandbox-backend oci -- uname -sm
```

```
⚙️  Baking OCI image 'sandbox-demo:<hash12>' (builder pin: <rev>)...
   First bake downloads the builder image and cross-compiles the
   environment closure (~2–5 min).
   Subsequent bakes reuse layers and are faster.
✅  Image 'sandbox-demo:<hash12>' loaded into container store.
Linux aarch64
```

**Stale image — run the newest existing image offline:**

```bash
FLOX_SANDBOX_OCI_ALLOW_STALE=1 \
  flox activate --sandbox enforce --sandbox-backend oci -- uname -sm
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
FLOX_SANDBOX_OCI_IMAGE=sandbox-demo:latest \
  flox activate --sandbox enforce --sandbox-backend oci -- uname -sm
```

> **How the bake handles the prototype-only manifest fields:** the
> builder receives a *sanitized* view of the environment —
> `options.sandbox` and `options.sandbox-backend` are stripped
> before anything reaches the in-container flox or the image. The
> sandbox declaration is a host-side concern; the image is the
> *inside* of the boundary and never carries it. No overrides or
> valves are needed to bake a sandbox-declaring environment; the
> image tag stays keyed to your real (unsanitized) lockfile. If a
> future manifest schema outruns the frozen builder pin, the bake
> fails fast with a one-line schema-preflight error naming
> `_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV` as the override.

---

## 1 · `warn` — see what your agent touches (~40s)

**"`warn` blocks nothing. It just reports every file and network
access outside the policy — a way to learn what your workload
actually needs before you lock it down."**

> The §1–§3 beats use `--sandbox-backend libsandbox` explicitly to
> override the manifest's `oci` default. libsandbox is advisory
> and provides `warn` and `prompt` modes; the OCI backend is
> `enforce`-only. Moving the demo dir from `/tmp` to `$HOME` has
> no effect on the warn output — the sensitive-path and network
> policy is identical regardless of where the project lives.

```bash
flox activate --sandbox warn --sandbox-backend libsandbox -- bash -c '
  cat ~/demo-secrets/.env >/dev/null    # reads a secret
  curl -sI https://example.com >/dev/null   # calls the network
  echo "agent ran fine"
'
```

Expected (PIDs vary run to run):

```
SANDBOX WARNING[cat:818]: /Users/you/demo-secrets/.env is not in the sandbox (sensitive)
SANDBOX WARNING[curl:821]: connect to example.com:443 (2606:4700:10::6814:179a) is not in the network policy
agent ran fine
```

**"The agent ran fine — nothing was blocked — but we can see it
touched a secret and reached the network, and every report names
the process that did it. Notice it even flags the secret as
`sensitive`. The secret lives outside the project on purpose —
that's exactly what the sandbox is for."**

---

## 2 · `enforce` — lock it down (~110s)

**"Now `enforce`. The default policy is tuned so a coding agent
still works out of the box — but the dangerous things are
blocked."**

### 2a — the agent works, zero friction

```bash
flox activate --sandbox enforce --sandbox-backend libsandbox -- bash -c '
  echo "    return 2" >> app.py        # edit a project file
  git commit -aqm "agent: tweak greet" # commit
  git log --oneline | head -1
  curl -sI https://github.com >/dev/null && echo "github: reachable"
  echo "agent work: done"
'
```

Expected (no SANDBOX lines; git hash varies):

```
57cdd57 agent: tweak greet
github: reachable
agent work: done
```

**"It edited the project, committed, and reached GitHub — with
zero prompts and zero denials. The default policy already allows
your project directory, the Nix store, and the common package
registries and git hosts."**

### 2b — but the blast radius is contained

```bash
# read a credential:
flox activate --sandbox enforce --sandbox-backend libsandbox -- \
  bash -c 'cat ~/demo-secrets/.env'
# write outside the project:
flox activate --sandbox enforce --sandbox-backend libsandbox -- \
  bash -c 'echo pwned > ~/sbx-pwned.txt'
# reach an un-approved host:
flox activate --sandbox enforce --sandbox-backend libsandbox -- \
  bash -c 'curl -sI https://example.com'
```

Expected (PIDs and IP addresses vary; curl prints one line per
address it tries, so the count varies with DNS):

```
SANDBOX ERROR[cat:957]: /Users/you/demo-secrets/.env is not in the sandbox (sensitive)
cat: /Users/you/demo-secrets/.env: Permission denied

SANDBOX ERROR[bash:983]: /Users/you/sbx-pwned.txt is not in the sandbox
bash: line 1: /Users/you/sbx-pwned.txt: Permission denied

SANDBOX ERROR[curl:1017]: connect to example.com:443 (2606:4700:10::ac42:93f3) is not in the network policy
SANDBOX ERROR[curl:1017]: connect to example.com:443 (104.20.23.154) is not in the network policy
```

> **`$HOME` vs `/tmp` policy note.** The demo project is now in
> `~/sandbox-demo` (not `/tmp/sandbox-demo`). libsandbox
> always-allows `/tmp` as a built-in prefix, so the old demo in
> `/tmp` would not have blocked writes to `~/sbx-pwned.txt`.
> Moving to `$HOME` means the project dir is NOT always-allowed —
> it is granted through the default-seed — and writes outside it
> (to `~/sbx-pwned.txt`) are correctly blocked. The demo behavior
> is unchanged from the audience's perspective; it is actually more
> faithful now.

**"Reading a secret — blocked. Writing a file outside the
project — blocked, and the denial is graceful: the command gets
`Permission denied`, your shell survives. Calling an unapproved
host — blocked. The agent edits your code and uses the network
it needs, but it can't exfiltrate secrets, trash your home
directory, or phone home somewhere you didn't allow."**

---

## 3 · `prompt` — tighten interactively (~110s)

**"`enforce` is great once you know your policy. `prompt` is how you
get there: when something's blocked, instead of just failing, the
request is queued and you decide — once, or forever. `prompt` is the
default: bare `--sandbox` means `--sandbox prompt`."**

### 3.1 — a legitimate access is denied and queued

```bash
flox activate --sandbox --sandbox-backend libsandbox -- \
  bash -c 'cat ~/demo-data/fixtures.csv'
```

Expected (PIDs vary):

```
ℹ Sandbox 'prompt' enabled (advisory; mediates file reads/writes).
  Out-of-policy access is denied and queued for approval.
    review queue:   flox sandbox
    approve a path: flox sandbox allow '<glob>'   (second terminal)
SANDBOX DENIED[cat:2086]: read /Users/you/demo-data/fixtures.csv (not in policy)
SANDBOX DENIED[cat:2086]: queued as req 1 — approve outside: flox sandbox
cat: /Users/you/demo-data/fixtures.csv: Permission denied
```

**"My agent needs a data file outside the project. Under `prompt` it
fails cleanly with a clear message — and it's queued for me to
approve. Approvals happen *outside* the session on purpose, so a
misbehaving agent can't approve itself."**

### 3.2 — approve it (persists for next time)

```bash
flox sandbox allow ~/demo-data/'**'
```

Expected:

```
✔ Saved grant '/Users/you/demo-data/**' to grants.toml — it applies at the next activation.
```

> If a `prompt` session is still running (e.g. you left 3.1's
> session open in another pane), the live broker answers instead:
> `✔ Saved grant '/Users/you/demo-data/**' (cleared 1 pending) —
> future sessions won't ask.` — and the grant reaches the running
> session within a few seconds.

### 3.3 — now it just works

```bash
flox activate --sandbox --sandbox-backend libsandbox -- \
  bash -c 'cat ~/demo-data/fixtures.csv'
```

Expected (the `prompt` banner always prints; no denials follow):

```
ℹ Sandbox 'prompt' enabled (advisory; mediates file reads/writes).
  Out-of-policy access is denied and queued for approval.
    review queue:   flox sandbox
    approve a path: flox sandbox allow '<glob>'   (second terminal)
order_id,amount
1001,42
```

### 3.4 — the policy is inspectable

```bash
flox sandbox list
```

Expected:

```
Saved grants for /Users/you/sandbox-demo/.flox
(/Users/you/sandbox-demo/.flox/cache/sandbox/grants.toml — edit by hand or flox sandbox allow|revoke)

  PATTERN                          OPS    SOURCE              ADDED       EVIDENCE
  /Users/you/demo-data/**          any    allow               2026-07-08  manual
  default-seed: 31 grants — use --all to show

Sensitive (never auto-granted, never folded into a directory grant):
  /Users/you/.ssh/** /Users/you/.aws/** /Users/you/.gnupg/** /Users/you/.kube/** /Users/you/.netrc /Users/you/.config/gh/** **/.env* **/.flox/cache/sandbox/**

21 saved filesystem grant(s) use 21 of 256 allow entries (0.6 of 16 KB); network grants are uncapped.
ℹ OPS is informational; saved grants allow all access kinds in this prototype.
```

**"One grant, and the data file is allowed forever — saved to a
plain, hand-editable file you can inspect. The `default-seed` row
is the out-of-box policy itself — git hosts, package registries,
your shell dotfiles, even flox's own metrics endpoint — every
implicit allowance is a visible, revocable grant; `--all` expands
them. Over a session or two the agent zeroes in on exactly the
policy it needs."**

### 3.5 — (say it, optionally show it) the agent can't approve itself

```bash
flox activate --sandbox --sandbox-backend libsandbox -- \
  bash -c 'flox sandbox allow /tmp/anything'
```

Expected (after the `prompt` banner; requires the prototype `flox`
first on PATH inside the session):

```
✘ ERROR: refusing to allow from inside the sandboxed session.
  Run it from another terminal: flox sandbox allow '<glob>'
```

---

## 4 · Backends — same UI, different isolation (~90s)

**"Everything above runs on libsandbox — the advisory loader
interposer that ships today. The same `flox sandbox` policy layer
sits over pluggable backends."**

```bash
flox sandbox backends
```

```
BACKEND       BOUNDARY     MACOS    LINUX     ENFORCES  LIVE-ASK  STATUS
libsandbox    advisory     native   native    no        yes       implemented
nix           host-kernel  native   native    yes       no        scaffolded
host-native   host-kernel  native   native    yes       no        implemented
srt           host-kernel  native   native    yes       yes       implemented
oci           container    linux-vm  native    yes       no        implemented
libkrun       hypervisor   linux-vm  native    yes       no        planned

Select a backend with FLOX_SANDBOX_BACKEND=<name>; the default is 'libsandbox'.
Only 'implemented' backends are wired into activation today.
```

> **`warn` and `prompt` are libsandbox-only.** They are advisory
> semantics — observe-but-allow, and deny-then-live-redeem — that
> only the loader interposer can provide. The enforcing backends
> implement **`enforce` only**; asking them for `warn` or `prompt`
> errors with a clear message rather than silently enforcing.

### `host-native` — the macOS kernel sandbox (no setup)

```bash
# advisory libsandbox: a system /bin/cat escapes the loader →
FLOX_SANDBOX_BACKEND=libsandbox flox activate --sandbox enforce -- \
  /bin/cat ~/.ssh/id_ed25519        # → prints the key (escaped)

# host-native: the kernel denies it →
flox activate --sandbox enforce --sandbox-backend host-native -- \
  /bin/cat ~/.ssh/id_ed25519        # → cat: ...: Operation not permitted
```

```bash
flox activate --sandbox warn --sandbox-backend host-native -- true
```

```
✘ ERROR: Sandbox backend 'host-native' enforces; it has no advisory 'warn' mode.
Use '--sandbox enforce' with this backend, or '--sandbox-backend libsandbox' for advisory 'warn'.
```

> `host-native` is **deny-by-default for your home directory**: it
> denies reading or writing all of `$HOME` except the project and
> Flox's own state. System and Nix reads stay open so flox runs.

### `srt` — Anthropic's sandbox-runtime (install: `flox install sandbox-runtime`)

```bash
flox activate --sandbox enforce --sandbox-backend srt -- \
  cat ~/.ssh/id_ed25519
# → cat: ...: Operation not permitted
```

Like host-native, srt is `enforce`-only and rejects `warn`/`prompt`
the same way. Two known rough edges: srt grants blanket write to
`/tmp`; a dev `flox` binary under `$HOME` can't be re-exec'd by
the deny-`$HOME` profile.

### `oci` — Apple Container (macOS 26+ / Apple silicon)

The manifest already declares `sandbox-backend = "oci"`, so
`flox activate --sandbox enforce` uses OCI by default. The
opening beat showed the consent-via-auto-activation path; here
is the explicit form:

```bash
flox activate --sandbox enforce --sandbox-backend oci -- uname -sm
# Linux aarch64
```

> Warm latency is ~0.7–1.0 s per run — the VM-boot tax.

**Isolation: the host filesystem is invisible** — only the project
directory is mounted (live, at its real path):

```bash
flox activate --sandbox enforce --sandbox-backend oci -- \
  ls /Users/you/.ssh
# ls: cannot access '/Users/you/.ssh': No such file or directory

flox activate --sandbox enforce --sandbox-backend oci -- \
  cat /Users/you/demo-secrets/.env
# cat: /Users/you/demo-secrets/.env: No such file or directory
```

**The project is live-mounted — reads and writes round-trip:**

```bash
flox activate --sandbox enforce --sandbox-backend oci -- cat app.py
# def greet():
#     return 1

flox activate --sandbox enforce --sandbox-backend oci -- \
  sh -c 'echo "# edited in guest" >> app.py'
tail -1 app.py
# # edited in guest                 ← the edit landed on the host
```

**OCI consent semantics (opening beat vs explicit form):**
- **Auto-activation** (`cd ~/sandbox-demo`): the prompt hook shows
  the consent prompt from ADR-006, then runs the sandboxed
  session in the foreground; when it ends you return to your
  shell, and the directory is session-suppressed until you leave
  and re-enter. A prior allow/deny for the env's auto-activation
  has no effect; entering a sandboxed session always re-consents.
- **Explicit form** (`flox activate --sandbox enforce --sandbox-backend oci -- cmd`):
  no consent prompt. Use this in scripts, CI, and agents.

Like the other enforcing backends, `oci` rejects `warn` and `prompt`
with the same message shape as host-native.

> **Historical note:** an earlier version of this demo claimed
> live-mounted projects were broken on macOS (DEV-130). That was a
> misdiagnosis — the reads always worked; the "empty read" symptom
> was the container-entrypoint argv re-expansion bug, since
> reframed in DEV-130 and fixed (flox/flox#4464, cherry-picked
> onto this branch).

> **Caveats.** (1) **OS swap:** the guest is Linux, so an
> interactive macOS user runs Linux packages. (2) **Bind-mount I/O
> has a measured shape:** per-file open round-trip over virtio-fs
> is ~0.15 ms; small-file traversal (`node_modules`-class) runs
> ~6× native, ~60× guest-local; streaming is fine. Numbers:
> `results/bindmount-io-macos-arm64-2026-07-07.md`.

### Selecting an unwired backend fails loudly, on purpose

```bash
flox activate --sandbox enforce --sandbox-backend nix -- true
```

```
✘ ERROR: Sandbox backend 'nix' is not yet wired into activation.
Wired backends: 'libsandbox' (default), 'host-native', 'srt', and 'oci'. Run 'flox sandbox backends' to see status, or unset FLOX_SANDBOX_BACKEND.
```

---

## 5 · Close (~20s)

**"So: two manifest lines plus a `cd` equals locked-down-by-default.
`warn` to observe what your agent touches, `enforce` to lock it
down, `prompt` to tighten the policy interactively. It's a
prototype — advisory on the libsandbox tier, structural on OCI —
but for the 'don't let my agent wreck my laptop' problem, it's
already useful today."**

```bash
bash demo/cleanup.sh   # afterwards, off-camera
```

> The benchmark data across all backends (startup, workload I/O,
> isolation red-team, DX parity) lives in the Forge slice:
> `slices/2026/06-sandboxed-activation-prototype/artifacts/`.

---

## Optional advanced beat — live approve-and-continue (needs a 2nd pane)

The single-terminal flow above approves *between* runs. The
broker also supports approving a **live, running** session: the
agent's blocked call is redeemed on its next retry, no restart.

Terminal A (leave it running):

```bash
flox activate --sandbox --sandbox-backend libsandbox
# inside the session:
cat ~/demo-data/fixtures.csv      # → denied + queued (req 1)
```

Terminal B:

```bash
cd ~/sandbox-demo
flox sandbox            # interactive review → approve req 1
```

Terminal A — run it again; it now succeeds. (A grant pushed to a
live session takes effect within a few seconds.)

---

## Spurious PID error in pty harnesses

During `script`-based capture of the consent + OCI boot sequence,
you may see:

```
✘ ERROR: PID <n> is not attached to the activation
```

This is a benign artifact of the `detach` call in `hook-env`'s
deactivation path running against the activation's state
directory. The error does not affect the activation flow — the
container boots and the `uname -sm` output is correct. Omit it
from projected captures but do not attempt to suppress it by
editing the binary.

If this error reproduces reproducibly in a real terminal session
(not a pty harness), report it as a bug with exact repro steps.
