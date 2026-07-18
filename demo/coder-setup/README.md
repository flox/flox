# coder-setup

The outer layer of the Coder sandbox demo: a Flox environment that
provisions a local **Coder** control plane *and* the presentation
shell, replacing the manual setup steps of `demo/CODER.md` §0. One of
these setup environments is planned per sandbox backend (siblings:
`openshell-setup`, `srt-setup`, …).

Coder (<https://coder.com>) is a self-hostable control plane for
remote development environments, provisioned through Terraform.
Its open-source core is AGPL-3.0 and needs **no API key** — the whole
loop runs on a laptop with Docker.

Published to FloxHub as `djsauble/coder-setup` (private to djsauble).
Layered usage:

```bash
flox activate -r djsauble/coder-setup   # outer layer: control plane + shell config
cd ~/sandbox-demo                        # project env layers on top (auto-activate)
```

What it does on activation:

- installs `coder` 2.x from the Flox Catalog (nixpkgs) — the CLI is
  both the `coder server` and the `coder` client. The catalog default
  resolves to a 0.x build, so the manifest pins `^2.33` (the 0.x line
  predates the non-interactive first-user flags and the non-deprecated
  `coder templates push` path this backend drives);
- runs `coder server` as a flox **service** on `127.0.0.1:3000`, so
  the control plane lives exactly as long as the activation. With
  `CODER_PG_CONNECTION_URL` unset, Coder downloads a **built-in
  PostgreSQL** on first start (from Maven — needs network once) and
  stores all state under `$FLOX_ENV_CACHE/coder`;
- runs a one-shot polling `login` service that bootstraps the first
  user and logs the CLI in **non-interactively** (`coder login
  --first-user-*`, no browser) using the `CODER_FIRST_USER_*` vars;
- exports the demo's feature flags and the planted `GITHUB_TOKEN`
  (`[vars]`), sets `FLOX_VERSION` and a `flox` alias from `$FLOX_BIN`
  (`[profile.bash/zsh]`), and plants the `~/demo-secrets` fixture
  (`hook.on-activate`).

On deactivation (`[profile.deactivate]`): removes `~/demo-secrets`.
The server + built-in DB stop with the activation (they are flox
services); their state under `$FLOX_ENV_CACHE/coder` persists (removed
by `demo/cleanup.sh`).

Still required on the host: **Docker Desktop running** (the workspace
container runs in Docker's Linux VM), the prototype `flox` binary
(export `FLOX_BIN` from the dev shell before activating), and
`demo/setup.sh` to create the `~/sandbox-demo` project env.

## How the backend uses this deployment

`flox activate --sandbox enforce --sandbox-backend coder` (which
`demo/setup.sh BACKEND=coder` wires into the project manifest):

1. bakes `sandbox-demo-coder:<hash12>` into Docker (shared OCI bake
   with the OpenShell compat layer);
2. generates a minimal docker-provider Terraform template under
   `~/sandbox-demo/.flox/cache/coder-template/main.tf` referencing the
   baked image;
3. pushes it (`coder templates push --yes`), creates a workspace
   (`coder create --yes`) — the container starts from the baked image —
   and *would* exec the activation via `coder ssh <workspace> -- …`.

The workspace filesystem is the container's, so the host `HOME` and
`~/demo-secrets` are invisible from inside — that is the boundary.

### ⚠️ Status: Scaffolded (the launch wall)

Steps 1–3's workspace *container* creation is verified end-to-end, but
the final `coder ssh` exec does **not** complete on a flox-baked image.
Coder's stock workspace-agent init script (the container entrypoint)
runs `coder --version | grep Coder` before registering; the flox bake's
compat layer provides `/bin/sh` and the `sandbox` user but **not**
`grep`/`head`/`wget` on the container `PATH`, so the script fails with
`grep: command not found`, the agent exits, and `coder create` times
out waiting for it. The fix is a **coreutils compat layer** in the bake
(or a template that pins a coreutils-carrying agent image and mounts
the flox closure) — tracked as an open question. Everything up to the
workspace container is real today; you can `docker exec` into it and
see the flox closure.

## ⚠️ Coder cannot enforce a network egress policy

Coder is a control plane and delegates enforcement to the underlying
runtime. The local `docker` provider has **no L7 domain-egress
vocabulary** and Coder ships no egress proxy of its own, so flox
**declines** any `[[options.sandbox.network]]` grant on this backend
rather than silently ignore it. To demo enforced per-endpoint egress,
use the `openshell` backend (its gateway carries an L7 proxy). The
`coder` demo project therefore declares **no** network grants.

## ⚠️ Before first use

1. **Port 3000 must be free.** The server service refuses to start and
   prints the offender if something else already listens there.
2. **Built-in PostgreSQL download.** The first `coder server` start
   fetches PostgreSQL binaries from Maven — one-time, needs network.
3. Cleanup: deactivate (stops the services, removes the planted
   secret), then `bash demo/cleanup.sh` (removes the coder state,
   demo env, workspaces, and images).

## Republishing after edits

This directory is the versioned source of truth. After editing the
manifest here:

```bash
flox pull djsauble/coder-setup /tmp/coder-setup-push
cp .flox/env/manifest.toml /tmp/coder-setup-push/.flox/env/manifest.toml
cd /tmp/coder-setup-push && flox edit -f .flox/env/manifest.toml && flox push
```
