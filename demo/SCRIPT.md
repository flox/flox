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
the only one. The same `flox sandbox` UI is designed to sit over
*pluggable* backends — kernel sandboxes, containers, micro-VMs —
so we can benchmark performance, isolation, and DX and pick a
default.

> **`warn` and `prompt` are libsandbox-only.** They are *advisory*
> semantics — observe-but-allow, and deny-then-live-redeem through the
> broker — that only the loader interposer can provide. The enforcing
> backends below (kernel / container / micro-VM) implement **`enforce`
> only**; asking them for `warn` or `prompt` errors with a clear message
> rather than silently enforcing (see the host-native note). So the
> three-mode walkthrough in §1–§3 is a *libsandbox* demo; on the other
> backends, use `--sandbox enforce`.

List the roster and what each one can (claim to) do:

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

The default backend reproduces today's behavior — the whole demo
above is `FLOX_SANDBOX_BACKEND=libsandbox`. Select a backend three
ways, in precedence order: the `--sandbox-backend` flag, the
`FLOX_SANDBOX_BACKEND` env var, or `options.sandbox-backend` in the
manifest (a project default) — e.g. `[options]` `sandbox-backend =
"host-native"`.

The headline tradeoff (macOS-arm64, warm `p50` startup): `libsandbox`
**52 ms** · `host-native` **72 ms** · `srt` **111 ms** · `oci` (Apple
Container) **668 ms**. Full three-axis numbers — startup, workload I/O,
the isolation red-team, and DX parity — are in the Forge slice's
`results/` (see the closing pointer).

### `host-native` — the macOS kernel sandbox (no setup)

`host-native` needs nothing installed: it wraps the whole activation in
the macOS kernel sandbox (`sandbox-exec`), built into the OS. Unlike
advisory `libsandbox`, it contains even SIP-protected system binaries —
the exact gap §4 admitted. Watch the *same* read get blocked where
`libsandbox` lets it through:

```bash
# advisory libsandbox: a system /bin/cat escapes the loader →
FLOX_SANDBOX_BACKEND=libsandbox flox activate --sandbox enforce -- \
  /bin/cat ~/.ssh/id_ed25519        # → prints the key (escaped)

# host-native: the kernel denies it →
flox activate --sandbox enforce --sandbox-backend host-native -- \
  /bin/cat ~/.ssh/id_ed25519        # → cat: ...: Operation not permitted
```

**`host-native` is `enforce`-only.** A `sandbox-exec` profile can only
allow or deny — there is no advisory "log-but-allow," and host-native has
no broker — so `warn` and `prompt` are rejected up front instead of
silently locking things down:

```bash
flox activate --sandbox warn --sandbox-backend host-native -- true
```

```
✘ ERROR: Sandbox backend 'host-native' enforces; it has no advisory 'warn' mode.
Use '--sandbox enforce' with this backend, or '--sandbox-backend libsandbox' for advisory 'warn'.
```

