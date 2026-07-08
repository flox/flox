---
title: FLOX-SANDBOX
section: 1
header: "Flox User Manuals"
...

# NAME

flox-sandbox - sandboxed activations: modes, backends, and policy management

# SYNOPSIS

```text
flox activate --sandbox[=(off|warn|enforce|prompt)]
     [--sandbox-backend=<backend>]
     [-- <command>]

flox [<general-options>] sandbox
     [-d=<path> | -r=<owner/name>]

flox [<general-options>] sandbox list
     [-d=<path> | -r=<owner/name>]
     [--all]

flox [<general-options>] sandbox allow
     [-d=<path> | -r=<owner/name>]
     <GLOB>

flox [<general-options>] sandbox revoke
     [-d=<path> | -r=<owner/name>]
     <GLOB>

flox [<general-options>] sandbox audit
     [-d=<path> | -r=<owner/name>]
     [--clear]

flox [<general-options>] sandbox backends
```

# DESCRIPTION

A sandboxed activation wraps everything that runs inside a
`flox activate` session — shells, tools, services, coding agents — in
an isolation boundary. The primary use case is running autonomous
coding agents safely: an agent inside the sandbox can use the tools
installed in the environment and work on the project, but cannot read
the rest of your filesystem or reach your credentials.

Sandboxing is requested per environment in the manifest, or per
invocation on the command line:

```toml
[options]
sandbox = "enforce"
sandbox-backend = "oci"
```

```console
$ flox activate --sandbox enforce --sandbox-backend oci -- <command>
```

The command-line flags take precedence over the manifest; the
`FLOX_SANDBOX_BACKEND` environment variable sits between them (flag,
then environment variable, then manifest, then the default
`libsandbox`).

When a manifest declares a sandbox, interactive auto-activation shows
a consent prompt before entering a sandboxed session
(`Enter '<path>' (sandboxed via oci)? [Y/n]`), so an in-progress
process cannot silently enter one on your behalf. The explicit
command form does not prompt; use it in scripts and CI.

This is an experimental prototype gated behind the `sandbox_activate`
feature flag; set `FLOX_FEATURES_SANDBOX_ACTIVATE=true` to use it.

# MODES

`off`
:   No sandbox. The default.

`warn`
:   Advisory: out-of-policy access is reported but allowed. Use it to
    learn what a workload touches before locking it down.
    Supported by the `libsandbox` backend only.

`enforce`
:   Out-of-policy access is denied. Supported by every backend; the
    only active mode on enforcing backends (`host-native`, `srt`,
    `oci`).

`prompt`
:   Out-of-policy access is denied and queued for approval from
    outside the session (see SUBCOMMANDS). A bare `--sandbox` is
    shorthand for `--sandbox prompt`.
    Supported by the `libsandbox` backend only.

`warn` and `prompt` are advisory semantics — observe-but-allow, and
deny-then-redeem — that only the loader-based `libsandbox` backend
can provide. Asking an enforcing backend for them fails with a clear
error rather than silently enforcing.

# BACKENDS

Run `flox sandbox backends` for the full capability listing
(boundary class, platform support, enforcement, live-ask, status).
In summary:

`libsandbox`
:   Advisory loader-based mediation; the default. The only backend
    with `warn` and `prompt` modes. Implemented.

`host-native`
:   Host-kernel enforcement (macOS `sandbox-exec`). Enforce-only.
    Implemented (macOS).

`srt`
:   Host-kernel enforcement via Anthropic's sandbox-runtime, with
    default-deny network egress. Enforce-only. Implemented.

`oci`
:   Container isolation (Linux micro-VM on macOS via Apple
    Container; podman on Linux). Enforce-only. Implemented —
    ships first.

`nix`, `libkrun`
:   Scaffolded and planned respectively. The `--sandbox-backend`
    flag accepts them, but selecting either fails at activation
    with a "not yet wired" error.

## oci — container isolation (ships first)

The `oci` backend runs the activation inside a Linux container:
Apple Container on macOS (macOS 26+, Apple silicon), where the
container is a lightweight micro-VM, or podman on Linux. This is the
strongest implemented boundary — the host filesystem is not filtered,
it is simply absent.

**Baking.** The environment's closure is cross-compiled for Linux and
baked into an OCI image, tagged `<env>:<hash12>` where the hash is
derived from the lockfile, plus a `<env>:latest` alias. The first
bake on a machine downloads the builder image and compiles into a
persistent cache volume (`flox-nix`) — expect a few minutes; later
bakes reuse the cache and image layers. When the environment changes,
the tag no longer matches and flox prompts to re-bake rather than
silently running a stale toolchain. After a successful bake, all
superseded tags for the environment are pruned; the store holds the
current image and the `latest` alias only.

**What crosses the boundary — and what does not:**

* The project directory is bind-mounted read-write at its real path.
  Reads and writes round-trip live; everything else written in the
  guest dies with the container.
* No other host path is mounted. `$HOME`, `~/.ssh`, `~/.aws`,
  `~/.config/gh`, `~/.netrc` do not exist inside the guest.
