# daytona-setup

The outer layer of the Daytona sandbox demo: a Flox environment
that provisions the presentation shell for the `daytona` backend
walkthrough, replacing the manual setup steps of `demo/DAYTONA.md`
§0. One of these setup environments exists per sandbox backend
(siblings: `openshell-setup`, `modal-setup`, `docker-sbx-setup`,
`ona-setup`, `e2b-setup`).

Like `modal-setup`, `ona-setup`, and `e2b-setup`, this env runs
**no local service**. Daytona is a cloud-API provider: the sandbox
lives in Daytona's cloud, reached over its API. So there is no
gateway to start, no TLS to generate, no port to guard — the setup
env's whole job is to configure the presentation shell, provide the
CLI toolchain, and surface the launch prerequisites.

Unlike `e2b-setup` (whose CLI is npm-only), the Daytona CLI **is in
the Flox catalog** as `daytona-bin` (binary name `daytona`), so this
env installs it directly:

```toml
[install]
daytona-bin.pkg-path = "daytona-bin"
```

Layered usage:

```bash
flox activate -r <owner>/daytona-setup   # outer layer: shell config + notes
cd ~/sandbox-demo                          # project env layers on top (auto-activate)
```

What it does on activation:

- installs `daytona-bin` (`[install]`) so the `daytona` CLI is on
  PATH without leaving the env;
- exports the demo's feature flags and the planted `GITHUB_TOKEN`
  (`[vars]`), and sets `FLOX_VERSION` plus a `flox` alias from
  `$FLOX_BIN` (`[profile.bash/zsh]`);
- checks the launch prerequisites non-interactively and prints a
  note for each that is missing (`hook.on-activate`):
  - **Docker** — required to bake the image before Daytona
    registers the snapshot;
  - **CLI** — verifies `daytona` resolved from `daytona-bin`;
  - **Auth** — probes `daytona whoami` (never opens a browser) and
    checks `DAYTONA_API_KEY`; notes when the CLI is present but
    unauthenticated;
  - **Registry** — reminds the operator to export
    `FLOX_SANDBOX_DAYTONA_REGISTRY` so the generated launcher's
    `Image.base(<ref>)` references a pullable ref;
- plants the `~/demo-secrets` fixture (`hook.on-activate`).

On deactivation (`[profile.deactivate]`): removes `~/demo-secrets`.

## Why no service

The `openshell-setup` sibling runs `openshell-gateway` as a flox
service because OpenShell's control plane is local. Daytona has no
local control plane — the API *is* the control plane, and it lives
in Daytona's cloud. The honest consequence is that this setup env
cannot launch the sandbox by itself; it can only ready the
presentation shell, provide the CLI toolchain, and do the local
hand-off. The launch itself needs:

1. **A Daytona account and API key.** Sign in with `daytona login`
   (browser OAuth) or export `DAYTONA_API_KEY=<key>` from the
   dashboard. This is the auth wall — every snapshot registration
   and sandbox launch calls the Daytona API.
2. **A registry Daytona can pull from.** The launcher registers the
   baked image as a snapshot via `Image.base(<ref>)`, so the
   locally baked image must be pushed. Set
   `FLOX_SANDBOX_DAYTONA_REGISTRY` to your registry prefix (e.g.
   `docker.io/<user>`).

Both are surfaced as notes at activation, not hard failures — the
*local* beats of `demo/DAYTONA.md` (bake, policy compilation,
launcher generation, preflight errors) work without either.

## The mutual-exclusivity twist

Daytona's per-sandbox egress vocabulary is three **mutually
exclusive** parameters — `domainAllowList` (domains + wildcards),
`networkAllowList` (IPv4 CIDR ranges), and `networkBlockAll`
(deny-all): at most one may be non-empty. flox's manifest grants
are host-scoped, so they compile onto `domainAllowList` (native,
faithful at the domain level). Two declared lossiness points fall
out of this shape:

- **Port is dropped.** Daytona filters per-domain, not per-port, so
  the `:443` in a grant does not scope the rule — every port to that
  domain is reachable.
- **CIDR grants are exclusive.** A CIDR-shaped grant cannot be
  combined with the domain list on one sandbox, so flox declines it
  rather than silently widening or dropping it.

There is also a ceiling worth naming to a customer: on **Tier 1/2**
organizations the org-level network policy overrides sandbox-level
settings entirely (Tier 3/4 permit custom per-sandbox settings).
Daytona also exposes a live update-network on a running sandbox — an
operator-initiated replacement, not a per-request ask.

Still required on the host: Docker Desktop running (to bake the
image before pushing), the prototype `flox` binary (export
`FLOX_BIN` from the dev shell before activating), the shell RC
prompt hook, and `demo/setup.sh` to create the `~/sandbox-demo`
project env.

## Verified

Locked against the catalog 2026-07-18. The interactive layered flow
and the remote sandbox launch are unrehearsed — this host has no
Daytona account or API key, and no `daytona` CLI beyond the one this
env installs; validation stopped at the auth wall. The local slice
was exercised end to end: preflight (CLI-missing bail,
too-old-version bail, unauthenticated bail, authenticated-via-env-key
pass), launcher generation (valid Python, `Image.base` snapshot ref,
`network_block_all=True` deny-all posture), CIDR-grant decline, and
the launch-boundary error naming the missing prerequisites. `flox
activate -d demo/daytona-setup -- true` succeeds.

## Republishing after edits

This directory is the versioned source of truth. After editing the
manifest here, re-lock with the local flox:

```bash
flox edit -f .flox/env/manifest.toml   # re-locks against the catalog
```

To publish to FloxHub, follow the same pull/copy/edit/push dance as
`modal-setup/README.md`: never `flox push` from inside this
committed directory — copy it out first, then push the copy.
