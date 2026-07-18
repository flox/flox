# vercel-sandbox-setup

The outer layer of the Vercel Sandbox demo: a Flox environment that
provisions the presentation shell for the `vercel-sandbox` backend
walkthrough, replacing the manual setup steps of
`demo/VERCEL-SANDBOX.md` §0. One of these setup environments exists
per sandbox backend (siblings: `openshell-setup`, `modal-setup`,
`docker-sbx-setup`, `ona-setup`, `e2b-setup`, `daytona-setup`,
`cognition-devin-setup`, `anjuna-setup`, `cursor-setup`).

Like `modal-setup`, `ona-setup`, and `e2b-setup`, this env runs **no
local service**. Vercel Sandbox is a cloud-API provider: the sandbox
lives in Vercel's cloud (a Firecracker microVM), reached over its
API. So there is no gateway to start, no TLS to generate, no port to
guard — the setup env's whole job is to configure the presentation
shell, provide the CLI toolchain, and surface the launch
prerequisites.

## The bootstrap-shaped difference

This is the **first bootstrap-shaped backend**, and it is unlike
every OCI backend that precedes it. The image backends
(`oci`, `modal`, `e2b`, `daytona`, `ona`, `cognition-devin`,
`anjuna`) all bake the environment into an OCI image and hand it to
the provider. Vercel Sandbox **cannot ingest an arbitrary image** on
its stock-runtime path: `Sandbox.create` boots one of a FIXED set of
base runtimes (`node22`, `node24`, `python3.13`; Amazon Linux 2023)
and seeds it from a git source. So flox does not bake. It generates:

- a **flox bootstrap** (`.flox/cache/vercel-sandbox-bootstrap.sh`)
  that installs Flox inside the running sandbox and activates the
  environment from FloxHub (`flox activate -r <owner>/<env>`); and
- a **`@vercel/sandbox` launcher**
  (`.flox/cache/vercel-sandbox-launch.mjs`) that creates the
  sandbox, uploads the bootstrap, and runs it.

Like `e2b-setup` (whose CLI ships only via npm), the Vercel CLI
(`vercel`) is a **Node package that is not in the Flox catalog /
nixpkgs**. So this env installs **`nodejs`** to provide `npm`, and
the operator installs the CLI with:

```bash
npm install -g vercel
```

Layered usage:

```bash
flox activate -r <owner>/vercel-sandbox-setup   # outer layer: shell config + notes
cd ~/sandbox-demo                                 # project env layers on top (auto-activate)
```

What it does on activation:

- installs `nodejs` (`[install]`) so `npm install -g vercel` works
  without leaving the env;
- exports the demo's feature flags and the planted `GITHUB_TOKEN`
  (`[vars]`), and sets `FLOX_VERSION` plus a `flox` alias from
  `$FLOX_BIN` (`[profile.bash/zsh]`);
- checks the launch prerequisites non-interactively and prints a
  note for each that is missing (`hook.on-activate`):
  - **CLI** — notes when `vercel` is not on PATH, with the `npm`
    install command;
  - **Auth** — probes `vercel whoami` (never opens a browser) and
    checks `VERCEL_OIDC_TOKEN`/`VERCEL_TOKEN`; notes when the CLI is
    present but unauthenticated;
  - **FloxHub push** — reminds the operator to `flox push` the demo
    env and export `FLOX_SANDBOX_VERCEL_FLOXHUB_REF` so the
    generated bootstrap activates a real ref, not a placeholder;
  - **Runtime** — validates any `FLOX_SANDBOX_VERCEL_RUNTIME`
    override against the accepted set (`node22`/`node24`/
    `python3.13`);
- plants the `~/demo-secrets` fixture (`hook.on-activate`).

On deactivation (`[profile.deactivate]`): removes `~/demo-secrets`.

## Why no service

The `openshell-setup` sibling runs `openshell-gateway` as a flox
service because OpenShell's control plane is local. Vercel Sandbox
has no local control plane — the API *is* the control plane, and it
lives in Vercel's cloud. The honest consequence is that this setup
env cannot launch the sandbox by itself; it can only ready the
presentation shell, provide the CLI toolchain, and do the local
hand-off. The launch itself needs:

1. **A Vercel account and token.** Free tier suffices. Sign in with
   `vercel login`, then `vercel link` + `vercel env pull` to
   download an OIDC token to `.env.local` (12-hour lifetime), or
   export `VERCEL_TOKEN` with `VERCEL_TEAM_ID`/`VERCEL_PROJECT_ID`
   from the dashboard. This is the auth wall — every
   `Sandbox.create` call authenticates against the Vercel API.
2. **The environment on FloxHub.** Because the runtime is fixed and
   no image is pushed, the bootstrap activates
   `flox activate -r <owner>/<env>`. Push the demo env
   (`flox push --owner <you>`) and export
   `FLOX_SANDBOX_VERCEL_FLOXHUB_REF=<you>/sandbox-demo`.

Both are surfaced as notes at activation, not hard failures — the
*local* beats of `demo/VERCEL-SANDBOX.md` (preflight, bootstrap +
launcher generation, the network-decline, the launch-boundary error)
work without either.

## The network gap — declared honestly

Vercel Sandbox is the one backend here whose SDK has **no per-sandbox
egress vocabulary at all**: `@vercel/sandbox`'s `ports` option
governs *inbound* exposure (`sandbox.domain(port)`), not outbound
filtering. There is no `allowedDomains`, `blockNetwork`, or CIDR
analog. So flox cannot compile a manifest egress grant onto this
provider. Rather than silently drop a grant (which would falsely
imply it was honored), flox **declines** any
`[[options.sandbox.network]]` rule with a clear error naming the
missing capability and pointing at a backend that has domain egress
(`openshell`, `e2b`, `daytona`). This is why its capabilities row
reads `domain-egress: no`, unlike every other cloud backend, and why
the demo env declares no network grants (see `demo/VERCEL-SANDBOX.md`
beat 3).

## The determinism tradeoff

A bootstrap-shaped provider forces a choice the image backends never
face. flox chose **FloxHub-remote activation**: the bootstrap pulls
the environment from FloxHub inside the sandbox. That activates the
FloxHub-pushed *revision*, not a byte-for-byte closure captured at
`flox activate --sandbox` time — a bounded, documented reproducibility
gap (the FloxHub revision is itself a locked environment). The
fully-deterministic alternative (pushing the content-addressed store
closure to an artifact store the sandbox pulls from) needs a shared
"bootstrap bundle" stage the prototype does not have yet. See
`demo/VERCEL-SANDBOX.md` beat 4.

Still required on the host: the prototype `flox` binary (export
`FLOX_BIN` from the dev shell before activating), the shell RC prompt
hook, and `demo/setup.sh` to create the `~/sandbox-demo` project env.

## Verified

Locked against the catalog 2026-07-18. The interactive layered flow
and the remote sandbox launch are unrehearsed — this host has no
Vercel account, token, or `vercel` CLI, and no FloxHub push of the
demo env; validation stopped at the auth wall. The local slice was
exercised end to end against the live `@vercel/sandbox` 2.7.1 /
`vercel` 56.3.1 surface: preflight (CLI-missing bail, too-old-version
bail, unauthenticated bail, authenticated-via-env-token pass),
bootstrap + launcher generation (the launcher is valid JavaScript —
`node --check` passes — with the bootstrap embedded as a
single-line JSON literal; the bootstrap is valid bash — `bash -n`
passes), the network-grant decline, and the launch-boundary error
naming the account and FloxHub-push prerequisites.
`flox activate -d demo/vercel-sandbox-setup -- true` succeeds.

## Republishing after edits

This directory is the versioned source of truth. After editing the
manifest here, re-lock with the local flox:

```bash
flox edit -f .flox/env/manifest.toml   # re-locks against the catalog
```

To publish to FloxHub, follow the same pull/copy/edit/push dance as
`modal-setup/README.md`: never `flox push` from inside this committed
directory — copy it out first, then push the copy.
