# Flox Extensions — Author Guide

This guide is for people who want to ship an extension that
other Flox users can install with `flox extension install
<owner>/<repo>`. It covers repo layout, the
`flox-extension.toml` manifest, release-asset naming, activation
semantics, and the local dev loop.

## Contents

- [Repo naming and discovery](#repo-naming-and-discovery)
- [Three kinds of extensions](#three-kinds-of-extensions)
- [The `flox-extension.toml` manifest](#the-flox-extensiontoml-manifest)
- [Release-asset naming](#release-asset-naming)
- [Environment stanza](#environment-stanza)
- [Local dev loop](#local-dev-loop)
- [Example extensions](#example-extensions)

## Repo naming and discovery

Extensions are GitHub repositories whose name begins with
`flox-`. A repo named `acme/flox-tidy` installs as the `tidy`
extension and is invoked as `flox tidy`.

Tag your repo with the `flox-extension` topic on GitHub so it
shows up in search:

```
https://github.com/topics/flox-extension
```

The `flox extension search` command queries this topic to surface
extensions.

## Three kinds of extensions

| Kind | What it is | When to pick |
|------|------------|--------------|
| **Script** | A shell, Python, or interpreted script committed to the repo. Flox `git clone`s and runs the executable directly. | Most extensions. Easy to ship, easy for users to read, no release engineering required. |
| **Binary** | A compiled binary shipped as a GitHub release asset. Flox downloads and extracts. | Tools written in Rust, Go, C/C++ where clone-and-run isn't viable. |
| **Local** | A directory installed via `--from-path`. Not distributed through GitHub. | Local development and iteration. See [local dev loop](#local-dev-loop). |

Script is the default: if your repo has no GitHub releases with
matching assets, Flox falls back to clone-install.

Binary is selected automatically when the latest GitHub release
has an asset Flox can match — you don't have to declare a kind
explicitly. See [release-asset naming](#release-asset-naming).

## The `flox-extension.toml` manifest

A `flox-extension.toml` at the repo root is optional for
script-kind extensions and required for binary-kind extensions
(to map assets to host platforms).

Minimal manifest:

```toml
schema = "1"

[extension]
name = "hello"
```

Full manifest with every field:

```toml
schema = "1"

[extension]
name = "deploy"
description = "Deploys things"

[extension.binary]
source = "github-release"
asset = "flox-deploy-{os}-{arch}.tar.gz"
sha256 = "cafe..."

[environment]
mode = "pinned"
inherit_name = "acme/dev"

[on_active]
inside = "error"
```

### `[extension]` table

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Extension name. Must match the repo `<name>` segment (repo `flox-<name>`). Lowercased, `[a-z0-9][a-z0-9_-]*`. |
| `description` | string | no | Short human description. Shown in `flox extension list` / `search` output. |

### `[extension.binary]` table

Present only for binary-kind extensions.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `source` | string | yes | Currently only `"github-release"` is supported. |
| `asset` | string | yes | Asset-name template. Placeholders `{name}`, `{os}`, `{arch}`, `{ext}` are expanded at resolution time. `{ext}` defaults to `tar.gz` (or `zip` on Windows). |
| `sha256` | string | no | Expected digest; reserved for future verification. |

### `[environment]` table

Controls how the extension interacts with Flox activations. See
[Environment stanza](#environment-stanza) below for the full
semantics.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `mode` | string | yes | One of `"inherit"`, `"none"`, `"pinned"`. |
| `inherit_name` | string | only when `mode = "pinned"` | Activation ref, e.g. `"acme/dev"`. |
| `inherit` | enum | no | Reserved for future use (`"current"`, `"default"`, `"named"`). Not consumed by Flox today. |

### `[on_active]` table

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `inside` | string | yes | One of `"override"` (default) or `"error"`. Controls behavior when the caller is already activated in a *different* environment than `inherit_name`. |

### `schema` field

Always `"1"` for the current schema version.

## Release-asset naming

For binary-kind extensions, Flox picks a release asset using this
priority order:

1. **Manifest template match.** If `flox-extension.toml` has
   `[extension.binary].asset`, Flox renders it for the host
   platform and looks for an exact asset-name match. Supported
   placeholders:
   - `{name}` — extension name (from `[extension].name`)
   - `{os}` — Rust's `std::env::consts::OS` for the host:
     `"macos"`, `"linux"`, or `"windows"`. Note this is
     **`macos`, not `darwin`** — the value is substituted
     verbatim, so a template rendering `…-darwin-…` will not
     match on a Mac.
   - `{arch}` — Rust's `std::env::consts::ARCH`:
     `"x86_64"` or `"aarch64"`
   - `{ext}` — `"tar.gz"` (default) or `"zip"` on Windows

   Only `{ext}` is computed; the rest are substituted verbatim
   from the host's values. If the rendered name does not match a
   release asset exactly, resolution falls through to the
   substring match below, which accepts both `darwin` and
   `macos` — so a `darwin`-named asset can still install, just
   not via this step.

2. **Substring match.** If the template isn't set or doesn't
   match, Flox falls back to a substring match against a list of
   platform tokens derived from the host. The tokens Flox looks
   for, in order:
   - On Linux x86_64: `linux-x86_64`, `linux-amd64`
   - On Linux aarch64: `linux-aarch64`, `linux-arm64`
   - On macOS x86_64: `darwin-x86_64`, `darwin-amd64`,
     `macos-x86_64`, `macos-amd64`
   - On macOS aarch64: `darwin-aarch64`, `darwin-arm64`,
     `macos-aarch64`, `macos-arm64`

3. **Rosetta fallback.** On Apple Silicon (`darwin-aarch64`), if
   no arm64 asset matches, Flox retries with the `x86_64`
   matchers and uses that asset under Rosetta. A
   `tracing::info!` line is logged when this happens.

If no asset matches any of the above, `flox extension install`
fails with a clear error naming the platform it tried to match.

### Recommended naming

The simplest path is to name assets along the substring
conventions. `flox-<name>-<os>-<arch>.tar.gz` works out of the
box on all platforms and needs no manifest template. For
example:

```
flox-deploy-linux-x86_64.tar.gz
flox-deploy-linux-aarch64.tar.gz
flox-deploy-darwin-x86_64.tar.gz
flox-deploy-darwin-aarch64.tar.gz
```

If your existing release pipeline uses a different naming
scheme, set `[extension.binary].asset` in the manifest to map it.

## Environment stanza

### `inherit` (default)

```toml
[environment]
mode = "inherit"
```

The extension runs inside the caller's current activation. All
`FLOX_*` environment variables are preserved. If the caller is
already inside `flox activate`, the extension sees that
activation.

### `none`

```toml
[environment]
mode = "none"
```

Flox scrubs `FLOX_*` and `_FLOX_*` variables before exec, then
overlays bookkeeping vars (`FLOX_EXTENSION_NAME`,
`FLOX_EXTENSION_VERSION`, `FLOX_EXTENSION_PATH`, `FLOX_BIN`).
Use this when the extension must not be influenced by
caller-side Flox state.

### `pinned`

```toml
[environment]
mode = "pinned"
inherit_name = "acme/dev"
```

The extension runs inside a specific activation. If the caller
is already activated in the pinned environment, Flox execs the
extension directly. Otherwise, Flox wraps the call with
`flox activate -r <owner>/<env> -- <extension>`.

Add `[on_active] inside = "error"` to make Flox refuse to run
the extension when the caller is activated in a *different*
environment:

```toml
[environment]
mode = "pinned"
inherit_name = "acme/dev"

[on_active]
inside = "error"
```

The default (`inside = "override"`) always wraps with
`flox activate -r` when there's a mismatch.

### Bookkeeping variables

Regardless of mode, Flox injects four variables into the
extension's environment:

| Variable | Value |
|----------|-------|
| `FLOX_EXTENSION_NAME` | The extension's declared name. |
| `FLOX_EXTENSION_VERSION` | Tag if pinned, else short SHA, else `-`. |
| `FLOX_EXTENSION_PATH` | Absolute path to the install directory. |
| `FLOX_BIN` | Path to the flox binary that dispatched the extension. |

Extensions can read these to find their own install directory
or to shell out to other `flox` subcommands.

## Local dev loop

During development, install your extension directly from a
local directory:

```console
$ cd ~/src/flox-hello
$ flox extension install --from-path .
✓ Installed flox-hello
$ flox hello
```

`--from-path` skips GitHub entirely. The installer derives the
name from the directory basename (stripping any leading
`flox-`), or reads it from `flox-extension.toml` if present.

To iterate, edit the source and re-install with `--force`:

```console
$ flox extension install --from-path . --force
```

The `--force` overrides the already-installed check. Alternately,
for pure script extensions, you can `flox extension remove` and
reinstall.

## Example extensions

- [**flox-hello-script**](https://github.com/flox/flox-hello-script)
  — the canonical "hello world" extension. Shell script,
  minimal `flox-extension.toml`, demonstrates the clone-and-run
  flow and the default `Inherit` activation mode. Install with
  `flox extension install flox/flox-hello-script`.

- [**flox-hello-local**](https://github.com/flox/flox-hello-local)
  — canonical local-authoring reference. Clone the repo and
  install from the working tree via `flox extension install
  --from-path .`; demonstrates the [local dev loop](#local-dev-loop)
  without going through GitHub at install time.

- [**flox-hello-binary**](https://github.com/flox/flox-hello-binary)
  — minimal binary-kind extension. Demonstrates the release-asset
  pipeline and the `[extension.binary]` manifest stanza. v0.1.0
  ships macOS assets only; Linux is tracked in the example repo's
  `PROJECTS.md` as P02.

No published example demonstrates `[environment] mode = "pinned"`
or the `[on_active] inside = "error"` variant yet. For working
usage of both, see the activation-mode cases in
`cli/tests/extension.bats`.

## See also

- [User guide](./user-guide.md)
- [Docs index](./README.md)
