# Demo: `flox activate --sandbox` (prototype)

A ~5-minute single-terminal walkthrough. **Bold** lines are
roughly what to *say*; fenced blocks are what to *type*. Every
command and its output below was verified on macOS (arm64)
against this prototype.

Prereqs (verified by `bash demo/setup.sh`, which creates
everything):

- your locally built `flox` (e.g. `target/debug`) is already
  first in PATH
- `FLOX_FEATURES_SANDBOX_ACTIVATE=true` is already exported in
  your shell

Run `bash demo/setup.sh` once, then:

```bash
cd /tmp/sandbox-demo
```

> The sandbox only mediates Nix-store / env-provided binaries.
> On macOS, system tools (`/usr/bin/curl`, `/bin/cat`) are
> SIP-protected and escape the loader, so the demo uses tools
> installed *into* the environment (`flox install …`, done by
> setup). For the same reason a sandboxed activation swaps a
> SIP-protected session shell (`/bin/zsh`) for the bash bundled
> with Flox — otherwise the shell's own redirections
> (`echo x > ~/file`) would escape the policy — and rewrites
> `SHELL` inside the session. Interactive sessions print an
> `ℹ Cannot mediate …` line explaining the swap; `-- CMD`
> invocations exec the command directly and stay quiet. That's
> an honest limitation, not a bug — call it out if asked.

> Expected blocks below are live captures with the username
> shown as `/Users/you`. PIDs in the `[exe:pid]` tags, resolved
> IPs, and git hashes vary run to run.

---

## 0 · Framing (~20s)

**"AI agents can do real damage — delete files, leak secrets,
call out to the network. Flox can now wrap an activation in a
sandbox so anything you run inside it — including a coding agent
— is contained. There are three modes: `warn` to observe,
`enforce` to lock down, and `prompt` to decide interactively."**

---

## 1 · `warn` — see what your agent touches (~40s)

**"`warn` blocks nothing. It just reports every file and network
access outside the policy — a way to learn what your workload
actually needs before you lock it down."**

```bash
flox activate --sandbox warn -- bash -c '
  cat ~/demo-secrets/.env >/dev/null    # reads a secret
  curl -sI https://example.com >/dev/null   # calls the network
  echo "agent ran fine"
'
```

Expected:

```
SANDBOX WARNING[cat:41624]: /Users/you/demo-secrets/.env is not in the sandbox (sensitive)
SANDBOX WARNING[curl:41625]: connect to example.com:443 (2606:4700:10::6814:179a) is not in the network policy
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
flox activate --sandbox enforce -- bash -c '
  echo "    return 2" >> app.py        # edit a project file
  git commit -aqm "agent: tweak greet" # commit
  git log --oneline | head -1
  curl -sI https://github.com >/dev/null && echo "github: reachable"
  echo "agent work: done"
'
```

Expected (no SANDBOX lines; hash varies):

```
<hash> agent: tweak greet
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
flox activate --sandbox enforce -- bash -c 'cat ~/demo-secrets/.env'
# write outside the project:
flox activate --sandbox enforce -- bash -c 'echo pwned > ~/sbx-pwned.txt'
# reach an un-approved host:
flox activate --sandbox enforce -- bash -c 'curl -sI https://example.com'
```

Expected (each blocked; curl prints one line per address it
tries, so the count varies with DNS):

```
SANDBOX ERROR[cat:41648]: /Users/you/demo-secrets/.env is not in the sandbox (sensitive)
cat: /Users/you/demo-secrets/.env: Permission denied

SANDBOX ERROR[bash:41657]: /Users/you/sbx-pwned.txt is not in the sandbox
bash: line 1: /Users/you/sbx-pwned.txt: Permission denied

SANDBOX ERROR[curl:41667]: connect to example.com:443 (2606:4700:10::ac42:93f3) is not in the network policy
SANDBOX ERROR[curl:41667]: connect to example.com:443 (104.20.23.154) is not in the network policy
```

**"Reading a secret — blocked. Writing a file outside the
project — blocked, and the denial is graceful: the command gets
`Permission denied`, your shell survives. Calling an unapproved
host — blocked. The agent edits your code and uses the network
it needs, but it can't exfiltrate secrets, trash your home
directory, or phone home somewhere you didn't allow. The same
holds inside an interactive session — the session shell itself
is mediated, so even a bare `echo pwned > ~/file` at the prompt
is denied."**

---

## 3 · `prompt` — tighten interactively (~110s)

**"`enforce` is great once you know your policy. `prompt` is how you
get there: when something's blocked, instead of just failing, the
request is queued and you decide — once, or forever. `prompt` is the
default: bare `--sandbox` means `--sandbox prompt`."**

### 3.1 — a legitimate access is denied and queued

```bash
flox activate --sandbox -- bash -c 'cat ~/demo-data/fixtures.csv'
```

Expected:

