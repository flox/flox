# cursor-setup

The outer layer of the Cursor (agent sandbox) demo: a Flox
environment that provisions the presentation shell for the `cursor`
backend walkthrough, replacing the manual setup steps of
`demo/CURSOR.md` §0. One of these setup environments exists per
sandbox backend (siblings: `openshell-setup`, `modal-setup`,
`ona-setup`).

Like `modal-setup` and `ona-setup`, this env runs **no local
service** and installs **no provider CLI**. But Cursor is different
from all the cloud siblings: it is a **local policy layer**. Cursor's
agent CLI (`agent`) runs its own sandbox on *this* machine,
re-skinning the host-native OS boundary (Seatbelt on macOS, Landlock
on Linux) with a product policy layer. Nothing is baked, nothing
leaves the laptop, and there is no Docker step. The setup env's whole
job is to configure the presentation shell and surface the alignment
prerequisites.

The `cursor` backend does not launch the agent. Cursor configures its
sandbox through settings files, not a programmatic launch API, so the
honest seam is **policy-layer alignment**: flox compiles the manifest
grants into Cursor's project permission config
(`<project>/.cursor/cli.json`) so Flox's environment-and-policy source
of truth and Cursor's enforcement *stack* instead of conflicting. The
agent CLI reads that config implicitly from the project directory.

Layered usage:

```bash
flox activate -r <owner>/cursor-setup   # outer layer: shell config + notes
cd ~/sandbox-demo                        # project env layers on top (auto-activate)
```

What it does on activation:

- exports the demo's feature flags and the planted `GITHUB_TOKEN`
  (`[vars]`), and sets `FLOX_VERSION` plus a `flox` alias from
  `$FLOX_BIN` (`[profile.bash/zsh]`);
- checks the alignment prerequisites non-interactively and prints a
  note for each that is missing (`hook.on-activate`):
  - **CLI** — notes when Cursor's `agent` CLI is not on PATH, with
    the `curl https://cursor.com/install | bash` install hint (fine
    for the local beats);
  - **Auth** — notes when `CURSOR_API_KEY` is unset (config
    generation works regardless; running the agent needs a Cursor
    account);
- plants the `~/demo-secrets` fixture (`hook.on-activate`).

On deactivation (`[profile.deactivate]`): removes `~/demo-secrets`.

## Why no service and no CLI in [install]

The `openshell-setup` sibling runs `openshell-gateway` as a flox
service because OpenShell's control plane is local. Cursor has no
control plane at all — its sandbox is a host-kernel policy layer with
no daemon. And Cursor's `agent` CLI installs via
`curl https://cursor.com/install | bash`, not the Flox catalog, so
there is no catalog package to add to `[install]`. Preflight
presence-detects the CLI instead.

The honest consequence is that this setup env cannot make the agent
run under the compiled policy by itself. That needs:

1. **Cursor's `agent` CLI on PATH.** Install with
   `curl https://cursor.com/install -fsS | bash`.
2. **A Cursor account.** The agent authenticates via `CURSOR_API_KEY`
   (or a one-time `agent` sign-in). flox probes the key
   non-interactively and never triggers a browser login.

Both are surfaced as notes at activation, not hard failures — the
*local* beats of `demo/CURSOR.md` (policy compilation, config
generation, preflight errors) work without either.

Still required on the host: the prototype `flox` binary (export
`FLOX_BIN` from the dev shell before activating), the shell RC prompt
hook, and `demo/setup.sh` to create the `~/sandbox-demo` project env.

## The load-bearing lossiness

Cursor's project config expresses policy as `permissions.allow` /
`permissions.deny` tokens — `Read(glob)`, `Write(glob)`,
`Shell(base)`, `WebFetch(domain)`, `Mcp(server:tool)` — where **deny
takes precedence over allow** (verified against cursor.com/docs,
2026-07-18). flox compiles the manifest's
`[[options.sandbox.network]]` grants onto `WebFetch(<host>)` allow
entries. Three lossiness axes, declared in the module docs and the
demo:

- **Web-fetch tool only.** `WebFetch` governs the agent's web-fetch
  tool, not arbitrary sockets.
- **Port-blind.** `WebFetch(domain)` carries no port; a grant's
  `:443` is dropped, and a non-443 endpoint is *declined* at compile
  time rather than silently widened.
- **Op-blind on the network axis.** The grant's `access` /
  `protocol` / `binary` scoping is not expressible through `WebFetch`
  and is dropped.

flox also mirrors the host-native secret posture into the deny list:
`.env` files and private keys are denied for both `Read` and `Write`,
so the agent cannot read or overwrite them even inside the project.

## Verified

Locked against the catalog 2026-07-18. The interactive layered flow
and running the agent under the compiled policy are unrehearsed —
this host has no Cursor account and no `agent` CLI; validation
stopped at the CLI/account wall. The local slice was exercised end to
end: preflight (CLI-absent bail with install hint, version gate via a
fake shim, non-interactive `CURSOR_API_KEY` auth probe), config
generation (valid JSON matching Cursor's schema `version: 1`, a
`:443` grant compiled into a `WebFetch` allow entry, secret-protection
deny tokens), the non-443 decline, and the launch-boundary error
naming the missing programmatic hook.

## Republishing after edits

This directory is the versioned source of truth. After editing the
manifest here, re-lock with the local flox:

```bash
flox edit -f .flox/env/manifest.toml   # re-locks against the catalog
```

To publish to FloxHub, follow the same pull/copy/edit/push dance as
`modal-setup/README.md`: never `flox push` from inside this committed
directory — copy it out first, then push the copy.
