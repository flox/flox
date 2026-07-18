# openshell-setup (EXPERIMENTAL — not yet rehearsed)

The outer layer of the sandbox demo: a Flox environment that
provisions the OpenShell control plane *and* the presentation
shell, replacing the manual setup steps of `demo/OPENSHELL.md` §0.
One of these setup environments is planned per sandbox backend
(hence the name — future siblings: `srt-setup`, `libkrun-setup`, …).

Published to FloxHub as `djsauble/openshell-setup` (private to
djsauble). Layered usage:

```bash
flox activate -r djsauble/openshell-setup   # outer layer: control plane + shell config
cd ~/sandbox-demo                           # project env layers on top (auto-activate)
```

What it does on activation:

- installs `djsauble/openshell` (0.0.86, repackaged release
  binaries — the catalog's own `openshell` 0.0.36 is far too old);
- generates gateway TLS material into `$FLOX_ENV_CACHE`;
- renders a `gateway.toml` (docker driver + bind mounts) into
  `$FLOX_ENV_CACHE` and points `OPENSHELL_GATEWAY_CONFIG` at it —
  no `~/.config/openshell/gateway.toml` edit, no restart dance;
- runs `openshell-gateway` as a flox **service**, so the gateway
  lives exactly as long as the activation;
- registers the gateway once via a one-shot polling service
  (`openshell gateway add … --name flox-demo`);
- exports the demo's feature flags and the planted `GITHUB_TOKEN`
  (`[vars]`), sets `FLOX_VERSION` and a `flox` alias from
  `$FLOX_BIN` (`[profile.bash/zsh]`), and plants the
  `~/demo-secrets` fixture (`hook.on-activate`).

On deactivation (`[profile.deactivate]`): removes `~/demo-secrets`.
The gateway service stops with the activation; the gateway
*registration* persists (removed by `demo/cleanup.sh`).

Still required on the host: Docker Desktop running, the prototype
`flox` binary (export `FLOX_BIN` from the dev shell before
activating), the shell RC prompt hook, `claude setup-token`, and
`demo/setup.sh` to create the `~/sandbox-demo` project env.

## ⚠️ Before first use

1. **`openshell gateway add` writes persistent state** under
   `~/.config/openshell/` (`gateways/flox-demo/`, and it may switch
   the *active* gateway selection). Do not activate this env on a
   host whose own gateway is already running — both want port
   17670. Switch back afterwards with
   `openshell gateway select <name>`.
2. Cleanup: deactivate (stops the gateway service, removes the
   planted secret), then `bash demo/cleanup.sh` (removes the
   gateway registration, demo env, and images).
3. Podman as a Docker Desktop replacement is deliberately **not**
   wired in yet: whether openshell-gateway's docker driver honors
   `DOCKER_HOST` (via its `gateway.env` mechanism) against a podman
   machine socket is unverified.

## Republishing after edits

This directory is the versioned source of truth. After editing the
manifest here:

```bash
flox pull djsauble/openshell-setup /tmp/openshell-setup-push
cp .flox/env/manifest.toml /tmp/openshell-setup-push/.flox/env/manifest.toml
cd /tmp/openshell-setup-push && flox edit -f .flox/env/manifest.toml && flox push
```
