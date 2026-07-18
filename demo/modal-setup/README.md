# modal-setup

The outer layer of the Modal sandbox demo: a Flox environment that
provisions the Modal client *and* the presentation shell, replacing
the manual setup steps of `demo/MODAL.md` §0. One of these setup
environments exists per sandbox backend (siblings:
`openshell-setup`, and future `srt-setup`, `libkrun-setup`, …).

Unlike `openshell-setup`, this env runs **no local service**. Modal
is cloud-remote: the sandbox lives in Modal's cloud, reached over
its API. So there is no gateway to start, no TLS to generate, no
port to guard — the setup env's whole job is to put the `modal`
client on PATH and configure the presentation shell.

Layered usage:

```bash
flox activate -r <owner>/modal-setup   # outer layer: client + shell config
cd ~/sandbox-demo                       # project env layers on top (auto-activate)
```

What it does on activation:

- installs `python313Packages.modal` (the `modal` CLI + the `modal`
  Python SDK the generated launcher imports);
- exports the demo's feature flags and the planted `GITHUB_TOKEN`
  (`[vars]`), and sets `FLOX_VERSION` plus a `flox` alias from
  `$FLOX_BIN` (`[profile.bash/zsh]`);
- checks the two launch prerequisites non-interactively and prints a
  note for each that is missing (`hook.on-activate`):
  - **Auth** — `modal token info` (cheap, no browser prompt)
    distinguishes CLI-present-but-unauthenticated from ready;
  - **Registry** — reminds the operator to export
    `FLOX_SANDBOX_MODAL_REGISTRY` so the generated launcher
    references a pullable image ref;
- plants the `~/demo-secrets` fixture (`hook.on-activate`).

On deactivation (`[profile.deactivate]`): removes `~/demo-secrets`.

## Why no service

The `openshell-setup` sibling runs `openshell-gateway` as a flox
service because OpenShell's control plane is local. Modal has no
local control plane — the API *is* the control plane, and it lives
in Modal's cloud. The honest consequence is that this setup env
cannot make the remote launch work by itself; it can only ready the
client. The launch itself needs:

1. **A Modal account and token** (`modal token new`; free tier
   suffices). Auth lands in `~/.modal.toml`.
2. **A registry Modal can pull from.** Modal ingests images by
   registry reference only (`Image.from_registry`), so the locally
   baked image must be pushed. Set `FLOX_SANDBOX_MODAL_REGISTRY` to
   your registry prefix (e.g. `docker.io/<user>`).

Both are surfaced as notes at activation, not hard failures — the
*local* beats of `demo/MODAL.md` (bake, policy compilation,
launcher generation, preflight errors) work without either.

Still required on the host: Docker Desktop running (to bake the
image before pushing), the prototype `flox` binary (export
`FLOX_BIN` from the dev shell before activating), the shell RC
prompt hook, and `demo/setup.sh` to create the `~/sandbox-demo`
project env.

## Verified

Locked against the catalog 2026-07-18 (`modal` resolves for
`aarch64-darwin` and `x86_64-linux`). The interactive layered flow
and the remote launch are unrehearsed — this host has no Modal
account or registry tonight; validation stopped at the preflight
auth wall.

## Republishing after edits

This directory is the versioned source of truth. After editing the
manifest here, re-lock with the local flox:

```bash
flox edit -f .flox/env/manifest.toml   # re-locks against the catalog
```

To publish to FloxHub, follow the same pull/copy/edit/push dance as
`openshell-setup/README.md`.
