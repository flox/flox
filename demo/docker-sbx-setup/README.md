# docker-sbx-setup

The outer layer of the Docker Sandboxes demo: a Flox environment
that provisions the `sbx` CLI *and* the presentation shell,
replacing the manual setup steps of `demo/DOCKER-SBX.md` §0. One of
these setup environments exists per sandbox backend (siblings:
`openshell-setup`, `modal-setup`, and future `srt-setup`,
`libkrun-setup`, …).

Docker Sandboxes runs each sandbox as a **local Linux microVM**
driven by the `sbx` CLI, which carries its own hypervisor. Unlike
`openshell-setup`, this env runs **no local service**: there is no
gateway to start, no TLS to generate, no port to guard. The setup
env's whole job is to put the `sbx` client on PATH and configure the
presentation shell.

Layered usage:

```bash
flox activate -r <owner>/docker-sbx-setup   # outer layer: client + shell config
cd ~/sandbox-demo                            # project env layers on top (auto-activate)
```

What it does on activation:

- installs `docker-sbx` (the `sbx` CLI — `run`, `policy`, `secret`,
  `kit`, `login`) from the catalog;
- exports the demo's feature flags and the planted `GITHUB_TOKEN`
  (`[vars]`), and sets `FLOX_VERSION` plus a `flox` alias from
  `$FLOX_BIN` (`[profile.bash/zsh]`);
- checks the launch prerequisites non-interactively and prints a
  note for each that is missing (`hook.on-activate`):
  - **`sbx` login** — reminds the operator to run `sbx login` once
    (browser OAuth), without triggering the flow;
  - **Docker daemon** — `docker info` (cheap, non-interactive)
    confirms the daemon that bakes and loads the image is reachable;
- plants the `~/demo-secrets` fixture (`hook.on-activate`).

On deactivation (`[profile.deactivate]`): removes `~/demo-secrets`.

## Why no service

The `openshell-setup` sibling runs `openshell-gateway` as a flox
service because OpenShell's control plane is local and long-running.
Docker Sandboxes has no such daemon to keep alive: the `sbx` CLI
starts a microVM on demand and tears it down on `sbx rm`. The honest
consequence is that this setup env cannot make the microVM launch
work by itself; it can only ready the client. The launch itself
needs:

1. **The `sbx` CLI, signed in.** Installed here from the catalog
   (`docker-sbx@0.34.0`); run `sbx login` once (browser OAuth; the
   free CLI tier suffices — only organization governance is paid).
   On hosts that use the bundled `docker sbx` subcommand instead of
   the standalone CLI, that path requires **Docker Desktop 4.60 or
   newer**.
2. **A base image that satisfies sbx's kit contract.** A
   `kind: sandbox` kit's base image must provide a non-root `agent`
   user at uid 1000 with passwordless sudo, a `/home/agent` home,
   and preserved HTTP proxy env. The flox bake adds a `sandbox`
   user, not sbx's `agent` user, so the baked image must be adapted
   (build on `docker/sandbox-templates:shell-docker`) before
   `sbx run --kit` can use it.

Both are surfaced as notes/errors, not silent successes — the
*local* beats of `demo/DOCKER-SBX.md` (bake, policy compilation, kit
generation, preflight errors) work without a completed launch.

Still required on the host: a running Docker daemon (to bake and
load the image), the prototype `flox` binary (export `FLOX_BIN` from
the dev shell before activating), the shell RC prompt hook, and
`demo/setup.sh` to create the `~/sandbox-demo` project env.

## Verified

Locked against the catalog 2026-07-18 (`docker-sbx@0.34.0` resolves
for `aarch64-darwin` and `x86_64-linux`). The env activates and puts
a real `sbx` on PATH; `sbx version` reports `v0.34.0`. The
interactive layered flow and the microVM launch are unrehearsed —
this host has `sbx` from the catalog but the walkthrough's `sbx run`
depends on a completed `sbx login` and an adapted base image;
validation stopped at the kit-generation / launch boundary.

## Republishing after edits

This directory is the versioned source of truth. After editing the
manifest here, re-lock with the local flox:

```bash
flox edit -f .flox/env/manifest.toml   # re-locks against the catalog
```

To publish to FloxHub, follow the same pull/copy/edit/push dance as
`openshell-setup/README.md` (never `flox push` from inside this
committed directory — push from a copy).
