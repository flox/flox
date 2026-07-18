# anjuna-setup

The outer layer of the Anjuna Security (TEE) sandbox demo: a Flox
environment that provisions the presentation shell for the `anjuna`
backend walkthrough, replacing the manual setup steps of
`demo/ANJUNA.md` §0. One of these setup environments exists per
sandbox backend (siblings: `openshell-setup`, `modal-setup`,
`docker-sbx-setup`, `ona-setup`, `e2b-setup`, `daytona-setup`,
`cognition-devin-setup`).

Like `cognition-devin-setup`, this env runs **no local service**.
Anjuna is a commercially licensed confidential-computing runtime:
the enclave lives on TEE hardware (AWS Nitro Enclaves, AMD SEV-SNP,
Intel SGX), driven by the license-gated `anjuna-nitro-cli`. So there
is no gateway to start, no TLS to generate, no port to guard — the
setup env's whole job is to configure the presentation shell and
surface the hand-off prerequisites.

Like `cognition-devin-setup`, it installs **no provider CLI**. The
`anjuna-nitro-cli` is presence-detected by the backend, not
required: flox generates the converter-config hand-off from the
baked image regardless, and the launch boundary names the license +
hardware walls. The Anjuna CLI is **not open source** — it is
distributed through Anjuna's private repository, so it is **not in
the Flox catalog** on any host. If Anjuna ever ships a freely
downloadable evaluation CLI that lands in the catalog, add it to
`[install]`.

Layered usage:

```bash
flox activate -r <owner>/anjuna-setup   # outer layer: shell config + notes
cd ~/sandbox-demo                         # project env layers on top (auto-activate)
```

What it does on activation:

- exports the demo's feature flags and the planted `GITHUB_TOKEN`
  (`[vars]`), and sets `FLOX_VERSION` plus a `flox` alias from
  `$FLOX_BIN` (`[profile.bash/zsh]`);
- checks the hand-off prerequisites non-interactively and prints a
  note for each that is missing (`hook.on-activate`):
  - **Docker** — required to bake the converter's input image;
  - **CLI** — notes when no `anjuna-nitro-cli` is on PATH (fine for
    the local beats; it is license-gated);
  - **Registry** — reminds the operator to export
    `FLOX_SANDBOX_ANJUNA_REGISTRY` so the generated `build-enclave`
    `--docker-uri` is pullable;
- plants the `~/demo-secrets` fixture (`hook.on-activate`).

On deactivation (`[profile.deactivate]`): removes `~/demo-secrets`.

## Why no service

The `openshell-setup` sibling runs `openshell-gateway` as a flox
service because OpenShell's control plane is local. Anjuna has no
local control plane — the runtime is a hardware enclave on a TEE
instance, driven by a host CLI. The honest consequence is that this
setup env cannot make the enclave build by itself; it can only ready
the presentation shell and the local hand-off. The build + run
itself needs:

1. **An Anjuna commercial license.** The `anjuna-nitro-cli` and
   runtime are not open source and are distributed through Anjuna's
   private repository. A warm partnership contact exists
   (#flox-external-anjuna-2025); a co-sell with Anjuna is the path
   to a backend-grade integration.
2. **TEE hardware.** The enclave needs SGX/SEV-SNP silicon or an AWS
   Nitro parent instance — a Linux cloud instance. macOS arm64 has
   none.
3. **A registry the Anjuna converter can pull from.** The
   `build-enclave` invocation references the baked substrate image,
   so the locally baked image must be pushed. Set
   `FLOX_SANDBOX_ANJUNA_REGISTRY` to your registry prefix (e.g.
   `docker.io/<user>`).

All three are surfaced as notes at activation, not hard failures —
the *local* beats of `demo/ANJUNA.md` (bake, policy compilation,
converter-config + build-invocation + attestation-binding
generation, preflight errors) work without any of them.

Still required on the host: Docker Desktop running (to bake the
converter's input image), the prototype `flox` binary (export
`FLOX_BIN` from the dev shell before activating), the shell RC
prompt hook, and `demo/setup.sh` to create the `~/sandbox-demo`
project env.

## Why a converter config, not a devcontainer or blueprint

The `ona-setup` sibling hands Ona a
`.devcontainer/devcontainer.json` that wraps an OCI image; the
`cognition-devin-setup` sibling hands Devin a `.devin/blueprint.yaml`
whose `initialize` step reproduces the closure. Anjuna's ingestion
contract is different again: Anjuna does **not** run an image and
does **not** build from a recipe — it **converts** an image. Its
`anjuna-nitro-cli build-enclave` step takes the baked image
(`--docker-uri`) plus an enclave-config YAML
(`--enclave-config-file`) and produces an *enclave image* (`.eif`)
that runs inside a hardware TEE. So the `anjuna` backend generates
`.flox/cache/anjuna/enclave-config.yaml` + `build-enclave.sh`, which
feed the converter — and binds the *expected enclave attestation
measurement* to the lockfile hash, so a relying party can prove the
enclave that runs is the one flox's reproducible environment
produced. The artifacts live under `.flox/cache/` (not the repo
root) because they are build inputs a credentialed operator
regenerates, like the modal launcher.

## Verified

Locked against the catalog 2026-07-18. The interactive layered flow
and the remote enclave build are unrehearsed — this host has no
Anjuna license, no TEE hardware, and no `anjuna-nitro-cli`;
validation stopped at the license + hardware walls. The local slice
was exercised end to end: preflight (Docker-present pass,
Docker-absent bail, CLI presence detection), converter-config +
build-script generation (deny-all → empty allowlist plus
`deny_all_egress: true` marker; `:443` grant compiled into
`allowed_hosts`; attestation binding recording the lockfile hash),
non-443 decline, and the launch-boundary error naming the missing
prerequisites. `flox activate -d demo/anjuna-setup -- true` succeeds.

## Republishing after edits

This directory is the versioned source of truth. After editing the
manifest here, re-lock with the local flox:

```bash
flox edit -f .flox/env/manifest.toml   # re-locks against the catalog
```

To publish to FloxHub, follow the same pull/copy/edit/push dance as
`cognition-devin-setup/README.md`: never `flox push` from inside
this committed directory — copy it out first, then push the copy.