> `host-native` is **deny-by-default for your home directory**: on an
> allow-default base it denies reading the contents of — and writing
> to — all of `$HOME` except the project and Flox's own state. So an
> arbitrary file like `~/Documents/notes` is blocked too, not just the
> known credential paths, and `.env` files stay secret even inside the
> project. System and Nix reads (outside `$HOME`) stay open so flox
> runs. The red-team battery confirms it contains every filesystem
> attack — reads (incl. SIP `/bin/cat`), overwrites, and new-file
> creation. A full-filesystem deny-default (also locking `/tmp` and
> other users' homes) is a further follow-up; the current lossiness is
> what `flox sandbox backends` declares.

### `srt` — Anthropic's sandbox-runtime (setup: install the tool)

**Setup.** `srt` is a third-party tool that must be on PATH:

```bash
flox install sandbox-runtime    # provides `srt`
```

It drives the *same* kernel boundary (Seatbelt on macOS / bubblewrap on
Linux) on **both** platforms and adds default-deny TCP egress that
`host-native` doesn't. Flox generates an srt policy mirroring the
deny-`$HOME` shape and re-execs under it:

```bash
flox activate --sandbox enforce --sandbox-backend srt -- cat ~/.ssh/id_ed25519
# → cat: ...: Operation not permitted
```

Like host-native, `srt` is **`enforce`-only** here: it rejects `warn` and
`prompt` the same way. (Its `flox sandbox backends` row shows `LIVE-ASK
yes` — srt *can* adjudicate live in principle, but flox's broker is not
wired to it in this prototype, so `prompt` is not offered yet.)

Because its TCP egress is default-deny, activate a **realized**
environment under it (cold catalog fetches would otherwise be blocked).
Two known rough edges the red-team surfaced: srt's generated settings
grant **blanket write to `/tmp`** (a file dropped there is not
contained — to be tightened), and a dev `flox` binary that lives under
`$HOME` can't be re-exec'd by the deny-`$HOME` profile (a real
`/nix/store` install is outside `$HOME` and unaffected).

### `oci` — Apple Container (macOS 26+): a real micro-VM

`oci` is the container/micro-VM tier, now **wired into the seam** —
`flox activate --sandbox enforce --sandbox-backend oci -- CMD` works
like any other backend. It gives the strongest filesystem isolation
in the roster: the host home is simply *absent* in the guest. The
runtime dependency is Apple Container alone — Apple's open-source
tool on the OS's own Virtualization framework, no Docker, no Podman,
no daemon.

**Setup (macOS 26+ / Apple silicon only).**

```bash
brew install container
container system kernel set --recommended
container system start
```

**The image bakes itself.** In this model *the image is the
environment*: the backend runs your containerized env with the
project live-mounted. Images are content-addressed to the lockfile
(`<env>:<hash12>`, with a `latest` convenience alias): the first
activation offers to bake, and after a `flox install` the hash
moves, so the next activation detects the drift and offers a
rebake. The whole build runs on Apple Container — no Docker, no
Podman, no skopeo, at build time or run time:

```bash
flox activate --sandbox enforce --sandbox-backend oci -- true
```

```
? OCI image 'sandbox-demo:a7f880489710' is stale (environment has
  changed since last bake).
  Existing image: sandbox-demo:latest
  Bake now? (~2–5 min on first bake; later bakes reuse layers) (Y/n)
```

Non-interactive contexts (CI, agents) never stall on a prompt —
they fail fast with guidance unless explicitly opted in:

```bash
FLOX_SANDBOX_OCI_AUTOBAKE=true \
  flox activate --sandbox enforce --sandbox-backend oci -- uname -sm
```

```
⚙️  Baking OCI image 'sandbox-demo:a7f880489710' …
   First bake downloads the builder image and cross-compiles the
   environment closure (~2–5 min).
✅  Image 'sandbox-demo:a7f880489710' loaded into container store.
Linux aarch64
```

Escape hatches, all loud: `FLOX_SANDBOX_OCI_ALLOW_STALE=1` runs the
newest existing image with a warning naming the expected tag
(offline / mid-iteration); `FLOX_SANDBOX_OCI_IMAGE=<ref>` pins an
explicit image and bypasses staleness entirely; and the manual
pipeline is now two commands
(`flox containerize --runtime container -f img.tar` +
`container image load --input img.tar`).

> Until flox/flox#4464 merges, bake with
> `_FLOX_CONTAINERIZE_FLAKE_REF_OR_REV=3b4774070ce0a804acf7da299940725454b19d64`
> exported so the image entrypoint carries the argv exec-semantics
> fix — images baked from the default builder pin re-introduce the
> extra expansion pass. Drop this note once the fix is on `main`
> and the pin advances.

**Run it — same surface, micro-VM boundary:**

```bash
flox activate --sandbox enforce --sandbox-backend oci -- uname -sm
# Linux aarch64
```

Warm latency is ~0.7–1.0 s per run — the VM-boot tax (vs ~72 ms
host-native). Command argv reaches the guest **verbatim** (the
image entrypoint carries the flox/flox#4464 exec-semantics fix):

```bash
flox activate --sandbox enforce --sandbox-backend oci -- \
  sh -c 'for x in 1 2 3; do echo "x=$x"; done; echo "shell=$0"'
# x=1
# x=2
# x=3
# shell=sh
```

**Isolation: the host filesystem is invisible** — only the project
directory is mounted (live, at its real path); everything else on
the host simply does not exist in the guest:

```bash
flox activate --sandbox enforce --sandbox-backend oci -- ls /Users/you/.ssh
# ls: cannot access '/Users/you/.ssh': No such file or directory
flox activate --sandbox enforce --sandbox-backend oci -- cat /Users/you/demo-secrets/.env
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

Like the other enforcing backends, `oci` is **`enforce`-only** —
`warn` and `prompt` are rejected with the same message shape as
host-native.

> Historical note: an earlier version of this demo claimed
> live-mounted projects were broken on macOS (**DEV-130**,
> https://linear.app/floxdotdev/issue/DEV-130). That was a
> misdiagnosis — the reads always worked; the "empty read" symptom
> was the container-entrypoint argv re-expansion bug, since
> reframed in DEV-130 and fixed (flox/flox#4464, cherry-picked
> onto this branch). Images baked before the fix (e.g. the old
> `octest:latest`) still carry the old entrypoint and its extra
> expansion pass — rebake to clear it.

> **Caveats.** Two big ones. (1) **OS swap:** the guest is Linux, so
> an interactive macOS user is running Linux packages, not their host
> tools. (2) **Bind-mount I/O has a measured shape:** the per-file
> open round-trip over virtio-fs is ~0.15 ms, so small-file traversal
> (`node_modules`-class) runs ~6× native and ~60× guest-local — and
> warm ≈ cold, caching doesn't rescue it — while streaming is fine
> (64 MB write+read in ~55 ms). Posture: live-mount project *source*,
> keep dependency trees guest-local (volume or image layer). Numbers:
> the Forge slice's `results/bindmount-io-macos-arm64-2026-07-07.md`.
> Plus the smaller ones: the ~0.7–1.0 s per-run VM boot above
> (measured 0.708 s wall on a cache-hit activation), and bakes run
> against a cold Nix store every time (no cache volume yet — a
> tracked follow-up on flox/flox#4466), so a rebake costs minutes,
> not seconds. Drift itself is no longer a caveat: the lockfile
> hash catches it and the backend offers the rebake.

### Selecting an unwired backend fails loudly, on purpose

A backend that is not wired into activation never silently falls back to
`libsandbox` — that would make a benchmark lie about which mechanism it
measured. `nix` (the Nix build sandbox as an activation backend) is
scaffolded but not yet wired, so:

```bash
flox activate --sandbox enforce --sandbox-backend nix -- true
```

```
✘ ERROR: Sandbox backend 'nix' is not yet wired into activation.
Wired backends: 'libsandbox' (default), 'host-native', 'srt', and 'oci'. Run 'flox sandbox backends' to see status, or unset FLOX_SANDBOX_BACKEND.
```

(`libkrun` prints the same way — the remaining micro-VM tier lands
behind the seam the same way `oci` did.)

As each backend lands, the same `flox activate` command starts working
with no change to the surface above. The benchmark harness that scores
the three tradeoffs across every backend lives in the Forge slice:
`slices/2026/06-sandboxed-activation-prototype/artifacts/`
(`benchmark-plan.md`, `backend-roster.md`, `backend-contract.md`,
`red-team-battery.md`, the runnable `bench/` scripts, and the
`results/` dataset).