```
ℹ Sandbox 'prompt' enabled (advisory; mediates file reads/writes).
  Out-of-policy access is denied and queued for approval.
    review queue:   flox sandbox
    approve a path: flox sandbox allow '<glob>'   (second terminal)
SANDBOX DENIED[cat:41681]: read /Users/you/demo-data/fixtures.csv (not in policy)
SANDBOX DENIED[cat:41681]: queued as req 1 — approve outside: flox sandbox
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
flox activate --sandbox -- bash -c 'cat ~/demo-data/fixtures.csv'
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
Saved grants for /private/tmp/sandbox-demo/.flox
(/private/tmp/sandbox-demo/.flox/cache/sandbox/grants.toml — edit by hand or flox sandbox allow|revoke)

  PATTERN                          OPS    SOURCE              ADDED       EVIDENCE
  /Users/djsauble/Code/flox/target/debug/** any    allow               2026-06-12  manual
  /Users/you/demo-data/**          any    allow               2026-06-12  manual
  default-seed: 31 grants — use --all to show

Sensitive (never auto-granted, never folded into a directory grant):
  /Users/you/.ssh/** /Users/you/.aws/** /Users/you/.gnupg/** /Users/you/.kube/** /Users/you/.netrc /Users/you/.config/gh/** **/.env* **/.flox/cache/sandbox/**

22 saved filesystem grant(s) use 22 of 256 allow entries (0.6 of 16 KB); network grants are uncapped.
ℹ OPS is informational; saved grants allow all access kinds in this prototype.
```

**"One grant, and the data file is allowed forever — saved to a
plain, hand-editable file you can inspect. The `default-seed` row
is the out-of-box policy itself — git hosts, package registries,
your shell dotfiles, even flox's own metrics endpoint — every
implicit allowance is a visible, revocable grant; `--all` expands
them. (The `target/debug` grant is my dev-build convenience —
setup added it so the prototype's own binaries run quietly inside
the session.) Over a session or two the agent zeroes in on
exactly the policy it needs, and you never had to turn the
sandbox off."**

### 3.5 — (say it, optionally show it) the agent can't approve itself

```bash
flox activate --sandbox -- bash -c 'flox sandbox allow /tmp/anything'
```

Expected (after the `prompt` banner):

```
✘ ERROR: refusing to allow from inside the sandboxed session.
  Run it from another terminal: flox sandbox allow '<glob>'
```

---

## 4 · Close (~20s)

**"So: `warn` to learn, `enforce` to lock down with a default
that keeps agents productive, and `prompt` to tighten the policy
interactively. It's a prototype — it's advisory, not bulletproof:
it covers cooperative, dynamically-linked tools, not static
binaries or system binaries that bypass the loader, and file
*metadata* (stat) isn't mediated yet. But for the 'don't let my
agent wreck my laptop' problem, it's already useful today."**

```bash
bash demo/cleanup.sh   # afterwards, off-camera
```

---

## Optional advanced beat — live approve-and-continue (needs a 2nd pane)

The single-terminal flow above approves *between* runs. The
broker also supports approving a **live, running** session: the
agent's blocked call is redeemed on its next retry, no restart.

Terminal A (leave it running):

```bash
flox activate --sandbox
# on macOS the session swaps to the Flox-bundled bash and says so:
#   ℹ Cannot mediate '/bin/zsh' inside the sandbox; using the bash
#     bundled with Flox for this session.
# inside the session:
cat ~/demo-data/fixtures.csv      # → denied + queued (req 1)
```

Terminal B:

```bash
cd /tmp/sandbox-demo
flox sandbox            # interactive review → approve req 1
```

Terminal A — run it again; it now succeeds. (A grant pushed to a
live session takes effect within a few seconds, so an agent's
own retry loop just works.)

---

## Backends — same UI, different isolation (experimental seam)

Everything above runs on **`libsandbox`**, the advisory loader
interposer that ships today. It is one enforcement mechanism, not
the only one. The same modes and the same `flox sandbox` UI are
designed to sit over *pluggable* backends — kernel sandboxes,
containers, micro-VMs — so we can benchmark performance,
isolation, and DX and pick a default. The backend is chosen with
the `FLOX_SANDBOX_BACKEND` environment variable.

List the roster and what each one can (claim to) do:

```bash
flox sandbox backends
```

```
BACKEND       BOUNDARY     MACOS    LINUX     ENFORCES  LIVE-ASK  STATUS
libsandbox    advisory     native   native    no        yes       implemented
nix           host-kernel  native   native    yes       no        scaffolded
host-native   host-kernel  native   native    yes       no        scaffolded
srt           host-kernel  native   native    yes       yes       scaffolded
oci           container    linux-vm  native    yes       no        scaffolded
libkrun       hypervisor   linux-vm  native    yes       no        planned

Select a backend with FLOX_SANDBOX_BACKEND=<name>; the default is 'libsandbox'.
Only 'implemented' backends are wired into activation today.
```

The default backend reproduces today's behavior — the whole demo
above is `FLOX_SANDBOX_BACKEND=libsandbox`:

```bash
FLOX_SANDBOX_BACKEND=libsandbox flox activate --sandbox enforce -- \
  bash -c 'cat ~/demo-secrets/.env'      # → blocked, as in §2b
```

Selecting a backend that is **not yet wired** fails loudly,
on purpose — it never silently falls back to libsandbox (that
would make a benchmark lie about which mechanism it measured):

```bash
FLOX_SANDBOX_BACKEND=host-native flox activate --sandbox enforce -- true
```

```
❌ ERROR: Sandbox backend 'host-native' is not yet wired into activation.
Only 'libsandbox' (the default) is implemented. Run 'flox sandbox backends'
to see status, or unset FLOX_SANDBOX_BACKEND.
```

As each backend lands, the same command starts working with no
change to the surface above. The benchmark harness that scores
the three tradeoffs across every backend lives in the Forge
slice: `slices/2026/06-sandboxed-activation-prototype/artifacts/`
(`benchmark-plan.md`, `backend-roster.md`, `backend-contract.md`,
`red-team-battery.md`, and the runnable `bench/` scripts).

