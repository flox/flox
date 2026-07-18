# cognition-devin-setup

The outer layer of the Cognition (Devin) sandbox demo: a Flox
environment that provisions the presentation shell for the
`cognition-devin` backend walkthrough, replacing the manual setup
steps of `demo/DEVIN.md` §0. One of these setup environments exists
per sandbox backend (siblings: `openshell-setup`, `modal-setup`,
`docker-sbx-setup`, `ona-setup`, `e2b-setup`, `daytona-setup`).

Like `ona-setup`, this env runs **no local service**. Devin is a
subscription agent runtime: the sandbox lives in Devin's cloud,
reached over its API. So there is no gateway to start, no TLS to
generate, no port to guard — the setup env's whole job is to
configure the presentation shell and surface the hand-off
prerequisites.

Like `ona-setup`, it installs **no provider CLI**. The Devin CLI is
presence-detected by the backend, not required: flox generates the
blueprint hand-off from the baked image regardless, and the launch
boundary names the subscription/partnership wall. No public `devin`
CLI is available in the Flox catalog on this host; if one ships
later, add it to `[install]`.

Layered usage:

```bash
flox activate -r <owner>/cognition-devin-setup  # outer layer: shell config + notes
cd ~/sandbox-demo                                 # project env layers on top (auto-activate)
```

What it does on activation:

- exports the demo's feature flags and the planted `GITHUB_TOKEN`
  (`[vars]`), and sets `FLOX_VERSION` plus a `flox` alias from
  `$FLOX_BIN` (`[profile.bash/zsh]`);
- checks the hand-off prerequisites non-interactively and prints a
  note for each that is missing (`hook.on-activate`):
  - **Docker** — required to bake the substrate image before
    Devin's build pulls it;
  - **CLI** — notes when no `devin` CLI is on PATH (fine for the
    local beats);
  - **Registry** — reminds the operator to export
    `FLOX_SANDBOX_COGNITION_DEVIN_REGISTRY` so the generated
    blueprint's image ref is pullable;
- plants the `~/demo-secrets` fixture (`hook.on-activate`).

On deactivation (`[profile.deactivate]`): removes `~/demo-secrets`.

## Why no service

The `openshell-setup` sibling runs `openshell-gateway` as a flox
service because OpenShell's control plane is local. Devin has no
local control plane — the API *is* the control plane, and it lives
in Devin's cloud. The honest consequence is that this setup env
cannot make the snapshot build by itself; it can only ready the
presentation shell and the local hand-off. The build + session
itself needs:

1. **A Devin subscription and a partnership.** Devin builds a
   snapshot from a blueprint through its own builder; there is no
   public sandbox/runtime-launch API that ingests an arbitrary
   image. A co-sell with Cognition's Sandbox/Infra team is the path
   to a backend-grade integration.
2. **A registry Devin's build can pull from.** The blueprint
   references the baked substrate image, so the locally baked image
   must be pushed. Set `FLOX_SANDBOX_COGNITION_DEVIN_REGISTRY` to
   your registry prefix (e.g. `docker.io/<user>`).

Both are surfaced as notes at activation, not hard failures — the
*local* beats of `demo/DEVIN.md` (bake, policy compilation,
blueprint generation, preflight errors) work without either.

Still required on the host: Docker Desktop running (to bake the
substrate image), the prototype `flox` binary (export `FLOX_BIN`
from the dev shell before activating), the shell RC prompt hook,
and `demo/setup.sh` to create the `~/sandbox-demo` project env.

## Why a blueprint, not a devcontainer

The `ona-setup` sibling hands Ona a `.devcontainer/devcontainer.json`
that wraps an OCI image. Devin's ingestion contract is different:
Devin does **not** consume an image directly. Its builder produces
a *snapshot* from a YAML *blueprint* (blueprint ≈ Dockerfile, build
≈ `docker build`, snapshot ≈ image, in Devin's own docs). So the
`cognition-devin` backend generates `.devin/blueprint.yaml`, whose
`initialize` step installs Flox and activates the locked
environment — the blueprint *reproduces* the closure inside Devin's
snapshot rather than wrapping a pre-baked image. The baked image is
still produced as the reproducible substrate a credentialed
operator can `docker load` and reference. This is the inverted
integration model: Flox supplies the environment definition Devin
runs inside, and Devin's runtime enforces.

## Verified

Locked against the catalog 2026-07-18. The interactive layered flow
and the remote snapshot build are unrehearsed — this host has no
Devin subscription or registry, and no `devin` CLI; validation
stopped at the subscription/partnership wall. The local slice was
exercised end to end: preflight (Docker-present pass, Docker-absent
bail, CLI presence detection), blueprint generation (deny-all →
empty allowlist plus `["*"]` denylist catch-all; `:443` grant
compiled into `allowed_domains`), non-443 decline, and the
launch-boundary error naming the missing prerequisites.
`flox activate -d demo/cognition-devin-setup -- true` succeeds.

## Republishing after edits

This directory is the versioned source of truth. After editing the
manifest here, re-lock with the local flox:

```bash
flox edit -f .flox/env/manifest.toml   # re-locks against the catalog
```

To publish to FloxHub, follow the same pull/copy/edit/push dance as
`ona-setup/README.md`: never `flox push` from inside this committed
directory — copy it out first, then push the copy.
