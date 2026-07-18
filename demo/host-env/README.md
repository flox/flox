# Demo host environment (EXPERIMENTAL — not yet rehearsed)

A host-side Flox environment that provisions the OpenShell control
plane for the sandbox demo, replacing the manual OpenShell
install/provisioning/config steps of `demo/OPENSHELL.md` §0.

Published to FloxHub as `djsauble/sandbox-demo-host` (private to
djsauble) — activate it from any machine with:

```bash
flox activate -r djsauble/sandbox-demo-host
```

This directory is the versioned source of truth; after editing the
manifest here, republish by copying it over a pulled copy
(`flox pull djsauble/sandbox-demo-host`, overwrite
`.flox/env/manifest.toml`, `flox edit -f`, `flox push`).

What it does:

- installs `djsauble/openshell` (0.0.86, repackaged release
  binaries — the catalog's own `openshell` 0.0.36 is far too old);
- generates gateway TLS material into `$FLOX_ENV_CACHE` on first
  activation;
- renders a `gateway.toml` (docker driver + bind mounts) into
  `$FLOX_ENV_CACHE` and points `OPENSHELL_GATEWAY_CONFIG` at it —
  no `~/.config/openshell/gateway.toml` edit, no restart dance;
- runs `openshell-gateway` as a flox **service**, so the gateway
  lives exactly as long as the activation;
- registers the gateway once via a one-shot polling service
  (`openshell gateway add … --name flox-demo`).

Still required on the host: Docker Desktop running, the prototype
`flox` binary (`$FLOX_BIN` from the dev shell), the feature-flag
exports, and `claude setup-token` (see `demo/OPENSHELL.md` §0).

## ⚠️ Before first use

1. **`openshell gateway add` writes persistent state** under
   `~/.config/openshell/` (`gateways/flox-demo/`, and it may switch
   the *active* gateway selection). If you have a working
   brew-installed gateway, do not activate this env right before a
   demo against that gateway — rehearse this env on its own first,
   and switch back afterwards with
   `openshell gateway select <name>`.
2. Cleanup: deactivate (stops the gateway service), then
   `rm -rf ~/.config/openshell/gateways/flox-demo` and re-select
   your previous gateway.
3. Podman as a Docker Desktop replacement is deliberately **not**
   wired in yet: whether openshell-gateway's docker driver honors
   `DOCKER_HOST` (via its `gateway.env` mechanism) against a podman
   machine socket is unverified. That spike is documented in the
   session notes; the env keeps assuming Docker Desktop until it
   passes.

Usage:

```bash
cd demo/host-env && flox activate
openshell status        # Status: Connected (after the services settle)
```
