# Demo: `flox activate --sandbox` (prototype)

A ~5-minute single-terminal walkthrough. **Bold** lines are
roughly what to *say*; fenced blocks are what to *type*. Every
command and its output below was verified on macOS (arm64)
against this prototype. Run `bash demo/setup.sh` first, then in
the same dev shell:

```bash
alias flox="$FLOX_BIN"
export FLOX_FEATURES_SANDBOX_ACTIVATE=true
cd ~/sandbox-demo
```

> The sandbox only mediates Nix-store / env-provided binaries.
> On macOS, system tools (`/usr/bin/curl`, `/bin/cat`) are
> SIP-protected and escape the loader, so the demo uses tools
> installed *into* the environment (`flox install …`, done by
> setup). That's an honest limitation, not a bug — call it out
> if asked.

---

## 0 · Framing (~20s)

**"AI agents can do real damage — delete files, leak secrets,
call out to the network. Flox can now wrap an activation in a
sandbox so anything you run inside it — including a coding agent
— is contained. There are three modes: `warn` to observe,
`enforce` to lock down, and `ask` to decide interactively."**

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
SANDBOX WARNING: /Users/you/demo-secrets/.env is not in the sandbox (sensitive)
SANDBOX WARNING: connect to example.com:443 (...) is not in the network policy
agent ran fine
```

**"The agent ran fine — nothing was blocked — but we can see it
touched a secret and reached the network. Notice it even flags
the secret as `sensitive`."**

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

Expected (no SANDBOX lines):

```
3ee3353 agent: tweak greet
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

Expected (each blocked):

```
SANDBOX ERROR: /Users/you/demo-secrets/.env is not in the sandbox (sensitive)
SANDBOX ERROR: /Users/you/sbx-pwned.txt is not in the sandbox
SANDBOX ERROR: connect to example.com:443 (...) is not in the network policy
```

**"Reading a secret — blocked. Writing a file outside the
project — blocked. Calling an unapproved host — blocked. The
agent edits your code and uses the network it needs, but it
can't exfiltrate secrets, trash your home directory, or phone
home somewhere you didn't allow."**

---

## 3 · `ask` — tighten interactively (~110s)

**"`enforce` is great once you know your policy. `ask` is how you
get there: when something's blocked, instead of just failing, the
request is queued and you decide — once, or forever."**

### 3.1 — a legitimate access is denied and queued

```bash
flox activate --sandbox ask -- bash -c 'cat ~/demo-data/fixtures.csv'
```

Expected:

```
SANDBOX DENIED: read /Users/you/demo-data/fixtures.csv (not in policy)
SANDBOX DENIED: queued as req 1 — approve outside: flox sandbox
cat: /Users/you/demo-data/fixtures.csv: Permission denied
```

**"My agent needs a data file outside the project. Under `ask` it
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

### 3.3 — now it just works, silently

```bash
flox activate --sandbox ask -- bash -c 'cat ~/demo-data/fixtures.csv'
```

Expected:

```
order_id,amount
1001,42
```

### 3.4 — the policy is inspectable

```bash
flox sandbox list
```

Expected (excerpt):

```
  PATTERN                    OPS    SOURCE     ADDED       EVIDENCE
  /Users/you/demo-data/**    any    allow      2026-06-12  manual
Sensitive (never auto-granted, never folded into a directory grant):
  ~/.ssh/** ~/.aws/** ... **/.env ...
```

**"One grant, and the data file is allowed forever — saved to a
plain, hand-editable file you can inspect. Over a session or two
the agent zeroes in on exactly the policy it needs, and you never
had to turn the sandbox off."**

### 3.5 — (say it, optionally show it) the agent can't approve itself

```bash
flox activate --sandbox ask -- bash -c 'flox sandbox allow /tmp/anything'
```

Expected:

```
✘ ERROR: refusing to allow from inside the sandboxed session.
  Run it from another terminal: flox sandbox allow '<glob>'
```

---

## 4 · Close (~20s)

**"So: `warn` to learn, `enforce` to lock down with a default
that keeps agents productive, and `ask` to tighten the policy
interactively. It's a prototype — it's advisory, not bulletproof:
it covers cooperative, dynamically-linked tools, not static
binaries or system binaries that bypass the loader. But for the
'don't let my agent wreck my laptop' problem, it's already
useful today."**

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
flox activate --sandbox ask
# inside the session:
cat ~/demo-data/fixtures.csv      # → denied + queued (req 1)
```

Terminal B:

```bash
cd ~/sandbox-demo
flox sandbox            # interactive review → approve req 1
```

Terminal A — run it again; it now succeeds. (A grant pushed to a
live session takes effect within ~2 seconds, so an agent's own
retry loop just works.)
