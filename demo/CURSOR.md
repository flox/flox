# Demo: `flox activate --sandbox` — the Cursor backend (prototype)

`cd` into a project and land in an activation whose policy is
**compiled into Cursor's own agent sandbox**: Flox defines *what the
environment is* and *what it's allowed to do*, and Cursor's local
sandbox — the same OS boundary it re-skins (Seatbelt on macOS,
Landlock on Linux) — enforces it around its coding agent. Two policy
layers, stacked instead of fighting.

**Bold** lines are what to *say*; fenced blocks are what to *type*.
The OCI-backend walkthrough is `demo/SCRIPT.md`; the two share
`demo/setup.sh` and `demo/cleanup.sh`.

**The pitch:** every other cloud backend ships the environment *out*
to a remote runtime. Cursor is the opposite — it's **local**. Cursor
already runs a host-native agent sandbox; the gap is that its policy
and Flox's policy don't know about each other. This backend closes
that: flox compiles the manifest grants into Cursor's project
permission config, so the layers align. Same manifest, one word
changed: `backend = "cursor"`.

---

## 0 · Setup

### One-time host prerequisites

1. **Cursor's `agent` CLI.** This is the coding-agent CLI (not the
   editor). Install it:

   ```bash
   curl https://cursor.com/install -fsS | bash
   ```

   Then authenticate — either export a key or sign in once:

   ```bash
   export CURSOR_API_KEY=<your-key>   # or: agent   (one-time sign-in)
   ```

   > A Cursor account is required to *run* the agent. It is **not**
   > required to generate the policy config — the local beats below
   > work without it, and the launch boundary names the wall.

2. **The setup env** — one command, in your presentation shell
   (export `FLOX_BIN` from the dev shell first):

   ```bash
   flox activate -r djsauble/cursor-setup
   ```

   This is the demo's *outer layer* — one setup env per sandbox
   backend. It runs no service and installs no provider CLI (Cursor
   is local, and `agent` installs via curl, not the catalog). It
   configures the shell: feature flags and the planted `GITHUB_TOKEN`
   (`[vars]`), `FLOX_VERSION` plus a `flox` alias from `$FLOX_BIN`
   (`[profile]`), and the `~/demo-secrets` fixture. Deactivating
   removes the planted secret (`[profile.deactivate]`). Stay in this
   activation for the whole demo.

   > Details, caveats, and troubleshooting:
   > `demo/cursor-setup/README.md`.

### Demo environment

Run once from the dev shell:

```bash
BACKEND=cursor bash demo/setup.sh
```

Same demo env as the OCI walkthrough (git, curl, which, python3,
`flox/claude-code`, an auto-starting web service, seeded `app.py` /
`index.html`); the manifest declares `backend = "cursor"` plus a
network grant for the agent's API endpoint:

```toml
[[options.sandbox.network]]
endpoint = "api.anthropic.com:443"
```

flox compiles this into Cursor's project config
(`<project>/.cursor/cli.json`) at activation time. **Note the
lossiness up front:** Cursor's project config has no `<host>:<port>`
egress vocabulary — the closest native construct is
`WebFetch(<domain>)`, which allowlists the agent's *web-fetch tool*
by domain. So the grant compiles to `WebFetch(api.anthropic.com)`:
domain faithful, but port-blind and web-fetch-only. Anything that is
not a `:443` endpoint is *declined*, not silently widened.

The setup env already configured your shell — just make sure the
prompt hook is in your shell's RC:

```bash
eval "$(flox hook-env --shell bash --shell-pid $$)"
```

Unlike the cloud backends, **there is no bake.** Cursor runs locally,
so the first `cd` is instant — flox writes the config and stops at
the launch boundary.

---

## 1 · Auto-activate and compile the policy

**"One `cd`, and flox compiles this environment's policy straight
into Cursor's own sandbox config — no bake, nothing shipped
anywhere."**

```bash
cd /tmp && cd ~/sandbox-demo
```

```
Enter '/Users/you/sandbox-demo' (sandboxed via cursor)? [Y/n]
```

Type `Y`. flox compiles the manifest grants into Cursor's project
permission config and then stops — honestly — at the launch
boundary:

```
❌ ERROR: The 'cursor' sandbox backend aligns Flox's policy with Cursor's
agent sandbox, but Cursor exposes no launch API that runs the agent under a
config path — so flox cannot re-exec the activation under it.
flox compiled the manifest grants into Cursor's project permission config
(web-fetch allowed: api.anthropic.com) and wrote it to:
  /Users/you/sandbox-demo/.cursor/cli.json
The 'agent' CLI reads '<project>/.cursor/cli.json' implicitly; the 'agent' CLI
is installed, and CURSOR_API_KEY is set.
Run the agent yourself from the project to pick up the compiled policy, e.g.
'cd /Users/you/sandbox-demo && agent'.
```