* No host environment variables are forwarded. Tokens exported in
  your shell (`GITHUB_TOKEN`, cloud credentials, `SSH_AUTH_SOCK`) are
  not visible to the guest. A process in the sandbox acts
  anonymously unless you deliberately provide a secret via the
  manifest (`[vars]`), the project directory, or an in-session login.
* Nothing credential-bearing is baked into the image: it carries the
  environment closure and activation context only, and the
  prototype-only `options.sandbox*` keys are stripped from the view
  the builder sees.

**Network is not restricted.** The guest has the container runtime's
default outbound network access. The isolation story for this
backend is filesystem and credentials; per-environment network
policies are planned.

**The image is self-sufficient.** The image always carries a shell
(`bash`) and `coreutils` independent of the manifest, and the
container entrypoint is flox's own activation binary from the
environment closure — an environment with no packages installed still
activates. Install the tools your workload runs; the sandbox itself
needs nothing from you.

Inside the guest, `flox` is a minimal shim: `flox deactivate` ends
the session; other subcommands print a notice and return 127.

**Operational valves** (environment variables, host side):

`FLOX_SANDBOX_OCI_AUTOBAKE=true`
:   Bake without prompting when the image is missing or stale. For
    CI and other non-interactive contexts.

`FLOX_SANDBOX_OCI_ALLOW_STALE=1`
:   Run the newest existing image even if stale (offline use). A
    warning names the expected tag.

`FLOX_SANDBOX_OCI_IMAGE=<ref>`
:   Run exactly this image ref, bypassing staleness logic entirely.

**Caveats.** The guest is Linux: on macOS you run the Linux builds
of your packages. Warm start costs ~1s (VM boot). Bind-mount I/O
over virtio-fs is fine for streaming but slow for many-small-file
traversals. The backend is enforce-only.

## libsandbox — advisory mediation (default backend)

A loader-based interposer that mediates file and outbound-network
access in-process, natively on macOS and Linux — no VM, no image,
instant start. It is advisory: only cooperative, dynamically linked
programs are mediated, and on macOS SIP-protected system binaries
escape it. In exchange it is the only backend with `warn` and
`prompt` modes and live approval.

The default policy is tuned so an agent works out of the box: the
project directory, `/nix/store` reads, common non-sensitive dev
configs, and the common package registries and git hosts are allowed;
credential and secret locations are denied even under `enforce` (see
SENSITIVE PATHS). See `flox-activate(1)` for the full policy
description; policy inspection and approval is documented under
SUBCOMMANDS below.

## host-native — the OS kernel sandbox

Kernel-enforced (macOS `sandbox-exec`; the Linux implementation is
not yet wired). Deny-by-default for your home directory: all of
`$HOME` except the project and Flox's own state is unreadable and
unwritable, while system and Nix-store reads stay open so flox runs.
Enforce-only.

## srt — Anthropic's sandbox-runtime

Kernel-enforced via the `sandbox-runtime` package
(`flox install sandbox-runtime`). Similar filesystem profile to
host-native, plus default-deny outbound network egress (a bundled
proxy with an empty allowlist) — the one host-kernel backend that
restricts the network. Enforce-only. Known rough edges: blanket
write access to `/tmp`; a dev `flox` binary under `$HOME` cannot be
re-exec'd under the deny-`$HOME` profile.

# POLICY MANAGEMENT (libsandbox)

The `flox sandbox` subcommands are the human-facing front end to the
sandbox broker that runs inside a `libsandbox` activation started
with `--sandbox prompt`. Under `prompt`, an out-of-policy file access
is denied and queued rather than silently allowed or blocked;
`flox sandbox` reviews that queue, approves or denies requests, and
inspects the saved grant set.

## Live approval

A grant approved while a `prompt` session is active takes effect
immediately in that session. The denied operation can be retried and
will succeed once the short negative cache expires — no re-activation
is needed. This is the core loop: a tool's read is denied and queued
in one terminal; the grant is approved in another with
`flox sandbox`; the tool retries and succeeds.

## Persistence

An "allow always" grant is written to `grants.toml` under the
environment's `.flox/cache/sandbox/` directory. At the next
activation those grants are folded into the sandbox allow-set, so a
path approved once is not asked about again. Grants are op-recorded
but op-blind in enforcement: a saved grant allows all access kinds on
its paths in later sessions.

## Default policy as grants

The first sandboxed activation of an environment seeds the default
policy into `grants.toml` as explicit `default-seed` grants: git
hosting and release hosts, the npm/PyPI/crates.io registries, shell
dotfile and dev-config reads, and Flox's own metrics endpoint. There
is no invisible policy: every default allowance is inspectable with
`flox sandbox list --all` and revocable with `flox sandbox revoke`. A
revoked default stays revoked — re-seeding is gated on a version
marker in the file, never on entry presence. Only loopback and Flox's
own service hosts (FloxHub, the Flox Catalog) remain hardcoded, since
revoking them would break flox itself; the sensitive set likewise
remains a hardcoded denylist and is never grantable by seeding.

## Audit log

