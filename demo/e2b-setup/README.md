# e2b-setup

The outer layer of the E2B sandbox demo: a Flox environment that
provisions the presentation shell for the `e2b` backend
walkthrough, replacing the manual setup steps of `demo/E2B.md` §0.
One of these setup environments exists per sandbox backend
(siblings: `openshell-setup`, `modal-setup`, `docker-sbx-setup`,
`ona-setup`).

Like `modal-setup` and `ona-setup`, this env runs **no local
service**. E2B is a cloud-API provider: the sandbox lives in E2B's
cloud, reached over its API. So there is no gateway to start, no TLS
to generate, no port to guard — the setup env's whole job is to
configure the presentation shell, provide the CLI toolchain, and
surface the launch prerequisites.

Unlike `modal-setup` (whose CLI ships in the catalog), the E2B CLI
(`@e2b/cli`) is a **Node package that is not in the Flox catalog /
nixpkgs** — only the Python SDK (`python3xxPackages.e2b`) is. So
this env installs **`nodejs`** to provide `npm`, and the operator
installs the CLI with:

```bash
npm install -g @e2b/cli
```

Layered usage:

```bash
flox activate -r <owner>/e2b-setup    # outer layer: shell config + notes
cd ~/sandbox-demo                       # project env layers on top (auto-activate)
```

What it does on activation:

- installs `nodejs` (`[install]`) so `npm install -g @e2b/cli` works
  without leaving the env;
- exports the demo's feature flags and the planted `GITHUB_TOKEN`
  (`[vars]`), and sets `FLOX_VERSION` plus a `flox` alias from
  `$FLOX_BIN` (`[profile.bash/zsh]`);
- checks the launch prerequisites non-interactively and prints a
  note for each that is missing (`hook.on-activate`):
  - **Docker** — required to bake the image before E2B's builder
    pulls it;
  - **CLI** — notes when `e2b` is not on PATH, with the `npm`
    install command;
  - **Auth** — probes `e2b auth info` (never opens a browser) and
    checks `E2B_API_KEY`/`E2B_ACCESS_TOKEN`; notes when the CLI is
    present but unauthenticated;
  - **Registry** — reminds the operator to export
    `FLOX_SANDBOX_E2B_REGISTRY` so the generated `e2b.Dockerfile`'s
    `FROM` references a pullable ref;
- plants the `~/demo-secrets` fixture (`hook.on-activate`).

On deactivation (`[profile.deactivate]`): removes `~/demo-secrets`.

## Why no service

The `openshell-setup` sibling runs `openshell-gateway` as a flox
service because OpenShell's control plane is local. E2B has no local
control plane — the API *is* the control plane, and it lives in
E2B's cloud. The honest consequence is that this setup env cannot
launch the sandbox by itself; it can only ready the presentation
shell, provide the CLI toolchain, and do the local hand-off. The
launch itself needs:

1. **An E2B account and API key.** Free tier with $100 credit. Sign
   in with `e2b auth login` (browser OAuth) or export
   `E2B_API_KEY=e2b_<key>` from the dashboard. This is the auth
   wall — every template build and sandbox launch calls the E2B
   API.
2. **A registry E2B's builder can pull from.** `e2b template build`
   reads an `e2b.Dockerfile` whose `FROM` is the baked image, so
   the locally baked image must be pushed. Set
   `FLOX_SANDBOX_E2B_REGISTRY` to your registry prefix (e.g.
   `docker.io/<user>`).

Both are surfaced as notes at activation, not hard failures — the
*local* beats of `demo/E2B.md` (bake, policy compilation, template
generation, preflight errors) work without either.

## The default-open twist

E2B is the one backend whose network default is **open**
(`allowInternetAccess = true`). flox's policy compile always writes
the explicit deny posture (`allow_internet_access = false` in
`e2b.toml`) and lists only the manifest's `:80`/`:443` hosts on top
— it never inherits E2B's open default. E2B filters by host/SNI on
ports 80/443 only and does not filter QUIC/UDP, a declared
lossiness. E2B also exposes a live `updateNetwork` (replace-not-
merge) on a running sandbox — the one true live network-grant
redemption in the cloud tier (see `demo/E2B.md` beat 5).

Still required on the host: Docker Desktop running (to bake the
image before pushing), the prototype `flox` binary (export
`FLOX_BIN` from the dev shell before activating), the shell RC
prompt hook, and `demo/setup.sh` to create the `~/sandbox-demo`
project env.

## Verified

Locked against the catalog 2026-07-18. The interactive layered flow
and the remote sandbox launch are unrehearsed — this host has no
E2B account, API key, or registry, and no `e2b` CLI; validation
stopped at the auth wall. The local slice was exercised end to end:
preflight (CLI-missing bail, too-old-version bail, unauthenticated
bail, authenticated-via-env-key pass), template generation
(`e2b.Dockerfile` `FROM` + valid `e2b.toml` with the forced
`allow_internet_access = false`, `:443` grant compiled into
`allowed_hosts`), non-80/443 decline, and the launch-boundary error
naming the missing prerequisites. `flox activate -d demo/e2b-setup
-- true` succeeds.

## Republishing after edits

This directory is the versioned source of truth. After editing the
manifest here, re-lock with the local flox:

```bash
flox edit -f .flox/env/manifest.toml   # re-locks against the catalog
```

To publish to FloxHub, follow the same pull/copy/edit/push dance as
`modal-setup/README.md`: never `flox push` from inside this
committed directory — copy it out first, then push the copy.
