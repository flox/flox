# ona-setup

The outer layer of the Ona (formerly Gitpod) sandbox demo: a Flox
environment that provisions the presentation shell for the `ona`
backend walkthrough, replacing the manual setup steps of
`demo/ONA.md` §0. One of these setup environments exists per
sandbox backend (siblings: `openshell-setup`, `modal-setup`,
`docker-sbx-setup`).

Like `modal-setup`, this env runs **no local service**. Ona is a
control-plane / cloud CDE: the workspace lives in Ona's cloud,
reached over its API. So there is no gateway to start, no TLS to
generate, no port to guard — the setup env's whole job is to
configure the presentation shell and surface the hand-off
prerequisites.

Unlike the other setup envs, it installs **no provider CLI**. The
Ona CLI is presence-detected by the backend, not required: flox
generates the devcontainer hand-off from the baked image
regardless, and the launch boundary names the account wall. No
public `ona` CLI is available in the Flox catalog on this host; if
one ships later, add it to `[install]`.

Layered usage:

```bash
flox activate -r <owner>/ona-setup    # outer layer: shell config + notes
cd ~/sandbox-demo                       # project env layers on top (auto-activate)
```

What it does on activation:

- exports the demo's feature flags and the planted `GITHUB_TOKEN`
  (`[vars]`), and sets `FLOX_VERSION` plus a `flox` alias from
  `$FLOX_BIN` (`[profile.bash/zsh]`);
- checks the hand-off prerequisites non-interactively and prints a
  note for each that is missing (`hook.on-activate`):
  - **Docker** — required to bake the image before Ona pulls it;
  - **CLI** — notes when no `ona`/`gitpod` CLI is on PATH (fine for
    the local beats);
  - **Registry** — reminds the operator to export
    `FLOX_SANDBOX_ONA_REGISTRY` so the generated devcontainer's
    `image` field references a pullable ref;
- plants the `~/demo-secrets` fixture (`hook.on-activate`).

On deactivation (`[profile.deactivate]`): removes `~/demo-secrets`.

## Why no service

The `openshell-setup` sibling runs `openshell-gateway` as a flox
service because OpenShell's control plane is local. Ona has no
local control plane — the API *is* the control plane, and it lives
in Ona's cloud. The honest consequence is that this setup env
cannot make the workspace open by itself; it can only ready the
presentation shell and the local hand-off. The open itself needs:

1. **An Ona account and an enterprise workspace.** Ona builds a
   workspace from a devcontainer in a git repository through its
   control plane. Post-OpenAI-acquisition (2026-06-11) trial
   access is uncertain — a partnership contact is likely required.
2. **A registry Ona can pull from.** The devcontainer references
   the baked image by `image` reference, so the locally baked
   image must be pushed. Set `FLOX_SANDBOX_ONA_REGISTRY` to your
   registry prefix (e.g. `docker.io/<user>`).

Both are surfaced as notes at activation, not hard failures — the
*local* beats of `demo/ONA.md` (bake, policy compilation,
devcontainer generation, preflight errors) work without either.

Still required on the host: Docker Desktop running (to bake the
image before pushing), the prototype `flox` binary (export
`FLOX_BIN` from the dev shell before activating), the shell RC
prompt hook, and `demo/setup.sh` to create the `~/sandbox-demo`
project env.

## Verified

Locked against the catalog 2026-07-18. The interactive layered flow
and the remote workspace open are unrehearsed — this host has no
Ona account or registry, and no `ona`/`gitpod` CLI; validation
stopped at the account/partnership wall. The local slice was
exercised end to end: preflight (Docker-present pass,
Docker-absent bail, CLI presence detection), devcontainer
generation (valid JSON, `:443` grant compiled into the allowlist),
non-443 decline, and the launch-boundary error naming the missing
prerequisites.

## Republishing after edits

This directory is the versioned source of truth. After editing the
manifest here, re-lock with the local flox:

```bash
flox edit -f .flox/env/manifest.toml   # re-locks against the catalog
```

To publish to FloxHub, follow the same pull/copy/edit/push dance as
`modal-setup/README.md`: never `flox push` from inside this
committed directory — copy it out first, then push the copy.