The sandbox engine appends every report it emits — warn-mode reports
and enforce/prompt denials, for file accesses, directory listings,
and network connects — to `audit.ndjson` beside `grants.toml`.
`flox sandbox audit` reads it directly, so denials are queryable
after the session ends and in every mode (warn and enforce run no
broker). Allowed accesses are never recorded, and records are
deduplicated to one per path per process.

## Self-approval guard

The approval verbs (`allow`, `revoke`) are refused when run from
inside the sandboxed session they would modify. The check is enforced
twice: once by this command (via the session env marker) and again by
the broker (via the connecting process's credentials, which an
environment-variable change cannot evade). A coding agent running
inside the sandbox therefore cannot approve its own pending requests.

This is friction plus audit, not containment. Every grant is
journaled, and `flox sandbox list` (and the activation banner) flag
any grant present in `grants.toml` but missing from the journal as
possibly self-approved.

# SUBCOMMANDS

`flox sandbox`
:   With no subcommand, print a status summary and, on a terminal with
    pending requests, an interactive review. Each request offers:
    allow this file for the session, allow this file always, allow
    everything in the parent directory always (only when the path is
    not sensitive), deny for the session, or decide later. Pressing
    Esc keeps a request queued.

`flox sandbox list [--all]`
:   List saved grants (pattern, ops, source, date, evidence), the
    current session grants, the sensitive set that is never
    auto-granted, and the allow-set cap consumption. Default-seed
    grants collapse into one summary row; pass `--all` to list them
    individually. Network grants do not count against the filesystem
    allow-set caps.

`flox sandbox allow <GLOB>`
:   Allow a path glob without prompting and save it to `grants.toml`.
    When a session is active the grant applies immediately and clears
    any matching pending requests; otherwise the grant is written for
    the next activation.

`flox sandbox revoke <GLOB>`
:   Remove a saved or session grant by its exact pattern. Revoking a
    network grant takes effect at the next activation: the network
    policy is compiled at session start and is not re-read live.

`flox sandbox audit [--clear]`
:   Show the recorded sandbox denials and warnings for the
    environment, aggregated by path, operation, and mode, with a
    count, the last-seen time, and the verdict. Works without an
    active session. `--clear` truncates the audit log only — it never
    touches grants.

`flox sandbox backends`
:   List the available backends with their boundary class,
    platform support, and implementation status.

# SENSITIVE PATHS

Credential and secret locations are never auto-granted and are never
folded into a directory grant (libsandbox backend). The default set
covers SSH keys, cloud and Kubernetes credentials, GPG, `.netrc`, the
GitHub CLI config, any `.env` file, and the sandbox grants directory
itself. Override it with `FLOX_SANDBOX_SENSITIVE` (a space-separated
list of globs).

Note that under the `oci` backend these paths are not filtered — they
are absent: no host path outside the project directory exists inside
the guest at all. A secret *inside* the project directory is visible
to the sandbox under every backend; that is the deliberate channel
for handing an agent a credential.

# ENVIRONMENT VARIABLES

`FLOX_FEATURES_SANDBOX_ACTIVATE`
:   Set to `true` to enable the sandbox prototype.

`FLOX_SANDBOX_BACKEND`
:   Select the backend when no `--sandbox-backend` flag is given;
    overrides the manifest.

`FLOX_SANDBOX_SENSITIVE`
:   Override the sensitive-path set (libsandbox).

`FLOX_SANDBOX_OCI_AUTOBAKE`, `FLOX_SANDBOX_OCI_ALLOW_STALE`,
`FLOX_SANDBOX_OCI_IMAGE`
:   OCI valves; see the `oci` backend section above.

# OPTIONS

```{.include}
./include/environment-options.md
./include/general-options.md
```

# EXAMPLES

Run one command in the OCI sandbox (no consent prompt):

```console
$ flox activate --sandbox enforce --sandbox-backend oci -- uname -sm
Linux aarch64
```

Bake non-interactively in CI:

```console
$ FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate -- make test
```

Review what the sandbox denied after a libsandbox session:
```console
$ flox sandbox audit
Sandbox audit for environment 'myproject'
  PATH                                     OP       MODE     COUNT  LAST SEEN         VERDICT
  /Users/dev/demo-secrets/.env             read     enforce  2      2026-06-11 17:02  denied
  example.com:443                          connect  enforce  1      2026-06-11 17:03  denied
```

Approve a queued read from a second terminal, then retry in the first:
```console
$ flox sandbox allow '~/.config/gh/hosts.yml'
✅ Saved grant '~/.config/gh/hosts.yml' (cleared 1 pending) — future sessions won't ask.
```

Allow a whole directory subtree at once:
```console
$ flox sandbox allow '~/.cargo/registry/**'
✅ Saved grant '~/.cargo/registry/**' — future sessions won't ask.
```

Inspect saved grants and cap consumption:
```console
$ flox sandbox list
Saved grants for environment 'myproject'
  ...
```

Remove a grant:
```console
$ flox sandbox revoke '~/.cargo/registry/**'
✅ Removed '~/.cargo/registry/**' from grants.toml.
```

# SEE ALSO

[`flox-activate(1)`](./flox-activate.md),
[`manifest.toml(5)`](./manifest.toml.md)