**"That's the honest wall. Cursor's sandbox is configured through
settings, not a launch hook flox can drive — so flox does the part it
can do perfectly (compile the policy) and hands off the part it
can't (running the agent)."**

---

## 2 · Read the compiled policy

**"Here's the whole story: Flox's grants, expressed in Cursor's own
vocabulary."**

```bash
cat ~/sandbox-demo/.cursor/cli.json
```

```json
{
  "permissions": {
    "allow": [
      "Read(**)",
      "Write(**)",
      "WebFetch(api.anthropic.com)"
    ],
    "deny": [
      "Read(**/.env*)",
      "Write(**/.env*)",
      "Read(**/*.key)",
      "Write(**/*.key)"
    ]
  },
  "version": 1
}
```

**"The project is read/write — that's the code the agent works on.
The one granted endpoint became a `WebFetch` allow entry. And the
deny list keeps secrets out of reach: `.env` files and private keys
are unreadable *and* unwritable, even inside the project — Cursor
gives deny precedence over allow, so those rules always win."**

**"That last part matters for the token beat. The setup env planted a
`GITHUB_TOKEN` and a `~/demo-secrets/.env`. Cursor is *local* — the
host filesystem is reachable, unlike the cloud backends — so the
protection isn't remoteness, it's this compiled deny rule."**

Show the planted secret on the host:

```bash
ls -a ~/demo-secrets/          # .  ..  .env
cat ~/sandbox-demo/.env 2>/dev/null || echo "(no project .env yet)"
```

**"When the agent runs under this config, a `Read(.env*)` is denied
by policy — the token never reaches the model, even though the file
is right there on disk."**

---

## 3 · Prove the boundary — the non-443 decline

**"The compile never lies about what Cursor can enforce. Ask for
something Cursor's web-fetch allowlist can't express, and flox
declines it — it doesn't quietly widen the grant."**

Add a non-443 grant to the manifest and re-activate:

```bash
flox edit    # add, under [options.sandbox]:
#   [[options.sandbox.network]]
#   endpoint = "db.example.com:5432"
cd /tmp && cd ~/sandbox-demo
```

```
❌ ERROR: The 'cursor' sandbox backend expresses egress as web-fetch domains
(WebFetch), which are HTTPS/443-shaped, but rule 'db.example.com:5432' targets
port 5432.
Rewrite the endpoint as 'db.example.com:443', or select a backend with per-port
egress (e.g. 'openshell').
```

**"No config is written — the compile fails before the write. That's
the contract: a grant Cursor can't faithfully enforce is declined,
never silently promoted to all-ports. Remove the `:5432` grant to
continue."**

---

## 4 · Run the agent under the compiled policy

> **This beat needs Cursor's `agent` CLI and a Cursor account.** On a
> host without them, the walk stops at beat 1's boundary error — that
> *is* the honest end of the local slice. With both present, run:

```bash
cd ~/sandbox-demo
agent "add a docstring to greet() in app.py"
```

**"The agent picks up `<project>/.cursor/cli.json` implicitly — the
policy flox compiled. It can edit the project (`Write(**)`), reach
`api.anthropic.com` through its web-fetch tool, and it's fenced out
of `.env` and key files by the deny rules — enforced by Cursor's
host-native sandbox, the same OS boundary it always re-skins."**

**"That's the division of labor: flox defines *what the environment
is* and derives the policy; Cursor *enforces* it locally around its
agent. Two layers, one source of truth."**

---

## 5 · Exit — nothing to tear down

Cursor is local and stateless from flox's side: the only artifact is
the config file in the project.

```bash
rm ~/sandbox-demo/.cursor/cli.json     # or keep it; it's committed policy
```

**"No sandbox to stop, no image to reap, no remote workspace to
close. flox wrote one config file aligning the two policy layers —
that's the entire footprint."**

---

## 6 · Reset

Deactivate the setup layer (its `profile.deactivate` removes the
planted secret), then:

```bash
bash demo/cleanup.sh
```

Removes the env, fixtures, and the generated `.cursor/cli.json` under
the demo project.

> Integration notes for the Cursor conversation (config schema,
> permission vocabulary, the missing launch hook, the
> `sandbox.mode`/`networkAccess` global-only knobs): the seam is a
> project-scoped **permission** config — a partnership would need a
> launch API that ingests a config path and execs the agent under it,
> or a project-scoped `sandbox.networkAccess` gate.
